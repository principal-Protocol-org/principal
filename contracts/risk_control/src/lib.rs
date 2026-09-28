//! RiskControl — protocol-level pause and circuit-breaker contract.
//!
//! # Roles
//! * **Admin** — can add/remove pausers, add/remove consumers, set circuit-breaker limits.
//! * **Pauser** — can call `pause()` unilaterally; cannot unpause (requires admin).
//! * **Consumer** — a registered contract address (e.g. `SYWrapper`, `PrincipalManager`)
//!   authorized to call `check_deposit`. Set via `add_consumer`/`remove_consumer`, mirroring
//!   the `set_minter` pattern used on `PTToken`/`YTToken` -- except this contract is meant to
//!   be shared across multiple callers, so it's an allow-list rather than a single address.
//!
//! # Circuit breaker
//! Two independent limits are enforced on every `check_deposit`:
//!
//! * a **protocol-wide** limit (`cb_limit`) on the cumulative underlying deposited across every
//!   asset in the current window, and
//! * an optional **per-asset** limit (`set_asset_limit`) on the cumulative underlying deposited
//!   for one asset in that asset's current window.
//!
//! A limit of `0` disables that limit. If either would be exceeded the call reverts, so an
//! over-limit deposit or mint fails atomically inside the transaction that attempted it -- there
//! is no separate manual call.
//!
//! # Window measured in ledgers, not wall-clock time
//! Windows are measured in Stellar **ledger sequence numbers** (`env.ledger().sequence()`), not
//! `env.ledger().timestamp()`. The sequence advances by exactly one per closed ledger and cannot
//! be nudged by a validator's close-time choice the way a timestamp can, so a window cannot be
//! stretched or shrunk to squeeze an extra allowance through. The default window is
//! `DEFAULT_WINDOW_LEDGERS` (17,280 ledgers, about 24 hours at the ~5 s ledger close time); the
//! admin can change it with `set_window_ledgers`. A window starts at the first counted deposit
//! after the previous one lapsed and resets once `window_ledgers` ledgers have elapsed.
//!
//! External contracts (SYWrapper, PrincipalManager) call `check_deposit` from inside their own
//! `deposit`/`mint`, so the breaker is wired directly into the protocol operations. This contract
//! is the single source of truth for protocol-level risk state.
//! `check_deposit` requires the caller to be a registered consumer -- without this, anyone
//! could call it directly for an arbitrary amount to exhaust a day's circuit-breaker budget
//! and block every legitimate depositor, at zero cost beyond a transaction fee. Found during a
//! post-implementation audit, before this contract was ever wired into a real deposit path.

#![no_std]
#![allow(deprecated)] // `env.events().publish` / `register_contract`: migration to `#[contractevent]` is tracked separately; event topics are kept stable for indexers.

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, panic_with_error, symbol_short, Address,
    Env,
};

/// Default length of a circuit-breaker window in ledgers (~24 h at 5 s per ledger).
pub const DEFAULT_WINDOW_LEDGERS: u32 = 17_280;

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
    NotConsumer = 8,
    AlreadyConsumer = 9,
    ZeroAmount = 10,
    AssetLimitTripped = 11,
    InvalidLimit = 12,
    InvalidWindow = 13,
}

#[contracttype]
pub enum DataKey {
    Admin,
    Paused,
    Pauser(Address),
    /// Registered caller authorized to invoke `check_deposit` (e.g. SYWrapper, PrincipalManager).
    Consumer(Address),
    /// Circuit breaker: max cumulative deposit in one window (underlying units, 0 = disabled).
    CbLimit,
    /// Cumulative deposit volume, across all assets, in the current window.
    CbVolume,
    /// Ledger sequence at which the current protocol-wide window started.
    CbWindowStart,
    /// Window length in ledgers.
    WindowLedgers,
    /// Per-asset limit (underlying units, 0 = none).
    AssetLimit(Address),
    /// Per-asset window: (start ledger, cumulative volume).
    AssetWindow(Address),
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
            .set(&DataKey::CbWindowStart, &env.ledger().sequence());
        env.storage()
            .instance()
            .set(&DataKey::WindowLedgers, &DEFAULT_WINDOW_LEDGERS);
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
        env.storage()
            .instance()
            .get(&DataKey::Paused)
            .unwrap_or(false)
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

