//! Regression tests for the defects found by the Tranche 1 internal audit (docs/TRANCHE_1_AUDIT.md).
//! Each test reproduces the scenario an auditor demonstrated and asserts the corrected behaviour.

use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    Address, Vec,
};

use principal_integration_tests::stack::*;
use principal_market_pool::Error as PoolError;
use principal_recovery_escrow::{Error as EscrowError, SeizeRequest};
use principal_router::Error as RouterError;

fn seeded(cfg: Config) -> Stack<'static> {
    let s = deploy(cfg);
    seed_pool(&s, 1_000 * SCALE, 950 * SCALE);
    s
}

// ---- AMM-01 / stale index: a plain transfer or a flash-redeem must not move or strand yield ----

#[test]
fn flash_redeem_leaves_the_sellers_accrued_yield_with_the_seller() {
    let s = seeded(Config::free(T0 + 180 * DAY));
    let pool = s.pool.address.clone();
    let holder = s.new_user();
    deposit_sy(&s, &holder, 100 * SCALE);
    let (yt, _) = s.router.swap_sy_for_yt(
        &holder,
        &pool,
        &(100 * SCALE),
        &0,
        &(100 * SCALE),
        &u64::MAX,
    );

    // 30 days pass and the oracle moves 1.00 -> 1.05; NOBODY calls update_yield_index, so the
    // stored index is stale when the holder sells.
    s.advance(T0 + 30 * DAY, SCALE * 105 / 100);
    assert_eq!(s.yt.accrued_yield_index(), 1_000_000_000_000);
    s.router.swap_yt_for_sy(&holder, &pool, &yt, &0, &u64::MAX);

    // The yield the holder earned while holding the YT is theirs and claimable; none of it was
    // handed to (or stranded in) the pool. 100 x (1 - 1/1.05) = 4.7619...
    assert_eq!(
        s.yt.pending_claim(&s.pool.address),
        0,
        "nothing stranded in the pool"
    );
    let claimable = s.yt.pending_claim(&holder);
    assert!(
        (claimable - 47_619_047).abs() <= 3,
        "holder's accrued yield {claimable}"
    );
    let paid = s.pm.claim_yield(&holder);
    assert!((paid - 47_619_047).abs() <= 3, "paid {paid}");
}

#[test]
fn a_plain_yt_transfer_does_not_hand_unsynced_yield_to_the_receiver() {
    let s = deploy(Config::free(LONG));
    let a = s.new_user();
    let b = s.new_user();
    let shares = deposit_sy(&s, &a, 100 * SCALE);
    s.pm.mint(&a, &shares);
    s.advance(T0 + 10 * DAY, SCALE * 11 / 10); // no update_yield_index
    s.yt.transfer(&a, &b, &(100 * SCALE));
    assert!((s.yt.pending_claim(&a) - 90_909_090).abs() <= 3);
    assert_eq!(s.yt.pending_claim(&b), 0);
}

// ---- AMM-02: the flash-mint price leg is guarded ----

#[test]
fn flash_mint_reverts_when_the_pt_sells_for_less_than_the_callers_max_net_cost() {
    let s = seeded(Config::free(T0 + 365 * DAY - 1));
    let pool = s.pool.address.clone();
    let victim = s.new_user();
    deposit_sy(&s, &victim, 100 * SCALE);

    // The honest price of the leg: what the 100 PT would fetch right now.
    let honest = s.pool.quote_pt_for_sy(&(100 * SCALE)).amount_out;
    let honest_cost = 100 * SCALE - honest;

    // A sandwicher first dumps PT, making the victim's PT sell for less.
    let attacker = s.new_user();
    let sh = deposit_sy(&s, &attacker, 400 * SCALE);
    s.pm.mint(&attacker, &sh);
    s.pool
        .swap_pt_for_sy(&attacker, &attacker, &(250 * SCALE), &0);

    // The victim signed with their honest-quote net cost (+1% tolerance): now it must revert.
    let max_cost = honest_cost + honest_cost / 100;
    let r = s.router.try_swap_sy_for_yt(
        &victim,
        &pool,
        &(100 * SCALE),
        &(100 * SCALE),
        &max_cost,
        &u64::MAX,
    );
    assert_eq!(r.err(), err_code(PoolError::SlippageExceeded as u32));
    // Nothing moved for the victim.
    assert_eq!(s.sy.balance_of(&victim), 100 * SCALE);
    assert_eq!(s.yt.balance(&victim), 0);
}

