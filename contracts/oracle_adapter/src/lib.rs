//! OracleAdapter — admin-submitted reference value feed with monotonic timestamps, a primary
//! plus optional fallback source, a deviation guard, and explicit availability signaling.
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
//!
//! # Hard failures vs. graceful rejections
//! Two different kinds of bad `set_reference_value` call exist, and they are handled
//! differently because Soroban cannot do both at once: a panicking call reverts *every* storage
//! write and discards *every* event from that invocation, so "record why this was rejected,
//! then abort" is not something one call can do.
//!
//! - **Malformed or misconfigured calls panic** (`Unauthorized`, `InvalidValue`,
//!   `TimestampInFuture`, `InvalidSource`, `FallbackNotConfigured`, and the config-time
//!   `InvalidStaleness`/`InvalidDeviation`): these indicate a broken caller, not a legitimate
//!   price tick, and today's established pattern (unchanged from before this extension) is to
//!   revert outright so the mistake is visible immediately, the same way a malformed call to
//!   any other contract in this codebase fails loudly.
//! - **A syntactically valid submission from a recognized source that is simply rejected by
//!   policy** (`ValueDecreased`, `TimestampTooOld`, `DeviationTooHigh`) **does not panic.** It
//!   returns `false` from `set_reference_value`, leaves `Price`/`Timestamp` untouched, and
//!   records the attempt in `LastRejectedValue`/`LastRejectedTimestamp`/`LastRejectReason`
//!   (readable via `last_rejection()`) plus a `ref_rej` event. This is required for a public
//!   dashboard to show "last rejection reason" from a contract read (not from a private relay's
//!   own log), which is only possible if the rejection is actually committed on-chain -- and a
//!   genuinely routine event (a feed's transient bad tick, or a fallback heartbeat with a stale
//!   timestamp) should not look like a broken transaction in monitoring. A caller that wants a
//!   crisp revert instead of a boolean can call `assert_available()` beforehand, or simulate the
//!   call and inspect the return value before submitting.

#![no_std]
#![allow(deprecated)] // `env.events().publish` / `register_contract`: migration to `#[contractevent]` is tracked separately; event topics are kept stable for indexers.

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, panic_with_error, symbol_short, Address,
    BytesN, Env, Symbol,
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
    /// No fresh value from any configured source -- raised only by `assert_available()`; the
    /// usual way to observe this is reading `is_available()`, which returns `false` instead of
    /// panicking. Kept distinct from `ValueDecreased`/`TimestampTooOld`/`DeviationTooHigh`: this
    /// is about the *current* feed state, not about rejecting one submission.
    OracleUnavailable = 8,
    /// A new value (above the current one) moved by more than `max_deviation_bps`. Checked only
    /// for increases: a decrease is already rejected first by `ValueDecreased`, so this exists
    /// specifically to catch large upward jumps a compromised or malfunctioning feed could push.
    DeviationTooHigh = 9,
    /// `source` does not match the configured primary source, nor the configured fallback (if
    /// any).
    InvalidSource = 10,
    /// `max_deviation_bps` is zero (would reject every legitimate increase, bricking the feed)
    /// or above `BPS_SCALE` (100%, a meaningless ceiling).
    InvalidDeviation = 11,
    /// `max_stale_seconds` is zero (would make the feed permanently unavailable).
    InvalidStaleness = 12,
    /// `source` does not match the primary source, and no fallback source is configured at all
    /// -- so whatever the caller intended, it cannot be a fallback submission either.
    FallbackNotConfigured = 13,
    /// A submitted value fell outside the admin-configured sane range (`set_value_bounds`), if
    /// one has been set. Checked before the deviation guard and even for a value otherwise
    /// exempt from it (the first-ever submission) -- see `set_value_bounds`'s doc for why.
    ValueOutOfBounds = 14,
}

#[contracttype]
pub enum DataKey {
    Admin,
    Price,
    Timestamp,
    PrimarySource,
    FallbackSource,
    ActiveSource,
    MaxStaleSeconds,
    MaxDeviationBps,
    LastRejectedValue,
    LastRejectedTimestamp,
    LastRejectReason,
    MinSaneValue,
    MaxSaneValue,
}

/// The most recent `set_reference_value` call this contract refused, if any. `reason` is one of
/// a small fixed set of short symbols (`decrease`, `stale_ts`, `deviate`, `fb_value`) rather than
/// a free-form string, so a dashboard can map it to display text without parsing.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct OracleRejection {
    pub value: i128,
    pub timestamp: u64,
    pub reason: Symbol,
}

