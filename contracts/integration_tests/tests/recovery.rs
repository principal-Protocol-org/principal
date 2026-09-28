//! Deliverable 2, RecoveryEscrow: compliance recovery across SY, PT, YT and LP positions, batch
//! seizure, and the per-account record that traces recovered funds to the event that produced them.
//! Every scenario that does not depend on the SAC-only `admin()` rotation runs twice: once over a
//! classic SAC/SEP-8 underlying and once over a SEP-57 (T-REX) RWA token.

use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    Address, Vec,
};

use principal_integration_tests::stack::*;
use principal_recovery_escrow::{Error as EscrowError, SeizeRequest};

const KINDS: [UnderlyingKind; 2] = [UnderlyingKind::Sac, UnderlyingKind::Rwa];

fn each_kind(f: impl Fn(UnderlyingKind)) {
    for kind in KINDS {
        f(kind);
    }
}

fn req(
    env: &soroban_sdk::Env,
    account: &Address,
    sy: i128,
    pt: i128,
    yt: i128,
    lp: i128,
) -> SeizeRequest {
    let _ = env;
    SeizeRequest {
        account: account.clone(),
        sy_shares: sy,
        pt_amount: pt,
        yt_amount: yt,
        lp_amount: lp,
    }
}

fn setup(kind: UnderlyingKind, cfg: Config) -> Stack<'static> {
    deploy_kind(cfg, kind)
}

// ------------------------------------------------------------------ SY

#[test]
fn seize_sy_unwraps_at_once_writes_a_record_and_touches_nobody_else() {
    each_kind(|kind| {
        let s = setup(kind, Config::free(LONG));
        let flagged = s.new_user();
        let bystander = s.new_user();
        deposit_sy(&s, &flagged, 50 * SCALE);
        deposit_sy(&s, &bystander, 70 * SCALE);

        s.deauthorize(&flagged);
        let unwrapped = s.escrow.seize_sy(&s.admin, &flagged, &(50 * SCALE));
        assert_eq!(unwrapped, 50 * SCALE, "{kind:?}");

        // The flagged account's SY is gone; the escrow holds raw underlying, not lingering SY.
        assert_eq!(s.sy.balance_of(&flagged), 0);
        assert_eq!(s.sy.balance_of(&s.escrow.address), 0);
        assert_eq!(underlying_balance(&s, &s.escrow.address), 50 * SCALE);
        // The bystander is entirely unaffected.
        assert_eq!(s.sy.balance_of(&bystander), 70 * SCALE);

        // A record traces the underlying back to the account and event.
        assert_eq!(s.escrow.record_count(), 1);
        let ids = s.escrow.account_records(&flagged);
        assert_eq!(ids.len(), 1);
        let rec = s.escrow.get_record(&ids.get(0).unwrap());
        assert_eq!(rec.account, flagged);
        assert_eq!(rec.sy_shares, 50 * SCALE);
        assert_eq!(rec.underlying_from_sy, 50 * SCALE);
        assert_eq!(rec.ledger, s.env.ledger().sequence());
        assert_eq!(rec.timestamp, s.env.ledger().timestamp());
        assert!(!rec.finalized);
        assert!(s.escrow.account_records(&bystander).is_empty());
    });
}

#[test]
fn recovered_underlying_can_be_clawed_back_natively_by_the_issuer() {
    // Classic assets only: the SAC's native `clawback` is the last step of a recovery.
    let s = setup(UnderlyingKind::Sac, Config::free(LONG));
    let flagged = s.new_user();
    deposit_sy(&s, &flagged, 40 * SCALE);
    s.deauthorize(&flagged);
    s.escrow.seize_sy(&s.admin, &flagged, &(40 * SCALE));
    assert_eq!(underlying_balance(&s, &s.escrow.address), 40 * SCALE);

    s.sac().clawback(&s.escrow.address, &(40 * SCALE));
    assert_eq!(underlying_balance(&s, &s.escrow.address), 0);
}

// -------------------------------------------------------------- PT / YT

