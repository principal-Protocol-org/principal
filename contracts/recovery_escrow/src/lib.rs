//! RecoveryEscrow — compliance recovery for SY, PT, YT and LP positions, per underlying asset.
//!
//! # Design
//! The native clawback of a Stellar Asset only applies to balances of the underlying asset itself.
//! It cannot reach SY, PT, YT or LP positions, since those are separate Soroban positions created by
//! Principal Protocol. This contract lets the underlying's real issuer authority (read live, never
//! cached or configured separately) forcibly move a restricted holder's positions here, then unwinds
//! them back toward the underlying asset so the issuer can execute their existing native clawback.
//!
//! This is the *only* place that authenticates the issuer and verifies a target is actually
//! deauthorized (`!authorized(account)`) before seizing anything. `SYWrapper`, `PTToken`, `YTToken`
//! and `MarketPool` each expose their own seize function, but none of them re-derive that authority:
//! they simply trust calls from the one `RecoveryEscrow` address configured on each (via their own
//! one-time `set_recovery_escrow`). Keeping the verification in one shared place means a bug cannot
//! be worked around by attacking a token contract directly.
//!
//! # No separate owner
//! This contract has no admin key of its own. Every entrypoint re-checks the issuer authority live,
//! through `principal_compliance` (`SAC.admin()` for a classic/SEP-8 asset; the operator capability
//! probe for a SEP-57 RWA token). A key rotation on the issuer's side is authoritative here
//! immediately, with nothing to update.
//!
//! # What each position becomes
//! | Position | On seizure                                                                              |
//! |----------|-----------------------------------------------------------------------------------------|
//! | SY       | seized, then unwrapped at once into the underlying (SY has no maturity)                 |
//! | LP       | seized, burned in the pool for its PT + SY; the SY leg is unwrapped at once, the PT leg is held like any other PT |
//! | PT, YT   | held fully backed in escrow until maturity, then settled into the underlying by `finalize_record` |
//!
//! Only the seized account's own positions move; no other depositor is touched.
//!
//! # Traceability: recovery records
//! Every seizure event writes a [`RecoveryRecord`] with a fresh sequential id: the account, the
//! ledger and time, and exactly what was seized and what it yielded. Ids are listed per account
//! (`account_records`), and `finalize_record` settles the PT/YT a record still holds and writes the
//! resulting underlying back onto that same record. Recovered funds therefore always trace to the
//! specific event and account that produced them.
//!
//! # Batching
//! `seize_batch` handles up to `MAX_BATCH` accounts in one transaction, authenticating the issuer
//! once and still verifying each target's deauthorization individually. `seize_all_positions`
//! builds the batch from each account's full balances. Both write one record per account.

#![no_std]
#![allow(deprecated)] // `env.events().publish` / `register_contract`: migration to `#[contractevent]` is tracked separately; event topics are kept stable for indexers.

use soroban_sdk::{
    contract, contractclient, contracterror, contractimpl, contracttype, panic_with_error,
    symbol_short, Address, Env, Vec,
};

use principal_compliance as compliance;

/// Upper bound on accounts per batch transaction.
///
/// Sized by the *ledger-entry footprint*, not CPU: seizing one account's SY + PT + YT + LP touches
/// ~10 write and ~14 read entries, and a Soroban transaction may touch at most 100 entries in total
/// (50 writes). Measured with every position type held: 3 accounts use ~82 entries and fit; 4 use
/// 97-103 and are over the limit in practice (`footprint ledger entries: 103 > 100`); 5 fail
/// outright. SY-only batches are far lighter, but one bound must be safe for the heaviest case.
pub const MAX_BATCH: u32 = 3;
const RECORD_TTL_LEDGERS: u32 = 518_400;

#[contractclient(name = "SYWrapperClient")]
pub trait SYWrapperInterface {
    fn underlying_address(env: Env) -> Address;
    fn seize(env: Env, caller: Address, account: Address, shares: i128) -> i128;
    fn withdraw(
        env: Env,
        from: Address,
        shares: i128,
        to: Address,
        min_underlying_out: i128,
    ) -> i128;
    fn balance_of(env: Env, account: Address) -> i128;
}

#[contractclient(name = "PTTokenClient")]
pub trait PTTokenInterface {
    fn underlying_address(env: Env) -> Address;
    fn seize(env: Env, caller: Address, account: Address, amount: i128) -> i128;
    fn balance(env: Env, account: Address) -> i128;
}