/// Percent-like parameters in this contract (`max_deviation_bps`) are expressed in basis points
/// of this scale, matching `MarketConfig`'s own `BPS` convention elsewhere in this codebase.
pub const BPS_SCALE: u32 = 10_000;

#[contract]
pub struct OracleAdapterContract;

#[contractimpl]
impl OracleAdapterContract {
    /// One-time setup. `max_stale_seconds` and `max_deviation_bps` are this feed's own safety
    /// policy (previously a caller-supplied argument to `is_fresh`, now centralized here so every
    /// consumer enforces the same thresholds without duplicating a constant). Sources are not set
    /// here -- call `set_sources` separately, before the first `set_reference_value`.
    pub fn initialize(env: Env, admin: Address, max_stale_seconds: u64, max_deviation_bps: u32) {
        if env.storage().instance().has(&DataKey::Admin) {
            panic_with_error!(&env, Error::AlreadyInitialized);
        }
        Self::validate_safety_params(&env, max_stale_seconds, max_deviation_bps);
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage()
            .instance()
            .set(&DataKey::MaxStaleSeconds, &max_stale_seconds);
        env.storage()
            .instance()
            .set(&DataKey::MaxDeviationBps, &max_deviation_bps);
    }

    /// Configure (or reconfigure) the primary and optional fallback source IDs. Each is a
    /// deterministic `BytesN<32>` -- for an off-chain feed, `sha256("principal-oracle-source:
    /// <network>:<asset>:<provider>:<feed>")` (see `docs/ORACLE_SAFETY_EXTENSION.md`); for an
    /// on-chain source, its contract address hash. Passing `fallback_source: None` removes any
    /// existing fallback.
    pub fn set_sources(
        env: Env,
        caller: Address,
        primary_source: BytesN<32>,
        fallback_source: Option<BytesN<32>>,
    ) {
        caller.require_auth();
        let admin = Self::require_admin(&env);
        if caller != admin {
            panic_with_error!(&env, Error::Unauthorized);
        }
        env.storage()
            .instance()
            .set(&DataKey::PrimarySource, &primary_source);
        match &fallback_source {
            Some(fb) => env.storage().instance().set(&DataKey::FallbackSource, fb),
            None => env.storage().instance().remove(&DataKey::FallbackSource),
        }
        env.events().publish(
            (symbol_short!("src_set"),),
            (primary_source, fallback_source),
        );
    }

    /// Update the feed's own safety thresholds without touching the configured sources.
    pub fn set_safety_params(
        env: Env,
        caller: Address,
        max_stale_seconds: u64,
        max_deviation_bps: u32,
    ) {
        caller.require_auth();
        let admin = Self::require_admin(&env);
        if caller != admin {
            panic_with_error!(&env, Error::Unauthorized);
        }
        Self::validate_safety_params(&env, max_stale_seconds, max_deviation_bps);
        env.storage()
            .instance()
            .set(&DataKey::MaxStaleSeconds, &max_stale_seconds);
        env.storage()
            .instance()
            .set(&DataKey::MaxDeviationBps, &max_deviation_bps);
    }

    /// Configure an optional sane absolute range checked against *every* accepted value,
    /// including the first-ever submission (which is otherwise exempt from the deviation guard,
    /// since there is nothing yet to deviate from). Unset by default -- no bound is enforced
    /// until this is called at least once.
    ///
    /// This exists to catch an operational mistake the deviation guard cannot: a relay
    /// decimal/scale bug (e.g. submitting an 18-decimal raw amount where the feed expects 8)
    /// that inflates the very first accepted value by many orders of magnitude. Once such a
    /// value were accepted, every subsequent *correct* submission would read as a `decrease`
    /// and be rejected forever -- there is no admin override that can move the stored value
    /// back down, because that would reopen exactly the insolvency path the monotonic-value
    /// invariant exists to close (see the module doc). A sane range catches the bad value before
    /// it is ever stored, which is the only safe place to catch it.
    pub fn set_value_bounds(env: Env, caller: Address, min_value: i128, max_value: i128) {
        caller.require_auth();
        let admin = Self::require_admin(&env);
        if caller != admin {
            panic_with_error!(&env, Error::Unauthorized);
        }
        if min_value <= 0 || max_value <= min_value {
            panic_with_error!(&env, Error::InvalidValue);
        }
        env.storage()
            .instance()
            .set(&DataKey::MinSaneValue, &min_value);
        env.storage()
            .instance()
            .set(&DataKey::MaxSaneValue, &max_value);
    }

