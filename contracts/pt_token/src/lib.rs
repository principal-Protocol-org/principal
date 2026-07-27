//! PTToken — standalone SEP-41 Principal Token.
//!
//! # Responsibilities
//! * Full SEP-41 token interface (transfer, transfer_from, approve, allowance, balance).
//! * Mint/burn restricted to a single registered minter (PrincipalManager), set once via
//!   `set_minter` after deployment to break the PTToken <-> PrincipalManager circular
//!   dependency (see TECHNICAL_SPECIFICATION.md §6, §17.1).
//! * Transfers gated by `Permissioning.is_allowed_for_asset(to, this_contract_address)`,
//!   layered on top of the coarser `Permissioning.is_allowed(to)` account-level gate.
//!
//! # Deviation from the original Phase 2 spec
//! TECHNICAL_SPECIFICATION.md §6.1 originally specified transfers checking only
//! `Permissioning.is_allowed(to)`. This implementation checks eligibility keyed to this
//! token's own contract address instead, so PTToken and YTToken can carry independent
//! eligibility policies (asymmetric permissioning) using allow-list infrastructure that
//! already exists — see PHASE2_DESIGN.md §1. The account-level gate still applies first;
//! per-asset eligibility narrows within it, it does not bypass it.

#![no_std]

use soroban_sdk::{
    contract, contracterror, contractclient, contractimpl, contracttype, panic_with_error,
    symbol_short, Address, Env, String,
};

/// TTL extension applied to every persistent per-user entry (~30 days at 5 s/ledger).
const BALANCE_TTL_LEDGERS: u32 = 518_400;

// ---------------------------------------------------------------------------
// External contract interfaces
// ---------------------------------------------------------------------------

#[contractclient(name = "PermClient")]
pub trait PermissioningInterface {
    fn is_allowed(env: Env, account: Address) -> bool;
    fn is_allowed_for_asset(env: Env, account: Address, asset: Address) -> bool;
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
}

// ---------------------------------------------------------------------------
// Storage key schema
// ---------------------------------------------------------------------------

#[contracttype]
pub enum DataKey {
    Admin,
    Minter,          // Option<Address>; absent until set_minter is called
    Permissioning,
    Maturity,        // u64 unix timestamp
    Name,
    Symbol,
    Decimals,
    TotalSupply,
    Balance(Address),
    Allowance(Address, Address), // (owner, spender) -> AllowanceValue
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
pub struct PTTokenContract;

#[contractimpl]
impl PTTokenContract {
    /// One-time initialization. `minter` is NOT set here — call `set_minter` once
    /// PrincipalManager is deployed, breaking the circular init dependency.
    pub fn initialize(
        env: Env,
        admin: Address,
        permissioning: Address,
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
        env.storage().instance().set(&DataKey::Maturity, &maturity);
        env.storage().instance().set(&DataKey::Name, &name);
        env.storage().instance().set(&DataKey::Symbol, &symbol);
        env.storage().instance().set(&DataKey::Decimals, &decimals);
        env.storage().instance().set(&DataKey::TotalSupply, &0_i128);
    }

    /// One-time minter registration, callable only by admin. Reverts if already set.
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
        // Both sides checked: if only `to` were gated, a revoked holder could freely dump PT
        // to any still-eligible party, defeating the point of eligibility enforcement (and
        // front-running any future PT-side clawback remediation) by moving out before being
        // frozen. Revocation freezes a holder's existing balance, not just new inbound transfers.
        Self::assert_permitted(&env, &from);
        Self::assert_permitted(&env, &to);

        let from_balance = Self::get_balance(&env, &from);
        if from_balance < amount {
            panic_with_error!(&env, Error::InsufficientBalance);
        }
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
        if allowance.amount < amount {
            panic_with_error!(&env, Error::InsufficientAllowance);
        }
        if allowance.expiration_ledger < env.ledger().sequence() {
            panic_with_error!(&env, Error::InsufficientAllowance);
        }

        let from_balance = Self::get_balance(&env, &from);
        if from_balance < amount {
            panic_with_error!(&env, Error::InsufficientBalance);
        }
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
        Self::set_balance(&env, &from, bal - amount);
        let total: i128 = env.storage().instance().get(&DataKey::TotalSupply).unwrap_or(0);
        env.storage()
            .instance()
            .set(&DataKey::TotalSupply, &(total - amount));

        env.events()
            .publish((symbol_short!("burn"),), (from, amount));
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
    use soroban_sdk::{testutils::Address as _, Address, Env, String};

    use principal_permissioning::{PermissioningContract, PermissioningContractClient};

    use super::{PTTokenContract, PTTokenContractClient};

    struct Fixture {
        env: Env,
        client: PTTokenContractClient<'static>,
        admin: Address,
        perm: PermissioningContractClient<'static>,
        perm_admin: Address,
        pt_id: Address,
    }

