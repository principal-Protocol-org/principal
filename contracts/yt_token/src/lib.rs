//! YTToken — standalone SEP-41 Yield Token with continuous yield accrual.
//!
//! # Responsibilities
//! * Full SEP-41 token interface, same shape as PTToken.
//! * Mint/burn restricted to a single registered minter (PrincipalManager), set once via
//!   `set_minter` (two-phase init, same rationale as PTToken).
//! * Transfers gated the same way as PTToken: `Permissioning.is_allowed(to)` (coarse) AND
//!   `Permissioning.is_allowed_for_asset(to, this_contract_address)` (per-instrument), so PT
//!   and YT can carry independent eligibility policies, plus `underlying_SAC.authorized(account)`
//!   — the mandatory floor inherited live from the actual issuer. See
//!   COMPLIANT_SETTLEMENT_DESIGN.md.
//! * Continuous yield accrual via a global index (TECHNICAL_SPECIFICATION.md §5.5), advanced
//!   by `update_yield_index` and claimed via `claim_yield`.
//! * `seize`: lets a pre-configured `RecoveryEscrow` forcibly move a restricted holder's YT
//!   balance to itself, without the holder's authorization.
//!
//! # Yield-accounting correctness
//! Balance changes (mint, burn, transfer in/out, seize) settle each affected account's pending
//! yield at their *old* balance against the *current* index before the balance moves, and reset
//! that account's snapshot index. Skipping this step is a classic reward-accounting bug class: an
//! account could otherwise receive yield accrued before it held the position (buying in right
//! before a large index update) or lose yield it had already earned (transferring out right
//! after one). `settle` is called on every path that changes a balance, not only on claim.
//!
//! # The yield index, and why it makes the market exactly solvent at any rate
//! The index is `G = INDEX_SCALE * SCALE / rate` -- "underlying per unit of notional", the
//! reciprocal of the oracle rate -- advanced by `update_yield_index` and rounded up. A holder of `N`
//! YT (notional units) whose account was last settled at index `G_s` has, at index `G`, accrued
//! `pending = N * (G_s - G) / INDEX_SCALE` underlying, which is exactly
//! `N * (1/r_s - 1/r_now)`.
//!
//! Why that, and not a percentage of notional: PT and YT are minted 1:1 in *notional* (USDC) units,
//! `N = shares * r_0`, but the yield accrues on the *underlying* the position holds, `N / r`. Over a
//! position's life the claims telescope:
//!
//! ```text
//!     PT at maturity        N / r_final
//!   + YT yield (all steps)  N * (1/r_0 - 1/r_final)
//!   = N / r_0               = the `shares` originally deposited, exactly.
//! ```
//!
//! so PT and YT together never claim more than the SY custody backing them, for *any* mint rate
//! `r_0` (USDY trades well above 1.0). An earlier version scaled yield by the running ratio
//! `r_s / r_now` instead, which is only equivalent when `r_0 == 1.0`: at `r_0 = 1.05` and
//! `r_final = 1.10` it paid 100.23 shares of claims against 100 shares of custody. The index is a
//! closed form of the *current* rate, so it also has no path dependence and accumulates no rounding
//! error across oracle updates (the additive variant before that was a Riemann-sum overstatement).
//!
//! # Yield stops at maturity
//! The first `update_yield_index` call made at or after maturity advances the factor one last
//! time from the fresh oracle rate and then **freezes** it: every later call is a no-op. YT
//! therefore accrues nothing after maturity, and the rate it froze at (`last_oracle_rate`) is the
//! single settlement rate `PrincipalManager.settle_all` hands to PT redemption, so PT and YT are
//! always settled against the same number. Because the call is permissionless, anyone (a keeper,
//! a redeemer, `settle_all`) can trigger the freeze; the settlement rate is the first fresh oracle
//! observation at or after maturity.
//!
//! # Market creation
//! `initialize` requires `admin` to equal the underlying SAC's actual `admin()` (read live),
//! matching `SYWrapper`'s market-creation gate — see its module docs for the full rationale.
//!
//! # `claim_yield` is minter-gated, not holder-gated
//! `claim_yield` used to authorize on `from` (the holder), making it a public, directly
//! callable entrypoint that settles and zeroes a holder's pending claim without ever paying any
//! underlying — `PrincipalManager` is the only place that turns the returned amount into a real
//! payment. A holder calling it directly (bypassing `PrincipalManager`) would permanently
//! consume their claim for nothing. It is now gated the same way `mint`/`burn` already are: only
//! the registered minter (`PrincipalManager`) can call it, which pairs the settle-and-zero step
//! with `PrincipalManager.claim_yield`'s corresponding `SYWrapper.withdraw` payment in the same
//! call. Found during audit review (H-03).

