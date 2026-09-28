//! End-to-end lifecycle against the real stack: wrap -> tokenize -> trade -> provide liquidity ->
//! claim yield -> settle at maturity -> redeem -> claim fees, with the documented example fees, plus
//! the maturity-settlement, recombination and fee-accounting behaviours on their own.

use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    Address,
};

use principal_integration_tests::stack::*;
use principal_manager::Error as PmError;

const FAR: u64 = u64::MAX;

fn shares_at(rate: i128, notional: i128) -> i128 {
    notional * SCALE / rate
}

#[test]
fn deposit_tokenize_trade_claim_settle_redeem_with_fees_stays_solvent() {
    let maturity = T0 + 180 * DAY;
    let s = deploy(Config::with_example_fees(maturity));
    let pool = s.pool.address.clone();

    // --- day 0: an LP seeds the pool; Alice wraps + tokenizes; Bob buys PT; Carol buys YT ------
    let (_, lp) = seed_pool(&s, 1_000 * SCALE, 950 * SCALE);

    let alice = s.new_user();
    s.sac().mint(&alice, &(500 * SCALE));
    let minted = s
        .router
        .wrap_and_mint(&alice, &pool, &(500 * SCALE), &0, &FAR);
    // 5 bps tokenization fee: 0.25 of the 500 SY is withheld, the rest becomes 499.75 PT + YT.
    assert_eq!(minted.fee_shares, 2_500_000);
    assert_eq!(minted.pt_minted, 4_997_500_000);
    assert_eq!(minted.yt_minted, 4_997_500_000);

    let bob = s.new_user();
    deposit_sy(&s, &bob, 100 * SCALE);
    let bob_pt = s
        .router
        .swap_sy_for_pt(&bob, &pool, &(100 * SCALE), &0, &FAR);
    assert!(bob_pt > 99 * SCALE);

    let carol = s.new_user();
    deposit_sy(&s, &carol, 200 * SCALE);
    let (carol_yt, _) = s
        .router
        .swap_sy_for_yt(&carol, &pool, &(200 * SCALE), &0, &FAR);
    assert!(carol_yt > 199 * SCALE && carol_yt < 200 * SCALE); // minus the 5 bps fee

    // --- day 30: rate 1.01. Alice claims mid-life yield (10% YT fee withheld) ------------------
    s.advance(T0 + 30 * DAY, SCALE * 101 / 100);
    let alice_claim = s.router.claim_yield(&alice, &pool);
    // gross = 499.75 * (1.01 - 1.00)/1.01 = 4.94802 ; net of the 10% YT fee = 4.45322
    let gross = 499.75_f64 * 0.01 / 1.01;
    let expected_net = (gross * 0.9 * 1e7) as i128;
    assert!(
        abs_diff(alice_claim, expected_net) < 50,
        "claimed {alice_claim}, expected ~{expected_net}"
    );
    assert_eq!(underlying_balance(&s, &alice), alice_claim);
    assert_eq!(
        s.pm.yt_balance(&alice),
        minted.yt_minted,
        "claiming never burns YT"
    );

    // --- day 90: more trading, an LP top-up and a partial exit --------------------------------
    s.advance(T0 + 90 * DAY, SCALE * 1025 / 1000);
    s.router
        .swap_pt_for_sy(&bob, &pool, &(30 * SCALE), &0, &FAR);
    let lp_bal = s.pool.lp_balance(&lp);
    s.router
        .remove_liquidity(&lp, &pool, &(lp_bal / 10), &0, &0, &FAR);

    // --- maturity: rate 1.04; a keeper (anyone) freezes the settlement rate -------------------
    let final_rate = SCALE * 104 / 100;
    s.advance(maturity, final_rate);
    assert_eq!(s.pm.settled_rate(), None);
    let keeper = Address::generate(&s.env);
    let _ = keeper; // permissionless: no auth needed beyond the tx itself
    assert_eq!(s.pm.settle_all(), final_rate);
    assert_eq!(s.pm.settled_rate(), Some(final_rate));

    // The oracle keeps rising after maturity; nothing about settlement moves.
    s.advance(maturity + 5 * DAY, SCALE * 110 / 100);
    assert_eq!(
        s.pm.settle_all(),
        final_rate,
        "settle_all is idempotent and frozen"
    );

    // --- everyone redeems ---------------------------------------------------------------------
    let alice_pt = s.pm.pt_balance(&alice);
    let alice_out = s
        .router
        .redeem_at_maturity(&alice, &pool, &alice_pt, &s.pm.yt_balance(&alice));
    // PT leg: 499.75 PT at the frozen 1.04 => 499.75/1.04 underlying.
    let expected_pt = alice_pt * SCALE / final_rate;
    assert!(abs_diff(alice_out.underlying_from_pt, expected_pt) <= 1);
    // YT leg: yield from her day-30 claim (rate 1.01) to the frozen 1.04, in underlying:
    // N * (1/1.01 - 1/1.04), less the 10% fee.
    let yt_gross = 499.75_f64 * (1.0 / 1.01 - 1.0 / 1.04);
    let yt_net = (yt_gross * 0.9 * 1e7) as i128;
    assert!(
        abs_diff(alice_out.underlying_from_yt, yt_net) < 30,
        "alice YT leg {} vs ~{yt_net}",
        alice_out.underlying_from_yt
    );

    let bob_pt = s.pm.pt_balance(&bob);
    let bob_out = s.router.redeem_at_maturity(&bob, &pool, &bob_pt, &0);
    assert!(abs_diff(bob_out.underlying_from_pt, bob_pt * SCALE / final_rate) <= 1);

    let carol_yt = s.pm.yt_balance(&carol);
    let carol_out = s.router.redeem_at_maturity(&carol, &pool, &0, &carol_yt);
    // Carol held YT from day 0: (1.04-1.00)/1.04 of her notional, less the 10% YT fee.
    let carol_expected = (carol_yt as f64 * (0.04 / 1.04) * 0.9) as i128;
    assert!(
        abs_diff(carol_out.underlying_from_yt, carol_expected) < 200,
        "carol {} vs {carol_expected}",
        carol_out.underlying_from_yt
    );

    // LP leaves after maturity, then redeems the PT leg at par-in-value.
    let lp_all = s.pool.lp_balance(&lp);
    s.router.remove_liquidity(&lp, &pool, &lp_all, &0, &0, &FAR);
    // (the LP also still holds the PT from its day-90 partial exit)
    let lp_pt = s.pm.pt_balance(&lp);
    let lp_out = s
        .router
        .redeem_at_maturity(&lp, &pool, &lp_pt, &s.pm.yt_balance(&lp));
    assert!(lp_out.underlying_from_pt > 0);
    assert!(lp_out.underlying_from_yt > 0, "the seeding LP kept its YT");

    // --- fees: both contracts accrued, split 20/80, and are claimable -----------------------
    let (pm_p, pm_c) = s.pm.accrued_fees();
    let (pool_p, pool_c) = s.pool.accrued_fees();
    assert!(pm_p > 0 && pm_c > pm_p * 3);
    assert!(pool_p > 0 && pool_c > pool_p * 3);
    assert_eq!(s.pm.claim_protocol_fees(), pm_p);
    assert_eq!(s.pm.claim_creator_fees(), pm_c);
    assert_eq!(s.pool.claim_protocol_fees(), pool_p);
    assert_eq!(s.pool.claim_creator_fees(), pool_c);
    assert_eq!(s.sy.balance_of(&s.treasury), pm_p + pool_p);
    assert_eq!(s.sy.balance_of(&s.admin), pm_c + pool_c);

    // --- solvency: after every holder has redeemed, PM's SY custody holds nothing but rounding
    // dust and unclaimed pool leftovers -- it is never short, and never sitting on real value. --
    // The only PT still outstanding is the pool's permanently locked MINIMUM_LIQUIDITY share.
    let (locked_pt, _) = s.pool.reserves();
    assert_eq!(s.pm.total_pt(), locked_pt);
    assert!(
        locked_pt < 5_000,
        "locked PT should be dust, was {locked_pt}"
    );
    assert_eq!(s.pm.total_yt(), 0);
    let dust = s.sy.balance_of(&s.pm.address);
    assert!(
        (0..10_000).contains(&dust),
        "PM should hold only dust after full redemption, held {dust}"
    );
}

