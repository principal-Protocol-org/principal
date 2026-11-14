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
//!
//! # Pulling from a SEP-40 feed, as an alternative primary source
//!
//! **Beyond the funded Tranche 2/3 spec** (`docs/TRANCHE_2_3_TECHNICAL_SPECIFICATION.md` §5.2,
//! §10.2): the spec's own mainnet oracle design is an off-chain relay service
//! (`services/oracle-relay`) that fetches, validates and converts a feed value off-chain, then
//! pushes it through `set_reference_value` below -- the existing, already-specified ABI. This
//! on-chain pull mechanism is an additional, explicitly optional capability layered on top, kept
//! because it is already built and tested and because a trust-minimized on-chain read is a
//! real improvement over trusting a relay's own say-so -- **not** a replacement for the
//! off-chain relay, which remains the primary, spec-compliant integration path. Treat
//! `set_feed_source`/`sync_from_feed` as needing their own explicit security review and sign-off
//! before any mainnet asset relies on them, separately from whatever review covers the relay
//! service itself: they are new attack surface the spec's own enumerated API/error lists
//! (§5.2.2, §5.2.3) do not include.
//!
//! `set_reference_value` above is push-based: some off-chain relay decides when to submit and
//! what. `sync_from_feed` is the pull-based alternative, for a mainnet asset with a real SEP-40
//! price oracle already on Stellar (e.g. RedStone's USDY/USD feed) as its primary source of
//! truth: anyone can call it (no admin, no `caller` argument at all) to read that feed's
//! `lastprice()` live and run the result through this contract's *own* freshness/deviation/
//! sane-bounds checks -- exactly the same checks `set_reference_value` enforces, so this
//! contract remains the single place those policies live regardless of which mechanism feeds it.
//! The two mechanisms coexist deliberately: `set_feed_source`/`sync_from_feed` for the primary,
//! trust-minimized path once a real feed exists for an asset; the existing
//! `set_sources`/`set_reference_value` push path stays fully functional as a manual fallback or
//! for an asset with no SEP-40 feed at all -- nothing above this section changes.
//!
//! Graceful rejection applies much more broadly here than in `set_reference_value`: a `None`
//! result, a non-positive price, or a future timestamp from the feed are *not* signs of a broken
//! caller the way they are for a direct admin submission -- `sync_from_feed` is meant to be
//! called by anyone, including an unattended keeper, so a feed having a bad tick must not revert
//! the whole call. Only `FeedNotConfigured` panics, because that is a genuine setup mistake
//! (nobody called `set_feed_source` yet), not routine feed noise.

#![no_std]
#![allow(deprecated)] // `env.events().publish` / `register_contract`: migration to `#[contractevent]` is tracked separately; event topics are kept stable for indexers.

use soroban_sdk::{
    contract, contractclient, contracterror, contractimpl, contracttype, panic_with_error,
    symbol_short, Address, BytesN, Env, Symbol,
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
    /// `sync_from_feed` called before any `set_feed_source` -- a setup mistake, not routine feed
    /// noise, so (unlike everything that feed itself can return) this panics.
    FeedNotConfigured = 15,
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
    FeedSource,
    FeedAsset,
    LastUpdateWasFeed,
}

/// SEP-40's own asset discriminant (ecosystem/sep-0040.md) -- a feed may quote several assets
/// against one `base()`, so a pull must say which one.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub enum Sep40Asset {
    Stellar(Address),
    Other(Symbol),
}

/// SEP-40's own return shape, field-for-field: `price` scaled by the feed's own `decimals()`
/// (not necessarily this contract's convention -- `sync_from_feed` rescales), `timestamp` in
/// Unix seconds.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct Sep40PriceData {
    pub price: i128,
    pub timestamp: u64,
}

/// Minimal SEP-40 consumer interface -- only the two calls `sync_from_feed` actually needs.
#[contractclient(name = "Sep40Client")]
pub trait Sep40Interface {
    fn decimals(env: Env) -> u32;
    fn lastprice(env: Env, asset: Sep40Asset) -> Option<Sep40PriceData>;
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

    /// Configure (or reconfigure) which SEP-40-compliant contract `sync_from_feed` pulls from,
    /// and which `Sep40Asset` to ask it for (one feed contract can quote several assets). Fully
    /// independent of `set_sources`/`set_reference_value` above -- both mechanisms can be
    /// configured at once, and whichever last successfully updated `Price`/`Timestamp` wins,
    /// same as primary vs. fallback already works for the push path.
    pub fn set_feed_source(env: Env, caller: Address, feed: Address, asset: Sep40Asset) {
        caller.require_auth();
        let admin = Self::require_admin(&env);
        if caller != admin {
            panic_with_error!(&env, Error::Unauthorized);
        }
        env.storage().instance().set(&DataKey::FeedSource, &feed);
        env.storage().instance().set(&DataKey::FeedAsset, &asset);
        env.events()
            .publish((symbol_short!("feed_set"),), (feed, asset));
    }

