//! Full compliance-recovery flow across the real stack: seize a deauthorized holder's SY/PT/YT
//! positions via RecoveryEscrow, then unwind PT/YT into real underlying at/after maturity.

mod common;

use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    token, Address,
};

use common::{deploy_stack, deposit_sy, grant_user, T0};
use principal_manager::SCALE;

#[test]
fn seize_sy_unwraps_immediately_for_a_flagged_depositor() {
    let s = deploy_stack(u64::MAX);
    let user = Address::generate(&s.env);
    grant_user(&s, &user);
    deposit_sy(&s, &user, 50 * SCALE);

    // Issuer flags the account on the real underlying SAC.
    token::StellarAssetClient::new(&s.env, &s.underlying).set_authorized(&user, &false);

    let seized = s.escrow.seize_sy(&s.admin, &user, &(50 * SCALE));
    assert_eq!(seized, 50 * SCALE);
    assert_eq!(s.sy.balance_of(&user), 0);
    // seize_sy unwraps in the same call -- the escrow ends up holding raw underlying, ready
    // for the issuer's native SAC clawback, not a lingering SY position.
    assert_eq!(s.sy.balance_of(&s.escrow.address), 0);
    assert_eq!(
        token::Client::new(&s.env, &s.underlying).balance(&s.escrow.address),
        50 * SCALE
    );
}

#[test]
fn seize_and_finalize_pt_and_yt_after_maturity() {
    let maturity = T0 + 500;
    let s = deploy_stack(maturity);
    let user = Address::generate(&s.env);
    grant_user(&s, &user);
    let shares = deposit_sy(&s, &user, 100 * SCALE);
    let result = s.pm.mint(&user, &shares);

    // Issuer flags the account, then seizes its PT and YT into the escrow (still at genesis
    // rate -- no yield has accrued yet, so seizing doesn't need to reconcile any).
    token::StellarAssetClient::new(&s.env, &s.underlying).set_authorized(&user, &false);
    let seized_pt = s.escrow.seize_pt(&s.admin, &user, &result.pt_minted);
    let seized_yt = s.escrow.seize_yt(&s.admin, &user, &result.yt_minted);
    assert_eq!(seized_pt, result.pt_minted);
    assert_eq!(seized_yt, result.yt_minted);
    assert_eq!(s.pt.balance(&user), 0);
    assert_eq!(s.yt.balance(&user), 0);
    assert_eq!(s.pt.balance(&s.escrow.address), result.pt_minted);
    assert_eq!(s.yt.balance(&s.escrow.address), result.yt_minted);

    // At/after maturity, the issuer finalizes both -- redeeming the escrow's own seized balance
    // through PrincipalManager and releasing real underlying to the escrow. Rate exactly doubles
    // (SCALE -> 2*SCALE) so PT's and YT's independently-floored share requirements divide the
    // 100*SCALE of custodied SY shares evenly, with no dust-rounding shortfall between the two
    // separate `SYWrapper.withdraw` calls.
    s.env.ledger().with_mut(|li| li.timestamp = maturity + 1);
    s.oracle
        .set_reference_value(&s.admin, &(SCALE * 2), &(maturity + 1));

    let pt_underlying = s.escrow.finalize_pt(&s.admin, &result.pt_minted);
    let yt_underlying = s.escrow.finalize_yt(&s.admin, &result.yt_minted);
    assert!(pt_underlying > 0);
    assert!(yt_underlying > 0);
    assert_eq!(s.pt.balance(&s.escrow.address), 0);
    assert_eq!(s.yt.balance(&s.escrow.address), 0);
    assert_eq!(
        token::Client::new(&s.env, &s.underlying).balance(&s.escrow.address),
        pt_underlying + yt_underlying
    );
    assert_eq!(s.escrow.underlying_address(), s.underlying);
}

#[test]
#[should_panic]
fn seize_requires_target_already_deauthorized() {
    let s = deploy_stack(u64::MAX);
    let user = Address::generate(&s.env);
    grant_user(&s, &user);
    deposit_sy(&s, &user, 10 * SCALE);

    // Never deauthorized on the underlying SAC -- must be rejected.
    s.escrow.seize_sy(&s.admin, &user, &(10 * SCALE));
}

#[test]
#[should_panic]
fn seize_requires_real_issuer_admin() {
    let s = deploy_stack(u64::MAX);
    let user = Address::generate(&s.env);
    grant_user(&s, &user);
    deposit_sy(&s, &user, 10 * SCALE);
    token::StellarAssetClient::new(&s.env, &s.underlying).set_authorized(&user, &false);

    let impostor = Address::generate(&s.env);
    s.escrow.seize_sy(&impostor, &user, &(10 * SCALE));
}
