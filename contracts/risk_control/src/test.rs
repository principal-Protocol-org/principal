use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    Address, Env,
};

use super::{RiskControlContract, RiskControlContractClient, DEFAULT_WINDOW_LEDGERS};

fn advance_ledgers(env: &Env, n: u32) {
    env.ledger().with_mut(|li| li.sequence_number += n);
}

/// Returns (env, client, admin, consumer, asset) with `consumer` already registered, since
/// almost every `check_deposit` test needs one.
fn setup(
    cb_limit: i128,
) -> (
    Env,
    RiskControlContractClient<'static>,
    Address,
    Address,
    Address,
) {
    let env = Env::default();
    env.mock_all_auths();
    let id = env.register_contract(None, RiskControlContract);
    let client = RiskControlContractClient::new(&env, &id);
    let admin = Address::generate(&env);
    client.initialize(&admin, &cb_limit);
    let consumer = Address::generate(&env);
    client.add_consumer(&admin, &consumer);
    let asset = Address::generate(&env);
    (env, client, admin, consumer, asset)
}

#[test]
fn pause_and_unpause() {
    let (_env, client, admin, _consumer, _asset) = setup(0);
    assert!(!client.is_paused());
    client.pause(&admin);
    assert!(client.is_paused());
    client.unpause(&admin);
    assert!(!client.is_paused());
}

#[test]
fn pauser_can_pause_but_not_unpause() {
    let (env, client, admin, _consumer, _asset) = setup(0);
    let pauser = Address::generate(&env);
    client.add_pauser(&admin, &pauser);
    client.pause(&pauser);
    assert!(client.is_paused());
    assert!(client.try_unpause(&pauser).is_err());
    assert!(client.is_paused());
}

#[test]
fn non_pauser_cannot_pause() {
    let (env, client, _admin, _consumer, _asset) = setup(0);
    let rando = Address::generate(&env);
    assert!(client.try_pause(&rando).is_err());
}

#[test]
fn circuit_breaker_accepts_up_to_the_exact_limit() {
    let (_env, client, _admin, consumer, asset) = setup(1_000_000);
    client.check_deposit(&consumer, &asset, &500_000_i128);
    client.check_deposit(&consumer, &asset, &500_000_i128); // exactly at the limit
    assert_eq!(client.get_cb_volume(), 1_000_000);
}

#[test]
fn circuit_breaker_trips_one_unit_over_the_limit() {
    let (_env, client, _admin, consumer, asset) = setup(1_000_000);
    client.check_deposit(&consumer, &asset, &500_000_i128);
    client.check_deposit(&consumer, &asset, &499_999_i128);
    assert!(client
        .try_check_deposit(&consumer, &asset, &2_i128)
        .is_err());
    // The rejected call left the counted volume untouched.
    assert_eq!(client.get_cb_volume(), 999_999);
    client.check_deposit(&consumer, &asset, &1_i128);
}

#[test]
fn circuit_breaker_disabled_when_limit_zero() {
    let (_env, client, _admin, consumer, asset) = setup(0);
    // Should not trip even for a huge deposit.
    client.check_deposit(&consumer, &asset, &(i128::MAX / 2));
    assert_eq!(client.get_cb_volume(), 0);
}

#[test]
fn check_deposit_rejects_zero_and_negative_amount() {
    let (_env, client, _admin, consumer, asset) = setup(1_000_000);
    assert!(client
        .try_check_deposit(&consumer, &asset, &0_i128)
        .is_err());
    assert!(client
        .try_check_deposit(&consumer, &asset, &(-1_i128))
        .is_err());
}

#[test]
fn check_deposit_overflow_reverts_instead_of_wrapping() {
    let (_env, client, _admin, consumer, asset) = setup(i128::MAX);
    client.check_deposit(&consumer, &asset, &(i128::MAX - 1));
    assert!(client
        .try_check_deposit(&consumer, &asset, &(i128::MAX - 1))
        .is_err());
}

#[test]
fn check_deposit_fails_when_paused() {
    let (_env, client, admin, consumer, asset) = setup(0);
    client.pause(&admin);
    assert!(client
        .try_check_deposit(&consumer, &asset, &1_i128)
        .is_err());
}

#[test]
fn check_deposit_requires_registered_consumer() {
    // The audit finding this closes: without this check, anyone could call check_deposit
    // directly to exhaust the circuit breaker budget and block real depositors.
    let (env, client, _admin, _consumer, asset) = setup(1_000_000);
    let rando = Address::generate(&env);
    assert!(client.try_check_deposit(&rando, &asset, &1_i128).is_err());
}