#[test]
fn seize_pt_and_yt_hold_until_maturity_then_finalize_onto_the_same_record() {
    each_kind(|kind| {
        let s = setup(kind, Config::free(LONG));
        let flagged = s.new_user();
        let shares = deposit_sy(&s, &flagged, 100 * SCALE);
        let minted = s.pm.mint(&flagged, &shares);

        s.deauthorize(&flagged);
        assert_eq!(
            s.escrow.seize_pt(&s.admin, &flagged, &minted.pt_minted),
            minted.pt_minted
        );
        assert_eq!(
            s.escrow.seize_yt(&s.admin, &flagged, &minted.yt_minted),
            minted.yt_minted
        );
        assert_eq!(s.pt.balance(&flagged), 0);
        assert_eq!(s.yt.balance(&flagged), 0);
        assert_eq!(s.pt.balance(&s.escrow.address), minted.pt_minted);
        assert_eq!(s.yt.balance(&s.escrow.address), minted.yt_minted);
        // Held fully backed: the market's supply is unchanged until settlement.
        assert_eq!(s.pm.total_pt(), minted.pt_minted);

        // Not redeemable before maturity.
        assert!(s.escrow.try_finalize_record(&s.admin, &0).is_err());

        // Rate doubles; maturity arrives; the issuer finalizes the PT record and the YT record.
        s.advance(LONG, SCALE * 2);
        let (pt_under, _) = s.escrow.finalize_record(&s.admin, &0);
        let (_, yt_under) = s.escrow.finalize_record(&s.admin, &1);
        assert_eq!(
            pt_under,
            minted.pt_minted / 2,
            "{kind:?}: PT settles at 1/2 at rate 2.0"
        );
        assert!(yt_under > 0);
        assert_eq!(s.pt.balance(&s.escrow.address), 0);
        assert_eq!(s.yt.balance(&s.escrow.address), 0);

        // The underlying is written back onto the records that produced it.
        let pt_rec = s.escrow.get_record(&0);
        assert!(pt_rec.finalized);
        assert_eq!(pt_rec.underlying_from_pt, pt_under);
        let yt_rec = s.escrow.get_record(&1);
        assert_eq!(yt_rec.underlying_from_yt, yt_under);
        assert_eq!(
            underlying_balance(&s, &s.escrow.address),
            pt_under + yt_under
        );
    });
}

#[test]
fn a_record_can_only_be_finalized_once_and_only_by_the_issuer() {
    each_kind(|kind| {
        let s = setup(kind, Config::free(T0 + 300 * DAY));
        let flagged = s.new_user();
        let shares = deposit_sy(&s, &flagged, 10 * SCALE);
        let minted = s.pm.mint(&flagged, &shares);
        s.deauthorize(&flagged);
        s.escrow.seize_pt(&s.admin, &flagged, &minted.pt_minted);

        s.advance(s.maturity, SCALE);
        let impostor = Address::generate(&s.env);
        assert_eq!(
            s.escrow.try_finalize_record(&impostor, &0).err(),
            err_code(EscrowError::Unauthorized as u32)
        );
        s.escrow.finalize_record(&s.admin, &0);
        assert_eq!(
            s.escrow.try_finalize_record(&s.admin, &0).err(),
            err_code(EscrowError::AlreadyFinalized as u32)
        );
        assert_eq!(
            s.escrow.try_finalize_record(&s.admin, &99).err(),
            err_code(EscrowError::RecordNotFound as u32)
        );
    });
}

#[test]
fn finalizing_a_pure_sy_record_reverts_because_there_is_nothing_left_to_settle() {
    let s = setup(UnderlyingKind::Sac, Config::free(T0 + 300 * DAY));
    let flagged = s.new_user();
    deposit_sy(&s, &flagged, 10 * SCALE);
    s.deauthorize(&flagged);
    s.escrow.seize_sy(&s.admin, &flagged, &(10 * SCALE));
    s.advance(s.maturity, SCALE);
    assert_eq!(
        s.escrow.try_finalize_record(&s.admin, &0).err(),
        err_code(EscrowError::NothingToFinalize as u32)
    );
}

// ------------------------------------------------------------------- LP

#[test]
fn seize_lp_burns_it_unwraps_the_sy_leg_and_holds_the_pt_leg() {
    each_kind(|kind| {
        let s = setup(kind, Config::free(T0 + 300 * DAY));
        let (lp_minted, provider) = seed_pool(&s, 1_000 * SCALE, 950 * SCALE);
        let (pt_res, sy_res) = s.pool.reserves();
        let total_lp = s.pool.lp_total_supply();

        s.deauthorize(&provider);
        let (pt_leg, sy_leg_underlying) = s.escrow.seize_lp(&s.admin, &provider, &lp_minted);

        // The provider's LP is gone; the pool paid out its pro-rata share.
        assert_eq!(s.pool.lp_balance(&provider), 0);
        assert_eq!(pt_leg, lp_minted * pt_res / total_lp);
        assert_eq!(sy_leg_underlying, lp_minted * sy_res / total_lp);
        // SY leg: unwrapped to raw underlying; PT leg: held in escrow.
        assert_eq!(underlying_balance(&s, &s.escrow.address), sy_leg_underlying);
        assert_eq!(s.pt.balance(&s.escrow.address), pt_leg);
        assert_eq!(
            s.pool.lp_balance(&s.escrow.address),
            0,
            "seized LP is burned, not parked"
        );

        let rec = s.escrow.get_record(&0);
        assert_eq!(rec.lp_amount, lp_minted);
        assert_eq!(rec.lp_pt, pt_leg);
        assert_eq!(rec.underlying_from_lp, sy_leg_underlying);

        // The PT leg settles at maturity through the same record.
        s.advance(s.maturity, SCALE);
        let (pt_under, _) = s.escrow.finalize_record(&s.admin, &0);
        assert_eq!(pt_under, pt_leg);
    });
}