    /// The configured SEP-40 feed contract and asset, or `None` if `set_feed_source` has never
    /// been called.
    pub fn feed_source(env: Env) -> Option<(Address, Sep40Asset)> {
        let feed: Option<Address> = env.storage().instance().get(&DataKey::FeedSource);
        let asset: Option<Sep40Asset> = env.storage().instance().get(&DataKey::FeedAsset);
        match (feed, asset) {
            (Some(feed), Some(asset)) => Some((feed, asset)),
            _ => None,
        }
    }

    /// Whether the currently-stored `Price`/`Timestamp` came from the last successful
    /// `sync_from_feed` (`true`) or the last successful `set_reference_value` (`false`).
    /// `false` before either has ever succeeded.
    pub fn last_update_was_feed(env: Env) -> bool {
        env.storage()
            .instance()
            .get(&DataKey::LastUpdateWasFeed)
            .unwrap_or(false)
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
        // If a value is already on record, the new range must still contain it. Found during a
        // final audit review: without this, an admin could set (or re-tighten) bounds that
        // exclude the live price -- after which no future submission could ever satisfy both
        // "within bounds" and "not a decrease", bricking the feed exactly the way this whole
        // mechanism exists to prevent, just self-inflicted instead of caused by a bad relay value.
        if let Some(current_price) = env.storage().instance().get::<_, i128>(&DataKey::Price) {
            if current_price < min_value || current_price > max_value {
                panic_with_error!(&env, Error::InvalidValue);
            }
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
                return Self::reject(&env, value, timestamp, symbol_short!("oob"), true);
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
            return Self::reject(&env, value, timestamp, symbol_short!("stale_ts"), false);
        }

        if is_fallback {
            // Conservative policy: the fallback can only refresh the timestamp of the value
            // already on record -- it never introduces a new price. Primary is the sole source
            // of truth for the value itself.
            if value != current_price {
                return Self::reject(&env, value, timestamp, symbol_short!("fb_value"), true);
            }
        } else if value < current_price {
            return Self::reject(&env, value, timestamp, symbol_short!("decrease"), true);
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
                return Self::reject(&env, value, timestamp, symbol_short!("deviate"), true);
            }
        }

