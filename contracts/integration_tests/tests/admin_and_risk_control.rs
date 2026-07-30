//! Admin rotation across every admin-bearing contract in the stack, and RiskControl's
//! pause/circuit-breaker lifecycle exercised the way a registered consumer (e.g. a future
//! SYWrapper/PrincipalManager wiring) would actually call it.

mod common;

use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::Address;

use common::{deploy_stack, deposit_sy, T0};
use principal_manager::SCALE;
use principal_risk_control::CB_WINDOW_SECS;

#[test]
fn admin_rotation_across_every_contract_and_market_still_functions() {
    let s = deploy_stack(u64::MAX);
    let new_admin = Address::generate(&s.env);

    s.oracle.transfer_admin(&s.admin, &new_admin);
    assert_eq!(s.oracle.get_admin(), new_admin);

    s.perm.transfer_admin(&s.admin, &new_admin);
    assert_eq!(s.perm.get_admin(), new_admin);

    s.risk.transfer_admin(&s.admin, &new_admin);
    assert_eq!(s.risk.get_admin(), new_admin);

    s.sy.transfer_admin(&s.admin, &new_admin);
    assert_eq!(s.sy.get_admin(), new_admin);

    // PTToken/YTToken have no transfer_admin -- their `admin` only ever gates the one-time
    // market-creation check and set_minter/set_recovery_escrow, never rotated afterward.
    assert_eq!(s.pt.get_admin(), s.admin);
    assert_eq!(s.yt.get_admin(), s.admin);

    s.pm.transfer_admin(&s.admin, &new_admin);
    assert_eq!(s.pm.get_admin(), new_admin);

    // Protocol still works end-to-end after every admin key has moved -- exercise the new
    // admin's own admin-gated calls (grant, pause/unpause, oracle update) and a real mint.
    let user = Address::generate(&s.env);
    s.perm.grant_account(&new_admin, &user);
    s.perm.grant_asset(&new_admin, &user, &s.pt.address);
    s.perm.grant_asset(&new_admin, &user, &s.yt.address);
    soroban_sdk::token::StellarAssetClient::new(&s.env, &s.underlying)
        .set_authorized(&user, &true);

    s.pm.set_paused(&new_admin, &true);
    s.pm.set_paused(&new_admin, &false);

    let shares = deposit_sy(&s, &user, 10 * SCALE);
    let result = s.pm.mint(&user, &shares);
    assert_eq!(result.pt_minted, 10 * SCALE);
}

#[test]
#[should_panic]
fn old_admin_cannot_act_after_rotation() {
    let s = deploy_stack(u64::MAX);
    let new_admin = Address::generate(&s.env);
    s.oracle.transfer_admin(&s.admin, &new_admin);

    // The old admin address no longer has authority.
    s.oracle
        .set_reference_value(&s.admin, &(SCALE * 11 / 10), &(T0 + 1));
}

#[test]
fn pauser_lifecycle_and_admin_only_unpause() {
    let s = deploy_stack(u64::MAX);
    let pauser = Address::generate(&s.env);
    s.risk.add_pauser(&s.admin, &pauser);

    s.risk.pause(&pauser);
    assert!(s.risk.is_paused());

    s.risk.unpause(&s.admin);
    assert!(!s.risk.is_paused());

    s.risk.remove_pauser(&s.admin, &pauser);
}

#[test]
#[should_panic]
fn removed_pauser_cannot_pause_again() {
    let s = deploy_stack(u64::MAX);
    let pauser = Address::generate(&s.env);
    s.risk.add_pauser(&s.admin, &pauser);
    s.risk.remove_pauser(&s.admin, &pauser);
    s.risk.pause(&pauser);
}

#[test]
fn registered_consumer_check_deposit_trips_and_resets_circuit_breaker() {
    let s = deploy_stack(u64::MAX);
    // Simulate the future wiring: SYWrapper's own address registered as a consumer.
    s.risk.add_consumer(&s.admin, &s.sy.address);
    assert!(s.risk.is_consumer(&s.sy.address));

    s.risk.set_cb_limit(&s.admin, &(100 * SCALE));
    assert_eq!(s.risk.get_cb_limit(), 100 * SCALE);

    s.risk.check_deposit(&s.sy.address, &(60 * SCALE));
    assert_eq!(s.risk.get_cb_volume(), 60 * SCALE);

    s.risk.check_deposit(&s.sy.address, &(40 * SCALE));
    assert_eq!(s.risk.get_cb_volume(), 100 * SCALE);

    // Window resets after CB_WINDOW_SECS -- volume drops back to just this deposit.
    s.env
        .ledger()
        .with_mut(|li| li.timestamp = T0 + CB_WINDOW_SECS + 1);
    s.risk.check_deposit(&s.sy.address, &(10 * SCALE));
    assert_eq!(s.risk.get_cb_volume(), 10 * SCALE);
}

#[test]
#[should_panic]
fn circuit_breaker_blocks_deposit_over_limit() {
    let s = deploy_stack(u64::MAX);
    s.risk.add_consumer(&s.admin, &s.sy.address);
    s.risk.set_cb_limit(&s.admin, &(100 * SCALE));
    s.risk.check_deposit(&s.sy.address, &(150 * SCALE));
}

#[test]
#[should_panic]
fn unregistered_consumer_cannot_check_deposit() {
    let s = deploy_stack(u64::MAX);
    s.risk.check_deposit(&s.sy.address, &(1 * SCALE));
}
