//! Liquidity provision (proportional and single-sided), LP token compliance, pool guards, and every
//! Router flow -- registry, deadlines, min-out guards, wrap/unwrap, recombine, redemption.

use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    Address,
};

use principal_integration_tests::stack::*;
use principal_market_pool::{Error as PoolError, MarketPoolContract, MarketPoolContractClient};
use principal_router::Error as RouterError;
use principal_sy_wrapper::Error as SyError;

fn seeded(kind: UnderlyingKind) -> (Stack<'static>, Address, i128) {
    let s = deploy_kind(Config::free(T0 + 180 * DAY), kind);
    let (lp_minted, lp) = seed_pool(&s, 1_000 * SCALE, 950 * SCALE);
    (s, lp, lp_minted)
}

// ------------------------------------------------------------ add / remove

#[test]
fn proportional_add_takes_the_current_ratio_and_leaves_the_surplus_with_the_caller() {
    let (s, _, _) = seeded(UnderlyingKind::Sac);
    let provider = s.new_user();
    let shares = deposit_sy(&s, &provider, 1_000 * SCALE);
    s.pm.mint(&provider, &(500 * SCALE));
    let sy_before = s.sy.balance_of(&provider);
    let pt_before = s.pt.balance(&provider);
    let supply_before = s.pool.lp_total_supply();
    let (pt_res, sy_res) = s.pool.reserves();
    let _ = shares;

    // Offer far more SY than the ratio needs: the PT side binds.
    let (pt_used, sy_used, lp) =
        s.pool
            .add_liquidity(&provider, &(100 * SCALE), &(400 * SCALE), &0);
    assert_eq!(pt_used, 100 * SCALE);
    // sy_used = ceil(lp * sy_res / total_lp): pool-favoring rounding, proportional to the pool.
    let expected_sy = 100 * SCALE * sy_res / pt_res;
    assert!(
        abs_diff(sy_used, expected_sy) < 5,
        "{sy_used} vs {expected_sy}"
    );
    assert_eq!(s.pool.lp_balance(&provider), lp);
    assert_eq!(s.pool.lp_total_supply(), supply_before + lp);
    // Only what was used left the caller's wallet.
    assert_eq!(s.pt.balance(&provider), pt_before - pt_used);
    assert_eq!(s.sy.balance_of(&provider), sy_before - sy_used);
    // The opening price is unchanged by a proportional deposit.
    let p = s.pool.pt_price();
    assert!((p as f64 / SCALE as f64 - 0.9937).abs() < 1e-3);
}

#[test]
fn remove_liquidity_returns_the_pro_rata_share_and_respects_min_out() {
    let (s, lp, lp_minted) = seeded(UnderlyingKind::Sac);
    let (pt_res, sy_res) = s.pool.reserves();
    let total = s.pool.lp_total_supply();
    let half = lp_minted / 2;
    let exp_pt = half * pt_res / total;
    let exp_sy = half * sy_res / total;

    assert_eq!(
        s.pool
            .try_remove_liquidity(&lp, &lp, &half, &(exp_pt + 1), &0)
            .err(),
        err_code(PoolError::SlippageExceeded as u32)
    );
    assert_eq!(
        s.pool
            .try_remove_liquidity(&lp, &lp, &half, &0, &(exp_sy + 1))
            .err(),
        err_code(PoolError::SlippageExceeded as u32)
    );
    let (pt_out, sy_out) = s.pool.remove_liquidity(&lp, &lp, &half, &exp_pt, &exp_sy);
    assert_eq!((pt_out, sy_out), (exp_pt, exp_sy));
    assert_eq!(s.pool.lp_balance(&lp), lp_minted - half);
    // Cannot remove more than held, or zero.
    assert_eq!(
        s.pool
            .try_remove_liquidity(&lp, &lp, &(lp_minted), &0, &0)
            .err(),
        err_code(PoolError::InsufficientLpBalance as u32)
    );
    assert_eq!(
        s.pool.try_remove_liquidity(&lp, &lp, &0, &0, &0).err(),
        err_code(PoolError::ZeroAmount as u32)
    );
}

#[test]
fn add_liquidity_input_validation_and_min_lp_out() {
    let (s, _, _) = seeded(UnderlyingKind::Sac);
    let p = s.new_user();
    deposit_sy(&s, &p, 500 * SCALE);
    s.pm.mint(&p, &(200 * SCALE));
    assert_eq!(
        s.pool.try_add_liquidity(&p, &0, &SCALE, &0).err(),
        err_code(PoolError::ZeroAmount as u32)
    );
    assert_eq!(
        s.pool.try_add_liquidity(&p, &SCALE, &0, &0).err(),
        err_code(PoolError::ZeroAmount as u32)
    );
    // Dust that mints no LP.
    assert_eq!(
        s.pool.try_add_liquidity(&p, &1, &1, &0).err(),
        err_code(PoolError::ZeroAmount as u32)
    );
    // min_lp_out is a hard floor.
    assert_eq!(
        s.pool
            .try_add_liquidity(&p, &(10 * SCALE), &(10 * SCALE), &(1_000 * SCALE))
            .err(),
        err_code(PoolError::SlippageExceeded as u32)
    );
}

#[test]
fn the_first_deposit_must_exceed_the_locked_minimum_liquidity() {
    let s = deploy(Config::free(T0 + 180 * DAY));
    let p = s.new_user();
    deposit_sy(&s, &p, 100 * SCALE);
    s.pm.mint(&p, &(50 * SCALE));
    // sqrt(1000 * 1000) = 1000 <= MINIMUM_LIQUIDITY: rejected.
    assert_eq!(
        s.pool.try_add_liquidity(&p, &1_000, &1_000, &0).err(),
        err_code(PoolError::MinimumLiquidity as u32)
    );
}

#[test]
fn a_donation_to_the_pool_cannot_skew_lp_pricing() {
    // LP is priced from the pool's internal reserves, never from live token balances, so tokens
    // sent straight to the pool address change nothing for the next depositor.
    let (s, lp, _) = seeded(UnderlyingKind::Sac);
    let (pt0, sy0) = s.pool.reserves();
    let supply0 = s.pool.lp_total_supply();
    let donor = s.new_user();
    deposit_sy(&s, &donor, 300 * SCALE);
    s.sy.transfer(&donor, &s.pool.address, &(300 * SCALE));
    assert_eq!(s.pool.reserves(), (pt0, sy0));
    assert_eq!(s.pool.lp_total_supply(), supply0);

    let p = s.new_user();
    deposit_sy(&s, &p, 300 * SCALE);
    s.pm.mint(&p, &(100 * SCALE));
    let (_, _, lp_out) = s.pool.add_liquidity(&p, &(100 * SCALE), &(300 * SCALE), &0);
    assert_eq!(lp_out, 100 * SCALE * supply0 / pt0);
    let _ = lp;
}

// ------------------------------------------------------------ single-sided