        env.storage().instance().set(&DataKey::Price, &value);
        env.storage()
            .instance()
            .set(&DataKey::Timestamp, &timestamp);
        env.storage()
            .instance()
            .set(&DataKey::ActiveSource, &source);
        env.storage()
            .instance()
            .set(&DataKey::LastUpdateWasFeed, &false);
        env.events()
            .publish((symbol_short!("ref_set"),), (value, timestamp));
        true
    }

    /// Scale this feed's price to `PROTOCOL_DECIMALS`, then run it through this contract's own
    /// freshness/monotonic-value/deviation/sane-bounds policy, exactly as a primary-source
    /// `set_reference_value` submission would -- except every failure here is a graceful
    /// rejection, never a panic (see the module doc for why `sync_from_feed` cannot treat a bad
    /// feed tick the way a malformed admin call is treated). Callable by anyone; there is
    /// nothing to authorize, since it only ever *reads* the configured feed.
    ///
    /// Returns `true` if accepted (`Price`/`Timestamp` updated, `last_update_was_feed() == true`
    /// afterward), `false` otherwise -- `last_rejection()` explains why, same as the push path.
    pub fn sync_from_feed(env: Env) -> bool {
        let (feed, asset) = Self::feed_source(env.clone())
            .unwrap_or_else(|| panic_with_error!(&env, Error::FeedNotConfigured));
        let client = Sep40Client::new(&env, &feed);
        let decimals = client.decimals();
        let data = match client.lastprice(&asset) {
            Some(data) => data,
            None => return Self::reject(&env, 0, 0, symbol_short!("no_data"), false),
        };
        if data.price <= 0 {
            return Self::reject(
                &env,
                data.price,
                data.timestamp,
                symbol_short!("bad_price"),
                false,
            );
        }
        let value = match Self::rescale_to_protocol_decimals(data.price, decimals) {
            Some(value) => value,
            None => {
                return Self::reject(
                    &env,
                    data.price,
                    data.timestamp,
                    symbol_short!("rescale"),
                    false,
                )
            }
        };
        if value <= 0 {
            return Self::reject(
                &env,
                value,
                data.timestamp,
                symbol_short!("bad_price"),
                false,
            );
        }
        if let Some((min, max)) = Self::value_bounds(env.clone()) {
            if value < min || value > max {
                return Self::reject(&env, value, data.timestamp, symbol_short!("oob"), true);
            }
        }
        if data.timestamp > env.ledger().timestamp() {
            return Self::reject(
                &env,
                value,
                data.timestamp,
                symbol_short!("future_ts"),
                false,
            );
        }

        let current_price: i128 = env.storage().instance().get(&DataKey::Price).unwrap_or(0);
        let current_ts: u64 = env
            .storage()
            .instance()
            .get(&DataKey::Timestamp)
            .unwrap_or(0u64);
        if data.timestamp <= current_ts {
            return Self::reject(
                &env,
                value,
                data.timestamp,
                symbol_short!("stale_ts"),
                false,
            );
        }
        if value < current_price {
            return Self::reject(&env, value, data.timestamp, symbol_short!("decrease"), true);
        } else if current_price > 0 {
            let max_bps = Self::max_deviation_bps(env.clone()) as i128;
            let within_bounds = (value - current_price)
                .checked_mul(BPS_SCALE as i128)
                .and_then(|scaled| scaled.checked_div(current_price))
                .is_some_and(|deviation_bps| deviation_bps <= max_bps);
            if !within_bounds {
                return Self::reject(&env, value, data.timestamp, symbol_short!("deviate"), true);
            }
        }

        env.storage().instance().set(&DataKey::Price, &value);
        env.storage()
            .instance()
            .set(&DataKey::Timestamp, &data.timestamp);
        env.storage()
            .instance()
            .set(&DataKey::LastUpdateWasFeed, &true);
        env.events()
            .publish((symbol_short!("ref_set"),), (value, data.timestamp));
        true
    }

    /// `PROTOCOL_DECIMALS` is this codebase's own reference-value scale (`SCALE = 1e7` in
    /// `MarketPool`/`PrincipalManager`/etc.) -- a SEP-40 feed's `decimals()` is its own,
    /// independent convention and is not guaranteed to match. Rescales by a plain power of ten
    /// in whichever direction is needed; a feed with *more* decimals than the protocol floors
    /// (loses at most a few raw units of precision, negligible next to real price magnitudes,
    /// consistent with floor-rounding used elsewhere in this codebase). `None` on overflow
    /// (treated as a graceful rejection by every caller, not a panic).
    fn rescale_to_protocol_decimals(value: i128, feed_decimals: u32) -> Option<i128> {
        const PROTOCOL_DECIMALS: u32 = 7;
        if feed_decimals == PROTOCOL_DECIMALS {
            return Some(value);
        }
        if feed_decimals < PROTOCOL_DECIMALS {
            let factor = 10i128.checked_pow(PROTOCOL_DECIMALS - feed_decimals)?;
            value.checked_mul(factor)
        } else {
            let factor = 10i128.checked_pow(feed_decimals - PROTOCOL_DECIMALS)?;
            Some(value / factor)
        }
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

    /// `significant` gates the single-slot `last_rejection()` view, not the `ref_rej` event
    /// (always emitted -- the event log is the complete, un-droppable audit trail). Found in
    /// review: `sync_from_feed` is callable by anyone, for free, at any time, and every call
    /// that found nothing wrong to report (`stale_ts`, `no_data`, a malformed tick) used to
    /// overwrite the same slot a genuine policy violation (`deviate`, `decrease`, `oob`,
    /// `fb_value`) had just written -- so anyone could silently mask an active deviation/
    /// decrease attempt from a monitor reading only `last_rejection()`, at zero cost, with no
    /// privilege at all. Only a reason that reflects an actual disallowed value change
    /// overwrites the slot now; a monitor that needs every rejection, not just the latest
    /// significant one, should read the `ref_rej` event log instead.
    fn reject(env: &Env, value: i128, timestamp: u64, reason: Symbol, significant: bool) -> bool {
        if significant {
            env.storage()
                .instance()
                .set(&DataKey::LastRejectedValue, &value);
            env.storage()
                .instance()
                .set(&DataKey::LastRejectedTimestamp, &timestamp);
            env.storage()
                .instance()
                .set(&DataKey::LastRejectReason, &reason);
        }
        env.events()
            .publish((symbol_short!("ref_rej"),), (value, timestamp, reason));
        false
    }
}

#[cfg(test)]
mod test;
