use soroban_sdk::{
    contract, contractimpl,
    testutils::{Address as _, Ledger as _},
    Address, BytesN, Env, Symbol,
};

use super::{
    Error, OracleAdapterContract, OracleAdapterContractClient, Sep40Asset, Sep40PriceData,
};

/// Minimal SEP-40 feed, for `sync_from_feed` tests only: `set_price`/`clear_price` let a test
/// drive exactly what `lastprice()` returns, including `None` (no data for that asset).
#[contract]
pub struct MockSep40Feed;

#[soroban_sdk::contracttype]
enum MockKey {
    Decimals,
    Price(Sep40Asset),
}

#[contractimpl]
impl MockSep40Feed {
    pub fn initialize(env: Env, decimals: u32) {
        env.storage().instance().set(&MockKey::Decimals, &decimals);
    }

    pub fn set_price(env: Env, asset: Sep40Asset, price: i128, timestamp: u64) {
        env.storage()
            .instance()
            .set(&MockKey::Price(asset), &(price, timestamp));
    }

    pub fn clear_price(env: Env, asset: Sep40Asset) {
        env.storage().instance().remove(&MockKey::Price(asset));
    }

    pub fn decimals(env: Env) -> u32 {
        env.storage().instance().get(&MockKey::Decimals).unwrap()
    }

    pub fn lastprice(env: Env, asset: Sep40Asset) -> Option<Sep40PriceData> {
        env.storage()
            .instance()
            .get::<_, (i128, u64)>(&MockKey::Price(asset))
            .map(|(price, timestamp)| Sep40PriceData { price, timestamp })
    }
}

/// What a `try_*` call returns for the contract error `e`.
fn err(e: Error) -> Option<Result<soroban_sdk::Error, soroban_sdk::InvokeError>> {
    Some(Ok(soroban_sdk::Error::from_contract_error(e as u32)))
}

/// A ledger clock far enough ahead that the fixture timestamps below are all in the past.
const NOW: u64 = 2_000_000_000;
const MAX_STALE: u64 = 3_600;
const MAX_DEVIATION_BPS: u32 = 1_000; // 10%

fn source(env: &Env, tag: u8) -> BytesN<32> {
    let mut bytes = [0u8; 32];
    bytes[0] = tag;
    BytesN::from_array(env, &bytes)
}

fn setup() -> (
    Env,
    OracleAdapterContractClient<'static>,
    Address,
    BytesN<32>,
) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = NOW);
    let contract_id = env.register_contract(None, OracleAdapterContract);
    let client = OracleAdapterContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let primary = source(&env, 1);
    client.initialize(&admin, &MAX_STALE, &MAX_DEVIATION_BPS);
    client.set_sources(&admin, &primary, &None);
    (env, client, admin, primary)
}

// --------------------------------------------------------------- initialize / set_safety_params

#[test]
fn initialize_rejects_zero_staleness() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register_contract(None, OracleAdapterContract);
    let client = OracleAdapterContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let r = client.try_initialize(&admin, &0_u64, &MAX_DEVIATION_BPS);
    assert_eq!(r.err(), err(Error::InvalidStaleness));
}

#[test]
fn initialize_rejects_zero_or_excessive_deviation() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register_contract(None, OracleAdapterContract);
    let client = OracleAdapterContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    assert_eq!(
        client.try_initialize(&admin, &MAX_STALE, &0_u32).err(),
        err(Error::InvalidDeviation)
    );
    assert_eq!(
        client.try_initialize(&admin, &MAX_STALE, &10_001_u32).err(),
        err(Error::InvalidDeviation)
    );
    // The boundary (exactly 100%) is accepted.
    client.initialize(&admin, &MAX_STALE, &10_000_u32);
}

#[test]
fn set_safety_params_updates_thresholds_and_validates_them() {
    let (_env, client, admin, _primary) = setup();
    client.set_safety_params(&admin, &7_200_u64, &500_u32);
    assert_eq!(client.max_stale_seconds(), 7_200_u64);
    assert_eq!(client.max_deviation_bps(), 500_u32);
    assert_eq!(
        client.try_set_safety_params(&admin, &0_u64, &500_u32).err(),
        err(Error::InvalidStaleness)
    );
}

#[test]
fn set_safety_params_by_non_admin_panics() {
    let (env, client, _admin, _primary) = setup();
    let impostor = Address::generate(&env);
    let r = client.try_set_safety_params(&impostor, &MAX_STALE, &MAX_DEVIATION_BPS);
    assert_eq!(r.err(), err(Error::Unauthorized));
}

#[test]
fn double_initialize_panics() {
    let (_env, client, admin, _primary) = setup();
    assert_eq!(
        client
            .try_initialize(&admin, &MAX_STALE, &MAX_DEVIATION_BPS)
            .err(),
        err(Error::AlreadyInitialized)
    );
}