#![no_std]
#![allow(deprecated)]
// `env.events().publish` / `register_contract`: migration to `#[contractevent]` is tracked separately; event topics are kept stable for indexers.
#![allow(clippy::too_many_arguments)] // Soroban `initialize` entrypoints wire many contracts by design.

use soroban_sdk::{
    contract, contractclient, contracterror, contractimpl, contracttype, panic_with_error,
    symbol_short, Address, Env, String,
};

use principal_compliance as compliance;

pub const SCALE: i128 = 10_000_000; // 1e7, matches PrincipalManager's SCALE

/// Fixed-point precision of the yield index and every account's snapshot of it (1e12). Finer than
/// `SCALE` so the one rounding in `INDEX_SCALE * SCALE / rate` is worth at most 1e-12 of a holder's
/// notional per settle.
pub const INDEX_SCALE: i128 = 1_000_000_000_000;

/// TTL extension applied to every persistent per-user entry (~30 days at 5 s/ledger).
const BALANCE_TTL_LEDGERS: u32 = 518_400;

/// Matches PrincipalManager's staleness window. Without this check, update_yield_index would
/// happily advance the accrual index off a rate the oracle relay stopped refreshing long ago —
/// every other oracle-consuming path in this codebase (PrincipalManager.mint/redeem) checks
/// freshness before using a rate; this one must too, for the same reason.
const MAX_ORACLE_STALENESS_SECS: u64 = 3_600;

// ---------------------------------------------------------------------------
// External contract interfaces
// ---------------------------------------------------------------------------

#[contractclient(name = "PermClient")]
pub trait PermissioningInterface {
    fn is_allowed(env: Env, account: Address) -> bool;
    fn is_allowed_for_asset(env: Env, account: Address, asset: Address) -> bool;
}

#[contractclient(name = "OracleClient")]
pub trait OracleInterface {
    fn get_reference_value(env: Env) -> i128;
    fn is_fresh(env: Env, max_stale_seconds: u64) -> bool;
}

// ---------------------------------------------------------------------------
// Error codes
// ---------------------------------------------------------------------------

#[contracterror]
#[derive(Copy, Clone, Debug, PartialEq)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    Unauthorized = 2,
    NotInitialized = 3,
    ZeroAmount = 4,
    InsufficientBalance = 5,
    InsufficientAllowance = 6,
    PermissionDenied = 7,
    MinterAlreadySet = 8,
    MinterNotSet = 9,
    OracleStale = 10,
    NotAuthorizedOnSac = 11,
    IssuerMismatch = 12,
    RecoveryEscrowAlreadySet = 13,
    NotRecoveryEscrow = 14,
}

// ---------------------------------------------------------------------------
// Storage key schema
// ---------------------------------------------------------------------------

#[contracttype]
pub enum DataKey {
    Admin,
    Minter,
    Permissioning,
    Underlying,     // Address of the underlying SAC, for authorization inheritance
    RecoveryEscrow, // Option<Address>; absent until set_recovery_escrow is called
    Oracle,
    Maturity,
    Name,
    Symbol,
    Decimals,
    TotalSupply,
    Balance(Address),
    Allowance(Address, Address),
    YieldIndex,     // i128, INDEX_SCALE * SCALE / rate, rounded up; only ever decreases
    LastOracleRate, // i128, high-water mark used to advance YieldIndex
    Frozen, // bool, set by the first update_yield_index at/after maturity; index never moves again
    LastClaimedIndex(Address), // i128, per-user snapshot of YieldIndex at their last settle
    PendingClaim(Address), // i128, settled-but-unclaimed yield, underlying units
}