// ---- L-01: a silent relay after maturity cannot lock PT once the index has frozen ----

#[test]
fn settlement_needs_no_oracle_once_the_yt_index_is_frozen() {
    let maturity = T0 + 100;
    let s = deploy_stack(maturity);
    let u = s.new_user();
    let shares = deposit_sy(&s, &u, 100 * SCALE);
    s.pm.mint(&u, &shares);

    s.advance(maturity, SCALE * 11 / 10);
    s.yt.update_yield_index(); // any keeper freezes the index at a fresh observation
    assert!(s.yt.is_frozen());

    // The relay then goes silent for a day.
    s.env.ledger().with_mut(|li| li.timestamp = maturity + DAY);
    assert_eq!(s.pm.settle_all(), SCALE * 11 / 10);
    let out = s.pm.redeem(&u, &(100 * SCALE), &(100 * SCALE));
    assert!(out.underlying_from_pt > 0);
}

// ---- C-01: rounding must never let claims exceed custody ----

#[test]
fn a_very_large_position_can_always_be_fully_redeemed() {
    // Two index round-ups could over-credit YT by up to one index unit per settlement; on a
    // 1e15-unit position at these rates that used to exceed custody by 1 187 units and make the
    // last redeem revert.
    let s = deploy(Config::free(T0 + 10 * DAY));
    let u = s.new_user();
    let n: i128 = 1_000_000_000_000_000;
    deposit_sy(&s, &u, n);
    s.advance(T0 + 100, 11_974_350);
    let m = s.pm.mint(&u, &n);
    s.advance(T0 + 200, 12_871_084);
    s.advance(s.maturity + 1, 12_871_084);
    s.pm.settle_all();
    let out = s.pm.redeem(&u, &m.pt_minted, &m.yt_minted);
    assert!(out.underlying_from_pt + out.underlying_from_yt <= n);
}

// ---- A-05 / E-01: accrued YT yield is recovered with the position ----

#[test]
fn seizing_yt_moves_its_accrued_yield_to_the_escrow_and_records_it() {
    for kind in [UnderlyingKind::Sac, UnderlyingKind::Rwa] {
        let s = deploy_kind(Config::free(T0 + 200 * DAY), kind);
        let flagged = s.new_user();
        let shares = deposit_sy(&s, &flagged, 100 * SCALE);
        let m = s.pm.mint(&flagged, &shares);

        s.advance(T0 + 50 * DAY, SCALE * 3 / 2); // 1.0 -> 1.5 while the account holds the YT
        s.deauthorize(&flagged);
        s.escrow.seize_yt(&s.admin, &flagged, &m.yt_minted);

        // The accrued yield (100 x (1 - 1/1.5) = 33.33) left the flagged account and is recorded.
        assert_eq!(s.yt.pending_claim(&flagged), 0, "{kind:?}");
        let rec = s.escrow.get_record(&0);
        assert!(
            (rec.yt_yield_at_seize - 333_333_333).abs() <= 3,
            "{kind:?}: recorded {}",
            rec.yt_yield_at_seize
        );
        assert_eq!(s.yt.pending_claim(&s.escrow.address), rec.yt_yield_at_seize);

        // At maturity the escrow recovers it: finalize pays at least the pre-seizure yield.
        s.advance(s.maturity, SCALE * 2);
        let (_, yt_under) = s.escrow.finalize_record(&s.admin, &0);
        assert!(yt_under >= rec.yt_yield_at_seize, "{kind:?}");
    }
}

// ---- A-04: the batch bound is achievable ----

