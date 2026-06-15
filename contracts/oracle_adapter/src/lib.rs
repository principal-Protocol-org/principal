#![no_std]

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, panic_with_error, symbol_short, Address,
    Env,
};

#[contracterror]
#[derive(Copy, Clone, Debug, PartialEq)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    Unauthorized = 2,
    InvalidValue = 3,
    TimestampTooOld = 4,
    NotInitialized = 5,
}

#[contracttype]
pub enum DataKey {
    Admin,
    Price,
    Timestamp,
}

#[contract]
pub struct OracleAdapterContract;

#[contractimpl]
impl OracleAdapterContract {
    pub fn initialize(env: Env, admin: Address) {
        if env.storage().instance().has(&DataKey::Admin) {
            panic_with_error!(&env, Error::AlreadyInitialized);
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
    }

    /// Update the reference value. `caller` must match the stored admin and must authorize.
    pub fn set_reference_value(env: Env, caller: Address, value: i128, timestamp: u64) {
        caller.require_auth();
        let admin: Address = Self::require_admin(&env);
        if caller != admin {
            panic_with_error!(&env, Error::Unauthorized);
        }
        if value <= 0 {
            panic_with_error!(&env, Error::InvalidValue);
        }
        let current_ts: u64 = env
            .storage()
            .instance()
            .get(&DataKey::Timestamp)
            .unwrap_or(0u64);
        if timestamp <= current_ts {
            panic_with_error!(&env, Error::TimestampTooOld);
        }
        env.storage().instance().set(&DataKey::Price, &value);
        env.storage().instance().set(&DataKey::Timestamp, &timestamp);
        env.events()
            .publish((symbol_short!("ref_set"),), (value, timestamp));
    }

    pub fn get_reference_value(env: Env) -> i128 {
        env.storage()
            .instance()
            .get(&DataKey::Price)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized))
    }

    pub fn get_reference_timestamp(env: Env) -> u64 {
        env.storage()
            .instance()
            .get(&DataKey::Timestamp)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized))
    }

    /// Returns true when the stored timestamp is within `max_stale_seconds` of the ledger clock.
    /// Uses `env.ledger().timestamp()` — callers cannot manipulate this value.
    pub fn is_fresh(env: Env, max_stale_seconds: u64) -> bool {
        let stored_ts: u64 = env
            .storage()
            .instance()
            .get(&DataKey::Timestamp)
            .unwrap_or(0u64);
        let ledger_ts = env.ledger().timestamp();
        if ledger_ts < stored_ts {
            return false;
        }
        ledger_ts - stored_ts <= max_stale_seconds
    }

    /// Two-step admin transfer: current admin authorizes and names a new admin.
    pub fn transfer_admin(env: Env, current_admin: Address, new_admin: Address) {
        current_admin.require_auth();
        let admin: Address = Self::require_admin(&env);
        if current_admin != admin {
            panic_with_error!(&env, Error::Unauthorized);
        }
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
}

#[cfg(test)]
mod test {
    use soroban_sdk::{
        testutils::{Address as _, Ledger as _},
        Address, Env,
    };

    use super::{OracleAdapterContract, OracleAdapterContractClient};

    fn setup() -> (Env, OracleAdapterContractClient<'static>, Address) {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register_contract(None, OracleAdapterContract);
        let client = OracleAdapterContractClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        client.initialize(&admin);
        (env, client, admin)
    }

    #[test]
    fn initialize_and_set_price() {
        let (_env, client, admin) = setup();
        client.set_reference_value(&admin, &103_00000000_i128, &1_700_000_000_u64);
        assert_eq!(client.get_reference_value(), 103_00000000_i128);
        assert_eq!(client.get_reference_timestamp(), 1_700_000_000_u64);
    }

    #[test]
    #[should_panic]
    fn double_initialize_panics() {
        let (_env, client, admin) = setup();
        client.initialize(&admin);
    }

    #[test]
    #[should_panic]
    fn unauthorized_price_update_panics() {
        let (env, client, admin) = setup();
        client.set_reference_value(&admin, &103_00000000_i128, &1_700_000_000_u64);
        let attacker = Address::generate(&env);
        client.set_reference_value(&attacker, &200_00000000_i128, &1_700_001_000_u64);
    }

    #[test]
    #[should_panic]
    fn stale_timestamp_rejected() {
        let (_env, client, admin) = setup();
        client.set_reference_value(&admin, &103_00000000_i128, &1_700_000_000_u64);
        client.set_reference_value(&admin, &103_00000000_i128, &1_700_000_000_u64);
    }

    #[test]
    #[should_panic]
    fn invalid_price_rejected() {
        let (_env, client, admin) = setup();
        client.set_reference_value(&admin, &0_i128, &1_700_000_000_u64);
    }

    #[test]
    fn admin_transfer() {
        let (env, client, admin) = setup();
        let new_admin = Address::generate(&env);
        client.transfer_admin(&admin, &new_admin);
        assert_eq!(client.get_admin(), new_admin);
    }

    #[test]
    fn is_fresh_within_staleness_window() {
        let (env, client, admin) = setup();
        // Ledger at t=1000; oracle set at t=900 → diff=100 ≤ 3600 → fresh.
        env.ledger().with_mut(|li| li.timestamp = 1_000);
        client.set_reference_value(&admin, &10_300_000_i128, &900_u64);
        assert!(client.is_fresh(&3_600_u64));
    }

    #[test]
    fn is_fresh_false_when_stale() {
        let (env, client, admin) = setup();
        // Ledger at t=1000; oracle set at t=1 → diff=999 > 100 → stale.
        env.ledger().with_mut(|li| li.timestamp = 1_000);
        client.set_reference_value(&admin, &10_300_000_i128, &1_u64);
        assert!(!client.is_fresh(&100_u64));
    }

    #[test]
    fn is_fresh_false_before_any_price_set() {
        let (env, client, _admin) = setup();
        // No price ever set → stored timestamp = 0; ledger = 5000; diff > 3600 → not fresh.
        env.ledger().with_mut(|li| li.timestamp = 5_000);
        assert!(!client.is_fresh(&3_600_u64));
    }

    #[test]
    fn is_fresh_at_exact_boundary() {
        let (env, client, admin) = setup();
        // diff == max_stale_seconds exactly → still fresh (≤ not <).
        env.ledger().with_mut(|li| li.timestamp = 1_000);
        client.set_reference_value(&admin, &10_000_000_i128, &600_u64); // diff = 400
        assert!(client.is_fresh(&400_u64)); // 400 ≤ 400 → fresh
        assert!(!client.is_fresh(&399_u64)); // 400 > 399 → stale
    }
}