#[contracttype]
#[derive(Clone)]
pub struct AllowanceValue {
    pub amount: i128,
    pub expiration_ledger: u32,
}

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

#[contract]
pub struct YTTokenContract;

#[contractimpl]
impl YTTokenContract {
    /// `admin` must be the underlying SAC's actual admin and must authorize this call.
    pub fn initialize(
        env: Env,
        admin: Address,
        permissioning: Address,
        underlying: Address,
        oracle: Address,
        maturity: u64,
        name: String,
        symbol: String,
        decimals: u32,
    ) {
        if env.storage().instance().has(&DataKey::Admin) {
            panic_with_error!(&env, Error::AlreadyInitialized);
        }
        admin.require_auth();
        compliance::init(&env, &underlying);
        if !compliance::is_authority(&env, &underlying, &admin) {
            panic_with_error!(&env, Error::IssuerMismatch);
        }

        // Genesis rate: read live from the oracle, not hardcoded to SCALE. If the market is
        // created when the real rate is already above SCALE (e.g. onboarding an
        // already-appreciated asset, or simply redeploying later in an asset's life), hardcoding
        // SCALE here would make the very first `update_yield_index` call treat the gap between
        // SCALE and the real rate as yield accrued since genesis -- overpaying whoever holds YT
        // at that moment for appreciation that happened before their position ever existed.
        // Requiring freshness here too catches deploying a market against a stale feed.
        let oracle_client = OracleClient::new(&env, &oracle);
        if !oracle_client.is_fresh(&MAX_ORACLE_STALENESS_SECS) {
            panic_with_error!(&env, Error::OracleStale);
        }
        let genesis_rate = oracle_client.get_reference_value();

        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage()
            .instance()
            .set(&DataKey::Permissioning, &permissioning);
        env.storage()
            .instance()
            .set(&DataKey::Underlying, &underlying);
        env.storage().instance().set(&DataKey::Oracle, &oracle);
        env.storage().instance().set(&DataKey::Maturity, &maturity);
        env.storage().instance().set(&DataKey::Name, &name);
        env.storage().instance().set(&DataKey::Symbol, &symbol);
        env.storage().instance().set(&DataKey::Decimals, &decimals);
        env.storage().instance().set(&DataKey::TotalSupply, &0_i128);
        // The genesis index is derived from the live genesis rate, so a market created when the
        // real rate is already above 1.0 baselines against it.
        env.storage()
            .instance()
            .set(&DataKey::YieldIndex, &Self::index_for(genesis_rate));
        env.storage()
            .instance()
            .set(&DataKey::LastOracleRate, &genesis_rate);
    }

    pub fn set_minter(env: Env, admin: Address, minter: Address) {
        Self::assert_admin(&env, &admin);
        if env.storage().instance().has(&DataKey::Minter) {
            panic_with_error!(&env, Error::MinterAlreadySet);
        }
        env.storage().instance().set(&DataKey::Minter, &minter);
        env.events().publish((symbol_short!("min_set"),), minter);
    }

    /// One-time wiring of the RecoveryEscrow contract authorized to call `seize`.
    pub fn set_recovery_escrow(env: Env, admin: Address, escrow: Address) {
        Self::assert_admin(&env, &admin);
        if env.storage().instance().has(&DataKey::RecoveryEscrow) {
            panic_with_error!(&env, Error::RecoveryEscrowAlreadySet);
        }
        env.storage()
            .instance()
            .set(&DataKey::RecoveryEscrow, &escrow);
        env.events().publish((symbol_short!("esc_set"),), escrow);
    }

    // --- SEP-41 token interface ---