// -------------------------------------------------------------------------------- source config

#[test]
fn set_sources_and_views() {
    let (env, client, admin, primary) = setup();
    assert_eq!(client.get_primary_source(), Some(primary.clone()));
    assert_eq!(client.get_fallback_source(), None);
    assert_eq!(client.get_active_source(), None);

    let fallback = source(&env, 2);
    client.set_sources(&admin, &primary, &Some(fallback.clone()));
    assert_eq!(client.get_fallback_source(), Some(fallback));

    // Removing the fallback: pass None again.
    client.set_sources(&admin, &primary, &None);
    assert_eq!(client.get_fallback_source(), None);
}

#[test]
fn set_sources_by_non_admin_panics() {
    let (env, client, _admin, primary) = setup();
    let impostor = Address::generate(&env);
    let r = client.try_set_sources(&impostor, &primary, &None);
    assert_eq!(r.err(), err(Error::Unauthorized));
}

#[test]
fn unrecognized_source_panics_invalid_source_when_fallback_is_configured() {
    let (env, client, admin, primary) = setup();
    let fallback = source(&env, 2);
    client.set_sources(&admin, &primary, &Some(fallback));
    let garbage = source(&env, 99);
    let r = client.try_set_reference_value(&admin, &garbage, &10_300_000_i128, &1_700_000_000_u64);
    assert_eq!(r.err(), err(Error::InvalidSource));
}

#[test]
fn unrecognized_source_panics_fallback_not_configured_when_no_fallback_exists() {
    let (_env, client, admin, _primary) = setup();
    let garbage = source(&_env, 99);
    let r = client.try_set_reference_value(&admin, &garbage, &10_300_000_i128, &1_700_000_000_u64);
    assert_eq!(r.err(), err(Error::FallbackNotConfigured));
}

#[test]
fn unrecognized_source_panics_invalid_source_when_nothing_is_configured_at_all() {
    // No `set_sources` call at all: `primary_source` is also `None`, so `FallbackNotConfigured`
    // (which presupposes a real primary) would be misleading -- this must be `InvalidSource`.
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = NOW);
    let contract_id = env.register_contract(None, OracleAdapterContract);
    let client = OracleAdapterContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    client.initialize(&admin, &MAX_STALE, &MAX_DEVIATION_BPS);
    let garbage = source(&env, 99);
    let r = client.try_set_reference_value(&admin, &garbage, &10_300_000_i128, &1_700_000_000_u64);
    assert_eq!(r.err(), err(Error::InvalidSource));
}

// --------------------------------------------------------------------------- primary submission

#[test]
fn initialize_and_set_price() {
    let (_env, client, admin, primary) = setup();
    client.set_reference_value(&admin, &primary, &103_00000000_i128, &1_700_000_000_u64);
    assert_eq!(client.get_reference_value(), 103_00000000_i128);
    assert_eq!(client.get_reference_timestamp(), 1_700_000_000_u64);
    assert_eq!(client.get_active_source(), Some(primary));
}

#[test]
fn value_decrease_is_rejected_gracefully_not_panicked() {
    // Soroban cannot persist a rejection record through a panic (the whole invocation, including
    // its events and storage writes, reverts), so a legitimate-looking submission that policy
    // rejects returns `false` and records why, instead of reverting -- see the module doc.
    let (_env, client, admin, primary) = setup();
    client.set_reference_value(&admin, &primary, &10_300_000_i128, &1_700_000_000_u64);
    let accepted =
        client.set_reference_value(&admin, &primary, &10_299_999_i128, &1_700_001_000_u64);
    assert!(!accepted);
    assert_eq!(client.get_reference_value(), 10_300_000_i128);
    assert_eq!(client.get_reference_timestamp(), 1_700_000_000_u64);
    let rejection = client.last_rejection().unwrap();
    assert_eq!(rejection.value, 10_299_999_i128);
    assert_eq!(rejection.timestamp, 1_700_001_000_u64);
    assert_eq!(rejection.reason, Symbol::new(&_env, "decrease"));
}

#[test]
fn value_may_stay_equal() {
    // A resubmission at the same price (e.g. a heartbeat refresh with no real price move) must
    // not be treated as a decrease.
    let (_env, client, admin, primary) = setup();
    client.set_reference_value(&admin, &primary, &10_300_000_i128, &1_700_000_000_u64);
    let accepted =
        client.set_reference_value(&admin, &primary, &10_300_000_i128, &1_700_001_000_u64);
    assert!(accepted);
    assert_eq!(client.get_reference_value(), 10_300_000_i128);
}