#[test]
fn allowance_transfer_and_mid_life_claim_then_redeem() {
    let maturity = T0 + 1_000;
    let s = deploy_stack(maturity);
    let alice = s.new_user();
    let bob = s.new_user();

    let shares = deposit_sy(&s, &alice, 100 * SCALE);
    let result = s.pm.mint(&alice, &shares);
    assert_eq!(result.pt_minted, 100 * SCALE);
    assert_eq!(s.pm.total_pt(), 100 * SCALE);
    assert_eq!(s.pm.maturity(), maturity);
    assert!(!s.pm.is_mature());
    assert_eq!(s.pm.underlying_address(), s.underlying);

    // Alice approves Bob to move 30 PT; Bob pulls it.
    let expiration = s.env.ledger().sequence() + 1_000;
    s.pt.approve(&alice, &bob, &(30 * SCALE), &expiration);
    s.pt.transfer_from(&bob, &alice, &bob, &(30 * SCALE));
    assert_eq!(s.pt.balance(&alice), 70 * SCALE);
    assert_eq!(s.pt.balance(&bob), 30 * SCALE);
    assert_eq!(s.pt.allowance(&alice, &bob), 0);

    // Clean 2x rate; Alice claims everything she is owed; the rate never moves again.
    s.advance(T0 + 10, SCALE * 2);
    let claimed = s.pm.claim_yield(&alice);
    assert!(claimed > 0);
    assert_eq!(underlying_balance(&s, &alice), claimed);
    assert_eq!(s.pm.yt_balance(&alice), 100 * SCALE);

    s.advance(maturity + 1, SCALE * 2);
    let alice_redeem = s.pm.redeem(&alice, &(70 * SCALE), &(100 * SCALE));
    assert!(alice_redeem.underlying_from_pt > 0);
    assert_eq!(
        alice_redeem.underlying_from_yt, 0,
        "everything was claimed before maturity"
    );
    let bob_redeem = s.pm.redeem(&bob, &(30 * SCALE), &0);
    assert!(bob_redeem.underlying_from_pt > 0);
    assert_eq!(s.pm.total_pt(), 0);
    assert_eq!(s.pm.total_yt(), 0);
}