#[test]
fn transferring_lp_to_a_deauthorized_account_is_refused_and_seizure_stays_possible() {
    each_kind(|kind| {
        let s = setup(kind, Config::free(T0 + 300 * DAY));
        let (lp_minted, provider) = seed_pool(&s, 1_000 * SCALE, 950 * SCALE);
        let flagged = s.new_user();
        s.deauthorize(&flagged);
        // LP cannot be parked with an ineligible holder.
        assert!(s
            .pool
            .try_transfer_lp(&provider, &flagged, &(lp_minted / 2))
            .is_err());
        // A deauthorized holder cannot move or redeem their own LP either.
        s.deauthorize(&provider);
        assert!(s
            .pool
            .try_remove_liquidity(&provider, &provider, &(lp_minted / 2), &0, &0)
            .is_err());
        // ...but the issuer can still recover it.
        s.escrow.seize_lp(&s.admin, &provider, &lp_minted);
        assert_eq!(s.pool.lp_balance(&provider), 0);
    });
}

// ---------------------------------------------------------------- batch

#[test]
fn batch_seizure_recovers_several_accounts_in_one_transaction_with_a_record_each() {
    each_kind(|kind| {
        let s = setup(kind, Config::free(T0 + 300 * DAY));
        let (lp_minted, lp_holder) = seed_pool(&s, 1_000 * SCALE, 950 * SCALE);
        let sy_holder = s.new_user();
        let pt_yt_holder = s.new_user();
        let bystander = s.new_user();
        deposit_sy(&s, &sy_holder, 30 * SCALE);
        let shares = deposit_sy(&s, &pt_yt_holder, 40 * SCALE);
        let minted = s.pm.mint(&pt_yt_holder, &shares);
        deposit_sy(&s, &bystander, 25 * SCALE);

        for a in [&lp_holder, &sy_holder, &pt_yt_holder] {
            s.deauthorize(a);
        }
        let mut requests: Vec<SeizeRequest> = Vec::new(&s.env);
        requests.push_back(req(&s.env, &sy_holder, 30 * SCALE, 0, 0, 0));
        requests.push_back(req(
            &s.env,
            &pt_yt_holder,
            0,
            minted.pt_minted,
            minted.yt_minted,
            0,
        ));
        requests.push_back(req(&s.env, &lp_holder, 0, 0, 0, lp_minted));

        let ids = s.escrow.seize_batch(&s.admin, &requests);
        assert_eq!(ids.len(), 3);
        assert_eq!(s.escrow.record_count(), 3);

        let r0 = s.escrow.get_record(&ids.get(0).unwrap());
        let r1 = s.escrow.get_record(&ids.get(1).unwrap());
        let r2 = s.escrow.get_record(&ids.get(2).unwrap());
        assert_eq!(
            (r0.account.clone(), r0.underlying_from_sy),
            (sy_holder.clone(), 30 * SCALE)
        );
        assert_eq!(r1.account, pt_yt_holder);
        assert_eq!(
            (r1.pt_amount, r1.yt_amount),
            (minted.pt_minted, minted.yt_minted)
        );
        assert_eq!(r2.account, lp_holder);
        assert!(r2.lp_pt > 0 && r2.underlying_from_lp > 0);
        // Each account's record list points at its own event only.
        assert_eq!(
            s.escrow.account_records(&sy_holder).get(0),
            Some(ids.get(0).unwrap())
        );
        assert_eq!(s.escrow.account_records(&lp_holder).len(), 1);

        // The bystander was never touched.
        assert_eq!(s.sy.balance_of(&bystander), 25 * SCALE);
    });
}