#[test]
fn unauthorized_price_update_panics() {
    let (env, client, admin, primary) = setup();
    client.set_reference_value(&admin, &primary, &103_00000000_i128, &1_700_000_000_u64);
    let attacker = Address::generate(&env);
    let r =
        client.try_set_reference_value(&attacker, &primary, &200_00000000_i128, &1_700_001_000_u64);
    assert_eq!(r.err(), err(Error::Unauthorized));
}

#[test]
fn stale_timestamp_is_rejected_gracefully() {
    let (_env, client, admin, primary) = setup();
    client.set_reference_value(&admin, &primary, &103_00000000_i128, &1_700_000_000_u64);
    // Same timestamp again, and an older one: both rejected without panicking. `stale_ts` is a
    // routine no-op, not a genuine policy violation, so it does not touch `last_rejection()`
    // (see `reject`'s own doc: only a reason reflecting an actual disallowed value change does).
    let same = client.set_reference_value(&admin, &primary, &103_00000000_i128, &1_700_000_000_u64);
    assert!(!same);
    assert!(client.last_rejection().is_none());
    let older =
        client.set_reference_value(&admin, &primary, &103_00000000_i128, &1_699_999_999_u64);
    assert!(!older);
}

#[test]
fn future_timestamp_rejected_so_the_feed_cannot_be_bricked() {
    // A timestamp ahead of the ledger clock (milliseconds instead of seconds, say) would read as
    // stale until real time caught up and lock out every correction, since each later timestamp
    // must exceed it. It is refused at submission instead -- this is malformed input, so it still
    // panics rather than being recorded as a graceful rejection.
    let (_env, client, admin, primary) = setup();
    client.set_reference_value(&admin, &primary, &10_300_000_i128, &1_700_000_000_u64);
    let r = client.try_set_reference_value(&admin, &primary, &10_300_000_i128, &(NOW * 1_000));
    assert_eq!(r.err(), err(Error::TimestampInFuture));
    // The exact ledger time is accepted (boundary), one second later is not.
    let r = client.try_set_reference_value(&admin, &primary, &10_300_000_i128, &(NOW + 1));
    assert_eq!(r.err(), err(Error::TimestampInFuture));
    client.set_reference_value(&admin, &primary, &10_300_000_i128, &NOW);
    assert_eq!(client.get_reference_timestamp(), NOW);
}

#[test]
fn invalid_price_rejected() {
    let (_env, client, admin, primary) = setup();
    let zero = client.try_set_reference_value(&admin, &primary, &0_i128, &1_700_000_000_u64);
    assert_eq!(zero.err(), err(Error::InvalidValue));
    let negative = client.try_set_reference_value(&admin, &primary, &(-5_i128), &1_700_000_000_u64);
    assert_eq!(negative.err(), err(Error::InvalidValue));
}

// ------------------------------------------------------------------------------- deviation guard

#[test]
fn deviation_within_bound_is_accepted() {
    let (_env, client, admin, primary) = setup();
    client.set_reference_value(&admin, &primary, &10_000_000_i128, &1_700_000_000_u64);
    // Exactly at the 10% boundary (1,000 bps) is still accepted.
    let accepted =
        client.set_reference_value(&admin, &primary, &11_000_000_i128, &1_700_001_000_u64);
    assert!(accepted);
    assert_eq!(client.get_reference_value(), 11_000_000_i128);
}

#[test]
fn deviation_over_bound_is_rejected_gracefully() {
    let (_env, client, admin, primary) = setup();
    client.set_reference_value(&admin, &primary, &10_000_000_i128, &1_700_000_000_u64);
    // 11% move, clear of the 10% boundary even after floor-rounding the bps division.
    let accepted =
        client.set_reference_value(&admin, &primary, &11_100_000_i128, &1_700_001_000_u64);
    assert!(!accepted);
    assert_eq!(client.get_reference_value(), 10_000_000_i128);
    assert_eq!(
        client.last_rejection().unwrap().reason,
        Symbol::new(&_env, "deviate")
    );
}

#[test]
fn deviation_guard_does_not_apply_to_the_first_ever_value() {
    // Nothing to deviate from yet -- any positive first value is accepted outright.
    let (_env, client, admin, primary) = setup();
    let accepted = client.set_reference_value(
        &admin,
        &primary,
        &1_000_000_000_000_i128,
        &1_700_000_000_u64,
    );
    assert!(accepted);
}

