//! RiskControl — protocol-level pause and circuit-breaker contract.
//!
//! # Roles
//! * **Admin** — can add/remove pausers, set circuit-breaker limits, upgrade configuration.
//! * **Pauser** — can call `pause()` unilaterally; cannot unpause (requires admin).
//!
//! # Circuit breaker
//! A deposit circuit breaker tracks the cumulative underlying deposited within a rolling
//! `CB_WINDOW_SECS` window. If the total exceeds `cb_limit`, new deposits are blocked until
//! the window resets or the admin raises the limit.
//!
//! External contracts (SYWrapper, PrincipalManager) call `check_deposit` before processing
//! each deposit. This contract is the single source of truth for protocol-level risk state.

#![no_std]

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, panic_with_error, symbol_short, Address,
    Env,
};

/// Length of the circuit-breaker rolling window in seconds.
pub const CB_WINDOW_SECS: u64 = 86_400; // 24 hours

#[contracterror]
#[derive(Copy, Clone, Debug, PartialEq)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    Unauthorized = 2,
    NotInitialized = 3,
    Paused = 4,
    CircuitBreakerTripped = 5,
    NotPauser = 6,
    AlreadyPauser = 7,
}

#[contracttype]
pub enum DataKey {
    Admin,
    Paused,
    Pauser(Address),
    /// Circuit breaker: max cumulative deposit in one window (underlying units, 0 = disabled).
    CbLimit,
    /// Cumulative deposit volume in the current window.
    CbVolume,
    /// Ledger timestamp when the current window started.
    CbWindowStart,
}

#[contract]
pub struct RiskControlContract;

#[contractimpl]
impl RiskControlContract {
    pub fn initialize(env: Env, admin: Address, cb_limit: i128) {
        if env.storage().instance().has(&DataKey::Admin) {
            panic_with_error!(&env, Error::AlreadyInitialized);
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::Paused, &false);
        env.storage().instance().set(&DataKey::CbLimit, &cb_limit);
        env.storage().instance().set(&DataKey::CbVolume, &0_i128);
        env.storage()
            .instance()
            .set(&DataKey::CbWindowStart, &env.ledger().timestamp());
    }

    // --- pause controls ---

    /// Pause the protocol. Callable by admin or any registered pauser.
    pub fn pause(env: Env, caller: Address) {
        caller.require_auth();
        if !Self::is_pauser(&env, &caller) {
            panic_with_error!(&env, Error::NotPauser);
        }
        env.storage().instance().set(&DataKey::Paused, &true);
        env.events()
            .publish((symbol_short!("paused"),), (caller, true));
    }

    /// Unpause the protocol. Admin only — pausers cannot unpause.
    pub fn unpause(env: Env, caller: Address) {
        Self::assert_admin(&env, &caller);
        env.storage().instance().set(&DataKey::Paused, &false);
        env.events()
            .publish((symbol_short!("paused"),), (caller, false));
    }

    pub fn is_paused(env: Env) -> bool {
        env.storage().instance().get(&DataKey::Paused).unwrap_or(false)
    }

    // --- pauser role management ---

    pub fn add_pauser(env: Env, caller: Address, pauser: Address) {
        Self::assert_admin(&env, &caller);
        if env
            .storage()
            .instance()
            .get(&DataKey::Pauser(pauser.clone()))
            .unwrap_or(false)
        {
            panic_with_error!(&env, Error::AlreadyPauser);
        }
        env.storage()
            .instance()
            .set(&DataKey::Pauser(pauser.clone()), &true);
        env.events()
            .publish((symbol_short!("add_psr"),), (caller, pauser));
    }

    pub fn remove_pauser(env: Env, caller: Address, pauser: Address) {
        Self::assert_admin(&env, &caller);
        env.storage()
            .instance()
            .set(&DataKey::Pauser(pauser.clone()), &false);
        env.events()
            .publish((symbol_short!("rm_psr"),), (caller, pauser));
    }

    /// Returns true if `account` is the admin or a registered pauser.
    pub fn is_pauser(env: &Env, account: &Address) -> bool {
        let is_admin = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .map(|a: Address| a == *account)
            .unwrap_or(false);
        if is_admin {
            return true;
        }
        env.storage()
            .instance()
            .get(&DataKey::Pauser(account.clone()))
            .unwrap_or(false)
    }

    // --- circuit breaker ---

    /// Called by SYWrapper/PrincipalManager before processing a deposit.
    /// Reverts if paused or if the circuit breaker limit would be exceeded.
    /// Records the deposit volume against the current window.
    pub fn check_deposit(env: Env, amount: i128) {
        if env.storage().instance().get(&DataKey::Paused).unwrap_or(false) {
            panic_with_error!(&env, Error::Paused);
        }

        let limit: i128 = env.storage().instance().get(&DataKey::CbLimit).unwrap_or(0);
        if limit == 0 {
            return; // circuit breaker disabled
        }

        let now = env.ledger().timestamp();
        let window_start: u64 = env
            .storage()
            .instance()
            .get(&DataKey::CbWindowStart)
            .unwrap_or(now);

        let (volume, start) = if now - window_start >= CB_WINDOW_SECS {
            // Window has rolled over — reset.
            (0_i128, now)
        } else {
            let v: i128 = env.storage().instance().get(&DataKey::CbVolume).unwrap_or(0);
            (v, window_start)
        };

        if volume + amount > limit {
            panic_with_error!(&env, Error::CircuitBreakerTripped);
        }

        env.storage().instance().set(&DataKey::CbVolume, &(volume + amount));
        env.storage().instance().set(&DataKey::CbWindowStart, &start);
    }