#[test]
fn removed_consumer_loses_check_deposit_access() {
    let (_env, client, admin, consumer, asset) = setup(1_000_000);
    client.remove_consumer(&admin, &consumer);
    assert!(!client.is_consumer(&consumer));
    assert!(client
        .try_check_deposit(&consumer, &asset, &1_i128)
        .is_err());
}

#[test]
fn add_duplicate_consumer_panics() {
    let (env, client, admin, _consumer, _asset) = setup(0);
    let other = Address::generate(&env);
    client.add_consumer(&admin, &other);
    assert!(client.try_add_consumer(&admin, &other).is_err());
}

#[test]
fn non_admin_cannot_unpause() {
    let (env, client, admin, _consumer, _asset) = setup(0);
    client.pause(&admin);
    let rando = Address::generate(&env);
    assert!(client.try_unpause(&rando).is_err()); // only admin may unpause
}

// ---- ledger-sequence window ----

#[test]
fn window_is_measured_in_ledgers_and_ignores_wall_clock_time() {
    let (env, client, _admin, consumer, asset) = setup(1_000_000);
    client.check_deposit(&consumer, &asset, &900_000_i128);

    // A year of wall-clock time passing without a single new ledger does not reset the window.
    env.ledger().with_mut(|li| li.timestamp += 365 * 86_400);
    assert_eq!(client.get_cb_volume(), 900_000);
    assert!(client
        .try_check_deposit(&consumer, &asset, &200_000_i128)
        .is_err());
}

#[test]
fn window_lapsing_decays_the_previous_volume_instead_of_hard_resetting_to_zero() {
    // Replaces the old `window_resets_after_exactly_window_ledgers`, which asserted a hard
    // tumbling reset (volume jumps straight to 0 the instant the window lapses). That is exactly
    // the bug this fix closes -- see `sliding_window_bounds_the_original_tumbling_reset_bypass_*`
    // below for the direct regression test. This test instead pins the new, correct mechanics.
    let (env, client, _admin, consumer, asset) = setup(1_000_000);
    let start = env.ledger().sequence();
    client.check_deposit(&consumer, &asset, &900_000_i128);
    assert_eq!(client.get_window_start(), start);
    assert_eq!(client.get_window_ledgers(), DEFAULT_WINDOW_LEDGERS);

    // One ledger before the boundary the window is still open -- unchanged from before.
    advance_ledgers(&env, DEFAULT_WINDOW_LEDGERS - 1);
    assert_eq!(client.get_cb_volume(), 900_000);
    assert!(client
        .try_check_deposit(&consumer, &asset, &200_000_i128)
        .is_err());

    // Exactly `window_ledgers` after the start, the window lapses -- but the 900k does not
    // vanish. It rolls into the "previous window", and at this very instant
    // (elapsed-in-new-window == 0) it still counts in full: the sliding window counter correctly
    // reports that the trailing `window`-ledger lookback still contains all of it.
    advance_ledgers(&env, 1);
    assert_eq!(client.get_cb_volume(), 900_000); // NOT 0 -- this is the fix
    assert_eq!(client.get_window_start(), env.ledger().sequence());
    // A fresh 900k deposit right here would have sailed through under the old tumbling reset
    // (the original 2x-bypass bug). It is correctly throttled now.
    assert!(client
        .try_check_deposit(&consumer, &asset, &900_000_i128)
        .is_err());
    // Only the remaining headroom (limit minus the still-fully-weighted previous window) is
    // available.
    client.check_deposit(&consumer, &asset, &100_000_i128);
    assert_eq!(client.get_cb_volume(), 1_000_000);

    // As the new current window advances, the previous window's weight decays linearly. Halfway
    // through it, roughly half of the original 900k has "slid out" of the trailing lookback.
    advance_ledgers(&env, DEFAULT_WINDOW_LEDGERS / 2);
    let half_decayed = client.get_cb_volume();
    assert!(half_decayed < 1_000_000 && half_decayed > 100_000);
}

#[test]
fn window_fully_lapses_after_two_full_windows_of_inactivity() {
    // Once the current *and* the preceding window are both entirely outside the trailing
    // `window`-ledger lookback (nothing happened for two full windows), the effective volume is
    // genuinely 0 again -- the sliding-window counter does not remember forever.
    let (env, client, _admin, consumer, asset) = setup(1_000_000);
    client.check_deposit(&consumer, &asset, &900_000_i128);
    advance_ledgers(&env, DEFAULT_WINDOW_LEDGERS * 2);
    assert_eq!(client.get_cb_volume(), 0);
    client.check_deposit(&consumer, &asset, &1_000_000_i128);
    assert_eq!(client.get_cb_volume(), 1_000_000);
}

