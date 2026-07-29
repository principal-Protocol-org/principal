//! YTToken — standalone SEP-41 Yield Token with continuous yield accrual.
//!
//! # Responsibilities
//! * Full SEP-41 token interface, same shape as PTToken.
//! * Mint/burn restricted to a single registered minter (PrincipalManager), set once via
//!   `set_minter` (two-phase init, same rationale as PTToken).
//! * Transfers gated the same way as PTToken: `Permissioning.is_allowed(to)` (coarse) AND
//!   `Permissioning.is_allowed_for_asset(to, this_contract_address)` (per-instrument), so PT
//!   and YT can carry independent eligibility policies. See COMPLIANT_SETTLEMENT_DESIGN.md §1.
//! * Continuous yield accrual via a global index (TECHNICAL_SPECIFICATION.md §5.5), advanced
//!   by `update_yield_index` and claimed via `claim_yield`.
//!
//! # Yield-accounting correctness
//! Balance changes (mint, burn, transfer in/out) settle each affected account's pending yield
//! at their *old* balance against the *current* index before the balance moves, and reset that
//! account's snapshot index. Skipping this step is a classic reward-accounting bug class: an
//! account could otherwise receive yield accrued before it held the position (buying in right
//! before a large index update) or lose yield it had already earned (transferring out right
//! after one). `_settle` is called on every path that changes a balance, not only on claim.

#![no_std]

use soroban_sdk::{
    contract, contracterror, contractclient, contractimpl, contracttype, panic_with_error,
    symbol_short, Address, Env, String,
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
}

// ---------------------------------------------------------------------------
// Storage key schema
// ---------------------------------------------------------------------------

#[contracttype]
pub enum DataKey {
    Admin,
    Minter,
    Permissioning,
    Oracle,
    Maturity,
    Name,
    Symbol,
    Decimals,
    TotalSupply,
    Balance(Address),
    Allowance(Address, Address),
    YieldIndex,          // i128, scaled by SCALE
    LastOracleRate,       // i128, high-water mark used to advance YieldIndex
    LastClaimedIndex(Address), // i128, per-user snapshot of YieldIndex
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
    pub fn initialize(
        env: Env,
        admin: Address,
        permissioning: Address,
        oracle: Address,
        maturity: u64,
        name: String,
        symbol: String,
        decimals: u32,
    ) {
        if env.storage().instance().has(&DataKey::Admin) {
            panic_with_error!(&env, Error::AlreadyInitialized);
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage()
            .instance()
            .set(&DataKey::Permissioning, &permissioning);
        env.storage().instance().set(&DataKey::Oracle, &oracle);
        env.storage().instance().set(&DataKey::Maturity, &maturity);
        env.storage().instance().set(&DataKey::Name, &name);
        env.storage().instance().set(&DataKey::Symbol, &symbol);
        env.storage().instance().set(&DataKey::Decimals, &decimals);
        env.storage().instance().set(&DataKey::TotalSupply, &0_i128);
        env.storage().instance().set(&DataKey::YieldIndex, &0_i128);
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
        env.events()
            .publish((symbol_short!("min_set"),), minter);
    }

    // --- SEP-41 token interface ---

    pub fn transfer(env: Env, from: Address, to: Address, amount: i128) {
        from.require_auth();
        if amount <= 0 {
            panic_with_error!(&env, Error::ZeroAmount);
        }
        // Both sides checked — see PTToken::transfer for why checking only `to` would let a
        // revoked holder dump YT before being frozen.
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
        env.storage().instance().get(&DataKey::Decimals).unwrap_or(0)
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
        Self::assert_permitted(&env, &to);
        Self::settle(&env, &to);

        let bal = Self::get_balance(&env, &to);
        Self::set_balance(&env, &to, bal + amount);
        let total: i128 = env.storage().instance().get(&DataKey::TotalSupply).unwrap_or(0);
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
        let total: i128 = env.storage().instance().get(&DataKey::TotalSupply).unwrap_or(0);
        env.storage()
            .instance()
            .set(&DataKey::TotalSupply, &(total - amount));

        env.events()
            .publish((symbol_short!("burn"),), (from, amount));
    }

    // --- yield accrual ---

    /// Permissionless: advances the global yield index using the current oracle rate.
    /// No-op if the rate hasn't increased since the last recorded high-water mark, matching
    /// the protocol-wide invariant that YT never accrues negative yield.
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
            let delta_index = (now_rate - last_rate) * SCALE / now_rate;
            let idx: i128 = env.storage().instance().get(&DataKey::YieldIndex).unwrap_or(0);
            env.storage()
                .instance()
                .set(&DataKey::YieldIndex, &(idx + delta_index));
            env.storage()
                .instance()
                .set(&DataKey::LastOracleRate, &now_rate);
            env.events()
                .publish((symbol_short!("idx_up"),), (idx + delta_index, now_rate));
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

    pub fn accrued_yield_index(env: Env) -> i128 {
        env.storage().instance().get(&DataKey::YieldIndex).unwrap_or(0)
    }

    pub fn last_claimed_index(env: Env, account: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::LastClaimedIndex(account))
            .unwrap_or(0)
    }

