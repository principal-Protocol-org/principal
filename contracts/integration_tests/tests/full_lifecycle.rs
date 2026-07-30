//! End-to-end lifecycle: deposit -> mint -> allowance-based transfer -> mid-life yield claim ->
//! maturity redemption, exercised against the real eight-contract stack (not per-contract mocks).

mod common;

use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    token, Address,
};

use common::{deploy_stack, deposit_sy, grant_user, T0};
use principal_manager::SCALE;

#[test]
fn full_split_transfer_claim_and_redeem() {
    let maturity = T0 + 1_000;
    let s = deploy_stack(maturity);

    let alice = Address::generate(&s.env);
    let bob = Address::generate(&s.env);
    grant_user(&s, &alice);
    grant_user(&s, &bob);

    // Alice deposits 100 USDY and splits it into PT + YT at genesis rate (1.0).
    let shares = deposit_sy(&s, &alice, 100 * SCALE);
    let result = s.pm.mint(&alice, &shares);
    assert_eq!(result.pt_minted, 100 * SCALE);
    assert_eq!(result.yt_minted, 100 * SCALE);

    // PrincipalManager's own views agree with the underlying token balances.
    assert_eq!(s.pm.pt_balance(&alice), 100 * SCALE);
    assert_eq!(s.pm.yt_balance(&alice), 100 * SCALE);
    assert_eq!(s.pm.total_pt(), 100 * SCALE);
    assert_eq!(s.pm.total_yt(), 100 * SCALE);
    assert_eq!(s.pm.maturity(), maturity);
    assert!(!s.pm.is_mature());
    assert_eq!(s.pm.underlying_address(), s.underlying);

    // Alice approves Bob to move 30 PT on her behalf; Bob pulls it via transfer_from.
    let expiration = s.env.ledger().sequence() + 1_000;
    s.pt.approve(&alice, &bob, &(30 * SCALE), &expiration);
    assert_eq!(s.pt.allowance(&alice, &bob), 30 * SCALE);
    s.pt.transfer_from(&bob, &alice, &bob, &(30 * SCALE));
    assert_eq!(s.pt.balance(&alice), 70 * SCALE);
    assert_eq!(s.pt.balance(&bob), 30 * SCALE);
    assert_eq!(s.pt.allowance(&alice, &bob), 0);

    // Oracle rate rises before maturity -- a clean 2x so the claim below matches the
    // single-jump formula exactly, with no floor-rounding residue to reconcile against PT's
    // independently-computed share of the same underlying pool. Alice claims accrued YT yield
    // without redeeming; the rate does not move again before maturity, so this is her only
    // claim and it captures everything she is ever owed.
    s.env.ledger().with_mut(|li| li.timestamp = T0 + 10);
    s.oracle
        .set_reference_value(&s.admin, &(SCALE * 2), &(T0 + 10));
    let claimed = s.pm.claim_yield(&alice);
    assert!(claimed > 0);
    assert_eq!(
        token::Client::new(&s.env, &s.underlying).balance(&alice),
        claimed
    );
    // Alice's YT balance is untouched by claiming -- only redemption burns it.
    assert_eq!(s.pm.yt_balance(&alice), 100 * SCALE);

    // Advance to and past maturity; oracle unchanged since the claim above.
    s.env.ledger().with_mut(|li| li.timestamp = maturity + 1);
    assert!(s.pm.is_mature());

    // Alice redeems her remaining PT and YT; Bob (who only ever held PT) redeems his share too.
    let alice_redeem = s.pm.redeem(&alice, &(70 * SCALE), &(100 * SCALE));
    assert!(alice_redeem.underlying_from_pt > 0);
    // Already claimed everything owed before maturity (rate never moved again), so nothing
    // further should be paid here.
    assert_eq!(alice_redeem.underlying_from_yt, 0);

    let bob_redeem = s.pm.redeem(&bob, &(30 * SCALE), &0);
    assert!(bob_redeem.underlying_from_pt > 0);

    assert_eq!(s.pm.total_pt(), 0);
    assert_eq!(s.pm.total_yt(), 0);
    assert_eq!(s.pm.pt_balance(&alice), 0);
    assert_eq!(s.pm.yt_balance(&alice), 0);
}

#[test]
fn multi_user_late_mint_does_not_dilute_early_holder_yield() {
    // Cross-contract version of YTToken's own late-buyer regression: a second user minting
    // through PrincipalManager after a rate move must not receive credit for it, and the first
    // user's claim must be unaffected by the second mint.
    let s = deploy_stack(u64::MAX);

    let early = Address::generate(&s.env);
    let late = Address::generate(&s.env);
    grant_user(&s, &early);
    grant_user(&s, &late);

    let early_shares = deposit_sy(&s, &early, 10 * SCALE);
    s.pm.mint(&early, &early_shares);

    s.env.ledger().with_mut(|li| li.timestamp = T0 + 5);
    s.oracle
        .set_reference_value(&s.admin, &(SCALE * 11 / 10), &(T0 + 5));

    let late_shares = deposit_sy(&s, &late, 10 * SCALE);
    s.pm.mint(&late, &late_shares);

    assert_eq!(s.pm.claim_yield(&late), 0);
    assert!(s.pm.claim_yield(&early) > 0);
}

#[test]
#[should_panic]
fn paused_market_blocks_mint() {
    let s = deploy_stack(u64::MAX);
    let user = Address::generate(&s.env);
    grant_user(&s, &user);
    let shares = deposit_sy(&s, &user, 10 * SCALE);

    s.pm.set_paused(&s.admin, &true);
    s.pm.mint(&user, &shares);
}

#[test]
fn unpausing_restores_mint() {
    let s = deploy_stack(u64::MAX);
    let user = Address::generate(&s.env);
    grant_user(&s, &user);
    let shares = deposit_sy(&s, &user, 10 * SCALE);

    s.pm.set_paused(&s.admin, &true);
    s.pm.set_paused(&s.admin, &false);
    let result = s.pm.mint(&user, &shares);
    assert_eq!(result.pt_minted, 10 * SCALE);
}
