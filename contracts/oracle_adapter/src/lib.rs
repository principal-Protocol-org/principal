//! OracleAdapter — admin-submitted reference value feed with monotonic timestamps.
//!
//! # Why the reference value is monotonically non-decreasing
//! The entire PT/YT settlement model is built on the assumption that the reference rate only
//! goes up: `YTToken.update_yield_index` is a no-op unless `now_rate > last_rate` ("YT never
//! accrues negative yield"), and `PrincipalManager`'s PT formula (`pt_amount * SCALE /
//! final_rate`) implicitly assumes `final_rate` is at or above the rate at issuance -- if
//! `final_rate` could fall below it, that formula would release *more* underlying than was
//! ever deposited for that PT, a real insolvency path, not just a pricing inaccuracy. This
//! matches the reference asset this protocol targets first (Ondo's USDY, a Treasury-backed,
//! NAV-appreciating token whose per-unit redemption value does not decrease in normal
//! operation) but does not hold for every conceivable "reference value" an oracle could feed
//! in (a genuine market price, which can fall). Given the settlement math above already,
//! silently, assumes non-decreasing, this contract enforces it explicitly rather than leaving
//! it as an unenforced deployment assumption -- found during an audit review.

#![no_std]
#![allow(deprecated)] // `env.events().publish` / `register_contract`: migration to `#[contractevent]` is tracked separately; event topics are kept stable for indexers.

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
    ValueDecreased = 6,
    TimestampInFuture = 7,
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
        let current_price: i128 = env.storage().instance().get(&DataKey::Price).unwrap_or(0);
        if value < current_price {
            panic_with_error!(&env, Error::ValueDecreased);
        }
        let current_ts: u64 = env
            .storage()
            .instance()
            .get(&DataKey::Timestamp)
            .unwrap_or(0u64);
        if timestamp <= current_ts {
            panic_with_error!(&env, Error::TimestampTooOld);
        }
        // A timestamp ahead of the ledger clock (e.g. milliseconds instead of seconds) would make
        // the feed read as stale until real time caught up *and* lock out every correction, since
        // later timestamps must exceed it. Reject it at the door.
        if timestamp > env.ledger().timestamp() {
            panic_with_error!(&env, Error::TimestampInFuture);
        }
        env.storage().instance().set(&DataKey::Price, &value);
        env.storage()
            .instance()
            .set(&DataKey::Timestamp, &timestamp);
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

    /// Single-step admin transfer: current admin authorizes and immediately names a new admin
    /// (no separate acceptance step from the new admin), matching every other contract's
    /// transfer_admin in this codebase.
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
mod test;
