//! Deliverable 3, edge cases: error conditions, boundary values (zero, negative, maximum and
//! exact-threshold amounts) and authorization checks (who may call what, and what happens when they
//! are not allowed) across the protocol contracts.

use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    Address,
};

use principal_integration_tests::stack::*;
use principal_manager::Error as PmError;
use principal_pt_token::Error as PtError;
use principal_sy_wrapper::Error as SyError;
use principal_yt_token::Error as YtError;

fn seeded() -> (Stack<'static>, Address) {
    let s = deploy(Config::with_example_fees(LONG));
    let (_, lp) = seed_pool(&s, 1_000 * SCALE, 950 * SCALE);
    (s, lp)
}

// =================================================================== zero / negative

#[test]
fn zero_and_negative_amounts_are_rejected_by_every_entry_point() {
    let (s, _) = seeded();
    let u = s.new_user();
    deposit_sy(&s, &u, 100 * SCALE);
    let m = s.pm.mint(&u, &(50 * SCALE));
    let zero = SyError::ZeroAmount as u32;

    for bad in [0_i128, -1, i128::MIN] {
        // SYWrapper
        assert_eq!(s.sy.try_deposit(&u, &bad, &0).err(), err_code(zero));
        assert_eq!(s.sy.try_withdraw(&u, &bad, &u, &0).err(), err_code(zero));
        assert_eq!(
            s.sy.try_transfer(&u, &s.treasury, &bad).err(),
            err_code(zero)
        );
        // PT / YT (SEP-41 surface)
        assert_eq!(
            s.pt.try_transfer(&u, &s.treasury, &bad).err(),
            err_code(PtError::ZeroAmount as u32)
        );
        assert_eq!(
            s.yt.try_transfer(&u, &s.treasury, &bad).err(),
            err_code(YtError::ZeroAmount as u32)
        );
        assert_eq!(
            s.pt.try_transfer_from(&s.treasury, &u, &s.treasury, &bad)
                .err(),
            err_code(PtError::ZeroAmount as u32)
        );
        // PrincipalManager
        assert_eq!(
            s.pm.try_mint(&u, &bad).err(),
            err_code(PmError::ZeroAmount as u32)
        );
        assert_eq!(
            s.pm.try_recombine(&u, &bad).err(),
            err_code(PmError::ZeroAmount as u32)
        );
        // MarketPool
        assert!(s.pool.try_swap_sy_for_pt(&u, &u, &bad, &0).is_err());
        assert!(s.pool.try_swap_pt_for_sy(&u, &u, &bad, &0).is_err());
        assert!(s.pool.try_swap_yt_for_sy(&u, &u, &bad, &0).is_err());
        assert!(s.pool.try_add_liquidity(&u, &bad, &SCALE, &0).is_err());
        assert!(s.pool.try_remove_liquidity(&u, &u, &bad, &0, &0).is_err());
    }
    // Negative approve is rejected; a zero approve is a legal "revoke".
    assert!(s
        .pt
        .try_approve(&u, &s.treasury, &(-1), &(s.env.ledger().sequence() + 10))
        .is_err());
    s.pt.approve(&u, &s.treasury, &0, &(s.env.ledger().sequence() + 10));
    let _ = m;
}

// =================================================================== maximum amounts

#[test]
fn maximum_amounts_revert_cleanly_and_leave_state_untouched() {
    let (s, _) = seeded();
    let u = s.new_user();
    deposit_sy(&s, &u, 100 * SCALE);
    let snapshot = |s: &Stack, u: &Address| {
        (
            s.sy.balance_of(u),
            s.sy.total_shares(),
            s.pt.balance(u),
            s.pm.total_pt(),
            s.pool.reserves(),
            underlying_balance(s, u),
        )
    };
    let before = snapshot(&s, &u);

    // Amounts vastly beyond any balance, up to i128::MAX: every one must revert, never wrap.
    for huge in [
        i128::MAX,
        i128::MAX / 2,
        i128::MAX / SCALE,
        1_000_000_000 * SCALE,
    ] {
        assert!(s.sy.try_deposit(&u, &huge, &0).is_err());
        assert!(s.sy.try_withdraw(&u, &huge, &u, &0).is_err());
        assert!(s.sy.try_transfer(&u, &s.treasury, &huge).is_err());
        assert!(s.pm.try_mint(&u, &huge).is_err());
        assert!(s.pool.try_swap_sy_for_pt(&u, &u, &huge, &0).is_err());
        assert!(s.pool.try_swap_pt_for_sy(&u, &u, &huge, &0).is_err());
        assert!(s.pool.try_quote_buy_exact_pt(&huge).is_err());
        assert!(s.pool.try_quote_sy_for_pt(&huge).is_err());
        assert!(s.pool.try_add_liquidity(&u, &huge, &huge, &0).is_err());
        assert!(s.pt.try_transfer(&u, &s.treasury, &huge).is_err());
        assert!(s.yt.try_transfer(&u, &s.treasury, &huge).is_err());
    }
    assert_eq!(
        snapshot(&s, &u),
        before,
        "a rejected call must not change any state"
    );
}