#[test]
fn single_sided_deposit_swaps_just_enough_and_mints_lp_for_nearly_all_of_the_sy() {
    let (s, _, _) = seeded(UnderlyingKind::Sac);
    let user = s.new_user();
    deposit_sy(&s, &user, 100 * SCALE);
    let split = s.pool.zap_swap_amount(&(100 * SCALE));
    // PT is at a small discount, so a bit under half the SY converts to PT.
    assert!(split > 40 * SCALE && split < 55 * SCALE, "split {split}");

    let (pt_res0, sy_res0) = s.pool.reserves();
    let (_pt_used, sy_used, lp) = s.pool.add_liquidity_single_sy(&user, &(100 * SCALE), &0);
    assert!(lp > 0);
    // Nearly all the SY was used; whatever is left is rounding-sized and was returned.
    let left = 100 * SCALE - sy_used;
    assert_eq!(s.sy.balance_of(&user), left);
    assert!(left < SCALE / 100, "leftover SY {left}");
    // Unlike the SY side, any PT the zap "bought" beyond what `lp` needed is never transferred to
    // the user at all -- see add_liquidity_single_sy's own doc comment: a dApp-integration review
    // found the transfer reverted PermissionDenied for any caller lacking PTToken's per-asset
    // grant, since a caller using this "LP only" zap has no reason to have requested one. It stays
    // in the pool's own reserve instead, so the user's PT balance is exactly 0, not just small.
    assert_eq!(s.pt.balance(&user), 0);
    // The pool's PT reserve is unchanged -- the "bought" PT beyond `pt_used` never left the
    // reserve ledger at all -- while the SY reserve grew by the deposit. The ratio moves by
    // exactly the same amount either way: whether the leftover PT is given to the user or kept in
    // the pool, it was never credited against this `lp` mint, so the ratio math is identical.
    let (pt_res1, sy_res1) = s.pool.reserves();
    assert_eq!(pt_res1, pt_res0);
    assert!(sy_res1 > sy_res0);
    let r0 = sy_res0 as f64 / pt_res0 as f64;
    let r1 = sy_res1 as f64 / pt_res1 as f64;
    // Net effect of a single-sided deposit: the pool gained ~100 SY and the PT it "bought" went
    // straight back in, so the SY/PT ratio rises by about 100/1000 -- and no more (fee-free).
    assert!(r1 > r0);
    let expected_r1 = (950.0 + 100.0) / 1000.0;
    assert!(
        (r1 - expected_r1).abs() < 0.01,
        "ratio {r1} vs ~{expected_r1}"
    );
    // Slippage guard and validation.
    assert_eq!(
        s.pool.try_add_liquidity_single_sy(&user, &0, &0).err(),
        err_code(PoolError::ZeroAmount as u32)
    );
}

/// Regression for the real bug a dApp-integration review found underneath the "1 in 12" failure
/// report: `add_liquidity_single_sy` used to transfer the zap's leftover PT dust to the caller,
/// which reverts `PermissionDenied` for anyone lacking PTToken's per-asset grant -- exactly a
/// caller using this "LP only" zap specifically to avoid ever holding PT directly. This user is
/// deliberately granted account-level standing (needed to touch SY/LP at all) but **not**
/// PTToken's per-asset grant, reproducing that exact gap. Before the fix this reverted; now it
/// must succeed, with the dust absorbed into the pool's reserve rather than transferred.
#[test]
fn single_sided_deposit_succeeds_for_a_caller_with_no_pt_asset_grant() {
    let (s, _, _) = seeded(UnderlyingKind::Sac);
    let user = Address::generate(&s.env);
    s.perm.grant_account(&s.admin, &user);
    s.authorize(&user);
    // Deliberately no `s.perm.grant_asset(&s.admin, &user, &s.pt.address)`.
    s.mint_underlying(&user, 100 * SCALE);
    let shares = s.sy.deposit(&user, &(100 * SCALE), &0);

    let (_, _, lp) = s.pool.add_liquidity_single_sy(&user, &shares, &0);
    assert!(lp > 0);
    assert_eq!(s.pt.balance(&user), 0);
}

/// Regression for the fix that takes `sy_in` from the caller up front (a plain, time-independent
/// argument) instead of transferring a curve-derived `split + sy_used` amount read from
/// `state_fresh()` -- the live version reverted `Error(Auth, InvalidAction)` because the amount
/// Soroban authorized at simulation time could differ, by a few raw units, from the amount
/// computed moments later at execution, once the ledger's timestamp had ticked forward. This test
/// cannot reproduce that specific authorization mismatch (`mock_all_auths()` bypasses per-argument
/// auth matching entirely), but it does pin down the refund invariant the rewrite must still
/// satisfy: the pool's own SY balance grows by exactly `sy_total_used`, never by the full `sy_in`,
/// so no part of the up-front transfer is left stranded in the pool once the refund runs.
#[test]
fn single_sided_deposit_leaves_no_residual_sy_stranded_in_the_pool() {
    let (s, _, _) = seeded(UnderlyingKind::Sac);
    let user = s.new_user();
    deposit_sy(&s, &user, 100 * SCALE);

    let pool_sy_before = s.sy.balance_of(&s.pool.address);
    let (_, sy_total_used, lp) = s.pool.add_liquidity_single_sy(&user, &(100 * SCALE), &0);
    assert!(lp > 0);
    let pool_sy_after = s.sy.balance_of(&s.pool.address);

    assert_eq!(pool_sy_after - pool_sy_before, sy_total_used);
    // Whatever the pool didn't keep came back to the user, not a third party.
    assert_eq!(s.sy.balance_of(&user), 100 * SCALE - sy_total_used);
}

#[test]
fn single_sided_deposit_on_an_empty_pool_is_refused() {
    let s = deploy(Config::free(T0 + 180 * DAY));
    let user = s.new_user();
    deposit_sy(&s, &user, 10 * SCALE);
    assert_eq!(
        s.pool
            .try_add_liquidity_single_sy(&user, &(10 * SCALE), &0)
            .err(),
        err_code(PoolError::InsufficientLiquidity as u32)
    );
}

// ------------------------------------------------------------ LP token

#[test]
fn lp_transfers_check_both_sides_and_move_the_position() {
    for kind in [UnderlyingKind::Sac, UnderlyingKind::Rwa] {
        let (s, lp, lp_minted) = seeded(kind);
        let friend = s.new_user();
        s.pool.transfer_lp(&lp, &friend, &(lp_minted / 4));
        assert_eq!(s.pool.lp_balance(&friend), lp_minted / 4);
        assert_eq!(s.pool.lp_balance(&lp), lp_minted - lp_minted / 4);

        assert_eq!(
            s.pool.try_transfer_lp(&lp, &friend, &0).err(),
            err_code(PoolError::ZeroAmount as u32)
        );
        assert_eq!(
            s.pool.try_transfer_lp(&friend, &lp, &lp_minted).err(),
            err_code(PoolError::InsufficientLpBalance as u32)
        );
        // Sender flagged by the issuer: frozen. Recipient flagged: cannot receive.
        let flagged = s.new_user();
        s.deauthorize(&flagged);
        assert_eq!(
            s.pool.try_transfer_lp(&lp, &flagged, &1).err(),
            err_code(PoolError::NotAuthorizedOnSac as u32)
        );
        s.deauthorize(&friend);
        assert_eq!(
            s.pool.try_transfer_lp(&friend, &lp, &1).err(),
            err_code(PoolError::NotAuthorizedOnSac as u32)
        );
    }
}