    pub fn transfer(env: Env, from: Address, to: Address, amount: i128) {
        from.require_auth();
        if amount <= 0 {
            panic_with_error!(&env, Error::ZeroAmount);
        }
        // Both sides checked — see PTToken::transfer for why checking only `to` would let a
        // revoked holder dump YT before being frozen.
        Self::assert_sac_authorized(&env, &from);
        Self::assert_sac_authorized(&env, &to);
        Self::assert_permitted(&env, &from);
        Self::assert_permitted(&env, &to);

        let from_balance = Self::get_balance(&env, &from);
        if from_balance < amount {
            panic_with_error!(&env, Error::InsufficientBalance);
        }
        Self::settle(&env, &from);
        Self::settle(&env, &to);

        Self::set_balance(&env, &from, from_balance - amount);
        let to_balance = Self::get_balance(&env, &to);
        Self::set_balance(&env, &to, to_balance + amount);

        env.events()
            .publish((symbol_short!("transfer"),), (from, to, amount));
    }

    pub fn transfer_from(env: Env, spender: Address, from: Address, to: Address, amount: i128) {
        spender.require_auth();
        if amount <= 0 {
            panic_with_error!(&env, Error::ZeroAmount);
        }
        Self::assert_sac_authorized(&env, &from);
        Self::assert_sac_authorized(&env, &to);
        Self::assert_permitted(&env, &from);
        Self::assert_permitted(&env, &to);

        let allowance = Self::get_allowance(&env, &from, &spender);
        if allowance.amount < amount || allowance.expiration_ledger < env.ledger().sequence() {
            panic_with_error!(&env, Error::InsufficientAllowance);
        }
        let from_balance = Self::get_balance(&env, &from);
        if from_balance < amount {
            panic_with_error!(&env, Error::InsufficientBalance);
        }
        Self::settle(&env, &from);
        Self::settle(&env, &to);

        Self::set_balance(&env, &from, from_balance - amount);
        let to_balance = Self::get_balance(&env, &to);
        Self::set_balance(&env, &to, to_balance + amount);
        Self::set_allowance(
            &env,
            &from,
            &spender,
            allowance.amount - amount,
            allowance.expiration_ledger,
        );

        env.events()
            .publish((symbol_short!("transfer"),), (from, to, amount));
    }

    pub fn approve(
        env: Env,
        from: Address,
        spender: Address,
        amount: i128,
        expiration_ledger: u32,
    ) {
        from.require_auth();
        if amount < 0 {
            panic_with_error!(&env, Error::ZeroAmount);
        }
        Self::set_allowance(&env, &from, &spender, amount, expiration_ledger);
        env.events().publish(
            (symbol_short!("approve"),),
            (from, spender, amount, expiration_ledger),
        );
    }

    pub fn allowance(env: Env, from: Address, spender: Address) -> i128 {
        Self::get_allowance(&env, &from, &spender).amount
    }

    pub fn balance(env: Env, account: Address) -> i128 {
        Self::get_balance(&env, &account)
    }