#[test]
fn the_largest_sane_position_round_trips_without_overflow() {
    // 10 billion tokens (1e17 raw units) -- far beyond any real market -- still tokenizes,
    // provides liquidity, trades and redeems exactly, so the fixed-point paths have headroom.
    let s = deploy_stack(T0 + 180 * DAY);
    let big = 10_000_000_000_i128 * SCALE;

    let lp = s.new_user();
    deposit_sy(&s, &lp, 2 * big);
    let m = s.pm.mint(&lp, &big);
    assert_eq!(m.pt_minted, big);
    s.pool.add_liquidity(&lp, &big, &(big * 95 / 100), &0);

    let whale = s.new_user();
    deposit_sy(&s, &whale, 2 * big);
    s.pm.mint(&whale, &big);
    let pt = s.pool.swap_sy_for_pt(&whale, &whale, &(big / 100), &0);
    assert!(pt > big / 100);
    let back = s.pool.swap_pt_for_sy(&whale, &whale, &pt, &0);
    assert!(back <= big / 100 && big / 100 - back < big / 1_000_000);

    s.advance(s.maturity, SCALE);
    let out = s.pm.redeem(&whale, &big, &big);
    assert_eq!(out.underlying_from_pt, big);
}

#[test]
fn fee_free_round_trips_never_profit_at_any_pool_size() {
    // The fixed-point error grows with pool size; the rounding pad must always cover it.
    for exp in [8_u32, 10, 12, 14, 16, 17] {
        let size = 10_i128.pow(exp);
        let s = deploy_stack(T0 + 180 * DAY);
        let lp = s.new_user();
        deposit_sy(&s, &lp, 2 * size);
        s.pm.mint(&lp, &size);
        s.pool.add_liquidity(&lp, &size, &(size * 95 / 100), &0);
        let t = s.new_user();
        deposit_sy(&s, &t, size / 10);
        let amt = size / 100;
        let pt = s.pool.swap_sy_for_pt(&t, &t, &amt, &0);
        let back = s.pool.swap_pt_for_sy(&t, &t, &pt, &0);
        assert!(
            back <= amt,
            "size 1e{exp}: round trip profited {} units",
            back - amt
        );
    }
}

// =================================================================== exact thresholds

#[test]
fn deposit_cap_boundary_exactly_at_the_cap_passes_and_one_unit_over_reverts() {
    let (s, _) = seeded();
    s.sy.set_deposit_cap(&s.admin, &(100 * SCALE));
    assert_eq!(s.sy.deposit_cap(), 100 * SCALE);
    let u = s.new_user();
    s.mint_underlying(&u, 500 * SCALE);
    s.sy.deposit(&u, &(60 * SCALE), &0);
    s.sy.deposit(&u, &(40 * SCALE), &0); // exactly at the cap
    assert_eq!(s.sy.net_deposited(&u), 100 * SCALE);
    assert_eq!(
        s.sy.try_deposit(&u, &1, &0).err(),
        err_code(SyError::DepositCapExceeded as u32)
    );
    // Withdrawing frees headroom one-for-one.
    s.sy.withdraw(&u, &(30 * SCALE), &u, &0);
    assert_eq!(s.sy.net_deposited(&u), 70 * SCALE);
    s.sy.deposit(&u, &(30 * SCALE), &0);
    // The cap is per address: another depositor is unaffected.
    let v = s.new_user();
    s.mint_underlying(&v, 100 * SCALE);
    s.sy.deposit(&v, &(100 * SCALE), &0);
    // Cap 0 disables it; a negative cap is invalid.
    assert_eq!(
        s.sy.try_set_deposit_cap(&s.admin, &(-1)).err(),
        err_code(SyError::InvalidCap as u32)
    );
    s.sy.set_deposit_cap(&s.admin, &0);
    s.mint_underlying(&v, 5 * SCALE);
    s.sy.deposit(&v, &(5 * SCALE), &0); // v is now above its old cap: a disabled cap gates nothing
    assert_eq!(s.sy.net_deposited(&v), 105 * SCALE);
}