#[contractclient(name = "YTTokenClient")]
pub trait YTTokenInterface {
    fn underlying_address(env: Env) -> Address;
    fn seize(env: Env, caller: Address, account: Address, amount: i128) -> i128;
    fn balance(env: Env, account: Address) -> i128;
    fn pending_claim(env: Env, account: Address) -> i128;
}

#[contractclient(name = "PoolClient")]
pub trait PoolInterface {
    fn underlying_address(env: Env) -> Address;
    fn seize_lp(env: Env, caller: Address, account: Address, amount: i128) -> i128;
    fn redeem_seized_lp(env: Env, caller: Address, lp: i128) -> (i128, i128);
    fn lp_balance(env: Env, account: Address) -> i128;
}

#[contracttype]
#[derive(Clone)]
pub struct RedeemResult {
    pub underlying_from_pt: i128,
    pub underlying_from_yt: i128,
}

#[contractclient(name = "PrincipalManagerClient")]
pub trait PrincipalManagerInterface {
    fn underlying_address(env: Env) -> Address;
    fn redeem(env: Env, from: Address, pt_amount: i128, yt_amount: i128) -> RedeemResult;
}

/// Instance-storage TTL applied by `bump` (~30 days at 5 s per ledger).
const INSTANCE_TTL_LEDGERS: u32 = 518_400;

#[contracterror]
#[derive(Copy, Clone, Debug, PartialEq)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    Unauthorized = 3,
    TargetStillAuthorized = 4,
    ZeroAmount = 5,
    PositionUnderlyingMismatch = 6,
    BatchTooLarge = 7,
    RecordNotFound = 8,
    AlreadyFinalized = 9,
    NothingToSeize = 10,
    NothingToFinalize = 11,
}

#[contracttype]
pub enum DataKey {
    Underlying,
    SYWrapper,
    PTToken,
    YTToken,
    PrincipalManager,
    Pool,
    RecordCount,
    Record(u64),
    AccountRecords(Address),
}

/// One account's positions to seize in a batch. A zero field means "leave that position alone".
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct SeizeRequest {
    pub account: Address,
    pub sy_shares: i128,
    pub pt_amount: i128,
    pub yt_amount: i128,
    pub lp_amount: i128,
}

/// The permanent trace of one recovery event for one account.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct RecoveryRecord {
    pub id: u64,
    pub account: Address,
    pub ledger: u32,
    pub timestamp: u64,
    /// SY shares seized directly, and the underlying they unwrapped to.
    pub sy_shares: i128,
    pub underlying_from_sy: i128,
    /// LP seized, and what the pool paid out for it: PT (held), SY shares and their underlying.
    pub lp_amount: i128,
    pub lp_pt: i128,
    pub lp_sy_shares: i128,
    pub underlying_from_lp: i128,
    /// PT seized directly / YT seized, held in escrow until `finalize_record`.
    pub pt_amount: i128,
    pub yt_amount: i128,
    /// Yield the seized YT had already accrued (underlying units, before the YT fee). It moves to
    /// the escrow's pending claim with the position, so it is recovered rather than stranded on a
    /// deauthorized account. Exact per record. Yield accruing *after* the seizure is pooled across
    /// every YT position the escrow holds, and is paid out by whichever `finalize_record` runs
    /// first -- see `underlying_from_yt`.
    pub yt_yield_at_seize: i128,
    /// Set by `finalize_record`: the underlying the held PT and YT settled into.
    pub finalized: bool,
    pub underlying_from_pt: i128,
    pub underlying_from_yt: i128,
}

#[contract]
pub struct RecoveryEscrowContract;

