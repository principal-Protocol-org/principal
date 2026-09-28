use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    Address, Env,
};

use super::{Error, OracleAdapterContract, OracleAdapterContractClient};

/// What a `try_*` call returns for the contract error `e`.
fn err(e: Error) -> Option<Result<soroban_sdk::Error, soroban_sdk::InvokeError>> {
    Some(Ok(soroban_sdk::Error::from_contract_error(e as u32)))
}

/// A ledger clock far enough ahead that the fixture timestamps below are all in the past.
const NOW: u64 = 2_000_000_000;

fn setup() -> (Env, OracleAdapterContractClient<'static>, Address) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = NOW);
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
fn value_decrease_rejected() {
    // The entire PT/YT settlement model assumes the reference rate never falls -- see
    // this module's doc comment for why a decrease must be rejected outright, not just
    // priced in.
    let (_env, client, admin) = setup();
    client.set_reference_value(&admin, &10_300_000_i128, &1_700_000_000_u64);
    let r = client.try_set_reference_value(&admin, &10_299_999_i128, &1_700_001_000_u64);
    assert_eq!(r.err(), err(Error::ValueDecreased));
    assert_eq!(client.get_reference_value(), 10_300_000_i128);
}

#[test]
fn value_may_stay_equal() {
    // A resubmission at the same price (e.g. a heartbeat refresh with no real price move)
    // must not be treated as a decrease.
    let (_env, client, admin) = setup();
    client.set_reference_value(&admin, &10_300_000_i128, &1_700_000_000_u64);
    client.set_reference_value(&admin, &10_300_000_i128, &1_700_001_000_u64);
    assert_eq!(client.get_reference_value(), 10_300_000_i128);
}

#[test]
fn double_initialize_panics() {
    let (_env, client, admin) = setup();
    assert_eq!(
        client.try_initialize(&admin).err(),
        err(Error::AlreadyInitialized)
    );
}

#[test]
fn unauthorized_price_update_panics() {
    let (env, client, admin) = setup();
    client.set_reference_value(&admin, &103_00000000_i128, &1_700_000_000_u64);
    let attacker = Address::generate(&env);
    let r = client.try_set_reference_value(&attacker, &200_00000000_i128, &1_700_001_000_u64);
    assert_eq!(r.err(), err(Error::Unauthorized));
}

#[test]
fn stale_timestamp_rejected() {
    let (_env, client, admin) = setup();
    client.set_reference_value(&admin, &103_00000000_i128, &1_700_000_000_u64);
    // Same timestamp again, and an older one: both rejected.
    let same = client.try_set_reference_value(&admin, &103_00000000_i128, &1_700_000_000_u64);
    assert_eq!(same.err(), err(Error::TimestampTooOld));
    let older = client.try_set_reference_value(&admin, &103_00000000_i128, &1_699_999_999_u64);
    assert_eq!(older.err(), err(Error::TimestampTooOld));
}

#[test]
fn future_timestamp_rejected_so_the_feed_cannot_be_bricked() {
    // A timestamp ahead of the ledger clock (milliseconds instead of seconds, say) would read as
    // stale until real time caught up and lock out every correction, since each later timestamp
    // must exceed it. It is refused at submission instead.
    let (_env, client, admin) = setup();
    client.set_reference_value(&admin, &10_300_000_i128, &1_700_000_000_u64);
    let r = client.try_set_reference_value(&admin, &10_300_000_i128, &(NOW * 1_000));
    assert_eq!(r.err(), err(Error::TimestampInFuture));
    // The exact ledger time is accepted (boundary), one second later is not.
    let r = client.try_set_reference_value(&admin, &10_300_000_i128, &(NOW + 1));
    assert_eq!(r.err(), err(Error::TimestampInFuture));
    client.set_reference_value(&admin, &10_300_000_i128, &NOW);
    assert_eq!(client.get_reference_timestamp(), NOW);
}

#[test]
fn invalid_price_rejected() {
    let (_env, client, admin) = setup();
    let zero = client.try_set_reference_value(&admin, &0_i128, &1_700_000_000_u64);
    assert_eq!(zero.err(), err(Error::InvalidValue));
    let negative = client.try_set_reference_value(&admin, &(-5_i128), &1_700_000_000_u64);
    assert_eq!(negative.err(), err(Error::InvalidValue));
}

#[test]
fn views_before_any_price_revert() {
    let (_env, client, _admin) = setup();
    assert_eq!(
        client.try_get_reference_value().err(),
        err(Error::NotInitialized)
    );
    assert_eq!(
        client.try_get_reference_timestamp().err(),
        err(Error::NotInitialized)
    );
}

#[test]
fn admin_transfer() {
    let (env, client, admin) = setup();
    let new_admin = Address::generate(&env);
    client.transfer_admin(&admin, &new_admin);
    assert_eq!(client.get_admin(), new_admin);
}

#[test]
fn unauthorized_transfer_admin_panics() {
    let (env, client, _admin) = setup();
    let impostor = Address::generate(&env);
    let new_admin = Address::generate(&env);
    let r = client.try_transfer_admin(&impostor, &new_admin);
    assert_eq!(r.err(), err(Error::Unauthorized));
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

#[test]
fn is_fresh_false_when_stored_timestamp_is_ahead_of_ledger_clock() {
    // Submissions can no longer be future-dated, but the ledger clock can still read *behind* the
    // stored timestamp (e.g. in tests, or after a clock correction). `is_fresh` must not treat
    // that as fresh via an underflowed subtraction; it explicitly returns false.
    let (env, client, admin) = setup();
    env.ledger().with_mut(|li| li.timestamp = 5_000);
    client.set_reference_value(&admin, &10_000_000_i128, &5_000_u64);
    env.ledger().with_mut(|li| li.timestamp = 1_000);
    assert!(!client.is_fresh(&3_600_u64));
}