#[test]
fn permissioning_narrows_who_may_trade_and_provide_liquidity() {
    let (s, _, _) = seeded(UnderlyingKind::Sac);
    let user = s.new_user();
    deposit_sy(&s, &user, 50 * SCALE);
    s.perm.revoke_account(&s.admin, &user);
    assert_eq!(
        s.pool.try_swap_sy_for_pt(&user, &user, &SCALE, &0).err(),
        err_code(PoolError::PermissionDenied as u32)
    );
    assert_eq!(
        s.pool.try_add_liquidity(&user, &SCALE, &SCALE, &0).err(),
        err_code(PoolError::PermissionDenied as u32)
    );
}

#[test]
fn deauthorized_holders_cannot_trade_or_provide_liquidity() {
    for kind in [UnderlyingKind::Sac, UnderlyingKind::Rwa] {
        let (s, _, _) = seeded(kind);
        let user = s.new_user();
        deposit_sy(&s, &user, 50 * SCALE);
        s.deauthorize(&user);
        assert_eq!(
            s.pool.try_swap_sy_for_pt(&user, &user, &SCALE, &0).err(),
            err_code(PoolError::NotAuthorizedOnSac as u32)
        );
        assert_eq!(
            s.pool.try_swap_pt_for_sy(&user, &user, &SCALE, &0).err(),
            err_code(PoolError::NotAuthorizedOnSac as u32)
        );
        assert_eq!(
            s.pool.try_swap_yt_for_sy(&user, &user, &SCALE, &0).err(),
            err_code(PoolError::NotAuthorizedOnSac as u32)
        );
        assert_eq!(
            s.pool.try_add_liquidity_single_sy(&user, &SCALE, &0).err(),
            err_code(PoolError::NotAuthorizedOnSac as u32)
        );
    }
}

// ------------------------------------------------------------ pool guards

#[test]
fn paused_pool_blocks_trading_and_liquidity_but_not_the_escrow() {
    let (s, lp, lp_minted) = seeded(UnderlyingKind::Sac);
    let user = s.new_user();
    deposit_sy(&s, &user, 10 * SCALE);
    s.pool.set_paused(&s.admin, &true);
    assert!(s.pool.is_paused());
    for r in [
        s.pool.try_swap_sy_for_pt(&user, &user, &SCALE, &0).err(),
        s.pool
            .try_remove_liquidity(&lp, &lp, &(lp_minted / 2), &0, &0)
            .err(),
        s.pool.try_transfer_lp(&lp, &user, &1).err(),
    ] {
        assert_eq!(r, err_code(PoolError::Paused as u32));
    }
    s.pool.set_paused(&s.admin, &false);
    s.pool.swap_sy_for_pt(&user, &user, &SCALE, &0);
}

#[test]
fn a_stale_oracle_blocks_swaps_but_never_blocks_an_lp_exit() {
    let (s, lp, lp_minted) = seeded(UnderlyingKind::Sac);
    let user = s.new_user();
    deposit_sy(&s, &user, 10 * SCALE);
    // Narrow the oracle's own window to the production default (3_600 s) for this test -- the
    // shared harness otherwise configures a 365-day window so no other test's oracle moves trip
    // the guard by accident.
    s.oracle
        .set_safety_params(&s.admin, &3_600_u64, &ORACLE_MAX_DEVIATION_BPS);
    // Two hours later the oracle has not been refreshed.
    s.env.ledger().with_mut(|li| li.timestamp += 7_200);
    assert_eq!(
        s.pool.try_swap_sy_for_pt(&user, &user, &SCALE, &0).err(),
        err_code(PoolError::OracleStale as u32)
    );
    // Withdrawing liquidity needs no oracle.
    s.pool.remove_liquidity(&lp, &lp, &(lp_minted / 2), &0, &0);
}

#[test]
fn pool_initialization_is_gated_validated_and_one_time() {
    let s = deploy(Config::free(T0 + 180 * DAY));
    let fresh =
        |s: &Stack| MarketPoolContractClient::new(&s.env, &s.env.register(MarketPoolContract, ()));
    // Only the issuer authority can create the pool.
    let stranger = Address::generate(&s.env);
    assert_eq!(
        fresh(&s).try_initialize(&stranger, &s.pm.address, &4).err(),
        err_code(PoolError::IssuerMismatch as u32)
    );
    // Stretch must be 1..=20 years.
    assert_eq!(
        fresh(&s).try_initialize(&s.admin, &s.pm.address, &0).err(),
        err_code(PoolError::InvalidStretch as u32)
    );
    assert_eq!(
        fresh(&s).try_initialize(&s.admin, &s.pm.address, &21).err(),
        err_code(PoolError::InvalidStretch as u32)
    );
    // A 180-day market cannot sit on a 1-year stretch? It can (0.49 years < 75%): accepted.
    fresh(&s).initialize(&s.admin, &s.pm.address, &1);
    // But a market longer than 75% of the stretch is refused: 365d market on a 1-year stretch.
    let long = deploy(Config::free(T0 + 365 * DAY));
    assert_eq!(
        fresh(&long)
            .try_initialize(&long.admin, &long.pm.address, &1)
            .err(),
        err_code(PoolError::MaturityTooFar as u32)
    );
    // Second initialize on the live pool.
    assert_eq!(
        s.pool.try_initialize(&s.admin, &s.pm.address, &4).err(),
        err_code(PoolError::AlreadyInitialized as u32)
    );
}

#[test]
fn pool_recovery_escrow_is_set_once_and_lp_seizure_is_escrow_only() {
    let (s, lp, lp_minted) = seeded(UnderlyingKind::Sac);
    assert_eq!(s.pool.recovery_escrow(), Some(s.escrow.address.clone()));
    assert_eq!(
        s.pool
            .try_set_recovery_escrow(&s.admin, &s.escrow.address)
            .err(),
        err_code(PoolError::RecoveryEscrowAlreadySet as u32)
    );
    let stranger = Address::generate(&s.env);
    assert_eq!(
        s.pool.try_seize_lp(&stranger, &lp, &lp_minted).err(),
        err_code(PoolError::NotRecoveryEscrow as u32)
    );
    assert_eq!(
        s.pool.try_redeem_seized_lp(&stranger, &lp_minted).err(),
        err_code(PoolError::NotRecoveryEscrow as u32)
    );
    // A pool with no escrow wired refuses seizure outright.
    let bare = MarketPoolContractClient::new(&s.env, &s.env.register(MarketPoolContract, ()));
    bare.initialize(&s.admin, &s.pm.address, &4);
    assert_eq!(
        bare.try_seize_lp(&s.escrow.address, &lp, &1).err(),
        err_code(PoolError::NotRecoveryEscrow as u32)
    );
    assert_eq!(bare.recovery_escrow(), None);
}

