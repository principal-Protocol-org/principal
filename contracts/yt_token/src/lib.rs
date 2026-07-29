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
//! # Why the index is multiplicative, not additive
//! `update_yield_index` tracks a compounding factor `F`, initialized to `SCALE` and updated on
//! every call as `F = F * last_rate / now_rate` -- a running product that telescopes exactly to
//! `SCALE * r_genesis / r_now`, regardless of how many intermediate calls happened or what the
//! intermediate rates were. An account's pending yield since its last settle (when `F` was
//! `F_settle`) is `balance * (F_settle - F_now) / F_settle`, which reduces to exactly
//! `balance * (r_now - r_settle) / r_now` -- the same economically-correct formula
//! `PrincipalManager` uses for PT redemption, path-independent by construction.
//!
//! An earlier version of this contract accumulated yield *additively*
//! (`idx += (r_new - r_old) * SCALE / r_new`, summed across every call). That is a Riemann-sum
//! approximation of `ln(r_now / r_genesis)`, which is provably always `>=` the correct
//! `(r_now - r_genesis) / r_now` for a rising rate, and strictly greater once there is more than
//! one intermediate step -- the gap grows with every additional `update_yield_index` call. Since
//! `PrincipalManager.redeem` treats this contract's `claim_yield` as the sole authoritative payer
//! of YT yield (see its own module docs), that overstatement was a real solvency bug: aggregate
//! PT + YT redemptions could exceed the underlying actually held, and `update_yield_index` is
//! permissionless with no rate limit, so it was also actively triggerable. The multiplicative
//! construction above closes this: it is exact for any number of steps, not just one.
//!
//! # Market creation
//! `initialize` requires `admin` to equal the underlying SAC's actual `admin()` (read live),
//! matching `SYWrapper`'s market-creation gate — see its module docs for the full rationale.

#![no_std]

use soroban_sdk::{
    contract, contractclient, contracterror, contractimpl, contracttype, panic_with_error,
    symbol_short, token, Address, Env, String,
};