#[test]
fn deviation_overflow_is_rejected_gracefully_not_panicked() {
    // An astronomically large submission would overflow the raw `(value - current_price) *
    // BPS_SCALE` multiplication; this must go through the same graceful-rejection path as an
    // ordinary over-threshold deviation, not panic (which would also discard the rejection record
    // this module's own doc promises callers can read).
    let (_env, client, admin, primary) = setup();
    client.set_reference_value(&admin, &primary, &1_i128, &1_700_000_000_u64);
    let accepted = client.set_reference_value(&admin, &primary, &i128::MAX, &1_700_001_000_u64);
    assert!(!accepted);
    assert_eq!(client.get_reference_value(), 1_i128);
    assert_eq!(
        client.last_rejection().unwrap().reason,
        Symbol::new(&_env, "deviate")
    );
}

// --------------------------------------------------------------------------------- value bounds

#[test]
fn value_bounds_default_unset() {
    let (_env, client, _admin, _primary) = setup();
    assert_eq!(client.value_bounds(), None);
}

#[test]
fn set_value_bounds_validates_the_range() {
    let (_env, client, admin, _primary) = setup();
    assert_eq!(
        client
            .try_set_value_bounds(&admin, &0_i128, &100_i128)
            .err(),
        err(Error::InvalidValue)
    );
    assert_eq!(
        client
            .try_set_value_bounds(&admin, &100_i128, &100_i128)
            .err(),
        err(Error::InvalidValue)
    );
    assert_eq!(
        client
            .try_set_value_bounds(&admin, &100_i128, &50_i128)
            .err(),
        err(Error::InvalidValue)
    );
    client.set_value_bounds(&admin, &1_i128, &100_i128);
    assert_eq!(client.value_bounds(), Some((1_i128, 100_i128)));
}

#[test]
fn set_value_bounds_rejects_a_range_excluding_the_current_price() {
    // Found during a final audit review: without this check, an admin could set (or re-tighten)
    // bounds that exclude the live price. After that, no future submission could ever be both
    // "within bounds" and "not a decrease" -- a self-inflicted version of exactly the brick this
    // mechanism exists to prevent.
    let (_env, client, admin, primary) = setup();
    client.set_reference_value(&admin, &primary, &10_000_000_i128, &1_700_000_000_u64);

    // Entirely below the current price.
    assert_eq!(
        client
            .try_set_value_bounds(&admin, &1_i128, &9_999_999_i128)
            .err(),
        err(Error::InvalidValue)
    );
    // Entirely above the current price.
    assert_eq!(
        client
            .try_set_value_bounds(&admin, &10_000_001_i128, &100_000_000_i128)
            .err(),
        err(Error::InvalidValue)
    );
    // A range that actually contains the current price is still accepted.
    client.set_value_bounds(&admin, &5_000_000_i128, &20_000_000_i128);
    assert_eq!(
        client.value_bounds(),
        Some((5_000_000_i128, 20_000_000_i128))
    );
    // The boundary itself (current price exactly equal to min or max) is inclusive, not rejected.
    client.set_value_bounds(&admin, &10_000_000_i128, &20_000_000_i128);
    client.set_value_bounds(&admin, &5_000_000_i128, &10_000_000_i128);
}

#[test]
fn set_value_bounds_by_non_admin_panics() {
    let (env, client, _admin, _primary) = setup();
    let impostor = Address::generate(&env);
    let r = client.try_set_value_bounds(&impostor, &1_i128, &100_i128);
    assert_eq!(r.err(), err(Error::Unauthorized));
}

#[test]
fn value_bounds_reject_an_out_of_range_first_ever_value() {
    // The exact scenario the bounds exist to catch: a relay decimal/scale bug inflates the
    // genesis submission by orders of magnitude. Without this check it would be accepted
    // outright (the deviation guard is exempt for the first value, by design -- see above), and
    // every subsequent *correct* submission would then read as a `decrease` and be rejected
    // forever, since nothing can move the stored value back down.
    let (_env, client, admin, primary) = setup();
    client.set_value_bounds(&admin, &1_i128, &100_000_000_i128); // sane range: up to 10x SCALE
    let accepted = client.set_reference_value(
        &admin,
        &primary,
        &1_000_000_000_000_i128,
        &1_700_000_000_u64,
    );
    assert!(!accepted);
    assert_eq!(
        client.last_rejection().unwrap().reason,
        Symbol::new(&_env, "oob")
    );
    // The feed is left unset, not stuck on the bad value -- a correct genesis value can still be
    // submitted afterward.
    assert!(client.try_get_reference_value().is_err());
    let corrected =
        client.set_reference_value(&admin, &primary, &10_000_000_i128, &1_700_001_000_u64);
    assert!(corrected);
}

#[test]
fn value_bounds_reject_an_out_of_range_subsequent_value() {
    let (_env, client, admin, primary) = setup();
    client.set_reference_value(&admin, &primary, &10_000_000_i128, &1_700_000_000_u64);
    client.set_value_bounds(&admin, &1_i128, &20_000_000_i128);
    let accepted =
        client.set_reference_value(&admin, &primary, &20_000_001_i128, &1_700_001_000_u64);
    assert!(!accepted);
    assert_eq!(client.get_reference_value(), 10_000_000_i128);
}