#[contractimpl]
impl RecoveryEscrowContract {
    /// Permissionless keeper call: extend this contract's *instance* storage (admin, config,
    /// reserves, totals, settlement rate, fee buckets) for ~30 days. Soroban does not bump instance
    /// TTL on ordinary reads or writes, so a long-dated market needs this called periodically (or
    /// an archived instance restored) -- see docs/DEPLOYMENT.md.
    pub fn bump(env: Env) {
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_TTL_LEDGERS, INSTANCE_TTL_LEDGERS);
    }

    /// One-time wiring. Every position contract (and the manager and pool) must report the same
    /// underlying as `underlying`.
    pub fn initialize(
        env: Env,
        underlying: Address,
        sy_wrapper: Address,
        pt_token: Address,
        yt_token: Address,
        principal_manager: Address,
        market_pool: Address,
    ) {
        if env.storage().instance().has(&DataKey::Underlying) {
            panic_with_error!(&env, Error::AlreadyInitialized);
        }
        let same = |other: Address| other == underlying;
        if !same(SYWrapperClient::new(&env, &sy_wrapper).underlying_address())
            || !same(PTTokenClient::new(&env, &pt_token).underlying_address())
            || !same(YTTokenClient::new(&env, &yt_token).underlying_address())
            || !same(PrincipalManagerClient::new(&env, &principal_manager).underlying_address())
            || !same(PoolClient::new(&env, &market_pool).underlying_address())
        {
            panic_with_error!(&env, Error::PositionUnderlyingMismatch);
        }
        compliance::init(&env, &underlying);
        let s = env.storage().instance();
        s.set(&DataKey::Underlying, &underlying);
        s.set(&DataKey::SYWrapper, &sy_wrapper);
        s.set(&DataKey::PTToken, &pt_token);
        s.set(&DataKey::YTToken, &yt_token);
        s.set(&DataKey::PrincipalManager, &principal_manager);
        s.set(&DataKey::Pool, &market_pool);
        s.set(&DataKey::RecordCount, &0_u64);
    }

    // ---------------------------------------------------------------- seizure

    /// Seize `shares` of `account`'s SY and unwrap it into raw underlying held here, ready for the
    /// issuer's native clawback. `caller` must be the issuer authority; `account` must already be
    /// deauthorized. Writes a recovery record; returns the underlying recovered.
    pub fn seize_sy(env: Env, caller: Address, account: Address, shares: i128) -> i128 {
        Self::assert_issuer_admin(&env, &caller);
        let rec = Self::seize_one(&env, &caller, &account, shares, 0, 0, 0);
        rec.underlying_from_sy
    }

    /// Seize `amount` of `account`'s PT, held here until `finalize_record`. Writes a record.
    pub fn seize_pt(env: Env, caller: Address, account: Address, amount: i128) -> i128 {
        Self::assert_issuer_admin(&env, &caller);
        let rec = Self::seize_one(&env, &caller, &account, 0, amount, 0, 0);
        rec.pt_amount
    }

    /// Seize `amount` of `account`'s YT, held here until `finalize_record`. Writes a record.
    pub fn seize_yt(env: Env, caller: Address, account: Address, amount: i128) -> i128 {
        Self::assert_issuer_admin(&env, &caller);
        let rec = Self::seize_one(&env, &caller, &account, 0, 0, amount, 0);
        rec.yt_amount
    }

    /// Seize `amount` of `account`'s LP: burn it in the pool for PT + SY, unwrap the SY leg at once
    /// and hold the PT leg. Writes a record; returns `(pt_received, underlying_from_sy_leg)`.
    pub fn seize_lp(env: Env, caller: Address, account: Address, amount: i128) -> (i128, i128) {
        Self::assert_issuer_admin(&env, &caller);
        let rec = Self::seize_one(&env, &caller, &account, 0, 0, 0, amount);
        (rec.lp_pt, rec.underlying_from_lp)
    }

    /// Seize several accounts' positions in one transaction. The issuer is authenticated once;
    /// every account is still individually checked for deauthorization, and one record is written
    /// per account. Reverts as a whole if any account is still authorized. Returns the record ids.
    pub fn seize_batch(env: Env, caller: Address, requests: Vec<SeizeRequest>) -> Vec<u64> {
        Self::assert_issuer_admin(&env, &caller);
        Self::run_batch(&env, &caller, &requests)
    }

    /// Seize *everything* the listed accounts hold across SY, PT, YT and LP, in one transaction.
    /// Accounts with no positions are skipped; reverts `NothingToSeize` if none had any.
    pub fn seize_all_positions(env: Env, caller: Address, accounts: Vec<Address>) -> Vec<u64> {
        Self::assert_issuer_admin(&env, &caller);
        // Bound the work before reading a single balance.
        if accounts.len() > MAX_BATCH {
            panic_with_error!(&env, Error::BatchTooLarge);
        }
        let sy = SYWrapperClient::new(&env, &Self::get(&env, &DataKey::SYWrapper));
        let pt = PTTokenClient::new(&env, &Self::get(&env, &DataKey::PTToken));
        let yt = YTTokenClient::new(&env, &Self::get(&env, &DataKey::YTToken));
        let pool = PoolClient::new(&env, &Self::get(&env, &DataKey::Pool));

        let mut requests: Vec<SeizeRequest> = Vec::new(&env);
        for account in accounts.iter() {
            let req = SeizeRequest {
                sy_shares: sy.balance_of(&account),
                pt_amount: pt.balance(&account),
                yt_amount: yt.balance(&account),
                lp_amount: pool.lp_balance(&account),
                account,
            };
            if req.sy_shares > 0 || req.pt_amount > 0 || req.yt_amount > 0 || req.lp_amount > 0 {
                requests.push_back(req);
            }
        }
        if requests.is_empty() {
            panic_with_error!(&env, Error::NothingToSeize);
        }
        Self::run_batch(&env, &caller, &requests)
    }

    // -------------------------------------------------------------- settlement

    /// Settle the PT and YT a record still holds (`pt_amount + lp_pt` PT and `yt_amount` YT) into
    /// the underlying, at or after maturity, and write the result onto that record. Only the issuer
    /// authority; once per record. Returns `(underlying_from_pt, underlying_from_yt)`.
    pub fn finalize_record(env: Env, caller: Address, id: u64) -> (i128, i128) {
        Self::assert_issuer_admin(&env, &caller);
        let mut rec = Self::load_record(&env, id);
        if rec.finalized {
            panic_with_error!(&env, Error::AlreadyFinalized);
        }
        let pt_total = rec.pt_amount + rec.lp_pt;
        if pt_total == 0 && rec.yt_amount == 0 {
            // A pure-SY (or SY-leg-only) record has nothing left to settle: it was fully unwrapped
            // when it was seized.
            panic_with_error!(&env, Error::NothingToFinalize);
        }
        let self_addr = env.current_contract_address();
        let result = PrincipalManagerClient::new(
            &env,
            &Self::get(&env, &DataKey::PrincipalManager),
        )
        .redeem(&self_addr, &pt_total, &rec.yt_amount);
        rec.finalized = true;
        rec.underlying_from_pt = result.underlying_from_pt;
        rec.underlying_from_yt = result.underlying_from_yt;
        Self::store_record(&env, &rec);
        env.events().publish(
            (symbol_short!("finalize"),),
            (
                id,
                rec.account,
                result.underlying_from_pt,
                result.underlying_from_yt,
            ),
        );
        (result.underlying_from_pt, result.underlying_from_yt)
    }

    // ------------------------------------------------------------------ views

    pub fn get_record(env: Env, id: u64) -> RecoveryRecord {
        Self::load_record(&env, id)
    }

    /// Ids of every recovery record written for `account`, oldest first.
    pub fn account_records(env: Env, account: Address) -> Vec<u64> {
        env.storage()
            .persistent()
            .get(&DataKey::AccountRecords(account))
            .unwrap_or_else(|| Vec::new(&env))
    }

    pub fn record_count(env: Env) -> u64 {
        env.storage()
            .instance()
            .get(&DataKey::RecordCount)
            .unwrap_or(0)
    }

    pub fn underlying_address(env: Env) -> Address {
        Self::get(&env, &DataKey::Underlying)
    }

    // -------------------------------------------------------- internal helpers

    fn get(env: &Env, key: &DataKey) -> Address {
        env.storage()
            .instance()
            .get(key)
            .unwrap_or_else(|| panic_with_error!(env, Error::NotInitialized))
    }

    fn run_batch(env: &Env, caller: &Address, requests: &Vec<SeizeRequest>) -> Vec<u64> {
        if requests.is_empty() {
            panic_with_error!(env, Error::ZeroAmount);
        }
        if requests.len() > MAX_BATCH {
            panic_with_error!(env, Error::BatchTooLarge);
        }
        let mut ids: Vec<u64> = Vec::new(env);
        for r in requests.iter() {
            let rec = Self::seize_one(
                env,
                caller,
                &r.account,
                r.sy_shares,
                r.pt_amount,
                r.yt_amount,
                r.lp_amount,
            );
            ids.push_back(rec.id);
        }
        ids
    }

    /// Seize whichever positions are non-zero for one already-authenticated recovery, and write the
    /// record. The caller has authenticated the issuer; this checks the target.
    fn seize_one(
        env: &Env,
        caller: &Address,
        account: &Address,
        sy_shares: i128,
        pt_amount: i128,
        yt_amount: i128,
        lp_amount: i128,
    ) -> RecoveryRecord {
        Self::assert_target_deauthorized(env, account);
        if sy_shares < 0 || pt_amount < 0 || yt_amount < 0 || lp_amount < 0 {
            panic_with_error!(env, Error::ZeroAmount);
        }
        if sy_shares == 0 && pt_amount == 0 && yt_amount == 0 && lp_amount == 0 {
            panic_with_error!(env, Error::ZeroAmount);
        }

        let this = env.current_contract_address();
        let sy = SYWrapperClient::new(env, &Self::get(env, &DataKey::SYWrapper));
        let mut rec = RecoveryRecord {
            id: 0,
            account: account.clone(),
            ledger: env.ledger().sequence(),
            timestamp: env.ledger().timestamp(),
            sy_shares: 0,
            underlying_from_sy: 0,
            lp_amount: 0,
            lp_pt: 0,
            lp_sy_shares: 0,
            underlying_from_lp: 0,
            pt_amount: 0,
            yt_amount: 0,
            yt_yield_at_seize: 0,
            finalized: false,
            underlying_from_pt: 0,
            underlying_from_yt: 0,
        };

        if sy_shares > 0 {
            sy.seize(&this, account, &sy_shares);
            rec.sy_shares = sy_shares;
            rec.underlying_from_sy = sy.withdraw(&this, &sy_shares, &this, &0);
        }
        if pt_amount > 0 {
            rec.pt_amount = PTTokenClient::new(env, &Self::get(env, &DataKey::PTToken))
                .seize(&this, account, &pt_amount);
        }
        if yt_amount > 0 {
            let yt = YTTokenClient::new(env, &Self::get(env, &DataKey::YTToken));
            let pending_before = yt.pending_claim(&this);
            rec.yt_amount = yt.seize(&this, account, &yt_amount);
            // `seize` settles both sides and moves the account's accrued yield to the escrow.
            rec.yt_yield_at_seize = yt.pending_claim(&this) - pending_before;
        }
        if lp_amount > 0 {
            let pool = PoolClient::new(env, &Self::get(env, &DataKey::Pool));
            pool.seize_lp(&this, account, &lp_amount);
            let (pt_leg, sy_leg) = pool.redeem_seized_lp(&this, &lp_amount);
            rec.lp_amount = lp_amount;
            rec.lp_pt = pt_leg;
            rec.lp_sy_shares = sy_leg;
            if sy_leg > 0 {
                rec.underlying_from_lp = sy.withdraw(&this, &sy_leg, &this, &0);
            }
        }

        // Allocate the id and persist.
        let id: u64 = env
            .storage()
            .instance()
            .get(&DataKey::RecordCount)
            .unwrap_or(0);
        rec.id = id;
        env.storage()
            .instance()
            .set(&DataKey::RecordCount, &(id + 1));
        Self::store_record(env, &rec);
        let key = DataKey::AccountRecords(account.clone());
        let mut list: Vec<u64> = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or_else(|| Vec::new(env));
        list.push_back(id);
        env.storage().persistent().set(&key, &list);
        env.storage()
            .persistent()
            .extend_ttl(&key, RECORD_TTL_LEDGERS, RECORD_TTL_LEDGERS);

        env.events().publish(
            (symbol_short!("recovery"),),
            (
                caller.clone(),
                id,
                account.clone(),
                rec.sy_shares,
                rec.pt_amount,
                rec.yt_amount,
                rec.lp_amount,
            ),
        );
        rec
    }

    fn store_record(env: &Env, rec: &RecoveryRecord) {
        let key = DataKey::Record(rec.id);
        env.storage().persistent().set(&key, rec);
        env.storage()
            .persistent()
            .extend_ttl(&key, RECORD_TTL_LEDGERS, RECORD_TTL_LEDGERS);
    }

    fn load_record(env: &Env, id: u64) -> RecoveryRecord {
        env.storage()
            .persistent()
            .get(&DataKey::Record(id))
            .unwrap_or_else(|| panic_with_error!(env, Error::RecordNotFound))
    }

    fn assert_issuer_admin(env: &Env, caller: &Address) {
        caller.require_auth();
        let underlying = Self::get(env, &DataKey::Underlying);
        if !compliance::is_authority(env, &underlying, caller) {
            panic_with_error!(env, Error::Unauthorized);
        }
    }

    fn assert_target_deauthorized(env: &Env, account: &Address) {
        let underlying = Self::get(env, &DataKey::Underlying);
        if compliance::is_authorized(env, &underlying, account) {
            panic_with_error!(env, Error::TargetStillAuthorized);
        }
    }
}
