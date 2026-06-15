#![no_std]

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, panic_with_error, symbol_short, Address,
    Env, Vec,
};

#[contracterror]
#[derive(Copy, Clone, Debug, PartialEq)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    Unauthorized = 2,
    NotInitialized = 3,
}

/// Storage key schema.
///
/// Account eligibility and per-asset eligibility are stored in `persistent` storage because they
/// are per-user entries that must survive across ledger closings but should be expired when no
/// longer needed. Protocol config (admin) is stored in `instance` storage.
#[contracttype]
pub enum DataKey {
    Admin,
    /// Global account allow-list: (AccountAllowed, address) → bool
    AccountAllowed(Address),
    /// Per-asset allow-list: (AssetAllowed, address, asset) → bool
    AssetAllowed(Address, Address),
}

/// How long (in ledgers) a newly written eligibility entry stays alive without extension.
/// ~30 days at 5s/ledger.
const ELIGIBILITY_TTL_LEDGERS: u32 = 518_400;

#[contract]
pub struct PermissioningContract;

#[contractimpl]
impl PermissioningContract {
    pub fn initialize(env: Env, admin: Address) {
        if env.storage().instance().has(&DataKey::Admin) {
            panic_with_error!(&env, Error::AlreadyInitialized);
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
    }

    // --- account-level eligibility ---

    pub fn grant_account(env: Env, caller: Address, account: Address) {
        Self::assert_admin(&env, &caller);
        let key = DataKey::AccountAllowed(account.clone());
        env.storage().persistent().set(&key, &true);
        env.storage()
            .persistent()
            .extend_ttl(&key, ELIGIBILITY_TTL_LEDGERS, ELIGIBILITY_TTL_LEDGERS);
        env.events()
            .publish((symbol_short!("acc_grant"),), (caller, account));
    }

    pub fn revoke_account(env: Env, caller: Address, account: Address) {
        Self::assert_admin(&env, &caller);
        env.storage()
            .persistent()
            .set(&DataKey::AccountAllowed(account.clone()), &false);
        env.events()
            .publish((symbol_short!("acc_rev"),), (caller, account));
    }

    /// Batch-grant multiple accounts in a single transaction to reduce overhead.
    pub fn grant_accounts(env: Env, caller: Address, accounts: Vec<Address>) {
        Self::assert_admin(&env, &caller);
        for account in accounts.iter() {
            let key = DataKey::AccountAllowed(account.clone());
            env.storage().persistent().set(&key, &true);
            env.storage()
                .persistent()
                .extend_ttl(&key, ELIGIBILITY_TTL_LEDGERS, ELIGIBILITY_TTL_LEDGERS);
        }
        env.events()
            .publish((symbol_short!("acc_batch"),), caller);
    }

    pub fn is_allowed(env: Env, account: Address) -> bool {
        env.storage()
            .persistent()
            .get(&DataKey::AccountAllowed(account))
            .unwrap_or(false)
    }

    // --- per-asset eligibility ---

    pub fn grant_asset(env: Env, caller: Address, account: Address, asset: Address) {
        Self::assert_admin(&env, &caller);
        let key = DataKey::AssetAllowed(account.clone(), asset.clone());
        env.storage().persistent().set(&key, &true);
        env.storage()
            .persistent()
            .extend_ttl(&key, ELIGIBILITY_TTL_LEDGERS, ELIGIBILITY_TTL_LEDGERS);
        env.events()
            .publish((symbol_short!("ast_grant"),), (caller, account, asset));
    }

    pub fn revoke_asset(env: Env, caller: Address, account: Address, asset: Address) {
        Self::assert_admin(&env, &caller);
        env.storage()
            .persistent()
            .set(&DataKey::AssetAllowed(account.clone(), asset.clone()), &false);
        env.events()
            .publish((symbol_short!("ast_rev"),), (caller, account, asset));
    }

    pub fn is_allowed_for_asset(env: Env, account: Address, asset: Address) -> bool {
        env.storage()
            .persistent()
            .get(&DataKey::AssetAllowed(account, asset))
            .unwrap_or(false)
    }

    // --- admin ---

    pub fn transfer_admin(env: Env, current_admin: Address, new_admin: Address) {
        Self::assert_admin(&env, &current_admin);
        env.storage().instance().set(&DataKey::Admin, &new_admin);
        env.events()
            .publish((symbol_short!("adm_xfer"),), (current_admin, new_admin));
    }

    pub fn get_admin(env: Env) -> Address {
        Self::require_admin(&env)
    }

    // --- internal helpers ---

    fn require_admin(env: &Env) -> Address {
        env.storage()
            .instance()
            .get(&DataKey::Admin)
            .unwrap_or_else(|| panic_with_error!(env, Error::NotInitialized))
    }

    fn assert_admin(env: &Env, caller: &Address) {
        caller.require_auth();
        let admin = Self::require_admin(env);
        if *caller != admin {
            panic_with_error!(env, Error::Unauthorized);
        }
    }
}

#[cfg(test)]
mod test {
    use soroban_sdk::{testutils::Address as _, vec, Address, Env};

    use super::{PermissioningContract, PermissioningContractClient};

    fn setup() -> (Env, PermissioningContractClient<'static>, Address) {
        let env = Env::default();
        env.mock_all_auths();
        let id = env.register_contract(None, PermissioningContract);
        let client = PermissioningContractClient::new(&env, &id);
        let admin = Address::generate(&env);
        client.initialize(&admin);
        (env, client, admin)
    }

    #[test]
    fn grant_and_check_account() {
        let (env, client, admin) = setup();
        let user = Address::generate(&env);
        assert!(!client.is_allowed(&user));
        client.grant_account(&admin, &user);
        assert!(client.is_allowed(&user));
    }

    #[test]
    fn revoke_account() {
        let (env, client, admin) = setup();
        let user = Address::generate(&env);
        client.grant_account(&admin, &user);
        client.revoke_account(&admin, &user);
        assert!(!client.is_allowed(&user));
    }

    #[test]
    fn grant_and_revoke_asset() {
        let (env, client, admin) = setup();
        let user = Address::generate(&env);
        let asset = Address::generate(&env);
        client.grant_asset(&admin, &user, &asset);
        assert!(client.is_allowed_for_asset(&user, &asset));
        client.revoke_asset(&admin, &user, &asset);
        assert!(!client.is_allowed_for_asset(&user, &asset));
    }

    #[test]
    fn batch_grant_accounts() {
        let (env, client, admin) = setup();
        let u1 = Address::generate(&env);
        let u2 = Address::generate(&env);
        let u3 = Address::generate(&env);
        client.grant_accounts(&admin, &vec![&env, u1.clone(), u2.clone(), u3.clone()]);
        assert!(client.is_allowed(&u1));
        assert!(client.is_allowed(&u2));
        assert!(client.is_allowed(&u3));
    }

    #[test]
    #[should_panic]
    fn unauthorized_grant_panics() {
        let (env, client, _admin) = setup();
        let attacker = Address::generate(&env);
        client.grant_account(&attacker, &Address::generate(&env));
    }

    #[test]
    fn admin_transfer() {
        let (env, client, admin) = setup();
        let new_admin = Address::generate(&env);
        client.transfer_admin(&admin, &new_admin);
        assert_eq!(client.get_admin(), new_admin);
    }
}