#[test]
fn pool_views_expose_the_market_wiring() {
    let (s, _, _) = seeded(UnderlyingKind::Sac);
    assert_eq!(s.pool.manager_address(), s.pm.address);
    assert_eq!(s.pool.sy_address(), s.sy.address);
    assert_eq!(s.pool.pt_address(), s.pt.address);
    assert_eq!(s.pool.yt_address(), s.yt.address);
    assert_eq!(s.pool.underlying_address(), s.underlying);
    assert_eq!(s.pool.config_address(), s.config.address);
    assert_eq!(s.pool.maturity(), s.maturity);
    let st = s.pool.pool_state();
    assert_eq!(st.pt_reserve, 1_000 * SCALE);
    assert_eq!(st.sy_reserve, 950 * SCALE);
    assert_eq!(st.rate, SCALE);
    assert_eq!(st.seconds_to_maturity, 180 * DAY);
    // Implied fixed rate is positive while PT trades at a discount and zero at maturity.
    assert!(s.pool.implied_rate() > 0);
    s.advance_flat(s.maturity);
    assert_eq!(s.pool.implied_rate(), 0);
}

// ------------------------------------------------------------------ router

#[test]
fn router_registry_is_admin_only_validated_and_required_by_every_flow() {
    let s = deploy(Config::free(T0 + 180 * DAY));
    let stranger = Address::generate(&s.env);
    assert!(s.router.is_registered(&s.pool.address));
    let info = s.router.market(&s.pool.address);
    assert_eq!(info.manager, s.pm.address);
    assert_eq!(info.underlying, s.underlying);

    // Duplicate registration.
    assert_eq!(
        s.router
            .try_register_market(&s.admin, &s.pool.address)
            .err(),
        err_code(RouterError::AlreadyRegistered as u32)
    );
    // Only the router admin registers markets.
    let other = MarketPoolContractClient::new(&s.env, &s.env.register(MarketPoolContract, ()));
    other.initialize(&s.admin, &s.pm.address, &4);
    assert_eq!(
        s.router
            .try_register_market(&stranger, &other.address)
            .err(),
        err_code(RouterError::Unauthorized as u32)
    );
    s.router.register_market(&s.admin, &other.address);
    assert!(s.router.is_registered(&other.address));

    // Every flow refuses an unregistered pool.
    let user = s.new_user();
    let unknown = Address::generate(&s.env);
    assert_eq!(
        s.router
            .try_swap_sy_for_pt(&user, &unknown, &1, &0, &u64::MAX)
            .err(),
        err_code(RouterError::MarketNotRegistered as u32)
    );
    assert_eq!(
        s.router.try_market(&unknown).err(),
        err_code(RouterError::MarketNotRegistered as u32)
    );
}

#[test]
fn router_resolves_market_info_directly_from_the_pool_so_it_cannot_disagree_with_it() {
    // `register_market` keeps no independent registry entry that could drift out of sync with
    // the pool: it reads `sy_address`/`pt_address`/`yt_address` straight off the pool being
    // registered, so a registered pool's `MarketInfo` cannot describe different tokens than the
    // pool itself trades. There is no "a pool whose manager disagrees with its own tokens"
    // scenario to construct against this design -- that can't happen by construction. What *can*
    // be checked is the positive-path property this relies on, and that each separately
    // registered pool gets its own independent `MarketInfo` (not a single, pool-address-blind
    // cache), using a second pool registered on this stack's own manager.
    let s = deploy(Config::free(T0 + 180 * DAY));
    let info = s.router.market(&s.pool.address);
    assert_eq!(info.sy, s.pool.sy_address());
    assert_eq!(info.pt, s.pool.pt_address());
    assert_eq!(info.yt, s.pool.yt_address());

    let other = MarketPoolContractClient::new(&s.env, &s.env.register(MarketPoolContract, ()));
    other.initialize(&s.admin, &s.pm.address, &4);
    s.router.register_market(&s.admin, &other.address);
    let other_info = s.router.market(&other.address);
    assert_eq!(other_info.sy, other.sy_address());
    assert_eq!(other_info.pt, other.pt_address());
    assert_eq!(other_info.yt, other.yt_address());
    assert_ne!(info, other_info);
}

#[test]
fn router_deadline_and_min_out_guards_apply_to_every_flow() {
    let (s, _, _) = seeded(UnderlyingKind::Sac);
    let pool = s.pool.address.clone();
    let user = s.new_user();
    s.mint_underlying(&user, 300 * SCALE);
    let past = T0 - 1;
    let dl = RouterError::DeadlineExpired as u32;

    assert_eq!(
        s.router
            .try_wrap_and_mint(&user, &pool, &SCALE, &0, &past)
            .err(),
        err_code(dl)
    );
    assert_eq!(
        s.router.try_unwrap(&user, &pool, &SCALE, &0, &past).err(),
        err_code(dl)
    );
    assert_eq!(
        s.router
            .try_swap_sy_for_pt(&user, &pool, &SCALE, &0, &past)
            .err(),
        err_code(dl)
    );
    assert_eq!(
        s.router
            .try_swap_pt_for_sy(&user, &pool, &SCALE, &0, &past)
            .err(),
        err_code(dl)
    );
    assert_eq!(
        s.router
            .try_swap_yt_for_sy(&user, &pool, &SCALE, &0, &past)
            .err(),
        err_code(dl)
    );
    assert_eq!(
        s.router
            .try_add_liquidity(&user, &pool, &SCALE, &SCALE, &0, &past)
            .err(),
        err_code(dl)
    );
    assert_eq!(
        s.router
            .try_add_liquidity_single_sy(&user, &pool, &SCALE, &0, &past)
            .err(),
        err_code(dl)
    );
    assert_eq!(
        s.router
            .try_remove_liquidity(&user, &pool, &1, &0, &0, &past)
            .err(),
        err_code(dl)
    );
    assert_eq!(
        s.router
            .try_recombine(&user, &pool, &SCALE, &0, &past)
            .err(),
        err_code(dl)
    );

    // min-out guards.
    let sl = RouterError::SlippageExceeded as u32;
    assert_eq!(
        s.router
            .try_wrap_and_mint(&user, &pool, &(10 * SCALE), &(11 * SCALE), &u64::MAX)
            .err(),
        err_code(sl)
    );
    s.router
        .wrap_and_mint(&user, &pool, &(100 * SCALE), &(100 * SCALE), &u64::MAX);
    assert_eq!(
        s.router
            .try_recombine(&user, &pool, &(10 * SCALE), &(11 * SCALE), &u64::MAX)
            .err(),
        err_code(sl)
    );
}

