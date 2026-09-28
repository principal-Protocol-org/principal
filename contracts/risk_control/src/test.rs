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
fn window_resets_after_exactly_window_ledgers() {
    let (env, client, _admin, consumer, asset) = setup(1_000_000);
    let start = env.ledger().sequence();
    client.check_deposit(&consumer, &asset, &900_000_i128);
    assert_eq!(client.get_window_start(), start);
    assert_eq!(client.get_window_ledgers(), DEFAULT_WINDOW_LEDGERS);

    // One ledger before the boundary the window is still open...
    advance_ledgers(&env, DEFAULT_WINDOW_LEDGERS - 1);
    assert_eq!(client.get_cb_volume(), 900_000);
    assert!(client
        .try_check_deposit(&consumer, &asset, &200_000_i128)
        .is_err());

    // ...and exactly `window_ledgers` after the start it lapses.
    advance_ledgers(&env, 1);
    assert_eq!(client.get_cb_volume(), 0);
    client.check_deposit(&consumer, &asset, &900_000_i128);
    assert_eq!(client.get_cb_volume(), 900_000); // restarted at 900k, not 1_800_000
    assert_eq!(client.get_window_start(), env.ledger().sequence());
}

#[test]
fn admin_can_change_the_window_length() {
    let (env, client, admin, consumer, asset) = setup(1_000);
    client.set_window_ledgers(&admin, &10);
    assert_eq!(client.get_window_ledgers(), 10);
    client.check_deposit(&consumer, &asset, &1_000_i128);
    advance_ledgers(&env, 10);
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
fn per_asset_window_resets_on_the_ledger_boundary() {
    let (env, client, admin, consumer, asset) = setup(0);
    client.set_asset_limit(&admin, &asset, &100);
    client.set_window_ledgers(&admin, &50);
    client.check_deposit(&consumer, &asset, &100_i128);
    advance_ledgers(&env, 49);
    assert!(client
        .try_check_deposit(&consumer, &asset, &1_i128)
        .is_err());
    advance_ledgers(&env, 1);
    assert_eq!(client.get_asset_volume(&asset), 0);
    client.check_deposit(&consumer, &asset, &100_i128);
}

#[test]
fn removing_a_per_asset_limit_reopens_the_asset() {
    let (_env, client, admin, consumer, asset) = setup(0);
    client.set_asset_limit(&admin, &asset, &10);
    client.check_deposit(&consumer, &asset, &10_i128);
    client.set_asset_limit(&admin, &asset, &0);
    client.check_deposit(&consumer, &asset, &1_000_000_i128);
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