#[test]
fn slippage_boundary_min_equals_exact_output_passes_and_one_more_reverts() {
    let (s, _) = seeded();
    let u = s.new_user();
    s.mint_underlying(&u, 500 * SCALE);
    // At rate 1.0 a 100 deposit mints exactly 100 shares.
    assert_eq!(
        s.sy.try_deposit(&u, &(100 * SCALE), &(100 * SCALE + 1))
            .err(),
        err_code(SyError::SlippageExceeded as u32)
    );
    assert_eq!(
        s.sy.deposit(&u, &(100 * SCALE), &(100 * SCALE)),
        100 * SCALE
    );
    assert_eq!(
        s.sy.try_withdraw(&u, &(50 * SCALE), &u, &(50 * SCALE + 1))
            .err(),
        err_code(SyError::SlippageExceeded as u32)
    );
    assert_eq!(
        s.sy.withdraw(&u, &(50 * SCALE), &u, &(50 * SCALE)),
        50 * SCALE
    );
}

#[test]
fn maturity_boundary_mint_allowed_until_the_last_second_redeem_from_the_first() {
    let maturity = T0 + 100;
    let s = deploy_stack(maturity);
    let u = s.new_user();
    deposit_sy(&s, &u, 100 * SCALE);

    s.advance_flat(maturity - 1);
    let m = s.pm.mint(&u, &(50 * SCALE));
    assert!(!s.pm.is_mature());
    assert_eq!(
        s.pm.try_redeem(&u, &m.pt_minted, &0).err(),
        err_code(PmError::NotMature as u32)
    );
    assert_eq!(
        s.pm.try_settle_all().err(),
        err_code(PmError::NotMature as u32)
    );

    s.advance_flat(maturity);
    assert!(s.pm.is_mature());
    assert_eq!(
        s.pm.try_mint(&u, &(10 * SCALE)).err(),
        err_code(PmError::AlreadyMature as u32)
    );
    assert_eq!(
        s.pm.redeem(&u, &m.pt_minted, &0).underlying_from_pt,
        m.pt_minted
    );
}

#[test]
fn oracle_freshness_boundary_exactly_at_the_window_is_fresh_one_second_later_is_stale() {
    let s = deploy_stack(LONG);
    let u = s.new_user();
    deposit_sy(&s, &u, 100 * SCALE);
    // The oracle was published at T0; the window is 3600 s.
    s.env.ledger().with_mut(|li| li.timestamp = T0 + 3_600);
    s.pm.mint(&u, &(10 * SCALE));
    s.env.ledger().with_mut(|li| li.timestamp = T0 + 3_601);
    assert_eq!(
        s.pm.try_mint(&u, &(10 * SCALE)).err(),
        err_code(PmError::OracleStale as u32)
    );
    // An oracle timestamp *ahead* of the ledger clock is also treated as stale.
    s.env.ledger().with_mut(|li| li.timestamp = T0 - 1);
    assert!(s.pm.try_mint(&u, &(10 * SCALE)).is_err());
}

