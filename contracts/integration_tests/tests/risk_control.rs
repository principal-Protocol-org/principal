//! Deliverable 2, RiskControl: the circuit breaker is wired directly into `SYWrapper.deposit` and
//! `PrincipalManager.mint`, so an over-limit deposit or mint reverts by itself -- no separate call.
//! The window is measured in ledger sequence numbers and a per-asset limit sits beside the
//! protocol-wide one. Also admin rotation across the whole stack.

use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    Address,
};

use principal_integration_tests::stack::*;
use principal_risk_control::{Error as RcError, DEFAULT_WINDOW_LEDGERS};

fn advance_ledgers(s: &Stack, n: u32) {
    s.env.ledger().with_mut(|li| li.sequence_number += n);
}

fn deposit_err(
    s: &Stack,
    user: &Address,
    amount: i128,
) -> Option<Result<soroban_sdk::Error, soroban_sdk::InvokeError>> {
    s.sy.try_deposit(user, &amount, &0).err()
}

#[test]
fn an_over_limit_deposit_reverts_automatically_with_no_manual_call() {
    let s = deploy_stack(LONG);
    s.risk.set_cb_limit(&s.admin, &(100 * SCALE));
    let user = s.new_user();
    s.sac().mint(&user, &(500 * SCALE));

    s.sy.deposit(&user, &(60 * SCALE), &0);
    assert_eq!(s.risk.get_cb_volume(), 60 * SCALE);
    // 60 + 50 > 100: the *deposit itself* trips the breaker.
    assert_eq!(
        deposit_err(&s, &user, 50 * SCALE),
        err_code(RcError::CircuitBreakerTripped as u32)
    );
    // Atomic: no shares minted, no underlying moved, volume unchanged.
    assert_eq!(s.sy.balance_of(&user), 60 * SCALE);
    assert_eq!(underlying_balance(&s, &user), 440 * SCALE);
    assert_eq!(s.risk.get_cb_volume(), 60 * SCALE);

    // Exactly filling the limit is fine; one more unit is not.
    s.sy.deposit(&user, &(40 * SCALE), &0);
    assert_eq!(s.risk.get_cb_volume(), 100 * SCALE);
    assert_eq!(
        deposit_err(&s, &user, 1),
        err_code(RcError::CircuitBreakerTripped as u32)
    );
}

#[test]
fn an_over_limit_mint_reverts_automatically() {
    let s = deploy_stack(LONG);
    let user = s.new_user();
    let shares = deposit_sy(&s, &user, 300 * SCALE);
    // Arm the breaker *after* the deposit so only the mint is counted.
    s.risk.set_cb_limit(&s.admin, &(100 * SCALE));

    s.pm.mint(&user, &(60 * SCALE));
    assert_eq!(s.risk.get_cb_volume(), 60 * SCALE);
    assert_eq!(
        s.pm.try_mint(&user, &(50 * SCALE)).err(),
        err_code(RcError::CircuitBreakerTripped as u32)
    );
    // Nothing was minted or moved by the rejected mint.
    assert_eq!(s.pm.pt_balance(&user), 60 * SCALE);
    assert_eq!(s.sy.balance_of(&user), shares - 60 * SCALE);
    s.pm.mint(&user, &(40 * SCALE));
    assert!(s.pm.try_mint(&user, &1).is_err());
}

#[test]
fn a_wrap_and_mint_through_the_router_is_counted_at_both_entry_points() {
    // Documented behaviour: SYWrapper.deposit and PrincipalManager.mint each report their volume,
    // so tokenizing fresh underlying counts twice against the window.
    let s = deploy_stack(LONG);
    s.risk.set_cb_limit(&s.admin, &(1_000 * SCALE));
    let user = s.new_user();
    s.sac().mint(&user, &(100 * SCALE));
    s.router
        .wrap_and_mint(&user, &s.pool.address, &(100 * SCALE), &0, &u64::MAX);
    assert_eq!(s.risk.get_cb_volume(), 200 * SCALE);
}

#[test]
fn window_is_ledger_sequence_based_and_ignores_wall_clock() {
    let s = deploy_stack(LONG);
    s.risk.set_cb_limit(&s.admin, &(100 * SCALE));
    let user = s.new_user();
    s.sac().mint(&user, &(500 * SCALE));
    s.sy.deposit(&user, &(100 * SCALE), &0);

    // A day of *wall-clock* time with no new ledgers does not reopen the window.
    s.advance_flat(T0 + DAY);
    assert!(deposit_err(&s, &user, SCALE).is_some());

    // One ledger short of the window: still closed. At exactly the window length: open again.
    advance_ledgers(&s, DEFAULT_WINDOW_LEDGERS - 1);
    assert!(deposit_err(&s, &user, SCALE).is_some());
    advance_ledgers(&s, 1);
    s.sy.deposit(&user, &(100 * SCALE), &0);
    assert_eq!(s.risk.get_cb_volume(), 100 * SCALE); // restarted, not 200
}