#[test]
fn router_wrap_unwrap_recombine_and_liquidity_flows_work_end_to_end() {
    let (s, _, _) = seeded(UnderlyingKind::Sac);
    let pool = s.pool.address.clone();
    let user = s.new_user();
    s.mint_underlying(&user, 500 * SCALE);

    // wrap_and_mint: underlying -> SY -> PT + YT.
    let m = s
        .router
        .wrap_and_mint(&user, &pool, &(200 * SCALE), &0, &u64::MAX);
    assert_eq!(m.pt_minted, 200 * SCALE);
    assert_eq!(underlying_balance(&s, &user), 300 * SCALE);

    // recombine: PT + YT -> SY, then unwrap: SY -> underlying. Round trip is lossless (free market).
    let sy_back = s
        .router
        .recombine(&user, &pool, &(200 * SCALE), &0, &u64::MAX);
    assert_eq!(sy_back, 200 * SCALE);
    let out = s
        .router
        .unwrap(&user, &pool, &sy_back, &(200 * SCALE), &u64::MAX);
    assert_eq!(out, 200 * SCALE);
    assert_eq!(underlying_balance(&s, &user), 500 * SCALE);

    // Liquidity via the router.
    s.router
        .wrap_and_mint(&user, &pool, &(100 * SCALE), &0, &u64::MAX);
    deposit_sy(&s, &user, 100 * SCALE);
    let (_, _, lp) =
        s.router
            .add_liquidity(&user, &pool, &(50 * SCALE), &(60 * SCALE), &0, &u64::MAX);
    assert_eq!(s.pool.lp_balance(&user), lp);
    let (_, _, lp2) = s
        .router
        .add_liquidity_single_sy(&user, &pool, &(30 * SCALE), &0, &u64::MAX);
    assert_eq!(s.pool.lp_balance(&user), lp + lp2);
    let (pt_out, sy_out) = s
        .router
        .remove_liquidity(&user, &pool, &(lp + lp2), &0, &0, &u64::MAX);
    assert!(pt_out > 0 && sy_out > 0);
    assert_eq!(s.pool.lp_balance(&user), 0);
}

/// The five Router functions added for underlying-denominated trading and the "keep YT" deposit
/// (requested by the dApp team so a user paying in the yield-bearing underlying -- USDY, BENJI --
/// never has to hold SY directly). Each wraps/unwraps or sequences existing, already-tested
/// primitives; this proves the composition round-trips correctly end to end.
#[test]
fn router_underlying_denominated_flows_and_keep_yt_work_end_to_end() {
    let (s, _, _) = seeded(UnderlyingKind::Sac);
    let pool = s.pool.address.clone();
    let user = s.new_user();
    s.mint_underlying(&user, 500 * SCALE);

    // swap_underlying_for_pt: pay underlying directly, no separate wrap call by the caller.
    let pt_out = s
        .router
        .swap_underlying_for_pt(&user, &pool, &(100 * SCALE), &0, &u64::MAX);
    assert!(pt_out > 0);
    assert_eq!(underlying_balance(&s, &user), 400 * SCALE);
    assert_eq!(s.pt.balance(&user), pt_out);

    // swap_pt_for_underlying: sell it back, receive underlying directly (fee + price impact, so
    // strictly less than the 100 SCALE spent).
    let underlying_back = s
        .router
        .swap_pt_for_underlying(&user, &pool, &pt_out, &0, &u64::MAX);
    assert!(underlying_back > 0 && underlying_back < 100 * SCALE);
    assert_eq!(s.pt.balance(&user), 0);
    assert_eq!(underlying_balance(&s, &user), 400 * SCALE + underlying_back);

    // swap_underlying_for_yt: flash-mint YT directly from underlying, no SY left in the caller's
    // hands mid-flow. `sy_in` is quoted off-chain first (SYWrapper.exchange_rate()), exactly as a
    // real caller would, rather than derived mid-execution -- see the function's own docs.
    let sy_in_yt = underlying_to_shares_for_test(&s, 50 * SCALE);
    let (yt_out, _underlying_change) = s.router.swap_underlying_for_yt(
        &user,
        &pool,
        &(50 * SCALE),
        &sy_in_yt,
        &0,
        &i128::MAX,
        &u64::MAX,
    );
    assert!(yt_out > 0);
    assert_eq!(s.yt.balance(&user), yt_out);

    // swap_yt_for_underlying: flash-redeem back to underlying directly.
    let underlying_from_yt = s
        .router
        .swap_yt_for_underlying(&user, &pool, &yt_out, &0, &u64::MAX);
    assert!(underlying_from_yt > 0);
    assert_eq!(s.yt.balance(&user), 0);

    // add_liquidity_keep_yt: underlying -> LP + YT, no swap leg at all. `mint_amount`,
    // `pt_desired` and `sy_desired` are all quoted off-chain first via
    // `quote_add_liquidity_keep_yt` (which itself uses `MarketPool.keep_yt_split`), exactly as a
    // real caller would, rather than derived mid-execution -- see the function's own docs for why
    // that distinction matters.
    let q = s.router.quote_add_liquidity_keep_yt(&pool, &(100 * SCALE));
    let result = s.router.add_liquidity_keep_yt(
        &user,
        &pool,
        &(100 * SCALE),
        &q.mint_amount,
        &q.pt_desired,
        &q.sy_desired,
        &0,
        &u64::MAX,
    );
    assert!(result.lp_minted > 0, "lp_minted {}", result.lp_minted);
    assert!(result.yt_minted > 0, "yt_minted {}", result.yt_minted);
    assert_eq!(s.pool.lp_balance(&user), result.lp_minted);
    assert_eq!(s.yt.balance(&user), result.yt_minted);

    // Deadline and slippage guards apply to the new functions exactly like the existing ones.
    let past = s.env.ledger().timestamp() - 1;
    assert_eq!(
        s.router
            .try_swap_underlying_for_pt(&user, &pool, &SCALE, &0, &past)
            .err(),
        err_code(RouterError::DeadlineExpired as u32)
    );
    s.mint_underlying(&user, 10 * SCALE);
    // The slippage check for this function lives in MarketPool.swap_sy_for_pt itself (this
    // wrapper forwards min_pt_out unchanged), exactly like the existing swap_sy_for_pt wrapper --
    // so the error code is MarketPool's SlippageExceeded, not the Router's own.
    assert_eq!(
        s.router
            .try_swap_underlying_for_pt(
                &user,
                &pool,
                &(10 * SCALE),
                &(1_000_000 * SCALE),
                &u64::MAX
            )
            .err(),
        err_code(PoolError::SlippageExceeded as u32)
    );
}