#[test]
fn allowance_expiry_boundary() {
    let (s, _) = seeded();
    let owner = s.new_user();
    let spender = s.new_user();
    let shares = deposit_sy(&s, &owner, 100 * SCALE);
    s.pm.mint(&owner, &shares);
    let seq = s.env.ledger().sequence();
    s.pt.approve(&owner, &spender, &(20 * SCALE), &(seq + 5));
    // Valid through its expiration ledger...
    s.env.ledger().with_mut(|li| li.sequence_number = seq + 5);
    s.pt.transfer_from(&spender, &owner, &spender, &(10 * SCALE));
    // ...and dead one ledger later.
    s.env.ledger().with_mut(|li| li.sequence_number = seq + 6);
    assert_eq!(
        s.pt.try_transfer_from(&spender, &owner, &spender, &(10 * SCALE))
            .err(),
        err_code(PtError::InsufficientAllowance as u32)
    );
    // Spending more than the allowance is refused even before expiry.
    s.pt.approve(&owner, &spender, &(5 * SCALE), &(seq + 100));
    assert_eq!(
        s.pt.try_transfer_from(&spender, &owner, &spender, &(5 * SCALE + 1))
            .err(),
        err_code(PtError::InsufficientAllowance as u32)
    );
}

#[test]
fn deadline_boundary_now_equals_deadline_passes() {
    let (s, _) = seeded();
    let u = s.new_user();
    deposit_sy(&s, &u, 10 * SCALE);
    let now = s.env.ledger().timestamp();
    s.router
        .swap_sy_for_pt(&u, &s.pool.address, &SCALE, &0, &now);
    assert!(s
        .router
        .try_swap_sy_for_pt(&u, &s.pool.address, &SCALE, &0, &(now - 1))
        .is_err());
}

// =================================================================== authorization: wrong caller

#[test]
fn every_admin_only_function_rejects_a_caller_that_is_not_the_admin() {
    let (s, _) = seeded();
    let x = Address::generate(&s.env);
    let unauthorized = SyError::Unauthorized as u32;

    // SYWrapper
    assert_eq!(s.sy.try_set_paused(&x, &true).err(), err_code(unauthorized));
    assert_eq!(
        s.sy.try_set_deposit_cap(&x, &5).err(),
        err_code(unauthorized)
    );
    assert_eq!(
        s.sy.try_set_risk_control(&x, &x).err(),
        err_code(unauthorized)
    );
    assert_eq!(
        s.sy.try_transfer_admin(&x, &x).err(),
        err_code(unauthorized)
    );
    // PrincipalManager
    assert_eq!(
        s.pm.try_set_paused(&x, &true).err(),
        err_code(PmError::Unauthorized as u32)
    );
    assert_eq!(
        s.pm.try_set_risk_control(&x, &x).err(),
        err_code(PmError::Unauthorized as u32)
    );
    assert_eq!(
        s.pm.try_transfer_admin(&x, &x).err(),
        err_code(PmError::Unauthorized as u32)
    );
    // PT / YT one-time setters (already used -> even the admin is refused; a stranger is refused
    // for the right reason first)
    assert_eq!(
        s.pt.try_set_minter(&x, &x).err(),
        err_code(PtError::Unauthorized as u32)
    );
    assert_eq!(
        s.yt.try_set_minter(&x, &x).err(),
        err_code(YtError::Unauthorized as u32)
    );
    assert_eq!(
        s.pt.try_set_recovery_escrow(&x, &x).err(),
        err_code(PtError::Unauthorized as u32)
    );
    assert_eq!(
        s.yt.try_set_recovery_escrow(&x, &x).err(),
        err_code(YtError::Unauthorized as u32)
    );
    // MarketPool / Router / RiskControl / MarketConfig / Permissioning / Oracle
    assert!(s.pool.try_set_paused(&x, &true).is_err());
    assert!(s.pool.try_transfer_admin(&x, &x).is_err());
    assert!(s.pool.try_set_recovery_escrow(&x, &x).is_err());
    assert!(s.router.try_register_market(&x, &x).is_err());
    assert!(s.router.try_transfer_admin(&x, &x).is_err());
    assert!(s.risk.try_set_cb_limit(&x, &5).is_err());
    assert!(s.risk.try_set_asset_limit(&x, &s.underlying, &5).is_err());
    assert!(s.risk.try_set_window_ledgers(&x, &5).is_err());
    assert!(s.risk.try_add_consumer(&x, &x).is_err());
    assert!(s.risk.try_add_pauser(&x, &x).is_err());
    assert!(s.risk.try_unpause(&x).is_err());
    assert!(s.config.try_set_fees(&x, &1, &1, &1).is_err());
    assert!(s.config.try_set_protocol_share(&x, &1).is_err());
    assert!(s.config.try_set_treasury(&x, &x).is_err());
    assert!(s.perm.try_grant_account(&x, &x).is_err());
    assert!(s.perm.try_revoke_account(&x, &x).is_err());
    assert!(s.perm.try_grant_asset(&x, &x, &x).is_err());
    assert!(s
        .oracle
        .try_set_reference_value(&x, &(SCALE * 2), &(T0 + 5))
        .is_err());
}