    pub fn decimals(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKey::Decimals)
            .unwrap_or(0)
    }

    pub fn name(env: Env) -> String {
        env.storage()
            .instance()
            .get(&DataKey::Name)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized))
    }

    pub fn symbol(env: Env) -> String {
        env.storage()
            .instance()
            .get(&DataKey::Symbol)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized))
    }

    // --- minter-only ---

    pub fn mint(env: Env, to: Address, amount: i128) {
        let minter = Self::require_minter(&env);
        minter.require_auth();
        if amount <= 0 {
            panic_with_error!(&env, Error::ZeroAmount);
        }
        Self::assert_sac_authorized(&env, &to);
        Self::assert_permitted(&env, &to);
        Self::settle(&env, &to);

        let bal = Self::get_balance(&env, &to);
        Self::set_balance(&env, &to, bal + amount);
        let total: i128 = env
            .storage()
            .instance()
            .get(&DataKey::TotalSupply)
            .unwrap_or(0);
        env.storage()
            .instance()
            .set(&DataKey::TotalSupply, &(total + amount));

        env.events().publish((symbol_short!("mint"),), (to, amount));
    }

    pub fn burn(env: Env, from: Address, amount: i128) {
        let minter = Self::require_minter(&env);
        minter.require_auth();
        if amount <= 0 {
            panic_with_error!(&env, Error::ZeroAmount);
        }
        let bal = Self::get_balance(&env, &from);
        if bal < amount {
            panic_with_error!(&env, Error::InsufficientBalance);
        }
        Self::settle(&env, &from);

        Self::set_balance(&env, &from, bal - amount);
        let total: i128 = env
            .storage()
            .instance()
            .get(&DataKey::TotalSupply)
            .unwrap_or(0);
        env.storage()
            .instance()
            .set(&DataKey::TotalSupply, &(total - amount));

        env.events()
            .publish((symbol_short!("burn"),), (from, amount));
    }

    // --- compliance recovery (seize) ---

    /// Forcibly move `amount` from `account`'s YT balance to the caller's own balance, without
    /// `account`'s authorization. Callable only by the configured `RecoveryEscrow` — see
    /// `SYWrapper::seize` for the full rationale. Settles both sides' pending yield first, same
    /// as any other balance-changing path, so the seizure doesn't shift already-accrued yield
    /// between the flagged account and the escrow.
    pub fn seize(env: Env, caller: Address, account: Address, amount: i128) -> i128 {
        caller.require_auth();
        let escrow = Self::require_escrow(&env);
        if caller != escrow {
            panic_with_error!(&env, Error::NotRecoveryEscrow);
        }
        if amount <= 0 {
            panic_with_error!(&env, Error::ZeroAmount);
        }

        let bal = Self::get_balance(&env, &account);
        if bal < amount {
            panic_with_error!(&env, Error::InsufficientBalance);
        }
        Self::settle(&env, &account);
        Self::settle(&env, &caller);

        Self::set_balance(&env, &account, bal - amount);
        let to_balance = Self::get_balance(&env, &caller);
        Self::set_balance(&env, &caller, to_balance + amount);

        env.events()
            .publish((symbol_short!("seize"),), (caller, account, amount));
        amount
    }

    // --- yield accrual ---

    /// Permissionless: advances the global yield index to `INDEX_SCALE * SCALE / oracle_rate`.
    /// No-op if the rate hasn't increased since the last recorded high-water mark, matching
    /// the protocol-wide invariant that YT never accrues negative yield. Because the index is a
    /// closed form of the current rate, the number of intermediate calls cannot change any
    /// holder's total -- see this module's doc comment.
    pub fn update_yield_index(env: Env) {
        if env
            .storage()
            .instance()
            .get(&DataKey::Frozen)
            .unwrap_or(false)
        {
            return;
        }
        let oracle_addr: Address = env
            .storage()
            .instance()
            .get(&DataKey::Oracle)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));
        let oracle = OracleClient::new(&env, &oracle_addr);
        if !oracle.is_fresh(&MAX_ORACLE_STALENESS_SECS) {
            panic_with_error!(&env, Error::OracleStale);
        }
        let now_rate = oracle.get_reference_value();
        let last_rate: i128 = env
            .storage()
            .instance()
            .get(&DataKey::LastOracleRate)
            .unwrap_or(SCALE);

        if now_rate > last_rate {
            let new_index = Self::index_for(now_rate);
            env.storage()
                .instance()
                .set(&DataKey::YieldIndex, &new_index);
            env.storage()
                .instance()
                .set(&DataKey::LastOracleRate, &now_rate);
            env.events()
                .publish((symbol_short!("idx_up"),), (new_index, now_rate));
        }

        // At or after maturity this update is the last one: freeze the index.
        let maturity: u64 = env
            .storage()
            .instance()
            .get(&DataKey::Maturity)
            .unwrap_or(u64::MAX);
        if env.ledger().timestamp() >= maturity {
            env.storage().instance().set(&DataKey::Frozen, &true);
            env.events()
                .publish((symbol_short!("frozen"),), now_rate.max(last_rate));
        }
    }

    /// True once the yield index has been frozen at maturity.
    pub fn is_frozen(env: Env) -> bool {
        env.storage()
            .instance()
            .get(&DataKey::Frozen)
            .unwrap_or(false)
    }

    /// The highest oracle rate the index has been advanced to. After `is_frozen()` this is the
    /// market's final settlement rate.
    pub fn last_oracle_rate(env: Env) -> i128 {
        env.storage()
            .instance()
            .get(&DataKey::LastOracleRate)
            .unwrap_or(SCALE)
    }

    /// Settles `from`'s pending yield up to the current index, then zeroes and returns it.
    /// Callable only by the registered minter (`PrincipalManager`), which is responsible for
    /// converting the returned amount into a real underlying payment via `SYWrapper.withdraw`
    /// in the same call -- see this module's doc comment (H-03).
    pub fn claim_yield(env: Env, caller: Address, from: Address) -> i128 {
        caller.require_auth();
        let minter = Self::require_minter(&env);
        if caller != minter {
            panic_with_error!(&env, Error::Unauthorized);
        }
        Self::settle(&env, &from);
        let key = DataKey::PendingClaim(from.clone());
        let amount: i128 = env.storage().persistent().get(&key).unwrap_or(0);
        env.storage().persistent().set(&key, &0_i128);
        env.events()
            .publish((symbol_short!("claim"),), (from, amount));
        amount
    }

    /// The current yield index `INDEX_SCALE * SCALE / rate` (see module docs) -- it only ever
    /// decreases as the oracle rate rises. Meaningful relative to an account's own
    /// `last_claimed_index`.
    pub fn accrued_yield_index(env: Env) -> i128 {
        env.storage()
            .instance()
            .get(&DataKey::YieldIndex)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized))
    }

    pub fn last_claimed_index(env: Env, account: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::LastClaimedIndex(account))
            .unwrap_or_else(|| Self::accrued_yield_index(env.clone()))
    }

    pub fn pending_claim(env: Env, account: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::PendingClaim(account))
            .unwrap_or(0)
    }

    // --- views ---

    pub fn total_supply(env: Env) -> i128 {
        env.storage()
            .instance()
            .get(&DataKey::TotalSupply)
            .unwrap_or(0)
    }

    pub fn maturity(env: Env) -> u64 {
        env.storage()
            .instance()
            .get(&DataKey::Maturity)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized))
    }

    pub fn minter(env: Env) -> Address {
        Self::require_minter(&env)
    }

    pub fn get_admin(env: Env) -> Address {
        Self::require_admin(&env)
    }

    pub fn recovery_escrow(env: Env) -> Address {
        Self::require_escrow(&env)
    }

    pub fn underlying_address(env: Env) -> Address {
        env.storage()
            .instance()
            .get(&DataKey::Underlying)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized))
    }

    pub fn permissioning_address(env: Env) -> Address {
        env.storage()
            .instance()
            .get(&DataKey::Permissioning)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized))
    }

    pub fn oracle_address(env: Env) -> Address {
        env.storage()
            .instance()
            .get(&DataKey::Oracle)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized))
    }

    // --- internal helpers ---

    /// Settle `account`'s pending yield at its balance *before* any change, against the
    /// current index, then advance its snapshot to the current index. Must be called on every path
    /// that mutates a balance (mint/burn/transfer/seize, both sides), before the balance itself
    /// changes.
    ///
    /// The index only ever decreases (see module docs), so an account has yield pending exactly
    /// when it has dropped below the account's own snapshot: `pending = bal * (last - index) /
    /// INDEX_SCALE = bal * (1/r_settle - 1/r_now)`.
    fn settle(env: &Env, account: &Address) {
        let index: i128 = env
            .storage()
            .instance()
            .get(&DataKey::YieldIndex)
            .unwrap_or_else(|| panic_with_error!(env, Error::NotInitialized));
        let last_key = DataKey::LastClaimedIndex(account.clone());
        // An account that never held YT has nothing pending; its first snapshot is "now".
        let last: i128 = env.storage().persistent().get(&last_key).unwrap_or(index);

        if index < last {
            let bal = Self::get_balance(env, account);
            if bal > 0 {
                let pending = bal * (last - index) / INDEX_SCALE;
                if pending > 0 {
                    let pc_key = DataKey::PendingClaim(account.clone());
                    let acc: i128 = env.storage().persistent().get(&pc_key).unwrap_or(0);
                    env.storage().persistent().set(&pc_key, &(acc + pending));
                    env.storage().persistent().extend_ttl(
                        &pc_key,
                        BALANCE_TTL_LEDGERS,
                        BALANCE_TTL_LEDGERS,
                    );
                }
            }
        }
        env.storage().persistent().set(&last_key, &index);
        env.storage()
            .persistent()
            .extend_ttl(&last_key, BALANCE_TTL_LEDGERS, BALANCE_TTL_LEDGERS);
    }

    /// `INDEX_SCALE * SCALE / rate`, rounded up (so any rounding under-pays YT, never over-pays).
    fn index_for(rate: i128) -> i128 {
        (INDEX_SCALE * SCALE + rate - 1) / rate
    }

    fn assert_permitted(env: &Env, account: &Address) {
        let perm_addr: Address = env
            .storage()
            .instance()
            .get(&DataKey::Permissioning)
            .unwrap_or_else(|| panic_with_error!(env, Error::NotInitialized));
        let client = PermClient::new(env, &perm_addr);
        if !client.is_allowed(account) {
            panic_with_error!(env, Error::PermissionDenied);
        }
        if !client.is_allowed_for_asset(account, &env.current_contract_address()) {
            panic_with_error!(env, Error::PermissionDenied);
        }
    }

    fn assert_sac_authorized(env: &Env, account: &Address) {
        let underlying: Address = env
            .storage()
            .instance()
            .get(&DataKey::Underlying)
            .unwrap_or_else(|| panic_with_error!(env, Error::NotInitialized));
        if !compliance::is_authorized(env, &underlying, account) {
            panic_with_error!(env, Error::NotAuthorizedOnSac);
        }
    }

    fn require_minter(env: &Env) -> Address {
        env.storage()
            .instance()
            .get(&DataKey::Minter)
            .unwrap_or_else(|| panic_with_error!(env, Error::MinterNotSet))
    }

    fn require_escrow(env: &Env) -> Address {
        env.storage()
            .instance()
            .get(&DataKey::RecoveryEscrow)
            .unwrap_or_else(|| panic_with_error!(env, Error::NotRecoveryEscrow))
    }

    fn require_admin(env: &Env) -> Address {
        env.storage()
            .instance()
            .get(&DataKey::Admin)
            .unwrap_or_else(|| panic_with_error!(env, Error::NotInitialized))
    }

    fn assert_admin(env: &Env, caller: &Address) {
        caller.require_auth();
        if *caller != Self::require_admin(env) {
            panic_with_error!(env, Error::Unauthorized);
        }
    }

    fn get_balance(env: &Env, account: &Address) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::Balance(account.clone()))
            .unwrap_or(0)
    }

    fn set_balance(env: &Env, account: &Address, amount: i128) {
        let key = DataKey::Balance(account.clone());
        env.storage().persistent().set(&key, &amount);
        env.storage()
            .persistent()
            .extend_ttl(&key, BALANCE_TTL_LEDGERS, BALANCE_TTL_LEDGERS);
    }

    fn get_allowance(env: &Env, from: &Address, spender: &Address) -> AllowanceValue {
        env.storage()
            .persistent()
            .get(&DataKey::Allowance(from.clone(), spender.clone()))
            .unwrap_or(AllowanceValue {
                amount: 0,
                expiration_ledger: 0,
            })
    }

    fn set_allowance(
        env: &Env,
        from: &Address,
        spender: &Address,
        amount: i128,
        expiration_ledger: u32,
    ) {
        let key = DataKey::Allowance(from.clone(), spender.clone());
        env.storage().persistent().set(
            &key,
            &AllowanceValue {
                amount,
                expiration_ledger,
            },
        );
        env.storage()
            .persistent()
            .extend_ttl(&key, BALANCE_TTL_LEDGERS, BALANCE_TTL_LEDGERS);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod test;