#[test]
fn multi_user_late_mint_does_not_dilute_early_holder_yield() {
    let s = deploy_stack(LONG);
    let early = s.new_user();
    let late = s.new_user();

    let early_shares = deposit_sy(&s, &early, 10 * SCALE);
    s.pm.mint(&early, &early_shares);
    s.advance(T0 + 5, SCALE * 11 / 10);
    let late_shares = deposit_sy(&s, &late, 10 * SCALE);
    s.pm.mint(&late, &late_shares);

    assert_eq!(s.pm.claim_yield(&late), 0);
    assert!(s.pm.claim_yield(&early) > 0);
}

// ---------------------------------------------------------------- settle_all

#[test]
fn settle_all_requires_maturity_and_a_fresh_oracle() {
    let maturity = T0 + 500;
    let s = deploy_stack(maturity);
    assert_eq!(
        s.pm.try_settle_all().err(),
        err_code(PmError::NotMature as u32)
    );
    // Mature but the oracle has not been refreshed within the staleness window.
    s.env
        .ledger()
        .with_mut(|li| li.timestamp = maturity + 10_000);
    assert_eq!(
        s.pm.try_settle_all().err(),
        err_code(PmError::OracleStale as u32)
    );
    // Refresh and settle.
    s.oracle
        .set_reference_value(&s.admin, &SCALE, &(maturity + 10_000));
    assert_eq!(s.pm.settle_all(), SCALE);
}