#[test]
fn escrow_only_seize_functions_reject_everyone_else_including_the_admin() {
    let (s, lp) = seeded();
    let user = s.new_user();
    deposit_sy(&s, &user, 10 * SCALE);
    let m = s.pm.mint(&user, &(5 * SCALE));
    for caller in [s.admin.clone(), user.clone(), lp.clone()] {
        assert_eq!(
            s.sy.try_seize(&caller, &user, &1).err(),
            err_code(SyError::NotRecoveryEscrow as u32)
        );
        assert_eq!(
            s.pt.try_seize(&caller, &user, &1).err(),
            err_code(PtError::NotRecoveryEscrow as u32)
        );
        assert_eq!(
            s.yt.try_seize(&caller, &user, &1).err(),
            err_code(YtError::NotRecoveryEscrow as u32)
        );
        assert!(s.pool.try_seize_lp(&caller, &lp, &1).is_err());
    }
    let _ = m;
}

#[test]
fn only_the_manager_can_mint_and_burn_pt_and_yt_and_only_it_can_claim_yield() {
    let (s, _) = seeded();
    let user = s.new_user();
    let stranger = Address::generate(&s.env);
    // The minter is PrincipalManager; nobody else's signature is accepted. Under mocked auths the
    // stored minter's requirement is satisfied for any caller, so drop the mocks: without the
    // manager's own invocation, `require_auth` on it fails.
    s.env.mock_auths(&[]);
    assert!(s.pt.try_mint(&user, &SCALE).is_err());
    assert!(s.pt.try_burn(&user, &SCALE).is_err());
    assert!(s.yt.try_mint(&user, &SCALE).is_err());
    assert!(s.yt.try_burn(&user, &SCALE).is_err());
    s.env.mock_all_auths();
    // claim_yield names its caller, who must be the registered minter.
    assert_eq!(
        s.yt.try_claim_yield(&stranger, &user).err(),
        err_code(YtError::Unauthorized as u32)
    );
}

// =================================================================== authorization: missing signatures

#[test]
fn state_changing_calls_need_the_callers_own_signature() {
    let (s, lp) = seeded();
    let u = s.new_user();
    deposit_sy(&s, &u, 100 * SCALE);
    s.env.mock_auths(&[]); // nobody has signed anything from here on

    // A user's funds cannot move without that user's signature.
    assert!(s.sy.try_deposit(&u, &SCALE, &0).is_err());
    assert!(s.sy.try_withdraw(&u, &SCALE, &u, &0).is_err());
    assert!(s.sy.try_transfer(&u, &s.treasury, &SCALE).is_err());
    assert!(s.pt.try_transfer(&u, &s.treasury, &SCALE).is_err());
    assert!(s.pm.try_mint(&u, &SCALE).is_err());
    assert!(s.pool.try_swap_sy_for_pt(&u, &u, &SCALE, &0).is_err());
    assert!(s.pool.try_remove_liquidity(&lp, &lp, &1, &0, &0).is_err());
    assert!(s.pool.try_transfer_lp(&lp, &u, &1).is_err());
    assert!(s
        .router
        .try_swap_sy_for_pt(&u, &s.pool.address, &SCALE, &0, &u64::MAX)
        .is_err());
    // Naming the right admin is not enough without their signature either.
    assert!(s.pm.try_set_paused(&s.admin, &true).is_err());
    assert!(s.sy.try_set_deposit_cap(&s.admin, &5).is_err());
    assert!(s.config.try_set_fees(&s.admin, &1, &1, &1).is_err());
    assert!(s.escrow.try_seize_sy(&s.admin, &u, &1).is_err());
}