    // --- consumer registration (who may call check_deposit) ---

    /// Register `consumer` (e.g. SYWrapper, PrincipalManager) as authorized to call
    /// `check_deposit`. Same shape as `add_pauser`: reverts if already registered, catching
    /// accidental double-registration.
    pub fn add_consumer(env: Env, caller: Address, consumer: Address) {
        Self::assert_admin(&env, &caller);
        if env
            .storage()
            .instance()
            .get(&DataKey::Consumer(consumer.clone()))
            .unwrap_or(false)
        {
            panic_with_error!(&env, Error::AlreadyConsumer);
        }
        env.storage()
            .instance()
            .set(&DataKey::Consumer(consumer.clone()), &true);
        env.events()
            .publish((symbol_short!("add_cons"),), (caller, consumer));
    }

    pub fn remove_consumer(env: Env, caller: Address, consumer: Address) {
        Self::assert_admin(&env, &caller);
        env.storage()
            .instance()
            .set(&DataKey::Consumer(consumer.clone()), &false);
        env.events()
            .publish((symbol_short!("rm_cons"),), (caller, consumer));
    }

    pub fn is_consumer(env: Env, account: Address) -> bool {
        env.storage()
            .instance()
            .get(&DataKey::Consumer(account))
            .unwrap_or(false)
    }

    // --- circuit breaker ---

    /// Called by a registered consumer (SYWrapper/PrincipalManager) from inside its own
    /// `deposit`/`mint`. Reverts if the caller isn't registered, if paused, if `amount <= 0`, if
    /// the protocol-wide limit would be exceeded, or if `asset`'s own limit would be exceeded.
    /// Records the volume against both windows.
    ///
    /// `caller` must be a registered consumer, not just any authenticated address -- without
    /// this, anyone could call this directly with an arbitrary amount to exhaust the window's
    /// budget and block every legitimate depositor, at zero cost beyond a transaction fee.
    pub fn check_deposit(env: Env, caller: Address, asset: Address, amount: i128) {
        caller.require_auth();
        if !env
            .storage()
            .instance()
            .get(&DataKey::Consumer(caller))
            .unwrap_or(false)
        {
            panic_with_error!(&env, Error::NotConsumer);
        }

        if env
            .storage()
            .instance()
            .get(&DataKey::Paused)
            .unwrap_or(false)
        {
            panic_with_error!(&env, Error::Paused);
        }

        if amount <= 0 {
            panic_with_error!(&env, Error::ZeroAmount);
        }

        let now = env.ledger().sequence();
        let window = Self::window_ledgers(&env);

        // Evaluate both limits before writing either, so a trip leaves no partial state.
        let limit: i128 = env.storage().instance().get(&DataKey::CbLimit).unwrap_or(0);
        let global = if limit > 0 {
            let (volume, start) = Self::current_global_window(&env, now, window);
            let total = volume
                .checked_add(amount)
                .unwrap_or_else(|| panic_with_error!(&env, Error::CircuitBreakerTripped));
            if total > limit {
                panic_with_error!(&env, Error::CircuitBreakerTripped);
            }
            Some((total, start))
        } else {
            None
        };

        let asset_limit: i128 = env
            .storage()
            .instance()
            .get(&DataKey::AssetLimit(asset.clone()))
            .unwrap_or(0);
        let per_asset = if asset_limit > 0 {
            let (start, volume) = Self::current_asset_window(&env, &asset, now, window);
            let total = volume
                .checked_add(amount)
                .unwrap_or_else(|| panic_with_error!(&env, Error::AssetLimitTripped));
            if total > asset_limit {
                panic_with_error!(&env, Error::AssetLimitTripped);
            }
            Some((start, total))
        } else {
            None
        };

        if let Some((total, start)) = global {
            env.storage().instance().set(&DataKey::CbVolume, &total);
            env.storage()
                .instance()
                .set(&DataKey::CbWindowStart, &start);
        }
        if let Some(w) = per_asset {
            env.storage()
                .instance()
                .set(&DataKey::AssetWindow(asset), &w);
        }
    }