pub const SCALE: i128 = 10_000_000; // 1e7, matches PrincipalManager's SCALE

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
    YieldIndex, // i128, multiplicative compounding factor, scaled by SCALE; starts at SCALE and
    // only ever decreases -- telescopes exactly to SCALE * r_genesis / r_now regardless of how
    // many update_yield_index calls happened in between (see module docs).
    LastOracleRate, // i128, high-water mark used to advance YieldIndex
    LastClaimedIndex(Address), // i128, per-user snapshot of YieldIndex at their last settle
    PendingClaim(Address),     // i128, settled-but-unclaimed yield, underlying units
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
        let sac_admin = token::StellarAssetClient::new(&env, &underlying).admin();
        if admin != sac_admin {
            panic_with_error!(&env, Error::IssuerMismatch);
        }
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
        // Genesis factor: SCALE, decreasing monotonically thereafter (see module docs).
        env.storage().instance().set(&DataKey::YieldIndex, &SCALE);
        env.storage()
            .instance()
            .set(&DataKey::LastOracleRate, &SCALE);
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

    /// Permissionless: advances the global yield factor using the current oracle rate.
    /// No-op if the rate hasn't increased since the last recorded high-water mark, matching
    /// the protocol-wide invariant that YT never accrues negative yield.
    ///
    /// Multiplies the running factor by `last_rate / now_rate` rather than adding
    /// `(now_rate - last_rate) / now_rate` -- see this module's doc comment for why the additive
    /// form is a solvency bug (it overstates yield, unboundedly, with every extra call) and the
    /// multiplicative form is exact for any number of calls.
    pub fn update_yield_index(env: Env) {
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
            let factor: i128 = env
                .storage()
                .instance()
                .get(&DataKey::YieldIndex)
                .unwrap_or(SCALE);
            let new_factor = factor * last_rate / now_rate;
            env.storage()
                .instance()
                .set(&DataKey::YieldIndex, &new_factor);
            env.storage()
                .instance()
                .set(&DataKey::LastOracleRate, &now_rate);
            env.events()
                .publish((symbol_short!("idx_up"),), (new_factor, now_rate));
        }
    }

    /// Settles the caller's pending yield up to the current index, then pays it out
    /// (in this POC: computed, zeroed, and returned — actual underlying transfer is a
    /// Router-integration milestone, matching PrincipalManager.redeem's existing convention).
    pub fn claim_yield(env: Env, from: Address) -> i128 {
        from.require_auth();
        Self::settle(&env, &from);
        let key = DataKey::PendingClaim(from.clone());
        let amount: i128 = env.storage().persistent().get(&key).unwrap_or(0);
        env.storage().persistent().set(&key, &0_i128);
        env.events()
            .publish((symbol_short!("claim"),), (from, amount));
        amount
    }

    /// The current global compounding factor (see module docs) -- starts at `SCALE` and only
    /// ever decreases as the oracle rate rises. Not a cumulative "amount of yield" by itself;
    /// meaningful only relative to an account's own `last_claimed_index`.
    pub fn accrued_yield_index(env: Env) -> i128 {
        env.storage()
            .instance()
            .get(&DataKey::YieldIndex)
            .unwrap_or(SCALE)
    }

    pub fn last_claimed_index(env: Env, account: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::LastClaimedIndex(account))
            .unwrap_or(SCALE)
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

    // --- internal helpers ---

    /// Settle `account`'s pending yield at its balance *before* any change, against the
    /// current global factor, then advance its snapshot to the current factor. Must be called
    /// on every path that mutates a balance (mint/burn/transfer/seize, both sides), before the
    /// balance itself changes.
    ///
    /// `factor` only ever decreases (see module docs), so an account has yield pending exactly
    /// when the global factor has dropped below its own snapshot from its last settle.
    /// `pending = bal * (last - factor) / last` -- dividing by the account's *own* snapshot,
    /// not by `SCALE` -- is what makes this telescope to the exact, path-independent
    /// `bal * (r_now - r_settle) / r_now` regardless of how many steps happened in between.
    fn settle(env: &Env, account: &Address) {
        let factor: i128 = env
            .storage()
            .instance()
            .get(&DataKey::YieldIndex)
            .unwrap_or(SCALE);
        let last_key = DataKey::LastClaimedIndex(account.clone());
        let last: i128 = env.storage().persistent().get(&last_key).unwrap_or(SCALE);

        if factor < last {
            let bal = Self::get_balance(env, account);
            if bal > 0 {
                let pending = bal * (last - factor) / last;
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
        env.storage().persistent().set(&last_key, &factor);
        env.storage()
            .persistent()
            .extend_ttl(&last_key, BALANCE_TTL_LEDGERS, BALANCE_TTL_LEDGERS);
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
        if !token::StellarAssetClient::new(env, &underlying).authorized(account) {
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
mod test {
    use soroban_sdk::{
        testutils::{Address as _, IssuerFlags, Ledger as _},
        token, Address, Env, String,
    };

    use principal_oracle_adapter::{OracleAdapterContract, OracleAdapterContractClient};
    use principal_permissioning::{PermissioningContract, PermissioningContractClient};

    use super::{YTTokenContract, YTTokenContractClient, SCALE};

    const T0: u64 = 1_000;

    struct Fixture {
        env: Env,
        client: YTTokenContractClient<'static>,
        admin: Address,
        underlying: Address,
        perm: PermissioningContractClient<'static>,
        perm_admin: Address,
        oracle: OracleAdapterContractClient<'static>,
        oracle_admin: Address,
        yt_id: Address,
    }

    fn setup() -> Fixture {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().with_mut(|li| li.timestamp = T0);

        let perm_id = env.register_contract(None, PermissioningContract);
        let perm = PermissioningContractClient::new(&env, &perm_id);
        let perm_admin = Address::generate(&env);
        perm.initialize(&perm_admin);

        let oracle_id = env.register_contract(None, OracleAdapterContract);
        let oracle = OracleAdapterContractClient::new(&env, &oracle_id);
        let oracle_admin = Address::generate(&env);
        oracle.initialize(&oracle_admin);
        oracle.set_reference_value(&oracle_admin, &SCALE, &T0);

        // admin doubles as the underlying SAC's real admin, satisfying the issuer-match check.
        // RevocableFlag lets tests simulate deauthorization.
        let admin = Address::generate(&env);
        let underlying_sac = env.register_stellar_asset_contract_v2(admin.clone());
        underlying_sac.issuer().set_flag(IssuerFlags::RevocableFlag);
        let underlying = underlying_sac.address();

        let yt_id = env.register_contract(None, YTTokenContract);
        let client = YTTokenContractClient::new(&env, &yt_id);
        client.initialize(
            &admin,
            &perm_id,
            &underlying,
            &oracle_id,
            &u64::MAX,
            &String::from_str(&env, "Yield Token USDY"),
            &String::from_str(&env, "YT-USDY"),
            &7,
        );

        Fixture {
            env,
            client,
            admin,
            underlying,
            perm,
            perm_admin,
            oracle,
            oracle_admin,
            yt_id,
        }
    }

    fn grant(f: &Fixture, user: &Address) {
        f.perm.grant_account(&f.perm_admin, user);
        f.perm.grant_asset(&f.perm_admin, user, &f.yt_id);
        token::StellarAssetClient::new(&f.env, &f.underlying).set_authorized(user, &true);
    }

    #[test]
    #[should_panic]
    fn initialize_rejects_admin_not_matching_sac_admin() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().with_mut(|li| li.timestamp = T0);

        let perm_id = env.register_contract(None, PermissioningContract);
        let real_sac_admin = Address::generate(&env);
        PermissioningContractClient::new(&env, &perm_id).initialize(&real_sac_admin);

        let oracle_id = env.register_contract(None, OracleAdapterContract);
        OracleAdapterContractClient::new(&env, &oracle_id).initialize(&real_sac_admin);

        let underlying = env
            .register_stellar_asset_contract_v2(real_sac_admin.clone())
            .address();

        let impostor = Address::generate(&env);
        let yt_id = env.register_contract(None, YTTokenContract);
        let client = YTTokenContractClient::new(&env, &yt_id);
        client.initialize(
            &impostor,
            &perm_id,
            &underlying,
            &oracle_id,
            &u64::MAX,
            &String::from_str(&env, "Yield Token USDY"),
            &String::from_str(&env, "YT-USDY"),
            &7,
        );
    }

    #[test]
    fn mint_and_balance() {
        let f = setup();
        let minter = Address::generate(&f.env);
        f.client.set_minter(&f.admin, &minter);
        let user = Address::generate(&f.env);
        grant(&f, &user);

        f.client.mint(&user, &1_000);
        assert_eq!(f.client.balance(&user), 1_000);
    }

    #[test]
    #[should_panic]
    fn mint_without_sac_authorization_panics() {
        let f = setup();
        let minter = Address::generate(&f.env);
        f.client.set_minter(&f.admin, &minter);

        let user = Address::generate(&f.env);
        f.perm.grant_account(&f.perm_admin, &user);
        f.perm.grant_asset(&f.perm_admin, &user, &f.yt_id);
        token::StellarAssetClient::new(&f.env, &f.underlying).set_authorized(&user, &false);

        f.client.mint(&user, &1_000);
    }

    #[test]
    fn no_yield_without_rate_increase() {
        let f = setup();
        let minter = Address::generate(&f.env);
        f.client.set_minter(&f.admin, &minter);
        let user = Address::generate(&f.env);
        grant(&f, &user);
        f.client.mint(&user, &(1_000 * SCALE));

        f.client.update_yield_index(); // rate unchanged since inception
        assert_eq!(f.client.accrued_yield_index(), SCALE); // factor untouched at genesis value
        assert_eq!(f.client.claim_yield(&user), 0);
    }

    #[test]
    fn yield_accrues_after_rate_increase() {
        let f = setup();
        let minter = Address::generate(&f.env);
        f.client.set_minter(&f.admin, &minter);
        let user = Address::generate(&f.env);
        grant(&f, &user);
        f.client.mint(&user, &(1_000 * SCALE)); // notional 1000 units at SCALE

        // Rate goes from 1.0 to 1.03.
        f.oracle
            .set_reference_value(&f.oracle_admin, &10_300_000, &(T0 + 1));
        f.env.ledger().with_mut(|li| li.timestamp = T0 + 1);
        f.client.update_yield_index();

        // new_factor = factor * last_rate / now_rate = SCALE * SCALE / 10_300_000
        let expected_factor = SCALE * SCALE / 10_300_000_i128;
        assert_eq!(f.client.accrued_yield_index(), expected_factor);

        // pending = bal * (last - factor) / last, with this user's `last` == SCALE (settled at
        // mint, before the index ever moved) -- reduces to bal * (SCALE - factor) / SCALE.
        let claimed = f.client.claim_yield(&user);
        let expected_claim = (1_000 * SCALE) * (SCALE - expected_factor) / SCALE;
        assert_eq!(claimed, expected_claim);
        // Exact match to the economically-correct single-shot formula: notional * (final_rate -
        // initial_rate) / final_rate -- confirming the multiplicative index reproduces it exactly.
        let exact = 1_000_i128 * (10_300_000 - SCALE) / 10_300_000;
        assert_eq!(claimed / SCALE, exact);

        // Claiming again immediately yields nothing further.
        assert_eq!(f.client.claim_yield(&user), 0);
    }

    #[test]
    fn yield_is_path_independent_across_many_intermediate_updates() {
        // The regression this fix exists for: whether the oracle rate moves from R0 to Rfinal
        // in one jump or many small steps, the claimable yield must be identical -- unlike the
        // old additive-index formula, which overstated yield more with every extra step (the
        // failure mode verified numerically during the audit: ~13.5% overstatement for a
        // 90-step, 30%-appreciation market).
        let f = setup();
        let minter = Address::generate(&f.env);
        f.client.set_minter(&f.admin, &minter);

        let user = Address::generate(&f.env);
        grant(&f, &user);
        f.client.mint(&user, &(1_000 * SCALE));

        let final_rate: i128 = 13_000_000; // 1.30, 30% total appreciation
        let n_steps = 30_u64;
        let mut ts = T0;
        for i in 1..=n_steps {
            ts += 1;
            let r = SCALE + (final_rate - SCALE) * (i as i128) / (n_steps as i128);
            f.oracle.set_reference_value(&f.oracle_admin, &r, &ts);
            f.env.ledger().with_mut(|li| li.timestamp = ts);
            f.client.update_yield_index();
        }

        let many_step_claim = f.client.claim_yield(&user);

        // What a single jump straight from SCALE to final_rate would have produced for the
        // same notional -- the exact, path-independent formula PT redemption also uses.
        let single_jump_expected = (1_000 * SCALE) * (final_rate - SCALE) / final_rate;

        // Not bit-exact -- 30 successive floor divisions accumulate a small, bounded rounding
        // residual -- but nowhere close to the old additive formula's unbounded, ever-growing
        // overstatement (which would have been ~13% high here, not ~0.001%).
        let diff = (many_step_claim - single_jump_expected).unsigned_abs();
        assert!(
            diff * 1_000_000 < single_jump_expected.unsigned_abs() * 100, // < 0.01% relative
            "multi-step claim {many_step_claim} diverged from single-jump {single_jump_expected} by more than floor-rounding dust"
        );
    }

    #[test]
    fn late_buyer_does_not_receive_prior_yield() {
        let f = setup();
        let minter = Address::generate(&f.env);
        f.client.set_minter(&f.admin, &minter);
        let alice = Address::generate(&f.env);
        let bob = Address::generate(&f.env);
        grant(&f, &alice);
        grant(&f, &bob);

        f.client.mint(&alice, &(1_000 * SCALE));

        // Rate rises before Bob ever holds YT.
        f.oracle
            .set_reference_value(&f.oracle_admin, &10_300_000, &(T0 + 1));
        f.env.ledger().with_mut(|li| li.timestamp = T0 + 1);
        f.client.update_yield_index();

        // Bob receives YT only now, after the index already moved.
        f.client.mint(&bob, &(500 * SCALE));

        // Bob's settled snapshot should already equal the current index, so he has
        // nothing pending despite the global index being nonzero.
        assert_eq!(f.client.pending_claim(&bob), 0);
        assert_eq!(f.client.claim_yield(&bob), 0);

        // Alice still gets her full accrued share.
        assert!(f.client.claim_yield(&alice) > 0);
    }

    #[test]
    fn transfer_settles_both_sides() {
        let f = setup();
        let minter = Address::generate(&f.env);
        f.client.set_minter(&f.admin, &minter);
        let alice = Address::generate(&f.env);
        let bob = Address::generate(&f.env);
        grant(&f, &alice);
        grant(&f, &bob);

        f.client.mint(&alice, &(1_000 * SCALE));
        f.oracle
            .set_reference_value(&f.oracle_admin, &10_300_000, &(T0 + 1));
        f.env.ledger().with_mut(|li| li.timestamp = T0 + 1);
        f.client.update_yield_index();

        // Alice transfers everything to Bob; her accrued yield up to this point must
        // remain hers (settled before the balance moves), not follow the tokens to Bob.
        f.client.transfer(&alice, &bob, &(1_000 * SCALE));

        let alice_claim = f.client.claim_yield(&alice);
        assert!(alice_claim > 0);
        assert_eq!(f.client.claim_yield(&bob), 0); // Bob owned nothing while the index moved
    }

    #[test]
    #[should_panic]
    fn revoked_holder_cannot_dump_yt_before_seizure() {
        let f = setup();
        let minter = Address::generate(&f.env);
        f.client.set_minter(&f.admin, &minter);
        let alice = Address::generate(&f.env);
        let bob = Address::generate(&f.env);
        grant(&f, &alice);
        grant(&f, &bob);
        f.client.mint(&alice, &(500 * SCALE));

        f.perm.revoke_account(&f.perm_admin, &alice);
        f.client.transfer(&alice, &bob, &(100 * SCALE)); // bob is still fully eligible
    }

    #[test]
    #[should_panic]
    fn deauthorized_on_sac_cannot_dump_yt_before_seizure() {
        let f = setup();
        let minter = Address::generate(&f.env);
        f.client.set_minter(&f.admin, &minter);
        let alice = Address::generate(&f.env);
        let bob = Address::generate(&f.env);
        grant(&f, &alice);
        grant(&f, &bob);
        f.client.mint(&alice, &(500 * SCALE));

        token::StellarAssetClient::new(&f.env, &f.underlying).set_authorized(&alice, &false);
        f.client.transfer(&alice, &bob, &(100 * SCALE));
    }

    #[test]
    #[should_panic]
    fn update_yield_index_blocked_by_stale_oracle() {
        // Without a freshness check, this would silently advance the accrual index off a rate
        // the oracle relay stopped refreshing long ago.
        let f = setup();
        // Ledger advances far past the oracle's last update without a fresh price ever landing.
        f.env.ledger().with_mut(|li| li.timestamp = T0 + 3_601);
        f.client.update_yield_index();
    }

    // --- seize (compliance recovery) ---

    #[test]
    fn seize_moves_balance_and_settles_both_sides() {
        let f = setup();
        let minter = Address::generate(&f.env);
        f.client.set_minter(&f.admin, &minter);
        let escrow = Address::generate(&f.env);
        f.client.set_recovery_escrow(&f.admin, &escrow);

        let bad_actor = Address::generate(&f.env);
        grant(&f, &bad_actor);
        f.client.mint(&bad_actor, &(1_000 * SCALE));

        // Rate rises before the seizure.
        f.oracle
            .set_reference_value(&f.oracle_admin, &10_300_000, &(T0 + 1));
        f.env.ledger().with_mut(|li| li.timestamp = T0 + 1);
        f.client.update_yield_index();

        let seized = f.client.seize(&escrow, &bad_actor, &(1_000 * SCALE));
        assert_eq!(seized, 1_000 * SCALE);
        assert_eq!(f.client.balance(&bad_actor), 0);
        assert_eq!(f.client.balance(&escrow), 1_000 * SCALE);

        // Yield accrued before seizure stays with the original holder, not the escrow.
        assert!(f.client.claim_yield(&bad_actor) > 0);
        assert_eq!(f.client.claim_yield(&escrow), 0);
    }

    #[test]
    #[should_panic]
    fn seize_requires_configured_escrow_caller() {
        let f = setup();
        let minter = Address::generate(&f.env);
        f.client.set_minter(&f.admin, &minter);
        let escrow = Address::generate(&f.env);
        let impostor = Address::generate(&f.env);
        f.client.set_recovery_escrow(&f.admin, &escrow);

        let bad_actor = Address::generate(&f.env);
        grant(&f, &bad_actor);
        f.client.mint(&bad_actor, &(500 * SCALE));

        f.client.seize(&impostor, &bad_actor, &(500 * SCALE));
    }
}