#[test]
fn sliding_window_bounds_the_original_tumbling_reset_bypass_to_roughly_one_times_the_limit() {
    // Regression for the audit finding: a hard tumbling reset let an attacker clear close to 2x
    // the configured limit within a span just over one window, by depositing near the limit
    // right before the boundary and again right after it (the instant the old code reset the
    // counter straight to zero). The sliding-window counter must throttle the second deposit
    // instead.
    let (env, client, _admin, consumer, asset) = setup(1_000_000);

    // Deposit A: right up against the limit, one ledger before the window would have closed.
    advance_ledgers(&env, DEFAULT_WINDOW_LEDGERS - 1);
    client.check_deposit(&consumer, &asset, &1_000_000_i128);
    assert_eq!(client.get_cb_volume(), 1_000_000);

    // One ledger later: exactly the instant the old tumbling design reset the counter to 0,
    // reopening the full limit for a second deposit within a span of just over `window` ledgers.
    advance_ledgers(&env, 1);
    // Fixed: Deposit A's volume rolls into the "previous window" and, right at this instant, is
    // still weighted in full (none of it has slid out of the trailing lookback yet) -- so a
    // second near-limit deposit here is correctly rejected, not sailed through.
    assert_eq!(client.get_cb_volume(), 1_000_000);
    assert!(client
        .try_check_deposit(&consumer, &asset, &1_000_000_i128)
        .is_err());
    // Across this whole span -- just over one window -- total throughput stays bounded at the
    // configured limit, not ~2x it.
    assert_eq!(client.get_cb_volume(), 1_000_000);
}

#[test]
fn admin_can_change_the_window_length() {
    let (env, client, admin, consumer, asset) = setup(1_000);
    client.set_window_ledgers(&admin, &10);
    assert_eq!(client.get_window_ledgers(), 10);
    client.check_deposit(&consumer, &asset, &1_000_i128);
    // Two full window-lengths later, both the deposited window and its (nonexistent)
    // predecessor are entirely outside the trailing lookback -- a full reset, so the limit
    // reopens fully (see `window_fully_lapses_after_two_full_windows_of_inactivity`).
    advance_ledgers(&env, 20);
    client.check_deposit(&consumer, &asset, &1_000_i128);
}

#[test]
fn window_length_must_be_positive_and_admin_only() {
    let (env, client, admin, _consumer, _asset) = setup(0);
    assert!(client.try_set_window_ledgers(&admin, &0).is_err());
    let rando = Address::generate(&env);
    assert!(client.try_set_window_ledgers(&rando, &5).is_err());
}

// ---- per-asset limit ----

#[test]
fn per_asset_limit_trips_independently_of_the_protocol_limit() {
    let (env, client, admin, consumer, asset) = setup(10_000);
    client.set_asset_limit(&admin, &asset, &1_000);
    assert_eq!(client.get_asset_limit(&asset), 1_000);

    client.check_deposit(&consumer, &asset, &1_000_i128); // exactly at the asset limit
    assert_eq!(client.get_asset_volume(&asset), 1_000);
    assert!(client
        .try_check_deposit(&consumer, &asset, &1_i128)
        .is_err());
    // The protocol-wide budget still has room, so a different (unlimited) asset is fine.
    let other = Address::generate(&env);
    client.check_deposit(&consumer, &other, &5_000_i128);
    assert_eq!(client.get_cb_volume(), 6_000);
    assert_eq!(client.get_asset_volume(&other), 0); // no per-asset limit => not tracked
}

#[test]
fn per_asset_limit_is_checked_alongside_the_protocol_limit() {
    let (env, client, admin, consumer, asset) = setup(1_500);
    client.set_asset_limit(&admin, &asset, &1_000);
    let other = Address::generate(&env);
    client.check_deposit(&consumer, &other, &1_000_i128);
    // Asset limit has room (500 < 1000 more) but the protocol-wide limit would be exceeded.
    assert!(client
        .try_check_deposit(&consumer, &asset, &600_i128)
        .is_err());
    // A rejected call must not leave the asset window partially charged.
    assert_eq!(client.get_asset_volume(&asset), 0);
    client.check_deposit(&consumer, &asset, &500_i128);
    assert_eq!(client.get_asset_volume(&asset), 500);
    assert_eq!(client.get_cb_volume(), 1_500);
}

#[test]
fn per_asset_limit_works_with_protocol_limit_disabled() {
    let (_env, client, admin, consumer, asset) = setup(0);
    client.set_asset_limit(&admin, &asset, &100);
    client.check_deposit(&consumer, &asset, &100_i128);
    assert!(client
        .try_check_deposit(&consumer, &asset, &1_i128)
        .is_err());
}