    /// The configured `(min_value, max_value)` sane range, or `None` if `set_value_bounds` has
    /// never been called.
    pub fn value_bounds(env: Env) -> Option<(i128, i128)> {
        let min: Option<i128> = env.storage().instance().get(&DataKey::MinSaneValue);
        let max: Option<i128> = env.storage().instance().get(&DataKey::MaxSaneValue);
        match (min, max) {
            (Some(min), Some(max)) => Some((min, max)),
            _ => None,
        }
    }

    /// Submit a new value from `source`, which must be the configured primary or fallback.
    /// Returns `true` if accepted (`Price`/`Timestamp`/`ActiveSource` updated), `false` if a
    /// *legitimate* submission was rejected by policy (see the module doc for which outcomes
    /// panic instead). `caller` must match the stored admin and authorize.
    ///
    /// The fallback source may only refresh the timestamp of the *current* value (it must submit
    /// exactly the current price); it can never move the value itself. Primary-source updates get
    /// the full monotonic-value and deviation checks.
    pub fn set_reference_value(
        env: Env,
        caller: Address,
        source: BytesN<32>,
        value: i128,
        timestamp: u64,
    ) -> bool {
        caller.require_auth();
        let admin: Address = Self::require_admin(&env);
        if caller != admin {
            panic_with_error!(&env, Error::Unauthorized);
        }
        if value <= 0 {
            panic_with_error!(&env, Error::InvalidValue);
        }
        // Checked before anything else about this submission, including before the first-ever
        // value's exemption from the deviation guard below -- see `set_value_bounds`'s doc.
        if let Some((min, max)) = Self::value_bounds(env.clone()) {
            if value < min || value > max {
                return Self::reject(&env, value, timestamp, symbol_short!("oob"));
            }
        }
        // A timestamp ahead of the ledger clock (e.g. milliseconds instead of seconds) would make
        // the feed read as stale until real time caught up *and* lock out every correction, since
        // later timestamps must exceed it. Reject it at the door, unconditionally.
        if timestamp > env.ledger().timestamp() {
            panic_with_error!(&env, Error::TimestampInFuture);
        }

        let primary: Option<BytesN<32>> = env.storage().instance().get(&DataKey::PrimarySource);
        let fallback: Option<BytesN<32>> = env.storage().instance().get(&DataKey::FallbackSource);
        let is_primary = primary.as_ref() == Some(&source);
        let is_fallback = !is_primary && fallback.as_ref() == Some(&source);
        if !is_primary && !is_fallback {
            // Distinguish "nothing is configured at all" and "this matches neither configured
            // source" (both InvalidSource) from "this isn't primary, and there is no fallback
            // for it to be" (FallbackNotConfigured) -- the middle case only makes sense once a
            // primary actually exists.
            if primary.is_some() && fallback.is_none() {
                panic_with_error!(&env, Error::FallbackNotConfigured);
            }
            panic_with_error!(&env, Error::InvalidSource);
        }

        let current_price: i128 = env.storage().instance().get(&DataKey::Price).unwrap_or(0);
        let current_ts: u64 = env
            .storage()
            .instance()
            .get(&DataKey::Timestamp)
            .unwrap_or(0u64);

        if timestamp <= current_ts {
            return Self::reject(&env, value, timestamp, symbol_short!("stale_ts"));
        }

        if is_fallback {
            // Conservative policy: the fallback can only refresh the timestamp of the value
            // already on record -- it never introduces a new price. Primary is the sole source
            // of truth for the value itself.
            if value != current_price {
                return Self::reject(&env, value, timestamp, symbol_short!("fb_value"));
            }
        } else if value < current_price {
            return Self::reject(&env, value, timestamp, symbol_short!("decrease"));
        } else if current_price > 0 {
            let max_bps = Self::max_deviation_bps(env.clone()) as i128;
            // `checked_mul`/`checked_div` rather than raw operators: with `overflow-checks =
            // true`, an astronomically large `value` (e.g. from the same scale bug
            // `set_value_bounds` is meant to catch, if no bound was configured) would otherwise
            // panic here instead of going through the graceful-rejection path this module's own
            // doc promises for "a syntactically valid submission ... rejected by policy" --
            // treat an overflow as "deviation far exceeds threshold" and reject the same way.
            let within_bounds = (value - current_price)
                .checked_mul(BPS_SCALE as i128)
                .and_then(|scaled| scaled.checked_div(current_price))
                .is_some_and(|deviation_bps| deviation_bps <= max_bps);
            if !within_bounds {
                return Self::reject(&env, value, timestamp, symbol_short!("deviate"));
            }
        }

        env.storage().instance().set(&DataKey::Price, &value);
        env.storage()
            .instance()
            .set(&DataKey::Timestamp, &timestamp);
        env.storage()
            .instance()
            .set(&DataKey::ActiveSource, &source);
        env.events()
            .publish((symbol_short!("ref_set"),), (value, timestamp));
        true
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
    /// Uses `env.ledger().timestamp()` — callers cannot manipulate this value. Kept as a
    /// caller-supplied-threshold primitive alongside `is_available()`; most callers should prefer
    /// `is_available()` so the threshold lives in one place (this contract's own config) instead
    /// of being duplicated as a constant in every consumer.
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

    /// `is_fresh` against this contract's own configured `max_stale_seconds`, so a consumer needs
    /// no threshold of its own. This is this feed's definition of "a usable price is available."
    pub fn is_available(env: Env) -> bool {
        let max_stale = Self::max_stale_seconds(env.clone());
        Self::is_fresh(env, max_stale)
    }

    /// Panics `OracleUnavailable` unless `is_available()` is true. Gives a caller that wants a
    /// crisp revert (rather than reading a boolean and deciding itself) a single cross-contract
    /// call to make that happen, propagating this contract's own error code unchanged.
    pub fn assert_available(env: Env) {
        if !Self::is_available(env.clone()) {
            panic_with_error!(&env, Error::OracleUnavailable);
        }
    }

    pub fn get_primary_source(env: Env) -> Option<BytesN<32>> {
        env.storage().instance().get(&DataKey::PrimarySource)
    }

    pub fn get_fallback_source(env: Env) -> Option<BytesN<32>> {
        env.storage().instance().get(&DataKey::FallbackSource)
    }

    /// Which configured source (primary or fallback) the currently-stored value last came from.
    /// `None` before the first accepted `set_reference_value`.
    pub fn get_active_source(env: Env) -> Option<BytesN<32>> {
        env.storage().instance().get(&DataKey::ActiveSource)
    }

    pub fn max_stale_seconds(env: Env) -> u64 {
        env.storage()
            .instance()
            .get(&DataKey::MaxStaleSeconds)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized))
    }

    pub fn max_deviation_bps(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKey::MaxDeviationBps)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized))
    }

    /// The most recent submission this contract refused by policy (not a malformed call -- see
    /// the module doc), or `None` if nothing has ever been rejected.
    pub fn last_rejection(env: Env) -> Option<OracleRejection> {
        let value: Option<i128> = env.storage().instance().get(&DataKey::LastRejectedValue);
        let timestamp: Option<u64> = env
            .storage()
            .instance()
            .get(&DataKey::LastRejectedTimestamp);
        let reason: Option<Symbol> = env.storage().instance().get(&DataKey::LastRejectReason);
        match (value, timestamp, reason) {
            (Some(value), Some(timestamp), Some(reason)) => Some(OracleRejection {
                value,
                timestamp,
                reason,
            }),
            _ => None,
        }
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

    fn validate_safety_params(env: &Env, max_stale_seconds: u64, max_deviation_bps: u32) {
        if max_stale_seconds == 0 {
            panic_with_error!(env, Error::InvalidStaleness);
        }
        if max_deviation_bps == 0 || max_deviation_bps > BPS_SCALE {
            panic_with_error!(env, Error::InvalidDeviation);
        }
    }

    fn reject(env: &Env, value: i128, timestamp: u64, reason: Symbol) -> bool {
        env.storage()
            .instance()
            .set(&DataKey::LastRejectedValue, &value);
        env.storage()
            .instance()
            .set(&DataKey::LastRejectedTimestamp, &timestamp);
        env.storage()
            .instance()
            .set(&DataKey::LastRejectReason, &reason);
        env.events()
            .publish((symbol_short!("ref_rej"),), (value, timestamp, reason));
        false
    }
}

#[cfg(test)]
mod test;