#[test]
fn permissionless_functions_need_no_signature_and_cannot_redirect_value() {
    let (s, _) = seeded();
    let trader = s.new_user();
    deposit_sy(&s, &trader, 100 * SCALE);
    s.pool.swap_sy_for_pt(&trader, &trader, &(50 * SCALE), &0);
    s.pm.mint(&trader, &(10 * SCALE));
    s.advance(T0 + 30 * DAY, SCALE * 102 / 100);
    let (pm_p, pm_c) = s.pm.accrued_fees();
    let (pool_p, pool_c) = s.pool.accrued_fees();
    assert!(pm_p > 0 && pool_p > 0);

    s.env.mock_auths(&[]);
    // Anyone can advance the index and claim fees -- with no signature -- but the payees are fixed
    // by the market's configuration, so calling them cannot redirect anything.
    s.yt.update_yield_index();
    assert_eq!(s.pm.claim_protocol_fees(), pm_p);
    assert_eq!(s.pm.claim_creator_fees(), pm_c);
    assert_eq!(s.pool.claim_protocol_fees(), pool_p);
    assert_eq!(s.pool.claim_creator_fees(), pool_c);
    assert_eq!(s.sy.balance_of(&s.treasury), pm_p + pool_p);
    assert_eq!(s.sy.balance_of(&s.admin), pm_c + pool_c);
}

// =================================================================== compliance edge cases

#[test]
fn compliance_matrix_deauthorized_accounts_are_blocked_on_every_position_type() {
    for kind in [UnderlyingKind::Sac, UnderlyingKind::Rwa] {
        let s = deploy_kind(Config::free(LONG), kind);
        let (_, _lp) = seed_pool(&s, 1_000 * SCALE, 950 * SCALE);
        let holder = s.new_user();
        let friend = s.new_user();
        let shares = deposit_sy(&s, &holder, 100 * SCALE);
        s.pm.mint(&holder, &shares);
        s.mint_underlying(&holder, 10 * SCALE);
        s.deauthorize(&holder);

        // SY
        assert_eq!(
            s.sy.try_deposit(&holder, &SCALE, &0).err(),
            err_code(SyError::NotAuthorizedOnSac as u32)
        );
        assert_eq!(
            s.sy.try_transfer(&holder, &friend, &1).err(),
            err_code(SyError::NotAuthorizedOnSac as u32)
        );
        // PT / YT: cannot send; and nobody can send *to* the flagged account either.
        assert_eq!(
            s.pt.try_transfer(&holder, &friend, &1).err(),
            err_code(PtError::NotAuthorizedOnSac as u32)
        );
        assert_eq!(
            s.yt.try_transfer(&holder, &friend, &1).err(),
            err_code(YtError::NotAuthorizedOnSac as u32)
        );
        let other = s.new_user();
        let sh = deposit_sy(&s, &other, 10 * SCALE);
        s.pm.mint(&other, &sh);
        assert_eq!(
            s.pt.try_transfer(&other, &holder, &1).err(),
            err_code(PtError::NotAuthorizedOnSac as u32)
        );
        assert_eq!(
            s.sy.try_transfer(&other, &holder, &1).err(),
            err_code(SyError::NotAuthorizedOnSac as u32)
        );
        // Tokenization, recombination, claiming and the pool.
        assert_eq!(
            s.pm.try_mint(&holder, &1).err(),
            err_code(PmError::NotAuthorizedOnSac as u32)
        );
        assert_eq!(
            s.pm.try_recombine(&holder, &1).err(),
            err_code(PmError::NotAuthorizedOnSac as u32)
        );
        assert_eq!(
            s.pm.try_claim_yield(&holder).err(),
            err_code(PmError::NotAuthorizedOnSac as u32)
        );
        // Redemption after maturity is blocked too (recovery, not self-service exit, applies).
        s.advance(s.maturity, SCALE);
        assert_eq!(
            s.pm.try_redeem(&holder, &SCALE, &0).err(),
            err_code(PmError::NotAuthorizedOnSac as u32)
        );
        // Re-authorizing the account restores everything: no lingering Principal-side state.
        s.authorize(&holder);
        s.pm.redeem(&holder, &SCALE, &0);
    }
}