#[test]
fn per_asset_window_lapsing_decays_instead_of_hard_resetting_to_zero() {
    let (env, client, admin, consumer, asset) = setup(0);
    client.set_asset_limit(&admin, &asset, &100);
    client.set_window_ledgers(&admin, &50);
    client.check_deposit(&consumer, &asset, &100_i128);
    advance_ledgers(&env, 49);
    assert!(client
        .try_check_deposit(&consumer, &asset, &1_i128)
        .is_err());

    // Exactly at the boundary: lapses, but decays rather than hard-resetting to 0 (the fix).
    advance_ledgers(&env, 1);
    assert_eq!(client.get_asset_volume(&asset), 100); // NOT 0
    assert!(client
        .try_check_deposit(&consumer, &asset, &1_i128)
        .is_err());

    // Halfway through the new window, roughly half of the old 100 has decayed out, opening up
    // some (but not the full) headroom again.
    advance_ledgers(&env, 25);
    let decayed = client.get_asset_volume(&asset);
    assert!(decayed < 100 && decayed > 0);
    client.check_deposit(&consumer, &asset, &1_i128);
}

#[test]
fn removing_a_per_asset_limit_reopens_the_asset() {
    // Found during audit review: this test previously asserted nothing beyond "doesn't panic",
    // so it could not have caught a regression in the disable path (e.g. volume bookkeeping
    // drifting from reality once the limit is lifted, or the limit silently not actually
    // reopening the asset). Now demonstrates the full claim: closed while a limit is set
    // (a deposit *over* the limit is rejected, not just one exactly at it), reopened once
    // lifted, and volume accounting continues to be tracked correctly either way.
    let (_env, client, admin, consumer, asset) = setup(0);
    client.set_asset_limit(&admin, &asset, &10);
    client.check_deposit(&consumer, &asset, &10_i128);
    assert_eq!(client.get_asset_volume(&asset), 10);
    assert!(client
        .try_check_deposit(&consumer, &asset, &1_i128)
        .is_err());
    client.set_asset_limit(&admin, &asset, &0);
    client.check_deposit(&consumer, &asset, &1_000_000_i128);
    // A disabled (limit == 0) asset isn't tracked at all -- the volume counter stays at
    // whatever it last was while a limit was in force, it does not keep accumulating.
    assert_eq!(client.get_asset_volume(&asset), 10);
}

#[test]
fn negative_limits_are_rejected_and_limits_are_admin_only() {
    let (env, client, admin, _consumer, asset) = setup(0);
    assert!(client.try_set_cb_limit(&admin, &(-1)).is_err());
    assert!(client.try_set_asset_limit(&admin, &asset, &(-1)).is_err());
    let rando = Address::generate(&env);
    assert!(client.try_set_cb_limit(&rando, &5).is_err());
    assert!(client.try_set_asset_limit(&rando, &asset, &5).is_err());
}

#[test]
fn raising_the_protocol_limit_reopens_a_tripped_breaker() {
    let (_env, client, admin, consumer, asset) = setup(100);
    client.check_deposit(&consumer, &asset, &100_i128);
    assert!(client
        .try_check_deposit(&consumer, &asset, &1_i128)
        .is_err());
    client.set_cb_limit(&admin, &200);
    assert_eq!(client.get_cb_limit(), 200);
    client.check_deposit(&consumer, &asset, &100_i128);
}

#[test]
fn remove_pauser_revokes_pause_permission() {
    let (env, client, admin, _consumer, _asset) = setup(0);
    let pauser = Address::generate(&env);
    client.add_pauser(&admin, &pauser);
    assert!(client.is_pauser(&pauser));
    // After removal the address is no longer a pauser — pause must panic.
    client.remove_pauser(&admin, &pauser);
    assert!(client.try_pause(&pauser).is_err());
}

#[test]
fn add_duplicate_pauser_panics() {
    let (env, client, admin, _consumer, _asset) = setup(0);
    let pauser = Address::generate(&env);
    client.add_pauser(&admin, &pauser);
    assert!(client.try_add_pauser(&admin, &pauser).is_err());
}

#[test]
fn admin_transfer() {
    let (env, client, admin, _consumer, _asset) = setup(0);
    let new_admin = Address::generate(&env);
    client.transfer_admin(&admin, &new_admin);
    assert_eq!(client.get_admin(), new_admin);
    // The old admin lost every admin power immediately.
    assert!(client.try_set_cb_limit(&admin, &5).is_err());
}

#[test]
fn double_initialize_panics() {
    let (_env, client, admin, _consumer, _asset) = setup(0);
    assert!(client.try_initialize(&admin, &0_i128).is_err());
}

#[test]
fn uninitialized_admin_reads_revert() {
    let env = Env::default();
    env.mock_all_auths();
    let id = env.register_contract(None, RiskControlContract);
    let client = RiskControlContractClient::new(&env, &id);
    assert!(client.try_get_admin().is_err());
}