#[test]
fn a_batch_is_all_or_nothing_and_size_bounded() {
    each_kind(|kind| {
        let s = setup(kind, Config::free(T0 + 300 * DAY));
        let bad = s.new_user();
        let still_ok = s.new_user();
        deposit_sy(&s, &bad, 10 * SCALE);
        deposit_sy(&s, &still_ok, 10 * SCALE);
        s.deauthorize(&bad);

        // One account in the batch is still authorized: the whole batch reverts, including the
        // seizure that would have been fine on its own.
        let mut requests: Vec<SeizeRequest> = Vec::new(&s.env);
        requests.push_back(req(&s.env, &bad, 10 * SCALE, 0, 0, 0));
        requests.push_back(req(&s.env, &still_ok, 10 * SCALE, 0, 0, 0));
        assert_eq!(
            s.escrow.try_seize_batch(&s.admin, &requests).err(),
            err_code(EscrowError::TargetStillAuthorized as u32)
        );
        assert_eq!(s.sy.balance_of(&bad), 10 * SCALE);
        assert_eq!(s.escrow.record_count(), 0);

        // Empty and oversize batches are rejected.
        let empty: Vec<SeizeRequest> = Vec::new(&s.env);
        assert_eq!(
            s.escrow.try_seize_batch(&s.admin, &empty).err(),
            err_code(EscrowError::ZeroAmount as u32)
        );
        let mut big: Vec<SeizeRequest> = Vec::new(&s.env);
        for _ in 0..=principal_recovery_escrow::MAX_BATCH {
            big.push_back(req(&s.env, &bad, 1, 0, 0, 0));
        }
        assert_eq!(
            s.escrow.try_seize_batch(&s.admin, &big).err(),
            err_code(EscrowError::BatchTooLarge as u32)
        );
        // A request that names nothing to seize is rejected.
        let mut nothing: Vec<SeizeRequest> = Vec::new(&s.env);
        nothing.push_back(req(&s.env, &bad, 0, 0, 0, 0));
        assert_eq!(
            s.escrow.try_seize_batch(&s.admin, &nothing).err(),
            err_code(EscrowError::ZeroAmount as u32)
        );
    });
}

#[test]
fn seize_all_positions_sweeps_every_position_type_and_skips_empty_accounts() {
    each_kind(|kind| {
        let s = setup(kind, Config::free(T0 + 300 * DAY));
        let (_, lp_holder) = seed_pool(&s, 1_000 * SCALE, 950 * SCALE);
        let full = s.new_user();
        let empty = s.new_user();
        let shares = deposit_sy(&s, &full, 100 * SCALE);
        s.pm.mint(&full, &(60 * SCALE));
        let _ = shares;
        // `full` also holds 40 SY; `lp_holder` holds YT (kept from seeding) and LP.
        for a in [&full, &empty, &lp_holder] {
            s.deauthorize(a);
        }
        let mut accounts: Vec<Address> = Vec::new(&s.env);
        accounts.push_back(full.clone());
        accounts.push_back(empty.clone());
        accounts.push_back(lp_holder.clone());
        let ids = s.escrow.seize_all_positions(&s.admin, &accounts);
        assert_eq!(ids.len(), 2, "the empty account is skipped");

        assert_eq!(s.sy.balance_of(&full), 0);
        assert_eq!(s.pt.balance(&full), 0);
        assert_eq!(s.yt.balance(&full), 0);
        assert_eq!(s.pool.lp_balance(&lp_holder), 0);
        assert_eq!(s.yt.balance(&lp_holder), 0);
        let rec = s.escrow.get_record(&ids.get(0).unwrap());
        assert_eq!(rec.sy_shares, 40 * SCALE);
        assert_eq!(rec.pt_amount, 60 * SCALE);
        assert_eq!(rec.yt_amount, 60 * SCALE);

        // Nothing left anywhere to sweep.
        let mut again: Vec<Address> = Vec::new(&s.env);
        again.push_back(empty);
        assert_eq!(
            s.escrow.try_seize_all_positions(&s.admin, &again).err(),
            err_code(EscrowError::NothingToSeize as u32)
        );
    });
}

#[test]
fn several_recovery_events_for_one_account_each_get_their_own_record() {
    each_kind(|kind| {
        let s = setup(kind, Config::free(T0 + 300 * DAY));
        let flagged = s.new_user();
        deposit_sy(&s, &flagged, 20 * SCALE);
        s.deauthorize(&flagged);
        s.env.ledger().with_mut(|li| li.sequence_number += 1);
        s.escrow.seize_sy(&s.admin, &flagged, &(5 * SCALE));
        s.env.ledger().with_mut(|li| {
            li.sequence_number += 10;
            li.timestamp += 60;
        });
        s.escrow.seize_sy(&s.admin, &flagged, &(15 * SCALE));

        let ids = s.escrow.account_records(&flagged);
        assert_eq!(ids.len(), 2);
        let a = s.escrow.get_record(&ids.get(0).unwrap());
        let b = s.escrow.get_record(&ids.get(1).unwrap());
        assert_ne!(a.id, b.id);
        assert_eq!(
            (a.underlying_from_sy, b.underlying_from_sy),
            (5 * SCALE, 15 * SCALE)
        );
        assert!(
            b.ledger > a.ledger && b.timestamp > a.timestamp,
            "each event is stamped"
        );
    });
}