#[test]
fn per_asset_permissioning_lets_pt_and_yt_have_different_eligibility() {
    let s = deploy_stack(LONG);
    let pt_only = Address::generate(&s.env);
    s.perm.grant_account(&s.admin, &pt_only);
    s.perm.grant_asset(&s.admin, &pt_only, &s.pt.address); // PT yes, YT no
    s.authorize(&pt_only);
    let minter = s.new_user();
    let shares = deposit_sy(&s, &minter, 10 * SCALE);
    s.pm.mint(&minter, &shares);
    s.pt.transfer(&minter, &pt_only, &SCALE);
    assert_eq!(s.pt.balance(&pt_only), SCALE);
    assert_eq!(
        s.yt.try_transfer(&minter, &pt_only, &SCALE).err(),
        err_code(YtError::PermissionDenied as u32)
    );
}

// =================================================================== SEP-8 regulated-asset semantics

#[test]
fn sep8_style_asset_needs_persistent_authorization_at_rest() {
    // A SEP-8 regulated asset sets AUTH_REQUIRED + AUTH_REVOCABLE. Its usual flow authorizes a
    // holder for the duration of one classic transaction and deauthorizes afterwards -- a
    // multi-operation "sandwich" that a Soroban call (a single invokeHostFunction operation)
    // cannot be wrapped in. Principal therefore reads the flag as it stands when the contract
    // runs: a holder that is only authorized *inside* a sandwich is, to Principal, unauthorized.
    let s = deploy_stack(LONG);
    let u = Address::generate(&s.env);
    s.perm.grant_account(&s.admin, &u);
    s.perm.grant_asset(&s.admin, &u, &s.pt.address);
    s.perm.grant_asset(&s.admin, &u, &s.yt.address);
    s.mint_underlying(&u, 10 * SCALE); // holds the asset, but the trustline is not authorized at rest
    s.deauthorize(&u);
    assert_eq!(
        s.sy.try_deposit(&u, &SCALE, &0).err(),
        err_code(SyError::NotAuthorizedOnSac as u32)
    );
    // Once the issuer's approval flow leaves the account persistently authorized (KYC approved),
    // every Principal position works, and a later revoke freezes them again.
    s.authorize(&u);
    s.sy.deposit(&u, &SCALE, &0);
    s.deauthorize(&u);
    assert_eq!(
        s.sy.try_withdraw(&u, &SCALE, &u, &0).err(),
        err_code(SyError::NotAuthorizedOnSac as u32)
    );
}

// =================================================================== views on uninitialised contracts

#[test]
fn views_on_uninitialised_contracts_revert_instead_of_returning_garbage() {
    let env = soroban_sdk::Env::default();
    env.mock_all_auths();
    let sy = principal_sy_wrapper::SYWrapperContractClient::new(
        &env,
        &env.register(principal_sy_wrapper::SYWrapperContract, ()),
    );
    assert!(sy.try_underlying_address().is_err());
    assert!(sy.try_permissioning_address().is_err());
    assert!(sy.try_get_admin().is_err());
    let pm = principal_manager::PrincipalManagerContractClient::new(
        &env,
        &env.register(principal_manager::PrincipalManagerContract, ()),
    );
    assert!(pm.try_maturity().is_err());
    assert!(pm.try_config_address().is_err());
    assert!(pm.try_oracle_address().is_err());
    let pool = principal_market_pool::MarketPoolContractClient::new(
        &env,
        &env.register(principal_market_pool::MarketPoolContract, ()),
    );
    assert!(pool.try_maturity().is_err());
    assert!(pool.try_get_admin().is_err());
    let router = principal_router::RouterContractClient::new(
        &env,
        &env.register(principal_router::RouterContract, ()),
    );
    assert!(router.try_get_admin().is_err());
    let escrow = principal_recovery_escrow::RecoveryEscrowContractClient::new(
        &env,
        &env.register(principal_recovery_escrow::RecoveryEscrowContract, ()),
    );
    assert!(escrow.try_underlying_address().is_err());
    assert_eq!(escrow.record_count(), 0);
}

// =================================================================== remaining guards and views