#[test]
fn per_asset_limit_is_enforced_on_chain_alongside_the_protocol_limit() {
    let s = deploy_stack(LONG);
    s.risk.set_cb_limit(&s.admin, &(1_000 * SCALE));
    s.risk
        .set_asset_limit(&s.admin, &s.underlying, &(50 * SCALE));
    let user = s.new_user();
    s.sac().mint(&user, &(500 * SCALE));

    s.sy.deposit(&user, &(50 * SCALE), &0);
    assert_eq!(s.risk.get_asset_volume(&s.underlying), 50 * SCALE);
    // The protocol-wide budget has plenty of room, but this asset is capped.
    assert_eq!(
        deposit_err(&s, &user, 1),
        err_code(RcError::AssetLimitTripped as u32)
    );
    // Raising the asset limit reopens it; the protocol limit still binds independently.
    s.risk
        .set_asset_limit(&s.admin, &s.underlying, &(2_000 * SCALE));
    s.sy.deposit(&user, &(100 * SCALE), &0);
    s.risk.set_cb_limit(&s.admin, &(160 * SCALE));
    assert_eq!(
        deposit_err(&s, &user, 20 * SCALE),
        err_code(RcError::CircuitBreakerTripped as u32)
    );
}

#[test]
fn global_pause_blocks_deposits_and_mints_until_the_admin_unpauses() {
    let s = deploy_stack(LONG);
    let pauser = Address::generate(&s.env);
    s.risk.add_pauser(&s.admin, &pauser);
    let user = s.new_user();
    let shares = deposit_sy(&s, &user, 100 * SCALE);

    s.risk.pause(&pauser);
    assert_eq!(
        deposit_err(&s, &user, SCALE),
        err_code(RcError::Paused as u32)
    );
    assert_eq!(
        s.pm.try_mint(&user, &shares).err(),
        err_code(RcError::Paused as u32)
    );
    // A pauser cannot undo it; the admin can.
    assert!(s.risk.try_unpause(&pauser).is_err());
    s.risk.unpause(&s.admin);
    assert_eq!(s.pm.mint(&user, &shares).pt_minted, 100 * SCALE);
}

#[test]
fn deregistering_a_consumer_fails_closed() {
    let s = deploy_stack(LONG);
    s.risk.set_cb_limit(&s.admin, &(1_000 * SCALE));
    let user = s.new_user();
    s.sac().mint(&user, &(100 * SCALE));

    s.risk.remove_consumer(&s.admin, &s.sy.address);
    // SYWrapper is no longer a registered consumer: deposits revert rather than skipping the check.
    assert_eq!(
        deposit_err(&s, &user, SCALE),
        err_code(RcError::NotConsumer as u32)
    );
    s.risk.add_consumer(&s.admin, &s.sy.address);
    s.sy.deposit(&user, &SCALE, &0);
}

#[test]
fn an_unregistered_caller_cannot_burn_the_budget_directly() {
    let s = deploy_stack(LONG);
    s.risk.set_cb_limit(&s.admin, &(100 * SCALE));
    let griefer = Address::generate(&s.env);
    assert_eq!(
        s.risk
            .try_check_deposit(&griefer, &s.underlying, &(100 * SCALE))
            .err(),
        err_code(RcError::NotConsumer as u32)
    );
    assert_eq!(s.risk.get_cb_volume(), 0);
}

// ------------------------------------------------------------- admin rotation

#[test]
fn admin_rotation_across_every_contract_and_the_market_still_functions() {
    let s = deploy_stack(LONG);
    let new_admin = Address::generate(&s.env);

    s.oracle.transfer_admin(&s.admin, &new_admin);
    s.perm.transfer_admin(&s.admin, &new_admin);
    s.risk.transfer_admin(&s.admin, &new_admin);
    s.sy.transfer_admin(&s.admin, &new_admin);
    s.pm.transfer_admin(&s.admin, &new_admin);
    s.pool.transfer_admin(&s.admin, &new_admin);
    s.router.transfer_admin(&s.admin, &new_admin);
    assert_eq!(s.oracle.get_admin(), new_admin);
    assert_eq!(s.perm.get_admin(), new_admin);
    assert_eq!(s.risk.get_admin(), new_admin);
    assert_eq!(s.sy.get_admin(), new_admin);
    assert_eq!(s.pm.get_admin(), new_admin);
    assert_eq!(s.pool.get_admin(), new_admin);
    assert_eq!(s.router.get_admin(), new_admin);
    // PT/YT have no transfer_admin: their admin only gates one-time setup.
    assert_eq!(s.pt.get_admin(), s.admin);

    let user = Address::generate(&s.env);
    s.perm.grant_account(&new_admin, &user);
    s.perm.grant_asset(&new_admin, &user, &s.pt.address);
    s.perm.grant_asset(&new_admin, &user, &s.yt.address);
    s.sac().set_authorized(&user, &true);
    s.pm.set_paused(&new_admin, &true);
    s.pm.set_paused(&new_admin, &false);
    s.pool.set_paused(&new_admin, &true);
    s.pool.set_paused(&new_admin, &false);
    let shares = deposit_sy(&s, &user, 10 * SCALE);
    assert_eq!(s.pm.mint(&user, &shares).pt_minted, 10 * SCALE);

    // The old admin lost every power.
    assert!(s
        .oracle
        .try_set_reference_value(&s.admin, &(SCALE * 2), &(T0 + 5))
        .is_err());
    assert!(s.pm.try_set_paused(&s.admin, &true).is_err());
    assert!(s.pool.try_set_paused(&s.admin, &true).is_err());
    assert!(s.risk.try_set_cb_limit(&s.admin, &5).is_err());
}

#[test]
fn pauser_lifecycle_and_admin_only_unpause() {
    let s = deploy_stack(LONG);
    let pauser = Address::generate(&s.env);
    s.risk.add_pauser(&s.admin, &pauser);
    s.risk.pause(&pauser);
    assert!(s.risk.is_paused());
    s.risk.unpause(&s.admin);
    assert!(!s.risk.is_paused());
    s.risk.remove_pauser(&s.admin, &pauser);
    assert!(s.risk.try_pause(&pauser).is_err());
}