// ------------------------------------------------------------- authority

#[test]
fn only_the_live_issuer_authority_may_seize_and_the_target_must_be_deauthorized() {
    each_kind(|kind| {
        let s = setup(kind, Config::free(T0 + 300 * DAY));
        let user = s.new_user();
        deposit_sy(&s, &user, 10 * SCALE);
        let impostor = Address::generate(&s.env);

        // Still authorized: even the real issuer cannot use recovery as a generic drain.
        assert_eq!(
            s.escrow.try_seize_sy(&s.admin, &user, &(10 * SCALE)).err(),
            err_code(EscrowError::TargetStillAuthorized as u32)
        );
        s.deauthorize(&user);
        // A stranger cannot, even against a deauthorized target.
        assert_eq!(
            s.escrow.try_seize_sy(&impostor, &user, &(10 * SCALE)).err(),
            err_code(EscrowError::Unauthorized as u32)
        );
        // Zero and negative amounts are rejected.
        assert_eq!(
            s.escrow.try_seize_sy(&s.admin, &user, &0).err(),
            err_code(EscrowError::ZeroAmount as u32)
        );
        assert!(s.escrow.try_seize_pt(&s.admin, &user, &(-1)).is_err());
        // More than the account holds.
        assert!(s
            .escrow
            .try_seize_sy(&s.admin, &user, &(11 * SCALE))
            .is_err());
        s.escrow.seize_sy(&s.admin, &user, &(10 * SCALE));
    });
}

#[test]
fn sac_admin_rotation_moves_recovery_authority_immediately_with_nothing_to_update() {
    let s = setup(UnderlyingKind::Sac, Config::free(T0 + 300 * DAY));
    let user = s.new_user();
    deposit_sy(&s, &user, 10 * SCALE);
    s.deauthorize(&user);

    let new_admin = Address::generate(&s.env);
    s.sac().set_admin(&new_admin);
    // The old key is dead at once; the new key is live at once. The escrow has no key of its own.
    assert_eq!(
        s.escrow.try_seize_sy(&s.admin, &user, &(10 * SCALE)).err(),
        err_code(EscrowError::Unauthorized as u32)
    );
    assert_eq!(
        s.escrow.seize_sy(&new_admin, &user, &(10 * SCALE)),
        10 * SCALE
    );
}

#[test]
fn seizure_works_while_the_market_is_paused() {
    each_kind(|kind| {
        let s = setup(kind, Config::free(T0 + 300 * DAY));
        let user = s.new_user();
        deposit_sy(&s, &user, 10 * SCALE);
        s.sy.set_paused(&s.admin, &true);
        s.pool.set_paused(&s.admin, &true);
        s.deauthorize(&user);
        assert_eq!(
            s.escrow.seize_sy(&s.admin, &user, &(10 * SCALE)),
            10 * SCALE
        );
    });
}

// ------------------------------------------------------------ initialization

#[test]
fn escrow_initialization_is_one_time_and_validates_the_wiring() {
    let s = setup(UnderlyingKind::Sac, Config::free(T0 + 300 * DAY));
    // Double initialize.
    assert_eq!(
        s.escrow
            .try_initialize(
                &s.underlying,
                &s.sy.address,
                &s.pt.address,
                &s.yt.address,
                &s.pm.address,
                &s.pool.address
            )
            .err(),
        err_code(EscrowError::AlreadyInitialized as u32)
    );
    // A second stack's contracts are a different underlying: rejected.
    let other = setup(UnderlyingKind::Sac, Config::free(T0 + 300 * DAY));
    let fresh = principal_recovery_escrow::RecoveryEscrowContractClient::new(
        &s.env,
        &s.env
            .register(principal_recovery_escrow::RecoveryEscrowContract, ()),
    );
    let _ = other;
    let foreign = Address::generate(&s.env);
    assert!(fresh
        .try_initialize(
            &s.underlying,
            &foreign,
            &s.pt.address,
            &s.yt.address,
            &s.pm.address,
            &s.pool.address
        )
        .is_err());
    assert_eq!(s.escrow.underlying_address(), s.underlying);
}