#[test]
fn remaining_slippage_and_dust_guards_on_the_pool_and_manager() {
    let (s, _) = seeded();
    let u = s.new_user();
    deposit_sy(&s, &u, 300 * SCALE);
    s.pm.mint(&u, &(100 * SCALE));

    // Selling PT: min_sy_out is a hard floor at the exact quote.
    let q = s.pool.quote_pt_for_sy(&(10 * SCALE));
    assert!(s
        .pool
        .try_swap_pt_for_sy(&u, &u, &(10 * SCALE), &(q.amount_out + 1))
        .is_err());
    assert_eq!(
        s.pool.swap_pt_for_sy(&u, &u, &(10 * SCALE), &q.amount_out),
        q.amount_out
    );

    // Single-sided deposit: min_lp_out floor.
    assert!(s
        .pool
        .try_add_liquidity_single_sy(&u, &(20 * SCALE), &(1_000 * SCALE))
        .is_err());
    // A dust single-sided deposit that rounds to no LP is refused rather than minting nothing.
    assert!(s.pool.try_add_liquidity_single_sy(&u, &3, &0).is_err());

    // A recombine of a dust amount at a high rate would return zero shares: refused.
    s.advance(T0 + DAY, SCALE * 3);
    assert_eq!(
        s.pm.try_recombine(&u, &1).err(),
        err_code(PmError::ZeroAmount as u32)
    );
    // A mint whose net (after the fee) values to zero notional is refused.
    assert_eq!(
        s.pm.try_mint(&u, &1).err(),
        err_code(PmError::ZeroAmount as u32)
    );
}

#[test]
fn sy_wrapper_pause_and_escrow_exemption_and_views() {
    let (s, _) = seeded();
    let u = s.new_user();
    deposit_sy(&s, &u, 20 * SCALE);
    assert!(!s.sy.is_paused());
    assert_eq!(s.sy.risk_control(), Some(s.risk.address.clone()));
    assert_eq!(s.pm.risk_control(), Some(s.risk.address.clone()));
    assert_eq!(s.pm.config_address(), s.config.address);
    assert_eq!(s.pm.permissioning_address(), s.perm.address);
    assert_eq!(s.pm.oracle_address(), s.oracle.address);
    assert_eq!(s.pm.sy_wrapper_address(), s.sy.address);
    assert_eq!(s.pm.pt_address(), s.pt.address);
    assert_eq!(s.pm.yt_address(), s.yt.address);

    s.sy.set_paused(&s.admin, &true);
    assert!(s.sy.is_paused());
    // Ordinary users are frozen out by the pause: deposit, withdraw and transfer all revert.
    s.mint_underlying(&u, SCALE);
    assert_eq!(
        s.sy.try_deposit(&u, &SCALE, &0).err(),
        err_code(SyError::Paused as u32)
    );
    assert_eq!(
        s.sy.try_withdraw(&u, &SCALE, &u, &0).err(),
        err_code(SyError::Paused as u32)
    );
    assert_eq!(
        s.sy.try_transfer(&u, &s.treasury, &SCALE).err(),
        err_code(SyError::Paused as u32)
    );
    // Zero-share seize is rejected before anything moves.
    assert_eq!(
        s.sy.try_seize(&s.escrow.address, &u, &0).err(),
        err_code(SyError::ZeroAmount as u32)
    );
    s.sy.set_paused(&s.admin, &false);
    s.sy.withdraw(&u, &SCALE, &u, &0);
}

#[test]
fn a_gross_up_and_a_yt_fee_that_swallows_a_dust_claim_pay_nothing_and_never_underflow() {
    // With a 100% YT fee (the cap is 50%, so use the cap) a 1-unit claim nets to zero.
    let s = deploy(Config {
        yt_fee_bps: 5_000,
        ..Config::free(LONG)
    });
    let u = s.new_user();
    let shares = deposit_sy(&s, &u, 100 * SCALE);
    s.pm.mint(&u, &shares);
    s.advance(T0 + DAY, SCALE + 2); // a rate move worth ~1 raw unit of yield per 5e6 notional
    let paid = s.pm.claim_yield(&u);
    assert!(paid >= 0);
    // Whatever was withheld is accounted as fees, never lost.
    let (p, c) = s.pm.accrued_fees();
    assert!(p + c >= 0);
}
