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
//! admin can change it with `set_window_ledgers`.
//!
//! # Sliding window counter, not a hard tumbling reset
//! A window is *not* a hard tumbling reset (jumping straight to a volume of zero the instant
//! `window_ledgers` have elapsed since it started). That design let roughly 2x the configured
//! limit clear within a span just over one window: deposit near the limit right before the
//! boundary, then again right after it, since the counter the second deposit was checked against
//! had just been zeroed. Instead, this implements the standard "sliding window counter"
//! approximation used by many production rate limiters: alongside the current window's raw
//! volume, the *immediately preceding* window's raw volume is also kept, and an "effective"
//! (weighted) volume is computed on every read as
//!
//! ```text
//! effective_volume = current_window_volume
//!                   + previous_window_volume * (window - elapsed_in_current_window) / window
//! ```
//!
//! i.e. the previous window's volume decays linearly to zero as `now` advances through the
//! current window, approximating a true sliding lookback of exactly `window` ledgers without
//! needing a fully bucketed accumulator (many fixed-size sub-windows). This bounds the worst case
//! close to 1x the configured limit instead of 2x. A new deposit only ever adds to the *current*
//! window's raw counter (never to the weighted estimate itself, which is recomputed fresh on
//! every call from the two raw counters). See `WindowState` for the exact boundary arithmetic.
//!
//! `get_cb_volume`/`get_asset_volume` return this weighted *effective* volume (not the raw
//! current-window counter) -- deliberately, since that is what is actually compared against the
//! limit on the next `check_deposit`, and returning the raw counter alone would reproduce the
//! same misleading "it reads as low/zero right at the boundary" impression that masked the
//! original bypass.
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
    /// Raw cumulative deposit volume, across all assets, counted in the *current* protocol-wide
    /// window so far (see the module doc's sliding-window-counter section).
    CbVolume,
    /// Ledger sequence at which the current protocol-wide window started.
    CbWindowStart,
    /// Raw cumulative deposit volume counted in the protocol-wide window immediately preceding
    /// the current one. 0 if there was no such window, or it is old enough that it no longer
    /// overlaps the trailing lookback at all (see `WindowState::at`).
    CbPrevVolume,
    /// Window length in ledgers.
    WindowLedgers,
    /// Per-asset limit (underlying units, 0 = none).
    AssetLimit(Address),
    /// Per-asset window: (start ledger of the *current* window, its raw volume so far, the raw
    /// volume of the immediately preceding window). See `WindowState`.
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
            .set(&DataKey::CbPrevVolume, &0_i128);
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
            let state = Self::global_window_state(&env, now, window);
            let effective = state.effective_volume(now, window);
            let total_effective = effective
                .checked_add(amount)
                .unwrap_or_else(|| panic_with_error!(&env, Error::CircuitBreakerTripped));
            if total_effective > limit {
                panic_with_error!(&env, Error::CircuitBreakerTripped);
            }
            // The new deposit only ever adds to the *current* window's raw counter -- the
            // weighted estimate above is derived fresh from the two raw counters on every call,
            // never stored itself.
            let new_volume = state
                .volume
                .checked_add(amount)
                .unwrap_or_else(|| panic_with_error!(&env, Error::CircuitBreakerTripped));
            Some((state.start, new_volume, state.prev_volume))
        } else {
            None
        };

        let asset_limit: i128 = env
            .storage()
            .instance()
            .get(&DataKey::AssetLimit(asset.clone()))
            .unwrap_or(0);
        let per_asset = if asset_limit > 0 {
            let state = Self::asset_window_state(&env, &asset, now, window);
            let effective = state.effective_volume(now, window);
            let total_effective = effective
                .checked_add(amount)
                .unwrap_or_else(|| panic_with_error!(&env, Error::AssetLimitTripped));
            if total_effective > asset_limit {
                panic_with_error!(&env, Error::AssetLimitTripped);
            }
            let new_volume = state
                .volume
                .checked_add(amount)
                .unwrap_or_else(|| panic_with_error!(&env, Error::AssetLimitTripped));
            Some((state.start, new_volume, state.prev_volume))
        } else {
            None
        };

        if let Some((start, volume, prev_volume)) = global {
            env.storage()
                .instance()
                .set(&DataKey::CbWindowStart, &start);
            env.storage().instance().set(&DataKey::CbVolume, &volume);
            env.storage()
                .instance()
                .set(&DataKey::CbPrevVolume, &prev_volume);
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

    /// The *effective* (weighted) volume against which the protocol-wide limit is currently
    /// checked -- see the module doc's sliding-window-counter section for why this, not the raw
    /// current-window counter, is what this getter returns. Decays smoothly rather than jumping
    /// to `0` the instant a window boundary is crossed.
    pub fn get_cb_volume(env: Env) -> i128 {
        let now = env.ledger().sequence();
        let window = Self::window_ledgers(&env);
        Self::global_window_state(&env, now, window).effective_volume(now, window)
    }

    pub fn get_asset_limit(env: Env, asset: Address) -> i128 {
        env.storage()
            .instance()
            .get(&DataKey::AssetLimit(asset))
            .unwrap_or(0)
    }

    /// The *effective* (weighted) volume against which `asset`'s limit is currently checked --
    /// see `get_cb_volume`.
    pub fn get_asset_volume(env: Env, asset: Address) -> i128 {
        let now = env.ledger().sequence();
        let window = Self::window_ledgers(&env);
        Self::asset_window_state(&env, &asset, now, window).effective_volume(now, window)
    }

    pub fn get_window_ledgers(env: Env) -> u32 {
        Self::window_ledgers(&env)
    }

    /// Ledger sequence at which the current protocol-wide window started, normalized to `now`
    /// the same way `get_cb_volume` is -- so this reflects the window `now` actually falls in
    /// even if no `check_deposit` has landed since a boundary was crossed, rather than a stale
    /// raw value left over from before the roll-over.
    pub fn get_window_start(env: Env) -> u32 {
        let now = env.ledger().sequence();
        let window = Self::window_ledgers(&env);
        Self::global_window_state(&env, now, window).start
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

    /// The protocol-wide window's state as of ledger `now`.
    fn global_window_state(env: &Env, now: u32, window: u32) -> WindowState {
        let start: u32 = env
            .storage()
            .instance()
            .get(&DataKey::CbWindowStart)
            .unwrap_or(now);
        let volume: i128 = env
            .storage()
            .instance()
            .get(&DataKey::CbVolume)
            .unwrap_or(0);
        let prev_volume: i128 = env
            .storage()
            .instance()
            .get(&DataKey::CbPrevVolume)
            .unwrap_or(0);
        WindowState::at(start, volume, prev_volume, now, window)
    }

    /// `asset`'s window state as of ledger `now`.
    fn asset_window_state(env: &Env, asset: &Address, now: u32, window: u32) -> WindowState {
        let (start, volume, prev_volume): (u32, i128, i128) = env
            .storage()
            .instance()
            .get(&DataKey::AssetWindow(asset.clone()))
            .unwrap_or((now, 0, 0));
        WindowState::at(start, volume, prev_volume, now, window)
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

/// A protocol-wide or per-asset circuit-breaker window, normalized to ledger `now`: `start` is
/// the ledger sequence the *current* window began, `volume` the raw amount counted in it so far,
/// and `prev_volume` the raw amount counted in the immediately preceding, now-closed window (`0`
/// if there was none, or it is too old to matter). See the module doc for why this replaces a
/// hard tumbling reset.
struct WindowState {
    start: u32,
    volume: i128,
    prev_volume: i128,
}

impl WindowState {
    /// Normalizes raw stored `(start, volume, prev_volume)` to ledger `now`, rolling over any
    /// window boundary that has elapsed since it was last written. After this returns,
    /// `now.saturating_sub(result.start) < window` always holds -- the window returned is always
    /// the one `now` currently falls in.
    fn at(
        stored_start: u32,
        stored_volume: i128,
        stored_prev: i128,
        now: u32,
        window: u32,
    ) -> Self {
        let elapsed = now.saturating_sub(stored_start);
        if elapsed < window {
            // Still inside the window that was last written -- no transition.
            Self {
                start: stored_start,
                volume: stored_volume,
                prev_volume: stored_prev,
            }
        } else if elapsed < window.saturating_mul(2) {
            // Exactly one window boundary has fully elapsed. The window that just closed becomes
            // the new "previous" window -- rolled over, not discarded to zero -- and a fresh
            // current window starts immediately after it, contiguous with the old one
            // (`stored_start + window`, not `now`) so `effective_volume` below is weighting the
            // true adjacent predecessor, not an arbitrarily-gapped one. The old `stored_prev`
            // (now two windows stale) is dropped: it no longer overlaps any trailing
            // `window`-sized lookback from `now` at all.
            Self {
                start: stored_start.saturating_add(window),
                volume: 0,
                prev_volume: stored_volume,
            }
        } else {
            // Two or more whole windows have elapsed with nothing recorded in between: both the
            // old current and old previous window are entirely outside any trailing
            // `window`-sized lookback from `now`, so neither contributes anything. A full reset,
            // anchored at `now` itself -- there is nothing left to slide.
            Self {
                start: now,
                volume: 0,
                prev_volume: 0,
            }
        }
    }

    /// The weighted estimate of "volume in the trailing `window`-ledger lookback ending at
    /// `now`", compared against a limit. The current window's raw volume counts in full; the
    /// previous window's raw volume is scaled down by how much of it has already slid out of
    /// the lookback as `now` advances through the current window:
    ///
    /// `effective = volume + prev_volume * (window - elapsed) / window`
    ///
    /// where `elapsed = now - start` (always `< window` here, since `at` always normalizes
    /// first).
    ///
    /// Boundary values, reasoned through explicitly:
    /// * `elapsed == 0` (the instant the current window starts, e.g. right after a boundary
    ///   roll-over): weight is `window / window == 1` -- the previous window counts in full,
    ///   correctly, since at that exact instant the entire previous window is still inside the
    ///   trailing lookback. This is precisely the case the original tumbling reset got wrong
    ///   (it read as `0` here instead).
    /// * `elapsed == window - 1` (one ledger before the *next* transition): weight is
    ///   `1 / window` -- vanishingly small but still nonzero, correctly reflecting that almost
    ///   none of the previous window remains in the lookback.
    fn effective_volume(&self, now: u32, window: u32) -> i128 {
        let elapsed = now.saturating_sub(self.start);
        let remaining = window.saturating_sub(elapsed); // > 0: elapsed < window post-`at`
        let weighted_prev = self
            .prev_volume
            .checked_mul(remaining as i128)
            .and_then(|v| v.checked_div(window as i128))
            // An overflowing multiplication only happens for an astronomically large
            // `prev_volume`; fall back to the unscaled (i.e. maximal, never-decayed) value rather
            // than `0` -- consistent with this codebase's convention elsewhere (`OracleAdapter`'s
            // deviation guard) of treating an overflow as "assume the worst, reject" rather than
            // silently under-counting.
            .unwrap_or(self.prev_volume);
        self.volume.saturating_add(weighted_prev)
    }
}

#[cfg(test)]
mod test;