/// Regression for a dApp-integration review's core finding: `swap_pt_for_underlying`,
/// `swap_yt_for_underlying`, `swap_underlying_for_yt` and `remove_liquidity_to_underlying` all
/// used to require `from` to hold **zero** pre-existing PT/SY (sweeping the *entire* balance via
/// `withdraw_all`/`swap_all_pt_for_sy`), which failed routinely: the "LP only" zap and Keep YT
/// deposits can themselves leave small PT/SY remainders, and simply having bought PT on some
/// earlier, unrelated trade is enough to be "holding PT" by definition. All four now sweep only
/// the gain *since a baseline* read at the start of each call (`withdraw_gained`/
/// `swap_gained_pt_for_sy`), so pre-existing, unrelated PT/SY must survive every one of them
/// untouched.
#[test]
fn underlying_denominated_router_flows_tolerate_pre_existing_pt_and_sy() {
    let (s, _, _) = seeded(UnderlyingKind::Sac);
    let pool = s.pool.address.clone();
    let user = s.new_user();
    s.mint_underlying(&user, 1_000 * SCALE);

    // Unrelated pre-existing PT and SY, from trades that have nothing to do with what follows.
    let stray_pt = s
        .router
        .swap_underlying_for_pt(&user, &pool, &(20 * SCALE), &0, &u64::MAX);
    assert!(stray_pt > 0);
    let stray_sy = deposit_sy(&s, &user, 15 * SCALE);
    assert!(stray_sy > 0);
    assert_eq!(s.pt.balance(&user), stray_pt);
    assert_eq!(s.sy.balance_of(&user), stray_sy);

    // swap_pt_for_underlying on a *second* batch of PT: the stray PT/SY above must survive.
    let pt_out = s
        .router
        .swap_underlying_for_pt(&user, &pool, &(30 * SCALE), &0, &u64::MAX);
    s.router
        .swap_pt_for_underlying(&user, &pool, &pt_out, &0, &u64::MAX);
    assert_eq!(s.pt.balance(&user), stray_pt, "stray PT survived the sell");
    assert_eq!(
        s.sy.balance_of(&user),
        stray_sy,
        "stray SY survived the sell"
    );

    // swap_underlying_for_yt then swap_yt_for_underlying: same check, on top of the same stray
    // balances (not reset between steps).
    let sy_in_yt = underlying_to_shares_for_test(&s, 25 * SCALE);
    let (yt_out, _) = s.router.swap_underlying_for_yt(
        &user,
        &pool,
        &(25 * SCALE),
        &sy_in_yt,
        &0,
        &i128::MAX,
        &u64::MAX,
    );
    assert!(yt_out > 0);
    assert_eq!(s.pt.balance(&user), stray_pt);
    assert_eq!(s.sy.balance_of(&user), stray_sy);
    s.router
        .swap_yt_for_underlying(&user, &pool, &yt_out, &0, &u64::MAX);
    assert_eq!(s.pt.balance(&user), stray_pt);
    assert_eq!(s.sy.balance_of(&user), stray_sy);

    // remove_liquidity_to_underlying: the LP position is created fresh, then removed in full,
    // with the stray PT and SY still sitting untouched in the user's wallet throughout.
    // `add_liquidity_single_underlying`'s own zap can add a little more SY dust on top of
    // `stray_sy` (the same small rounding refund `single_sided_deposit_...` above already
    // documents) -- legitimate, not a regression -- so the PT check still pins down the exact
    // original `stray_pt` (PT dust is never refunded at all anymore, see that fix above), while
    // the SY baseline for the *next* assertion is whatever the balance legitimately is now.
    let sy_in_add = underlying_to_shares_for_test(&s, 40 * SCALE);
    let (_, _, lp) = s.router.add_liquidity_single_underlying(
        &user,
        &pool,
        &(40 * SCALE),
        &sy_in_add,
        &0,
        &u64::MAX,
    );
    assert!(lp > 0);
    assert_eq!(s.pt.balance(&user), stray_pt);
    let sy_before_remove = s.sy.balance_of(&user);
    assert!(
        sy_before_remove >= stray_sy,
        "stray SY survived the zap: {sy_before_remove} vs {stray_sy}"
    );

    let underlying_out = s
        .router
        .remove_liquidity_to_underlying(&user, &pool, &lp, &0, &u64::MAX);
    assert!(underlying_out > 0);
    assert_eq!(s.pool.lp_balance(&user), 0);
    assert_eq!(
        s.pt.balance(&user),
        stray_pt,
        "stray PT survived remove_liquidity_to_underlying"
    );
    assert_eq!(
        s.sy.balance_of(&user),
        sy_before_remove,
        "the SY balance held before remove_liquidity_to_underlying survived it untouched"
    );
}

/// Regression for a dApp-reported defect: `flash_mint_yt_from_underlying` used to revert
/// `ZeroAmount` whenever the wrap produced no surplus over the real net cost -- including
/// `underlying_budget = 0` (the whole cost covered by SY the buyer already held), forcing a
/// caller to always over-budget by a margin. `withdraw_gained` now tolerates a zero (or negative,
/// meaning this call spent into the pre-existing balance rather than growing it) gain, returning
/// `0` for `underlying_back` instead of reverting. `max_net_cost` is given a generous margin here
/// deliberately -- it is a separate, real economic slippage guard a caller chooses, not part of
/// what this fix changes; this test isolates the budget/surplus defect from that guard.
#[test]
fn flash_mint_yt_from_underlying_tolerates_a_budget_with_no_surplus() {
    let (s, _, _) = seeded(UnderlyingKind::Sac);
    let pool = s.pool.address.clone();
    let sy_in = 100 * SCALE;

    // Budget of 0: the whole net cost is covered by SY the buyer already held, generously padded
    // so the exact cost (whatever it turns out to be) is irrecoverably negative against it --
    // i.e. this call can only ever spend into the pre-existing balance, never grow it.
    let buyer = s.new_user();
    let padded = deposit_sy(&s, &buyer, 10 * sy_in);
    let (yt_out, charged, underlying_back) = s.router.flash_mint_yt_from_underlying(
        &buyer,
        &pool,
        &sy_in,
        &0,
        &sy_in,
        &0,
        &0,
        &u64::MAX,
    );
    assert!(yt_out > 0);
    assert!(charged > 0 && charged < sy_in);
    assert_eq!(underlying_back, 0);
    assert_eq!(s.sy.balance_of(&buyer), padded - charged);
}