#[test]
fn value_within_bounds_is_still_accepted() {
    let (_env, client, admin, primary) = setup();
    client.set_value_bounds(&admin, &1_i128, &100_000_000_i128);
    let accepted =
        client.set_reference_value(&admin, &primary, &10_000_000_i128, &1_700_000_000_u64);
    assert!(accepted);
}

// --------------------------------------------------------------------------------------- fallback

#[test]
fn fallback_may_refresh_timestamp_only() {
    let (env, client, admin, primary) = setup();
    let fallback = source(&env, 2);
    client.set_sources(&admin, &primary, &Some(fallback.clone()));
    client.set_reference_value(&admin, &primary, &10_300_000_i128, &1_700_000_000_u64);

    // Same value, newer timestamp, submitted via fallback: accepted, active source flips.
    let accepted =
        client.set_reference_value(&admin, &fallback, &10_300_000_i128, &1_700_001_000_u64);
    assert!(accepted);
    assert_eq!(client.get_reference_value(), 10_300_000_i128);
    assert_eq!(client.get_reference_timestamp(), 1_700_001_000_u64);
    assert_eq!(client.get_active_source(), Some(fallback));
}

#[test]
fn fallback_cannot_move_the_value_even_within_the_deviation_bound() {
    let (env, client, admin, primary) = setup();
    let fallback = source(&env, 2);
    client.set_sources(&admin, &primary, &Some(fallback.clone()));
    client.set_reference_value(&admin, &primary, &10_300_000_i128, &1_700_000_000_u64);

    // A fallback value that differs from the current price -- even a small, in-bound move -- is
    // rejected gracefully: the fallback's only job is to keep the timestamp fresh.
    let accepted =
        client.set_reference_value(&admin, &fallback, &10_300_001_i128, &1_700_001_000_u64);
    assert!(!accepted);
    assert_eq!(client.get_reference_value(), 10_300_000_i128);
    assert_eq!(client.get_reference_timestamp(), 1_700_000_000_u64);
    assert_eq!(
        client.last_rejection().unwrap().reason,
        Symbol::new(&env, "fb_value")
    );
}

#[test]
fn fallback_never_hides_a_stale_timestamp() {
    let (env, client, admin, primary) = setup();
    let fallback = source(&env, 2);
    client.set_sources(&admin, &primary, &Some(fallback.clone()));
    client.set_reference_value(&admin, &primary, &10_300_000_i128, &1_700_000_000_u64);

    // Same value, but not a newer timestamp -- still a graceful rejection, not a free pass.
    let accepted =
        client.set_reference_value(&admin, &fallback, &10_300_000_i128, &1_700_000_000_u64);
    assert!(!accepted);
}

// ------------------------------------------------------------------------- availability signaling

#[test]
fn is_available_follows_the_contract_s_own_configured_staleness() {
    let (env, client, admin, primary) = setup();
    env.ledger().with_mut(|li| li.timestamp = 1_000);
    client.set_reference_value(&admin, &primary, &10_300_000_i128, &900_u64);
    assert!(client.is_available()); // diff = 100 <= MAX_STALE (3_600)

    env.ledger()
        .with_mut(|li| li.timestamp = 900 + MAX_STALE + 1);
    assert!(!client.is_available());
}

#[test]
fn assert_available_panics_oracle_unavailable_when_stale() {
    let (env, client, admin, primary) = setup();
    env.ledger().with_mut(|li| li.timestamp = 1_000);
    client.set_reference_value(&admin, &primary, &10_300_000_i128, &900_u64);
    env.ledger()
        .with_mut(|li| li.timestamp = 900 + MAX_STALE + 1);
    let r = client.try_assert_available();
    assert_eq!(r.err(), err(Error::OracleUnavailable));
}

#[test]
fn assert_available_is_silent_when_fresh() {
    let (env, client, admin, primary) = setup();
    env.ledger().with_mut(|li| li.timestamp = 1_000);
    client.set_reference_value(&admin, &primary, &10_300_000_i128, &900_u64);
    client.assert_available(); // must not panic
}

// ----------------------------------------------------------------------------------------- views