    /// Set the protocol-wide window limit (underlying units); `0` disables it.
    pub fn set_cb_limit(env: Env, caller: Address, new_limit: i128) {
        Self::assert_admin(&env, &caller);
        if new_limit < 0 {
            panic_with_error!(&env, Error::InvalidLimit);
        }
        env.storage().instance().set(&DataKey::CbLimit, &new_limit);
        env.events()
            .publish((symbol_short!("cb_limit"),), (caller, new_limit));
    }

    /// Set the limit for one underlying asset (underlying units); `0` disables it.
    pub fn set_asset_limit(env: Env, caller: Address, asset: Address, new_limit: i128) {
        Self::assert_admin(&env, &caller);
        if new_limit < 0 {
            panic_with_error!(&env, Error::InvalidLimit);
        }
        env.storage()
            .instance()
            .set(&DataKey::AssetLimit(asset.clone()), &new_limit);
        env.events()
            .publish((symbol_short!("ast_limit"),), (caller, asset, new_limit));
    }

    /// Change the window length, in ledgers. Takes effect for the next `check_deposit`.
    pub fn set_window_ledgers(env: Env, caller: Address, ledgers: u32) {
        Self::assert_admin(&env, &caller);
        if ledgers == 0 {
            panic_with_error!(&env, Error::InvalidWindow);
        }
        env.storage()
            .instance()
            .set(&DataKey::WindowLedgers, &ledgers);
        env.events()
            .publish((symbol_short!("cb_window"),), (caller, ledgers));
    }

    pub fn get_cb_limit(env: Env) -> i128 {
        env.storage().instance().get(&DataKey::CbLimit).unwrap_or(0)
    }

    /// Volume counted in the current protocol-wide window (`0` once that window has lapsed).
    pub fn get_cb_volume(env: Env) -> i128 {
        let now = env.ledger().sequence();
        Self::current_global_window(&env, now, Self::window_ledgers(&env)).0
    }

    pub fn get_asset_limit(env: Env, asset: Address) -> i128 {
        env.storage()
            .instance()
            .get(&DataKey::AssetLimit(asset))
            .unwrap_or(0)
    }

    /// Volume counted in `asset`'s current window (`0` once that window has lapsed).
    pub fn get_asset_volume(env: Env, asset: Address) -> i128 {
        let now = env.ledger().sequence();
        Self::current_asset_window(&env, &asset, now, Self::window_ledgers(&env)).1
    }

    pub fn get_window_ledgers(env: Env) -> u32 {
        Self::window_ledgers(&env)
    }

    /// Ledger sequence at which the current protocol-wide window started.
    pub fn get_window_start(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKey::CbWindowStart)
            .unwrap_or(0)
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

    fn window_ledgers(env: &Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKey::WindowLedgers)
            .unwrap_or(DEFAULT_WINDOW_LEDGERS)
    }

    /// (volume, start) of the protocol-wide window as of ledger `now`: a lapsed window reads as
    /// empty and restarts at `now`.
    fn current_global_window(env: &Env, now: u32, window: u32) -> (i128, u32) {
        let start: u32 = env
            .storage()
            .instance()
            .get(&DataKey::CbWindowStart)
            .unwrap_or(now);
        if now.saturating_sub(start) >= window {
            (0, now)
        } else {
            let v: i128 = env
                .storage()
                .instance()
                .get(&DataKey::CbVolume)
                .unwrap_or(0);
            (v, start)
        }
    }

    /// (start, volume) of `asset`'s window as of ledger `now`.
    fn current_asset_window(env: &Env, asset: &Address, now: u32, window: u32) -> (u32, i128) {
        let (start, volume): (u32, i128) = env
            .storage()
            .instance()
            .get(&DataKey::AssetWindow(asset.clone()))
            .unwrap_or((now, 0));
        if now.saturating_sub(start) >= window {
            (now, 0)
        } else {
            (start, volume)
        }
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
}

#[cfg(test)]
mod test;