    pub fn pending_claim(env: Env, account: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::PendingClaim(account))
            .unwrap_or(0)
    }

    // --- views ---

    pub fn total_supply(env: Env) -> i128 {
        env.storage().instance().get(&DataKey::TotalSupply).unwrap_or(0)
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

    // --- internal helpers ---

    /// Settle `account`'s pending yield at its balance *before* any change, against the
    /// current global index, then advance its snapshot to the current index. Must be called
    /// on every path that mutates a balance (mint/burn/transfer, both sides), before the
    /// balance itself changes.
    fn settle(env: &Env, account: &Address) {
        let idx: i128 = env.storage().instance().get(&DataKey::YieldIndex).unwrap_or(0);
        let last_key = DataKey::LastClaimedIndex(account.clone());
        let last: i128 = env.storage().persistent().get(&last_key).unwrap_or(0);

        if idx > last {
            let bal = Self::get_balance(env, account);
            if bal > 0 {
                let pending = bal * (idx - last) / SCALE;
                if pending > 0 {
                    let pc_key = DataKey::PendingClaim(account.clone());
                    let acc: i128 = env.storage().persistent().get(&pc_key).unwrap_or(0);
                    env.storage().persistent().set(&pc_key, &(acc + pending));
                    env.storage()
                        .persistent()
                        .extend_ttl(&pc_key, BALANCE_TTL_LEDGERS, BALANCE_TTL_LEDGERS);
                }
            }
        }
        env.storage().persistent().set(&last_key, &idx);
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

    fn require_minter(env: &Env) -> Address {
        env.storage()
            .instance()
            .get(&DataKey::Minter)
            .unwrap_or_else(|| panic_with_error!(env, Error::MinterNotSet))
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
        testutils::{Address as _, Ledger as _},
        Address, Env, String,
    };

    use principal_oracle_adapter::{OracleAdapterContract, OracleAdapterContractClient};
    use principal_permissioning::{PermissioningContract, PermissioningContractClient};

    use super::{YTTokenContract, YTTokenContractClient, SCALE};

    const T0: u64 = 1_000;

    struct Fixture {
        env: Env,
        client: YTTokenContractClient<'static>,
        admin: Address,
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

        let yt_id = env.register_contract(None, YTTokenContract);
        let client = YTTokenContractClient::new(&env, &yt_id);
        let admin = Address::generate(&env);
        client.initialize(
            &admin,
            &perm_id,
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
    fn no_yield_without_rate_increase() {
        let f = setup();
        let minter = Address::generate(&f.env);
        f.client.set_minter(&f.admin, &minter);
        let user = Address::generate(&f.env);
        grant(&f, &user);
        f.client.mint(&user, &(1_000 * SCALE));

        f.client.update_yield_index(); // rate unchanged since inception
        assert_eq!(f.client.accrued_yield_index(), 0);
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

        // delta_index = (10_300_000 - 10_000_000) * SCALE / 10_300_000
        let expected_index = (10_300_000_i128 - SCALE) * SCALE / 10_300_000_i128;
        assert_eq!(f.client.accrued_yield_index(), expected_index);

        let claimed = f.client.claim_yield(&user);
        let expected_claim = (1_000 * SCALE) * expected_index / SCALE;
        assert_eq!(claimed, expected_claim);

        // Claiming again immediately yields nothing further.
        assert_eq!(f.client.claim_yield(&user), 0);
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
    fn revoked_holder_cannot_dump_yt_before_remediation() {
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
    fn update_yield_index_blocked_by_stale_oracle() {
        // Without a freshness check, this would silently advance the accrual index off a rate
        // the oracle relay stopped refreshing long ago.
        let f = setup();
        // Ledger advances far past the oracle's last update without a fresh price ever landing.
        f.env
            .ledger()
            .with_mut(|li| li.timestamp = T0 + 3_601);
        f.client.update_yield_index();
    }
}