#[test]
fn batch_bound_is_three_and_is_checked_before_any_balance_is_read() {
    assert_eq!(principal_recovery_escrow::MAX_BATCH, 3);
    let s = deploy_stack(T0 + 200 * DAY);
    let mut accounts: Vec<Address> = Vec::new(&s.env);
    for _ in 0..5 {
        accounts.push_back(Address::generate(&s.env)); // never granted: would fail if read
    }
    assert_eq!(
        s.escrow.try_seize_all_positions(&s.admin, &accounts).err(),
        err_code(EscrowError::BatchTooLarge as u32)
    );
    let mut requests: Vec<SeizeRequest> = Vec::new(&s.env);
    for a in accounts.iter() {
        requests.push_back(SeizeRequest {
            account: a,
            sy_shares: 1,
            pt_amount: 0,
            yt_amount: 0,
            lp_amount: 0,
        });
    }
    assert_eq!(
        s.escrow.try_seize_batch(&s.admin, &requests).err(),
        err_code(EscrowError::BatchTooLarge as u32)
    );
}

#[test]
fn a_full_batch_at_the_bound_of_every_position_type_succeeds() {
    let s = deploy_stack(T0 + 200 * DAY);
    seed_pool(&s, 1_000 * SCALE, 950 * SCALE);
    let mut accounts: Vec<Address> = Vec::new(&s.env);
    for _ in 0..principal_recovery_escrow::MAX_BATCH {
        let u = s.new_user();
        let sh = deposit_sy(&s, &u, 300 * SCALE);
        s.pm.mint(&u, &(100 * SCALE));
        let (_, _, _) = s.pool.add_liquidity(&u, &(50 * SCALE), &(40 * SCALE), &0);
        let _ = sh;
        s.deauthorize(&u);
        accounts.push_back(u);
    }
    let ids = s.escrow.seize_all_positions(&s.admin, &accounts);
    assert_eq!(ids.len(), principal_recovery_escrow::MAX_BATCH);
}

// ---- oracle: a future timestamp can no longer brick the feed (see also the oracle unit tests) ----

#[test]
fn a_future_dated_oracle_update_is_rejected_and_the_market_keeps_working() {
    let s = deploy_stack(LONG);
    let u = s.new_user();
    let shares = deposit_sy(&s, &u, 10 * SCALE);
    let r = s
        .oracle
        .try_set_reference_value(&s.admin, &SCALE, &((T0 + 5) * 1_000));
    assert!(r.is_err());
    s.advance(T0 + 5, SCALE);
    s.pm.mint(&u, &shares);
}

// ---- AMM-04: LP exit while paused ----

#[test]
fn a_paused_pool_blocks_lp_exit_while_live_but_never_after_maturity() {
    let s = deploy(Config::free(T0 + 180 * DAY));
    let (lp_minted, lp) = seed_pool(&s, 1_000 * SCALE, 950 * SCALE);
    s.pool.set_paused(&s.admin, &true);
    assert_eq!(
        s.pool
            .try_remove_liquidity(&lp, &lp, &(lp_minted / 2), &0, &0)
            .err(),
        err_code(PoolError::Paused as u32)
    );
    s.advance_flat(s.maturity);
    let (pt, sy) = s.pool.remove_liquidity(&lp, &lp, &(lp_minted / 2), &0, &0);
    assert!(pt > 0 && sy > 0);
}

// ---- AMM-06 and keeper functions ----

#[test]
fn router_markets_can_be_unregistered_and_contracts_can_be_bumped() {
    let s = deploy_stack(T0 + 180 * DAY);
    let stranger = Address::generate(&s.env);
    assert_eq!(
        s.router
            .try_unregister_market(&stranger, &s.pool.address)
            .err(),
        err_code(RouterError::Unauthorized as u32)
    );
    s.router.unregister_market(&s.admin, &s.pool.address);
    assert!(!s.router.is_registered(&s.pool.address));
    assert_eq!(
        s.router
            .try_unregister_market(&s.admin, &s.pool.address)
            .err(),
        err_code(RouterError::MarketNotRegistered as u32)
    );

    // Permissionless instance-TTL keeper calls exist on every long-lived contract and need no auth.
    s.env.mock_auths(&[]);
    s.sy.bump();
    s.pt.bump();
    s.yt.bump();
    s.pm.bump();
    s.pool.bump();
    s.router.bump();
    s.config.bump();
    s.escrow.bump();
}