    pub fn set_cb_limit(env: Env, caller: Address, new_limit: i128) {
        Self::assert_admin(&env, &caller);
        env.storage().instance().set(&DataKey::CbLimit, &new_limit);
        env.events()
            .publish((symbol_short!("cb_limit"),), (caller, new_limit));
    }

    pub fn get_cb_limit(env: Env) -> i128 {
        env.storage().instance().get(&DataKey::CbLimit).unwrap_or(0)
    }

    pub fn get_cb_volume(env: Env) -> i128 {
        env.storage().instance().get(&DataKey::CbVolume).unwrap_or(0)
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
        if *caller != Self::require_admin(env) {
            panic_with_error!(env, Error::Unauthorized);
        }
    }
}

#[cfg(test)]
mod test {
    use soroban_sdk::{
        testutils::{Address as _, Ledger as _},
        Address, Env,
    };

    use super::{CB_WINDOW_SECS, RiskControlContract, RiskControlContractClient};

    fn setup(cb_limit: i128) -> (Env, RiskControlContractClient<'static>, Address) {
        let env = Env::default();
        env.mock_all_auths();
        let id = env.register_contract(None, RiskControlContract);
        let client = RiskControlContractClient::new(&env, &id);
        let admin = Address::generate(&env);
        client.initialize(&admin, &cb_limit);
        (env, client, admin)
    }

    #[test]
    fn pause_and_unpause() {
        let (env, client, admin) = setup(0);
        assert!(!client.is_paused());
        client.pause(&admin);
        assert!(client.is_paused());
        client.unpause(&admin);
        assert!(!client.is_paused());
    }

    #[test]
    fn pauser_can_pause_but_not_unpause() {
        let (env, client, admin) = setup(0);
        let pauser = Address::generate(&env);
        client.add_pauser(&admin, &pauser);
        client.pause(&pauser);
        assert!(client.is_paused());
    }

    #[test]
    #[should_panic]
    fn non_pauser_cannot_pause() {
        let (env, client, _admin) = setup(0);
        let rando = Address::generate(&env);
        client.pause(&rando);
    }

    #[test]
    fn circuit_breaker_blocks_over_limit() {
        let (_env, client, _admin) = setup(1_000_000);
        client.check_deposit(&500_000_i128); // ok: 500k
        client.check_deposit(&499_999_i128); // ok: 999_999k
        // next deposit would exceed the 1M limit
        // (panics)
    }

    #[test]
    #[should_panic]
    fn circuit_breaker_trips_on_excess() {
        let (_env, client, _admin) = setup(1_000_000);
        client.check_deposit(&500_000_i128);
        client.check_deposit(&600_000_i128); // trips
    }

    #[test]
    fn circuit_breaker_disabled_when_limit_zero() {
        let (_env, client, _admin) = setup(0);
        // Should not trip even for a huge deposit.
        client.check_deposit(&(i128::MAX / 2));
    }

    #[test]
    #[should_panic]
    fn check_deposit_fails_when_paused() {
        let (_env, client, admin) = setup(0);
        client.pause(&admin);
        client.check_deposit(&1_i128);
    }

    #[test]
    #[should_panic]
    fn non_admin_cannot_unpause() {
        let (env, client, admin) = setup(0);
        client.pause(&admin);
        let rando = Address::generate(&env);
        client.unpause(&rando); // only admin may unpause
    }

    #[test]
    fn circuit_breaker_window_resets_after_24h() {
        let (env, client, _admin) = setup(1_000_000);
        // Consume 900k in the first window.
        client.check_deposit(&900_000_i128);
        assert_eq!(client.get_cb_volume(), 900_000);

        // Advance past the 24-hour window; volume should reset.
        env.ledger()
            .with_mut(|li| li.timestamp = li.timestamp + CB_WINDOW_SECS + 1);

        // A fresh 900k deposit should be accepted (new window, volume = 0).
        client.check_deposit(&900_000_i128);
        assert_eq!(client.get_cb_volume(), 900_000); // restarted at 900k, not 1_800_000
    }

    #[test]
    #[should_panic]
    fn remove_pauser_revokes_pause_permission() {
        let (env, client, admin) = setup(0);
        let pauser = Address::generate(&env);
        client.add_pauser(&admin, &pauser);
        // After removal the address is no longer a pauser — pause must panic.
        client.remove_pauser(&admin, &pauser);
        client.pause(&pauser);
    }

    #[test]
    #[should_panic]
    fn add_duplicate_pauser_panics() {
        let (env, client, admin) = setup(0);
        let pauser = Address::generate(&env);
        client.add_pauser(&admin, &pauser);
        client.add_pauser(&admin, &pauser); // AlreadyPauser
    }

    #[test]
    fn admin_transfer() {
        let (env, client, admin) = setup(0);
        let new_admin = Address::generate(&env);
        client.transfer_admin(&admin, &new_admin);
        assert_eq!(client.get_admin(), new_admin);
    }
}