    fn setup() -> Fixture {
        let env = Env::default();
        env.mock_all_auths();

        let perm_id = env.register_contract(None, PermissioningContract);
        let perm = PermissioningContractClient::new(&env, &perm_id);
        let perm_admin = Address::generate(&env);
        perm.initialize(&perm_admin);

        let pt_id = env.register_contract(None, PTTokenContract);
        let client = PTTokenContractClient::new(&env, &pt_id);
        let admin = Address::generate(&env);
        client.initialize(
            &admin,
            &perm_id,
            &u64::MAX,
            &String::from_str(&env, "Principal Token USDY"),
            &String::from_str(&env, "PT-USDY"),
            &7,
        );

        Fixture {
            env,
            client,
            admin,
            perm,
            perm_admin,
            pt_id,
        }
    }

    fn grant(f: &Fixture, user: &Address) {
        f.perm.grant_account(&f.perm_admin, user);
        f.perm.grant_asset(&f.perm_admin, user, &f.pt_id);
    }

    #[test]
    fn mint_requires_minter_set() {
        let f = setup();
        let user = Address::generate(&f.env);
        grant(&f, &user);
        // No set_minter call yet -> mint must fail.
        let result = f.client.try_mint(&user, &100);
        assert!(result.is_err());
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
        assert_eq!(f.client.total_supply(), 1_000);
    }

    #[test]
    #[should_panic]
    fn set_minter_twice_panics() {
        let f = setup();
        let m1 = Address::generate(&f.env);
        let m2 = Address::generate(&f.env);
        f.client.set_minter(&f.admin, &m1);
        f.client.set_minter(&f.admin, &m2);
    }

    #[test]
    fn transfer_between_eligible_accounts() {
        let f = setup();
        let minter = Address::generate(&f.env);
        f.client.set_minter(&f.admin, &minter);

        let alice = Address::generate(&f.env);
        let bob = Address::generate(&f.env);
        grant(&f, &alice);
        grant(&f, &bob);

        f.client.mint(&alice, &500);
        f.client.transfer(&alice, &bob, &200);

        assert_eq!(f.client.balance(&alice), 300);
        assert_eq!(f.client.balance(&bob), 200);
    }

    #[test]
    #[should_panic]
    fn transfer_to_account_without_asset_grant_panics() {
        // Bob is on the global allow-list but was never granted PT specifically —
        // this is the asymmetric-permissioning enforcement path.
        let f = setup();
        let minter = Address::generate(&f.env);
        f.client.set_minter(&f.admin, &minter);

        let alice = Address::generate(&f.env);
        let bob = Address::generate(&f.env);
        grant(&f, &alice);
        f.perm.grant_account(&f.perm_admin, &bob); // account-level only, no PT asset grant

        f.client.mint(&alice, &500);
        f.client.transfer(&alice, &bob, &100);
    }

    #[test]
    #[should_panic]
    fn revoked_holder_cannot_dump_pt_before_remediation() {
        // If transfer only checked `to`, a revoked account could freely move its PT to any
        // still-eligible party the instant it suspected remediation was coming.
        let f = setup();
        let minter = Address::generate(&f.env);
        f.client.set_minter(&f.admin, &minter);

        let alice = Address::generate(&f.env);
        let bob = Address::generate(&f.env);
        grant(&f, &alice);
        grant(&f, &bob);
        f.client.mint(&alice, &500);

        f.perm.revoke_account(&f.perm_admin, &alice);
        f.client.transfer(&alice, &bob, &100); // bob is still fully eligible
    }

    #[test]
    #[should_panic]
    fn transfer_insufficient_balance_panics() {
        let f = setup();
        let minter = Address::generate(&f.env);
        f.client.set_minter(&f.admin, &minter);

        let alice = Address::generate(&f.env);
        let bob = Address::generate(&f.env);
        grant(&f, &alice);
        grant(&f, &bob);

        f.client.mint(&alice, &100);
        f.client.transfer(&alice, &bob, &200);
    }

    #[test]
    fn approve_and_transfer_from() {
        let f = setup();
        let minter = Address::generate(&f.env);
        f.client.set_minter(&f.admin, &minter);

        let alice = Address::generate(&f.env);
        let bob = Address::generate(&f.env);
        let spender = Address::generate(&f.env);
        grant(&f, &alice);
        grant(&f, &bob);

        f.client.mint(&alice, &500);
        f.client.approve(&alice, &spender, &300, &(f.env.ledger().sequence() + 100));
        f.client.transfer_from(&spender, &alice, &bob, &200);

        assert_eq!(f.client.balance(&alice), 300);
        assert_eq!(f.client.balance(&bob), 200);
        assert_eq!(f.client.allowance(&alice, &spender), 100);
    }

    #[test]
    fn burn_reduces_supply() {
        let f = setup();
        let minter = Address::generate(&f.env);
        f.client.set_minter(&f.admin, &minter);

        let user = Address::generate(&f.env);
        grant(&f, &user);
        f.client.mint(&user, &1_000);
        f.client.burn(&user, &400);

        assert_eq!(f.client.balance(&user), 600);
        assert_eq!(f.client.total_supply(), 600);
    }
}