#[test]
fn yt_stops_accruing_at_maturity_and_pt_uses_the_frozen_rate() {
    let maturity = T0 + 1_000;
    let s = deploy_stack(maturity);
    let holder = s.new_user();
    let shares = deposit_sy(&s, &holder, 100 * SCALE);
    s.pm.mint(&holder, &shares);

    s.advance(maturity, SCALE * 12 / 10);
    s.pm.settle_all();
    assert!(s.yt.is_frozen());
    let index_at_settle = s.yt.accrued_yield_index();

    // A big post-maturity oracle jump changes nothing: the YT index and PT rate stay frozen.
    s.advance(maturity + 100, SCALE * 2);
    s.yt.update_yield_index();
    assert_eq!(s.yt.accrued_yield_index(), index_at_settle);

    let out = s.pm.redeem(&holder, &(100 * SCALE), &(100 * SCALE));
    // PT: 100 / 1.2 ; YT: 100 * (0.2/1.2).
    assert!(abs_diff(out.underlying_from_pt, 100 * SCALE * 10 / 12) <= 1);
    assert!(abs_diff(out.underlying_from_yt, 100 * SCALE * 2 / 12) <= 1);
    // Together they account for the full 100 underlying deposited (no post-maturity leakage).
    assert!(abs_diff(out.underlying_from_pt + out.underlying_from_yt, 100 * SCALE) <= 2);
}

#[test]
fn redeem_settles_implicitly_and_a_stale_oracle_after_settlement_does_not_block_holders() {
    let maturity = T0 + 1_000;
    let s = deploy_stack(maturity);
    let a = s.new_user();
    let b = s.new_user();
    let sa = deposit_sy(&s, &a, 10 * SCALE);
    let sb = deposit_sy(&s, &b, 10 * SCALE);
    s.pm.mint(&a, &sa);
    s.pm.mint(&b, &sb);

    s.advance(maturity, SCALE * 11 / 10);
    // No settle_all: the first redeem settles.
    s.pm.redeem(&a, &(10 * SCALE), &0);
    assert_eq!(s.pm.settled_rate(), Some(SCALE * 11 / 10));
    // The oracle then goes stale for days; already-settled holders can still redeem and claim.
    s.env
        .ledger()
        .with_mut(|li| li.timestamp = maturity + 30 * DAY);
    s.pm.redeem(&b, &(10 * SCALE), &(10 * SCALE));
}

// --------------------------------------------------------------- recombine

#[test]
fn recombine_returns_sy_at_the_current_rate_and_keeps_accrued_yield_claimable() {
    let s = deploy_stack(LONG);
    let user = s.new_user();
    let shares = deposit_sy(&s, &user, 100 * SCALE);
    s.pm.mint(&user, &shares);

    s.advance(T0 + 10, SCALE * 125 / 100);
    let back = s.pm.recombine(&user, &(100 * SCALE));
    // 100 notional at 1.25 = 80 SY; the other 20 SY back the yield the YT accrued.
    assert_eq!(back, 80 * SCALE);
    assert_eq!(s.sy.balance_of(&user), 80 * SCALE);
    assert_eq!(s.pm.pt_balance(&user), 0);
    assert_eq!(s.pm.yt_balance(&user), 0);
    // The accrued yield survives the burn and is paid on the next claim: 100 * 0.25/1.25 = 20.
    assert_eq!(s.pm.claim_yield(&user), 20 * SCALE);
    // Nothing is left over in custody.
    assert_eq!(s.sy.balance_of(&s.pm.address), 0);
}

#[test]
fn recombine_validates_amounts_maturity_and_balances() {
    let maturity = T0 + 1_000;
    let s = deploy_stack(maturity);
    let user = s.new_user();
    let shares = deposit_sy(&s, &user, 10 * SCALE);
    s.pm.mint(&user, &shares);
    assert_eq!(
        s.pm.try_recombine(&user, &0).err(),
        err_code(PmError::ZeroAmount as u32)
    );
    // More than held.
    assert!(s.pm.try_recombine(&user, &(11 * SCALE)).is_err());
    // After maturity recombination is closed (redeem instead).
    s.advance(maturity, SCALE);
    assert_eq!(
        s.pm.try_recombine(&user, &(SCALE)).err(),
        err_code(PmError::AlreadyMature as u32)
    );
}