/// A zero gain is only silently fine when the caller didn't actually require getting something
/// back: `withdraw_gained`/`swap_gained_pt_for_sy` must still enforce a nonzero `min_*_out` as a
/// `SlippageExceeded` revert rather than returning 0.
#[test]
fn gained_sweeps_still_enforce_slippage_on_a_zero_gain() {
    let (s, _, _) = seeded(UnderlyingKind::Sac);
    let user = s.new_user();
    s.mint_underlying(&user, 10 * SCALE);
    let shares = s.sy.deposit(&user, &(10 * SCALE), &0);
    let baseline = shares; // nothing gained since this exact balance
    assert_eq!(
        s.sy.try_withdraw_gained(&user, &baseline, &user, &1).err(),
        err_code(SyError::SlippageExceeded as u32)
    );
    // With no minimum required, the same zero-gain call is fine and returns 0.
    assert_eq!(s.sy.withdraw_gained(&user, &baseline, &user, &0), 0);

    let pt_out = s.pool.swap_sy_for_pt(&user, &user, &shares, &0);
    let pt_baseline = pt_out; // nothing gained since this exact balance
    assert_eq!(
        s.pool
            .try_swap_gained_pt_for_sy(&user, &user, &pt_baseline, &1)
            .err(),
        err_code(PoolError::SlippageExceeded as u32)
    );
    assert_eq!(
        s.pool.swap_gained_pt_for_sy(&user, &user, &pt_baseline, &0),
        0
    );
}

/// Each of the five quotes added on top of §4.1's functions composes `MarketPool`'s own
/// SY-denominated quote with `SYWrapper.exchange_rate()`, replicating the exact rounding its real
/// call applies. Proven here against the real executed outcome, not just by inspection -- same
/// style as `contracts/integration_tests/tests/amm.rs`'s `quote_*_matches_the_real_*_exactly`
/// tests for the `MarketPool`-level quotes these build on.
#[test]
fn router_quotes_for_underlying_denominated_flows_match_the_real_outcome_exactly() {
    let (s, _, _) = seeded(UnderlyingKind::Sac);
    let pool = s.pool.address.clone();
    let user = s.new_user();
    s.mint_underlying(&user, 500 * SCALE);

    let q = s.router.quote_underlying_for_pt(&pool, &(100 * SCALE));
    let pt_out = s
        .router
        .swap_underlying_for_pt(&user, &pool, &(100 * SCALE), &0, &u64::MAX);
    assert_eq!(q.amount_out, pt_out, "quote_underlying_for_pt");

    let q = s.router.quote_pt_for_underlying(&pool, &pt_out);
    let underlying_back = s
        .router
        .swap_pt_for_underlying(&user, &pool, &pt_out, &0, &u64::MAX);
    assert_eq!(q.amount_out, underlying_back, "quote_pt_for_underlying");

    let sy_in_yt = underlying_to_shares_for_test(&s, 50 * SCALE);
    let q = s.router.quote_underlying_for_yt(&pool, &(50 * SCALE));
    let (yt_out, underlying_change) = s.router.swap_underlying_for_yt(
        &user,
        &pool,
        &(50 * SCALE),
        &sy_in_yt,
        &0,
        &i128::MAX,
        &u64::MAX,
    );
    assert_eq!(q.yt_out, yt_out, "quote_underlying_for_yt yt_out");
    assert_eq!(
        q.underlying_back, underlying_change,
        "quote_underlying_for_yt underlying_back"
    );

    let q = s.router.quote_yt_for_underlying(&pool, &yt_out);
    let underlying_from_yt = s
        .router
        .swap_yt_for_underlying(&user, &pool, &yt_out, &0, &u64::MAX);
    assert_eq!(q.amount_out, underlying_from_yt, "quote_yt_for_underlying");

    let q = s.router.quote_add_liquidity_keep_yt(&pool, &(100 * SCALE));
    assert_eq!(
        q.mint_amount + q.sy_desired,
        underlying_to_shares_for_test(&s, 100 * SCALE),
        "mint_amount + sy_desired must account for the full wrapped amount"
    );
    assert!(q.lp_estimate > 0, "lp_estimate {}", q.lp_estimate);
    let result = s.router.add_liquidity_keep_yt(
        &user,
        &pool,
        &(100 * SCALE),
        &q.mint_amount,
        &q.pt_desired,
        &q.sy_desired,
        &0,
        &u64::MAX,
    );
    assert_eq!(
        q.yt_out, result.yt_minted,
        "quote_add_liquidity_keep_yt yt_out"
    );
    assert_eq!(
        q.lp_estimate, result.lp_minted,
        "quote_add_liquidity_keep_yt lp_estimate"
    );

    // quote_add_liquidity_underlying / quote_lp_to_underlying: the two quotes a dApp-integration
    // review found missing for add_liquidity_single_underlying / remove_liquidity_to_underlying.
    let q = s
        .router
        .quote_add_liquidity_underlying(&pool, &(60 * SCALE));
    let sy_in_add = underlying_to_shares_for_test(&s, 60 * SCALE);
    assert_eq!(q.sy_in, sy_in_add, "quote_add_liquidity_underlying sy_in");
    let (pt_used, sy_used, lp_minted) = s.router.add_liquidity_single_underlying(
        &user,
        &pool,
        &(60 * SCALE),
        &q.sy_in,
        &0,
        &u64::MAX,
    );
    assert_eq!(q.pt_used, pt_used, "quote_add_liquidity_underlying pt_used");
    assert_eq!(q.sy_used, sy_used, "quote_add_liquidity_underlying sy_used");
    assert_eq!(
        q.lp_minted, lp_minted,
        "quote_add_liquidity_underlying lp_minted"
    );

    let q = s.router.quote_lp_to_underlying(&pool, &lp_minted);
    let underlying_out =
        s.router
            .remove_liquidity_to_underlying(&user, &pool, &lp_minted, &0, &u64::MAX);
    assert_eq!(
        q.underlying_out, underlying_out,
        "quote_lp_to_underlying underlying_out"
    );
}

/// `underlying_amount * RATE_SCALE / exchange_rate()`, matching `SYWrapper`'s own
/// `underlying_to_shares` exactly (its `total_shares == 0` bootstrap case folds into the same
/// formula, since `exchange_rate()` itself returns `RATE_SCALE` then).
fn underlying_to_shares_for_test(s: &Stack<'_>, underlying_amount: i128) -> i128 {
    underlying_amount * SCALE / s.sy.exchange_rate()
}

// ------------------------------------------------- true flash-mint YT, via the Router

/// `Router.flash_mint_yt` is a thin forward to `MarketPool.flash_mint_yt`: the buyer funds only
/// the net cost, quoted beforehand via `MarketPool.quote_sy_for_yt`, never the full `sy_in`
/// notional `Router.swap_sy_for_yt` requires.
#[test]
fn router_flash_mint_yt_only_requires_the_net_cost() {
    let (s, _, _) = seeded(UnderlyingKind::Sac);
    let pool = s.pool.address.clone();
    let sy_in = 100 * SCALE;
    let quote = s.pool.quote_sy_for_yt(&sy_in);
    let net_cost = sy_in - quote.sy_back;

    let buyer = s.new_user();
    deposit_sy(&s, &buyer, net_cost);
    let (yt_out, charged) = s
        .router
        .flash_mint_yt(&buyer, &pool, &sy_in, &0, &net_cost, &u64::MAX);

    assert_eq!(yt_out, quote.yt_out);
    assert_eq!(charged, net_cost);
    assert_eq!(s.yt.balance(&buyer), yt_out);
    assert_eq!(s.sy.balance_of(&buyer), 0);

    // Deadline and slippage guards apply, same as every other Router flow.
    let past = s.env.ledger().timestamp() - 1;
    assert_eq!(
        s.router
            .try_flash_mint_yt(&buyer, &pool, &sy_in, &0, &net_cost, &past)
            .err(),
        err_code(RouterError::DeadlineExpired as u32)
    );
}