#[test]
fn views_before_any_price_revert() {
    let (_env, client, _admin, _primary) = setup();
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
fn last_rejection_is_none_before_any_rejection() {
    let (_env, client, _admin, _primary) = setup();
    assert_eq!(client.last_rejection(), None);
}

#[test]
fn admin_transfer() {
    let (env, client, admin, _primary) = setup();
    let new_admin = Address::generate(&env);
    client.transfer_admin(&admin, &new_admin);
    assert_eq!(client.get_admin(), new_admin);
}

#[test]
fn unauthorized_transfer_admin_panics() {
    let (env, client, _admin, _primary) = setup();
    let impostor = Address::generate(&env);
    let new_admin = Address::generate(&env);
    let r = client.try_transfer_admin(&impostor, &new_admin);
    assert_eq!(r.err(), err(Error::Unauthorized));
}

#[test]
fn is_fresh_within_staleness_window() {
    let (env, client, admin, primary) = setup();
    // Ledger at t=1000; oracle set at t=900 → diff=100 ≤ 3600 → fresh.
    env.ledger().with_mut(|li| li.timestamp = 1_000);
    client.set_reference_value(&admin, &primary, &10_300_000_i128, &900_u64);
    assert!(client.is_fresh(&3_600_u64));
}

#[test]
fn is_fresh_false_when_stale() {
    let (env, client, admin, primary) = setup();
    // Ledger at t=1000; oracle set at t=1 → diff=999 > 100 → stale.
    env.ledger().with_mut(|li| li.timestamp = 1_000);
    client.set_reference_value(&admin, &primary, &10_300_000_i128, &1_u64);
    assert!(!client.is_fresh(&100_u64));
}

#[test]
fn is_fresh_false_before_any_price_set() {
    let (env, client, _admin, _primary) = setup();
    // No price ever set → stored timestamp = 0; ledger = 5000; diff > 3600 → not fresh.
    env.ledger().with_mut(|li| li.timestamp = 5_000);
    assert!(!client.is_fresh(&3_600_u64));
}

#[test]
fn is_fresh_at_exact_boundary() {
    let (env, client, admin, primary) = setup();
    // diff == max_stale_seconds exactly → still fresh (≤ not <).
    env.ledger().with_mut(|li| li.timestamp = 1_000);
    client.set_reference_value(&admin, &primary, &10_000_000_i128, &600_u64); // diff = 400
    assert!(client.is_fresh(&400_u64)); // 400 ≤ 400 → fresh
    assert!(!client.is_fresh(&399_u64)); // 400 > 399 → stale
}

#[test]
fn is_fresh_false_when_stored_timestamp_is_ahead_of_ledger_clock() {
    // Submissions can no longer be future-dated, but the ledger clock can still read *behind* the
    // stored timestamp (e.g. in tests, or after a clock correction). `is_fresh` must not treat
    // that as fresh via an underflowed subtraction; it explicitly returns false.
    let (env, client, admin, primary) = setup();
    env.ledger().with_mut(|li| li.timestamp = 5_000);
    client.set_reference_value(&admin, &primary, &10_000_000_i128, &5_000_u64);
    env.ledger().with_mut(|li| li.timestamp = 1_000);
    assert!(!client.is_fresh(&3_600_u64));
}

// --------------------------------------------------------------- sync_from_feed (SEP-40 pull)

fn feed_asset(env: &Env) -> Sep40Asset {
    Sep40Asset::Stellar(Address::generate(env))
}

/// `setup()` plus a `MockSep40Feed` at `feed_decimals`, configured as this oracle's feed source
/// for `asset`. The feed itself starts with no price set (`lastprice()` returns `None`) --
/// individual tests call `feed.set_price(...)` as needed.
fn setup_with_feed(
    feed_decimals: u32,
) -> (
    Env,
    OracleAdapterContractClient<'static>,
    Address,
    Sep40Asset,
    soroban_sdk::Address,
) {
    let (env, client, admin, _primary) = setup();
    let feed_id = env.register_contract(None, MockSep40Feed);
    let feed = crate::test::MockSep40FeedClient::new(&env, &feed_id);
    feed.initialize(&feed_decimals);
    let asset = feed_asset(&env);
    client.set_feed_source(&admin, &feed_id, &asset);
    (env, client, admin, asset, feed_id)
}

#[test]
fn sync_from_feed_accepts_a_price_already_at_protocol_scale() {
    let (env, client, _admin, asset, feed_id) = setup_with_feed(7);
    let feed = MockSep40FeedClient::new(&env, &feed_id);
    feed.set_price(&asset, &10_500_000_i128, &100_u64);
    env.ledger().with_mut(|li| li.timestamp = 200);

    assert!(client.sync_from_feed());
    assert_eq!(client.get_reference_value(), 10_500_000);
    assert_eq!(client.get_reference_timestamp(), 100);
    assert!(client.last_update_was_feed());
}

#[test]
fn sync_from_feed_rescales_down_from_a_feed_with_more_decimals() {
    // Feed at 8 decimals: 150_000_000 == 1.5. Protocol is 7 decimals: 1.5 == 15_000_000.
    let (env, client, _admin, asset, feed_id) = setup_with_feed(8);
    let feed = MockSep40FeedClient::new(&env, &feed_id);
    feed.set_price(&asset, &150_000_000_i128, &100_u64);
    env.ledger().with_mut(|li| li.timestamp = 200);

    assert!(client.sync_from_feed());
    assert_eq!(client.get_reference_value(), 15_000_000);
}

#[test]
fn sync_from_feed_rescales_up_from_a_feed_with_fewer_decimals() {
    // Feed at 5 decimals: 150_000 == 1.5. Protocol is 7 decimals: 1.5 == 15_000_000.
    let (env, client, _admin, asset, feed_id) = setup_with_feed(5);
    let feed = MockSep40FeedClient::new(&env, &feed_id);
    feed.set_price(&asset, &150_000_i128, &100_u64);
    env.ledger().with_mut(|li| li.timestamp = 200);

    assert!(client.sync_from_feed());
    assert_eq!(client.get_reference_value(), 15_000_000);
}

#[test]
fn sync_from_feed_rejects_gracefully_when_the_feed_has_no_data() {
    let (env, client, _admin, _asset, _feed_id) = setup_with_feed(7);
    env.ledger().with_mut(|li| li.timestamp = 200);

    assert!(!client.sync_from_feed());
    // "no_data" is routine feed noise, not a policy violation -- it does not touch
    // `last_rejection()` (see `reject`'s own doc).
    assert!(client.last_rejection().is_none());
}

#[test]
fn sync_from_feed_rejects_a_non_positive_price_gracefully() {
    let (env, client, _admin, asset, feed_id) = setup_with_feed(7);
    let feed = MockSep40FeedClient::new(&env, &feed_id);
    feed.set_price(&asset, &0_i128, &100_u64);
    env.ledger().with_mut(|li| li.timestamp = 200);

    assert!(!client.sync_from_feed());
    // A malformed tick from the feed, not an attempted policy-violating value -- routine, does
    // not touch `last_rejection()`.
    assert!(client.last_rejection().is_none());
}

#[test]
fn sync_from_feed_rejects_a_future_timestamp_gracefully_unlike_set_reference_value() {
    // The push path panics TimestampInFuture for this -- sync_from_feed must not: the feed, not
    // the caller, produced the bad data, and the caller here could be an unattended keeper.
    let (env, client, _admin, asset, feed_id) = setup_with_feed(7);
    let feed = MockSep40FeedClient::new(&env, &feed_id);
    env.ledger().with_mut(|li| li.timestamp = 100);
    feed.set_price(&asset, &10_000_000_i128, &500_u64); // ahead of the ledger clock

    assert!(!client.sync_from_feed());
    // The feed's fault, not an attempted policy-violating value -- routine, does not touch
    // `last_rejection()`.
    assert!(client.last_rejection().is_none());
}

#[test]
fn sync_from_feed_rejects_a_stale_or_repeated_timestamp() {
    let (env, client, admin, asset, feed_id) = setup_with_feed(7);
    let feed = MockSep40FeedClient::new(&env, &feed_id);
    env.ledger().with_mut(|li| li.timestamp = 1_000);
    client.set_reference_value(
        &admin,
        &BytesN::from_array(&env, &{
            let mut b = [0u8; 32];
            b[0] = 1;
            b
        }),
        &10_000_000_i128,
        &100_u64,
    );
    feed.set_price(&asset, &10_500_000_i128, &100_u64); // same timestamp as what's stored

    assert!(!client.sync_from_feed());
    // No new data from the feed, not an attempted policy-violating value -- routine, does not
    // touch `last_rejection()`. This is exactly the case the fix closes: before it, anyone could
    // call `sync_from_feed` with stale data for free to mask a genuine prior `deviate`/`decrease`
    // rejection from this view.
    assert!(client.last_rejection().is_none());
}

#[test]
fn sync_from_feed_rejects_a_decrease() {
    let (env, client, admin, asset, feed_id) = setup_with_feed(7);
    let feed = MockSep40FeedClient::new(&env, &feed_id);
    let primary = BytesN::from_array(&env, &{
        let mut b = [0u8; 32];
        b[0] = 1;
        b
    });
    client.set_reference_value(&admin, &primary, &10_000_000_i128, &100_u64);
    feed.set_price(&asset, &9_000_000_i128, &200_u64);
    env.ledger().with_mut(|li| li.timestamp = 300);

    assert!(!client.sync_from_feed());
    assert_eq!(
        client.last_rejection().unwrap().reason,
        Symbol::new(&env, "decrease")
    );
    // Rejection must not disturb the value a push submission already established.
    assert_eq!(client.get_reference_value(), 10_000_000);
    assert!(!client.last_update_was_feed());
}

#[test]
fn sync_from_feed_rejects_excess_deviation() {
    let (env, client, admin, asset, feed_id) = setup_with_feed(7);
    let feed = MockSep40FeedClient::new(&env, &feed_id);
    let primary = BytesN::from_array(&env, &{
        let mut b = [0u8; 32];
        b[0] = 1;
        b
    });
    client.set_reference_value(&admin, &primary, &10_000_000_i128, &100_u64);
    // MAX_DEVIATION_BPS is 1_000 (10%); +50% is well over it.
    feed.set_price(&asset, &15_000_000_i128, &200_u64);
    env.ledger().with_mut(|li| li.timestamp = 300);

    assert!(!client.sync_from_feed());
    assert_eq!(
        client.last_rejection().unwrap().reason,
        Symbol::new(&env, "deviate")
    );
}

#[test]
fn a_routine_rejection_cannot_mask_a_prior_significant_one() {
    // Regression for the review finding this fix closes: `sync_from_feed` is callable by
    // anyone, for free, at any time -- before this fix, a call that found nothing wrong to
    // report (e.g. "no new data yet") would overwrite the single `last_rejection()` slot a
    // genuine policy violation had just written, letting anyone mask an active deviation
    // attempt from a monitor that reads only that view.
    let (env, client, admin, asset, feed_id) = setup_with_feed(7);
    let feed = MockSep40FeedClient::new(&env, &feed_id);
    let primary = BytesN::from_array(&env, &{
        let mut b = [0u8; 32];
        b[0] = 1;
        b
    });
    client.set_reference_value(&admin, &primary, &10_000_000_i128, &100_u64);
    // A genuine deviation attempt: +50%, well over the 10% bound.
    feed.set_price(&asset, &15_000_000_i128, &200_u64);
    env.ledger().with_mut(|li| li.timestamp = 300);
    assert!(!client.sync_from_feed());
    assert_eq!(
        client.last_rejection().unwrap().reason,
        Symbol::new(&env, "deviate")
    );

    // Anyone, with no privilege at all, immediately calls again with no new data (the feed's
    // timestamp repeats the value still on record) -- a routine no-op rejection that must not be
    // able to hide the `deviate` event above.
    feed.set_price(&asset, &9_000_000_i128, &100_u64);
    assert!(!client.sync_from_feed());
    // Still "deviate", unchanged -- the routine rejection above did not touch the slot.
    assert_eq!(
        client.last_rejection().unwrap().reason,
        Symbol::new(&env, "deviate")
    );
}

#[test]
fn sync_from_feed_respects_value_bounds() {
    let (env, client, admin, asset, feed_id) = setup_with_feed(7);
    let feed = MockSep40FeedClient::new(&env, &feed_id);
    client.set_value_bounds(&admin, &1_i128, &12_000_000_i128);
    feed.set_price(&asset, &13_000_000_i128, &100_u64);
    env.ledger().with_mut(|li| li.timestamp = 200);

    assert!(!client.sync_from_feed());
    assert_eq!(
        client.last_rejection().unwrap().reason,
        Symbol::new(&env, "oob")
    );
}

#[test]
fn sync_from_feed_panics_when_no_feed_is_configured() {
    let (_env, client, _admin, _primary) = setup();
    let r = client.try_sync_from_feed();
    assert_eq!(r.err(), err(Error::FeedNotConfigured));
}

#[test]
fn set_feed_source_requires_admin() {
    let (env, client, _admin, _primary) = setup();
    let not_admin = Address::generate(&env);
    let feed_id = env.register_contract(None, MockSep40Feed);
    let asset = feed_asset(&env);
    let r = client.try_set_feed_source(&not_admin, &feed_id, &asset);
    assert_eq!(r.err(), err(Error::Unauthorized));
}

#[test]
fn last_update_was_feed_toggles_correctly_between_push_and_pull() {
    let (env, client, admin, asset, feed_id) = setup_with_feed(7);
    let feed = MockSep40FeedClient::new(&env, &feed_id);
    let primary = BytesN::from_array(&env, &{
        let mut b = [0u8; 32];
        b[0] = 1;
        b
    });
    assert!(!client.last_update_was_feed()); // nothing accepted yet

    client.set_reference_value(&admin, &primary, &10_000_000_i128, &100_u64);
    assert!(!client.last_update_was_feed());

    feed.set_price(&asset, &10_200_000_i128, &200_u64);
    env.ledger().with_mut(|li| li.timestamp = 300);
    assert!(client.sync_from_feed());
    assert!(client.last_update_was_feed());

    env.ledger().with_mut(|li| li.timestamp = 500);
    client.set_reference_value(&admin, &primary, &10_300_000_i128, &400_u64);
    assert!(!client.last_update_was_feed());
}