// ---------------------------------------------------------------------- fees

#[test]
fn tokenization_fee_is_withheld_accrued_and_claimable_by_treasury_and_creator() {
    let s = deploy(Config::with_example_fees(LONG));
    let user = s.new_user();
    let shares = deposit_sy(&s, &user, 1_000 * SCALE);
    let r = s.pm.mint(&user, &shares);
    // 5 bps of 1000 SY = 0.5 SY.
    assert_eq!(r.fee_shares, 5_000_000);
    assert_eq!(r.pt_minted, 1_000 * SCALE - 5_000_000);
    assert_eq!(s.pm.accrued_fees(), (1_000_000, 4_000_000)); // 20% / 80%
                                                             // The withheld shares stay in PM's custody.
    assert_eq!(s.sy.balance_of(&s.pm.address), shares);

    assert_eq!(s.pm.claim_protocol_fees(), 1_000_000);
    assert_eq!(s.pm.claim_creator_fees(), 4_000_000);
    assert_eq!(s.sy.balance_of(&s.treasury), 1_000_000);
    assert_eq!(s.sy.balance_of(&s.admin), 4_000_000);
    assert_eq!(s.pm.accrued_fees(), (0, 0));
    assert_eq!(
        s.pm.try_claim_creator_fees().err(),
        err_code(PmError::NothingToClaim as u32)
    );
}

#[test]
fn issuer_can_retune_fees_and_principal_can_retune_its_share() {
    let s = deploy(Config::with_example_fees(LONG));
    // Issuer (market creator) sets the tokenization fee to 0 => next mint is fee-free.
    s.config.set_fees(&s.admin, &0, &1_000, &10);
    let user = s.new_user();
    let shares = deposit_sy(&s, &user, 100 * SCALE);
    assert_eq!(s.pm.mint(&user, &shares).fee_shares, 0);

    // Principal moves its share to 50%: the next fee splits evenly.
    s.config.set_fees(&s.admin, &100, &1_000, &10); // 1% tokenization fee
    s.config.set_protocol_share(&s.protocol_admin, &5_000);
    let shares2 = deposit_sy(&s, &user, 100 * SCALE);
    let r = s.pm.mint(&user, &shares2);
    assert_eq!(r.fee_shares, SCALE); // 1% of 100 SY
    assert_eq!(s.pm.accrued_fees(), (SCALE / 2, SCALE / 2));
}

#[test]
fn zero_fee_market_takes_nothing() {
    let s = deploy_stack(T0 + 1_000);
    let user = s.new_user();
    let shares = deposit_sy(&s, &user, 100 * SCALE);
    let r = s.pm.mint(&user, &shares);
    assert_eq!(r.fee_shares, 0);
    assert_eq!(s.pm.accrued_fees(), (0, 0));
    s.advance(T0 + 500, SCALE * 2);
    // No YT fee either: claim pays the full 50 (100 * (2-1)/2).
    assert_eq!(s.pm.claim_yield(&user), 50 * SCALE);
}

// -------------------------------------------------------------- pause switch

#[test]
fn paused_market_blocks_mint_and_unpausing_restores_it() {
    let s = deploy_stack(LONG);
    let user = s.new_user();
    let shares = deposit_sy(&s, &user, 10 * SCALE);

    s.pm.set_paused(&s.admin, &true);
    assert_eq!(
        s.pm.try_mint(&user, &shares).err(),
        err_code(PmError::Paused as u32)
    );
    s.pm.set_paused(&s.admin, &false);
    assert_eq!(s.pm.mint(&user, &shares).pt_minted, 10 * SCALE);
}

#[test]
fn shares_at_helper_is_consistent() {
    assert_eq!(shares_at(SCALE, 5), 5);
}