/// Same true flash-mint, for a caller who holds the underlying rather than SY: the Router wraps
/// only `underlying_budget`, not the full notional, and unwraps any surplus between the budget
/// and the real net cost straight back to underlying -- a later integration review found that
/// leaving it as SY cost a caller an extra signature before any further same-transaction call
/// using `withdraw_all` (see the function's own docs).
#[test]
fn router_flash_mint_yt_from_underlying_wraps_only_the_budget() {
    let (s, _, _) = seeded(UnderlyingKind::Sac);
    let pool = s.pool.address.clone();
    let sy_in = 100 * SCALE;
    let quote = s.pool.quote_sy_for_yt(&sy_in);
    let net_cost = sy_in - quote.sy_back;
    let underlying_budget = net_cost + 5 * SCALE; // generous: covers the cost plus a margin

    let buyer = s.new_user();
    s.mint_underlying(&buyer, underlying_budget);
    let underlying_before = underlying_balance(&s, &buyer);

    let (yt_out, charged, underlying_back) = s.router.flash_mint_yt_from_underlying(
        &buyer,
        &pool,
        &sy_in,
        &0,
        &net_cost,
        &underlying_budget,
        &0,
        &u64::MAX,
    );

    assert_eq!(yt_out, quote.yt_out);
    assert_eq!(charged, net_cost);
    assert_eq!(underlying_back, underlying_budget - net_cost);
    assert_eq!(s.yt.balance(&buyer), yt_out);
    // The margin above the real net cost comes back as underlying, not stranded SY: net effect
    // on the buyer's underlying balance is exactly `-net_cost`, and no SY is left behind.
    assert_eq!(underlying_balance(&s, &buyer), underlying_before - net_cost);
    assert_eq!(s.sy.balance_of(&buyer), 0);
}

/// `underlying_budget` is a separate, caller-chosen amount, not derived from `max_net_cost` --
/// nothing stops a caller from passing a budget below the real net cost. That must revert
/// cleanly (the pool simply cannot pull more SY than the buyer ends up holding), and because a
/// Soroban panic rolls back the *entire* transaction, the earlier underlying-to-SY wrap must be
/// undone too, not left half-applied.
#[test]
fn router_flash_mint_yt_from_underlying_reverts_cleanly_when_the_budget_is_below_the_net_cost() {
    let (s, _, _) = seeded(UnderlyingKind::Sac);
    let pool = s.pool.address.clone();
    let sy_in = 100 * SCALE;
    let quote = s.pool.quote_sy_for_yt(&sy_in);
    let net_cost = sy_in - quote.sy_back;
    let underlying_budget = net_cost - 1; // one unit short of the real cost

    let buyer = s.new_user();
    s.mint_underlying(&buyer, underlying_budget);
    let underlying_before = underlying_balance(&s, &buyer);

    let r = s.router.try_flash_mint_yt_from_underlying(
        &buyer,
        &pool,
        &sy_in,
        &0,
        &net_cost,
        &underlying_budget,
        &0,
        &u64::MAX,
    );
    assert!(r.is_err());
    assert_eq!(underlying_balance(&s, &buyer), underlying_before);
    assert_eq!(s.yt.balance(&buyer), 0);
    assert_eq!(s.sy.balance_of(&buyer), 0);
}

#[test]
fn router_acts_as_the_user_so_a_deauthorized_user_gains_nothing_by_using_it() {
    for kind in [UnderlyingKind::Sac, UnderlyingKind::Rwa] {
        let (s, _, _) = seeded(kind);
        let pool = s.pool.address.clone();
        let user = s.new_user();
        s.mint_underlying(&user, 50 * SCALE);
        s.deauthorize(&user);
        assert!(s
            .router
            .try_wrap_and_mint(&user, &pool, &(10 * SCALE), &0, &u64::MAX)
            .is_err());
        assert!(s
            .router
            .try_swap_sy_for_pt(&user, &pool, &SCALE, &0, &u64::MAX)
            .is_err());
        // The true flash-mint variants too -- found as a coverage gap during a final audit
        // review: both are thin forwards to MarketPool.flash_mint_yt with from.require_auth()
        // first, same as every other Router call above, but neither had its own dedicated
        // deauthorized-user check.
        assert!(s
            .router
            .try_flash_mint_yt(&user, &pool, &SCALE, &0, &SCALE, &u64::MAX)
            .is_err());
        assert!(s
            .router
            .try_flash_mint_yt_from_underlying(
                &user,
                &pool,
                &SCALE,
                &0,
                &SCALE,
                &SCALE,
                &0,
                &u64::MAX
            )
            .is_err());
        // The Router itself holds nothing and has no balance anywhere.
        assert_eq!(s.sy.balance_of(&s.router.address), 0);
        assert_eq!(s.pt.balance(&s.router.address), 0);
        assert_eq!(s.yt.balance(&s.router.address), 0);
        assert_eq!(underlying_balance(&s, &s.router.address), 0);
    }
}

#[test]
fn router_admin_and_initialization() {
    let s = deploy(Config::free(T0 + 180 * DAY));
    assert_eq!(
        s.router.try_initialize(&s.admin).err(),
        err_code(RouterError::AlreadyInitialized as u32)
    );
    let new_admin = Address::generate(&s.env);
    let stranger = Address::generate(&s.env);
    assert_eq!(
        s.router.try_transfer_admin(&stranger, &new_admin).err(),
        err_code(RouterError::Unauthorized as u32)
    );
    s.router.transfer_admin(&s.admin, &new_admin);
    assert_eq!(s.router.get_admin(), new_admin);
}

#[test]
fn redeem_and_claim_through_the_router_after_maturity() {
    let (s, lp, _) = seeded(UnderlyingKind::Sac);
    let pool = s.pool.address.clone();
    let user = s.new_user();
    s.mint_underlying(&user, 100 * SCALE);
    s.router
        .wrap_and_mint(&user, &pool, &(100 * SCALE), &0, &u64::MAX);
    s.advance(T0 + 90 * DAY, SCALE * 105 / 100);
    let claimed = s.router.claim_yield(&user, &pool);
    assert!(claimed > 0);
    s.advance(s.maturity, SCALE * 110 / 100);
    let out = s
        .router
        .redeem_at_maturity(&user, &pool, &(100 * SCALE), &(100 * SCALE));
    assert!(out.underlying_from_pt > 0);
    assert!(out.underlying_from_yt > 0);
    let _ = lp;
}
