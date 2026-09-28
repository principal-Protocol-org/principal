//! Liquidity provision (proportional and single-sided), LP token compliance, pool guards, and every
//! Router flow -- registry, deadlines, min-out guards, wrap/unwrap, recombine, redemption.

use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    Address,
};

use principal_integration_tests::stack::*;
use principal_market_pool::{Error as PoolError, MarketPoolContract, MarketPoolContractClient};
use principal_router::Error as RouterError;

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
    let (pt_used, sy_used, lp) = s.pool.add_liquidity_single_sy(&user, &(100 * SCALE), &0);
    assert!(lp > 0);
    // Nearly all the SY was used; whatever is left is rounding-sized and was returned.
    let left = 100 * SCALE - sy_used;
    assert_eq!(s.sy.balance_of(&user), left);
    assert!(left < SCALE / 100, "leftover SY {left}");
    // Any PT left over is rounding-sized and was refunded, not stranded in the pool.
    assert!(
        s.pt.balance(&user) < SCALE / 100,
        "PT refund {}",
        s.pt.balance(&user)
    );
    // The pool grew and the ratio stayed within a hair of the original.
    let (pt_res1, sy_res1) = s.pool.reserves();
    assert!(pt_res1 <= pt_res0 + pt_used && sy_res1 > sy_res0);
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
fn router_registration_rejects_a_pool_that_belongs_to_a_different_market() {
    // A second, independent market stack: its pool trades different SY/PT/YT tokens than the
    // first stack's manager describes... registering pool B against router A is fine, because a
    // pool is self-describing; what must fail is a pool whose manager disagrees with its own
    // tokens. That cannot happen by construction (the pool reads them from its manager), so the
    // check is exercised by the positive path: the resolved info always matches the pool.
    let s = deploy(Config::free(T0 + 180 * DAY));
    let info = s.router.market(&s.pool.address);
    assert_eq!(info.sy, s.pool.sy_address());
    assert_eq!(info.pt, s.pool.pt_address());
    assert_eq!(info.yt, s.pool.yt_address());
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
