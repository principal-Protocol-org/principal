use soroban_sdk::{
    testutils::{Address as _, IssuerFlags, Ledger as _},
    token, Address, Env, String,
};

use principal_oracle_adapter::{OracleAdapterContract, OracleAdapterContractClient};
use principal_permissioning::{PermissioningContract, PermissioningContractClient};

use super::{YTTokenContract, YTTokenContractClient, INDEX_SCALE, SCALE};

const T0: u64 = 1_000;

struct Fixture {
    env: Env,
    client: YTTokenContractClient<'static>,
    admin: Address,
    underlying: Address,
    perm: PermissioningContractClient<'static>,
    perm_admin: Address,
    oracle: OracleAdapterContractClient<'static>,
    oracle_admin: Address,
    yt_id: Address,
}

fn setup() -> Fixture {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = T0);

    let perm_id = env.register_contract(None, PermissioningContract);
    let perm = PermissioningContractClient::new(&env, &perm_id);
    let perm_admin = Address::generate(&env);
    perm.initialize(&perm_admin);

    let oracle_id = env.register_contract(None, OracleAdapterContract);
    let oracle = OracleAdapterContractClient::new(&env, &oracle_id);
    let oracle_admin = Address::generate(&env);
    oracle.initialize(&oracle_admin);
    oracle.set_reference_value(&oracle_admin, &SCALE, &T0);

    // admin doubles as the underlying SAC's real admin, satisfying the issuer-match check.
    // RevocableFlag lets tests simulate deauthorization.
    let admin = Address::generate(&env);
    let underlying_sac = env.register_stellar_asset_contract_v2(admin.clone());
    underlying_sac.issuer().set_flag(IssuerFlags::RevocableFlag);
    let underlying = underlying_sac.address();

    let yt_id = env.register_contract(None, YTTokenContract);
    let client = YTTokenContractClient::new(&env, &yt_id);
    client.initialize(
        &admin,
        &perm_id,
        &underlying,
        &oracle_id,
        &u64::MAX,
        &String::from_str(&env, "Yield Token USDY"),
        &String::from_str(&env, "YT-USDY"),
        &7,
    );

    Fixture {
        env,
        client,
        admin,
        underlying,
        perm,
        perm_admin,
        oracle,
        oracle_admin,
        yt_id,
    }
}

fn grant(f: &Fixture, user: &Address) {
    f.perm.grant_account(&f.perm_admin, user);
    f.perm.grant_asset(&f.perm_admin, user, &f.yt_id);
    token::StellarAssetClient::new(&f.env, &f.underlying).set_authorized(user, &true);
}

#[test]
#[should_panic]
fn initialize_rejects_admin_not_matching_sac_admin() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = T0);

    let perm_id = env.register_contract(None, PermissioningContract);
    let real_sac_admin = Address::generate(&env);
    PermissioningContractClient::new(&env, &perm_id).initialize(&real_sac_admin);

    let oracle_id = env.register_contract(None, OracleAdapterContract);
    OracleAdapterContractClient::new(&env, &oracle_id).initialize(&real_sac_admin);

    let underlying = env
        .register_stellar_asset_contract_v2(real_sac_admin.clone())
        .address();

    let impostor = Address::generate(&env);
    let yt_id = env.register_contract(None, YTTokenContract);
    let client = YTTokenContractClient::new(&env, &yt_id);
    client.initialize(
        &impostor,
        &perm_id,
        &underlying,
        &oracle_id,
        &u64::MAX,
        &String::from_str(&env, "Yield Token USDY"),
        &String::from_str(&env, "YT-USDY"),
        &7,
    );
}

#[test]
#[should_panic]
fn initialize_rejects_stale_oracle() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = T0 + 3_601);

    let perm_id = env.register_contract(None, PermissioningContract);
    let admin = Address::generate(&env);
    PermissioningContractClient::new(&env, &perm_id).initialize(&admin);

    let oracle_id = env.register_contract(None, OracleAdapterContract);
    let oracle = OracleAdapterContractClient::new(&env, &oracle_id);
    oracle.initialize(&admin);
    oracle.set_reference_value(&admin, &SCALE, &T0); // stale by the time we initialize below

    let underlying = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();

    let yt_id = env.register_contract(None, YTTokenContract);
    YTTokenContractClient::new(&env, &yt_id).initialize(
        &admin,
        &perm_id,
        &underlying,
        &oracle_id,
        &u64::MAX,
        &String::from_str(&env, "Yield Token USDY"),
        &String::from_str(&env, "YT-USDY"),
        &7,
    );
}

#[test]
fn genesis_above_scale_does_not_overpay_first_mint() {
    // H-01 regression: if a market is created when the real oracle rate is already above
    // SCALE (e.g. onboarding an asset that has already appreciated, or simply redeploying
    // later in its life), the very first mint must not receive credit for the gap between
    // SCALE and the real genesis rate -- that "yield" was never earned by anyone holding
    // this specific YT.
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = T0);

    let perm_id = env.register_contract(None, PermissioningContract);
    let perm = PermissioningContractClient::new(&env, &perm_id);
    let perm_admin = Address::generate(&env);
    perm.initialize(&perm_admin);

    let oracle_id = env.register_contract(None, OracleAdapterContract);
    let oracle = OracleAdapterContractClient::new(&env, &oracle_id);
    let admin = Address::generate(&env);
    oracle.initialize(&admin);
    // Real rate is already 1.05 at market genesis -- not SCALE.
    oracle.set_reference_value(&admin, &10_500_000_i128, &T0);

    let underlying_sac = env.register_stellar_asset_contract_v2(admin.clone());
    underlying_sac.issuer().set_flag(IssuerFlags::RevocableFlag);
    let underlying = underlying_sac.address();

    let yt_id = env.register_contract(None, YTTokenContract);
    let client = YTTokenContractClient::new(&env, &yt_id);
    client.initialize(
        &admin,
        &perm_id,
        &underlying,
        &oracle_id,
        &u64::MAX,
        &String::from_str(&env, "Yield Token USDY"),
        &String::from_str(&env, "YT-USDY"),
        &7,
    );

    let minter = Address::generate(&env);
    client.set_minter(&admin, &minter);
    let user = Address::generate(&env);
    perm.grant_account(&perm_admin, &user);
    perm.grant_asset(&perm_admin, &user, &yt_id);
    token::StellarAssetClient::new(&env, &underlying).set_authorized(&user, &true);

    client.mint(&user, &(1_000 * SCALE));

    // No update_yield_index has run yet, and the rate hasn't moved since genesis --
    // the first mint must not be able to claim anything.
    assert_eq!(client.claim_yield(&minter, &user), 0);

    client.update_yield_index();
    // The index baselines against the live genesis rate (1.05), not 1.0, and never moved.
    let genesis_index = (INDEX_SCALE * SCALE + 10_500_000 - 1) / 10_500_000;
    assert_eq!(client.accrued_yield_index(), genesis_index);
    assert_eq!(client.claim_yield(&minter, &user), 0);
}

#[test]
fn mint_and_balance() {
    let f = setup();
    let minter = Address::generate(&f.env);
    f.client.set_minter(&f.admin, &minter);
    let user = Address::generate(&f.env);
    grant(&f, &user);

    f.client.mint(&user, &1_000);
    assert_eq!(f.client.balance(&user), 1_000);
}

#[test]
#[should_panic]
fn mint_without_sac_authorization_panics() {
    let f = setup();
    let minter = Address::generate(&f.env);
    f.client.set_minter(&f.admin, &minter);

    let user = Address::generate(&f.env);
    f.perm.grant_account(&f.perm_admin, &user);
    f.perm.grant_asset(&f.perm_admin, &user, &f.yt_id);
    token::StellarAssetClient::new(&f.env, &f.underlying).set_authorized(&user, &false);

    f.client.mint(&user, &1_000);
}

#[test]
fn no_yield_without_rate_increase() {
    let f = setup();
    let minter = Address::generate(&f.env);
    f.client.set_minter(&f.admin, &minter);
    let user = Address::generate(&f.env);
    grant(&f, &user);
    f.client.mint(&user, &(1_000 * SCALE));

    f.client.update_yield_index(); // rate unchanged since inception
    assert_eq!(f.client.accrued_yield_index(), INDEX_SCALE); // factor untouched at genesis value
    assert_eq!(f.client.claim_yield(&minter, &user), 0);
}

#[test]
fn yield_accrues_after_rate_increase() {
    let f = setup();
    let minter = Address::generate(&f.env);
    f.client.set_minter(&f.admin, &minter);
    let user = Address::generate(&f.env);
    grant(&f, &user);
    f.client.mint(&user, &(1_000 * SCALE)); // notional 1000 units at SCALE

    // Rate goes from 1.0 to 1.03.
    f.env.ledger().with_mut(|li| li.timestamp = T0 + 1);
    f.oracle
        .set_reference_value(&f.oracle_admin, &10_300_000, &(T0 + 1));
    f.client.update_yield_index();

    // new_factor = ceil(factor * last_rate / now_rate) = ceil(INDEX_SCALE * SCALE / 10_300_000)
    let expected_factor = (INDEX_SCALE * SCALE + 10_300_000 - 1) / 10_300_000_i128;
    assert_eq!(f.client.accrued_yield_index(), expected_factor);

    // pending = bal * (last - factor) / last, with this user's `last` == SCALE (settled at
    // mint, before the index ever moved) -- reduces to bal * (SCALE - factor) / SCALE.
    let claimed = f.client.claim_yield(&minter, &user);
    let expected_claim = (1_000 * SCALE) * (INDEX_SCALE - expected_factor) / INDEX_SCALE;
    assert_eq!(claimed, expected_claim);
    // Exact match to the economically-correct single-shot formula: notional * (final_rate -
    // initial_rate) / final_rate -- confirming the multiplicative index reproduces it exactly.
    let exact = 1_000_i128 * (10_300_000 - SCALE) / 10_300_000;
    assert_eq!(claimed / SCALE, exact);

    // Claiming again immediately yields nothing further.
    assert_eq!(f.client.claim_yield(&minter, &user), 0);
}

#[test]
fn yield_is_path_independent_across_many_intermediate_updates() {
    // The regression this fix exists for: whether the oracle rate moves from R0 to Rfinal
    // in one jump or many small steps, the claimable yield must be identical -- unlike the
    // old additive-index formula, which overstated yield more with every extra step (the
    // failure mode verified numerically during the audit: ~13.5% overstatement for a
    // 90-step, 30%-appreciation market).
    let f = setup();
    let minter = Address::generate(&f.env);
    f.client.set_minter(&f.admin, &minter);

    let user = Address::generate(&f.env);
    grant(&f, &user);
    f.client.mint(&user, &(1_000 * SCALE));

    let final_rate: i128 = 13_000_000; // 1.30, 30% total appreciation
    let n_steps = 30_u64;
    let mut ts = T0;
    for i in 1..=n_steps {
        ts += 1;
        let r = SCALE + (final_rate - SCALE) * (i as i128) / (n_steps as i128);
        f.env.ledger().with_mut(|li| li.timestamp = ts);
        f.oracle.set_reference_value(&f.oracle_admin, &r, &ts);
        f.client.update_yield_index();
    }

    let many_step_claim = f.client.claim_yield(&minter, &user);

    // What a single jump straight from SCALE to final_rate would have produced for the
    // same notional -- the exact, path-independent formula PT redemption also uses.
    let single_jump_expected = (1_000 * SCALE) * (final_rate - SCALE) / final_rate;

    // Not bit-exact -- 30 successive floor divisions accumulate a small, bounded rounding
    // residual -- but nowhere close to the old additive formula's unbounded, ever-growing
    // overstatement (which would have been ~13% high here, not ~0.001%).
    let diff = (many_step_claim - single_jump_expected).unsigned_abs();
    assert!(
        diff * 1_000_000 < single_jump_expected.unsigned_abs() * 100, // < 0.01% relative
        "multi-step claim {many_step_claim} diverged from single-jump {single_jump_expected} by more than floor-rounding dust"
    );
}

#[test]
fn late_buyer_does_not_receive_prior_yield() {
    let f = setup();
    let minter = Address::generate(&f.env);
    f.client.set_minter(&f.admin, &minter);
    let alice = Address::generate(&f.env);
    let bob = Address::generate(&f.env);
    grant(&f, &alice);
    grant(&f, &bob);

    f.client.mint(&alice, &(1_000 * SCALE));

    // Rate rises before Bob ever holds YT.
    f.env.ledger().with_mut(|li| li.timestamp = T0 + 1);
    f.oracle
        .set_reference_value(&f.oracle_admin, &10_300_000, &(T0 + 1));
    f.client.update_yield_index();

    // Bob receives YT only now, after the index already moved.
    f.client.mint(&bob, &(500 * SCALE));

    // Bob's settled snapshot should already equal the current index, so he has
    // nothing pending despite the global index being nonzero.
    assert_eq!(f.client.pending_claim(&bob), 0);
    assert_eq!(f.client.claim_yield(&minter, &bob), 0);

    // Alice still gets her full accrued share.
    assert!(f.client.claim_yield(&minter, &alice) > 0);
}

#[test]
fn transfer_settles_both_sides() {
    let f = setup();
    let minter = Address::generate(&f.env);
    f.client.set_minter(&f.admin, &minter);
    let alice = Address::generate(&f.env);
    let bob = Address::generate(&f.env);
    grant(&f, &alice);
    grant(&f, &bob);

    f.client.mint(&alice, &(1_000 * SCALE));
    f.env.ledger().with_mut(|li| li.timestamp = T0 + 1);
    f.oracle
        .set_reference_value(&f.oracle_admin, &10_300_000, &(T0 + 1));
    f.client.update_yield_index();

    // Alice transfers everything to Bob; her accrued yield up to this point must
    // remain hers (settled before the balance moves), not follow the tokens to Bob.
    f.client.transfer(&alice, &bob, &(1_000 * SCALE));

    let alice_claim = f.client.claim_yield(&minter, &alice);
    assert!(alice_claim > 0);
    assert_eq!(f.client.claim_yield(&minter, &bob), 0); // Bob owned nothing while the index moved
}

#[test]
#[should_panic]
fn revoked_holder_cannot_dump_yt_before_seizure() {
    let f = setup();
    let minter = Address::generate(&f.env);
    f.client.set_minter(&f.admin, &minter);
    let alice = Address::generate(&f.env);
    let bob = Address::generate(&f.env);
    grant(&f, &alice);
    grant(&f, &bob);
    f.client.mint(&alice, &(500 * SCALE));

    f.perm.revoke_account(&f.perm_admin, &alice);
    f.client.transfer(&alice, &bob, &(100 * SCALE)); // bob is still fully eligible
}

#[test]
#[should_panic]
fn deauthorized_on_sac_cannot_dump_yt_before_seizure() {
    let f = setup();
    let minter = Address::generate(&f.env);
    f.client.set_minter(&f.admin, &minter);
    let alice = Address::generate(&f.env);
    let bob = Address::generate(&f.env);
    grant(&f, &alice);
    grant(&f, &bob);
    f.client.mint(&alice, &(500 * SCALE));

    token::StellarAssetClient::new(&f.env, &f.underlying).set_authorized(&alice, &false);
    f.client.transfer(&alice, &bob, &(100 * SCALE));
}

#[test]
#[should_panic]
fn update_yield_index_blocked_by_stale_oracle() {
    // Without a freshness check, this would silently advance the accrual index off a rate
    // the oracle relay stopped refreshing long ago.
    let f = setup();
    // Ledger advances far past the oracle's last update without a fresh price ever landing.
    f.env.ledger().with_mut(|li| li.timestamp = T0 + 3_601);
    f.client.update_yield_index();
}

// --- seize (compliance recovery) ---

#[test]
fn seize_moves_balance_and_settles_both_sides() {
    let f = setup();
    let minter = Address::generate(&f.env);
    f.client.set_minter(&f.admin, &minter);
    let escrow = Address::generate(&f.env);
    f.client.set_recovery_escrow(&f.admin, &escrow);

    let bad_actor = Address::generate(&f.env);
    grant(&f, &bad_actor);
    f.client.mint(&bad_actor, &(1_000 * SCALE));

    // Rate rises before the seizure.
    f.env.ledger().with_mut(|li| li.timestamp = T0 + 1);
    f.oracle
        .set_reference_value(&f.oracle_admin, &10_300_000, &(T0 + 1));
    f.client.update_yield_index();

    let seized = f.client.seize(&escrow, &bad_actor, &(1_000 * SCALE));
    assert_eq!(seized, 1_000 * SCALE);
    assert_eq!(f.client.balance(&bad_actor), 0);
    assert_eq!(f.client.balance(&escrow), 1_000 * SCALE);

    // Yield accrued before seizure moves with the position to the escrow: the flagged account
    // can never claim it (it is deauthorized), so leaving it behind would strand it.
    assert_eq!(f.client.claim_yield(&minter, &bad_actor), 0);
    let recovered = f.client.claim_yield(&minter, &escrow);
    // 1000 YT * (1/1.00 - 1/1.03) = 29.126...
    assert!(
        (recovered - 291_262_135).abs() <= 2,
        "recovered {recovered}"
    );
}

#[test]
fn a_plain_transfer_does_not_hand_unsynced_yield_to_the_receiver() {
    // The index is advanced only by `update_yield_index`; a transfer must not settle against a
    // stale one. Rate 1.0 -> 1.1 with NO update_yield_index call, then a plain transfer.
    let f = setup();
    let minter = Address::generate(&f.env);
    f.client.set_minter(&f.admin, &minter);
    let alice = Address::generate(&f.env);
    let bob = Address::generate(&f.env);
    grant(&f, &alice);
    grant(&f, &bob);
    f.client.mint(&alice, &(100 * SCALE));

    f.env.ledger().with_mut(|li| li.timestamp = T0 + 1);
    f.oracle
        .set_reference_value(&f.oracle_admin, &11_000_000, &(T0 + 1));
    f.client.transfer(&alice, &bob, &(100 * SCALE));

    // Alice earned the whole 1.0 -> 1.1 move while she held the YT; Bob earned none of it.
    let alice_claim = f.client.claim_yield(&minter, &alice);
    let bob_claim = f.client.claim_yield(&minter, &bob);
    // 100 * (1 - 1/1.1) = 9.0909...
    assert!((alice_claim - 90_909_090).abs() <= 2, "alice {alice_claim}");
    assert_eq!(bob_claim, 0);
}

#[test]
#[should_panic]
fn seize_requires_configured_escrow_caller() {
    let f = setup();
    let minter = Address::generate(&f.env);
    f.client.set_minter(&f.admin, &minter);
    let escrow = Address::generate(&f.env);
    let impostor = Address::generate(&f.env);
    f.client.set_recovery_escrow(&f.admin, &escrow);

    let bad_actor = Address::generate(&f.env);
    grant(&f, &bad_actor);
    f.client.mint(&bad_actor, &(500 * SCALE));

    f.client.seize(&impostor, &bad_actor, &(500 * SCALE));
}

#[test]
#[should_panic]
fn claim_yield_rejects_non_minter_caller() {
    // H-03 regression: claim_yield must reject a caller that isn't the registered minter,
    // even though the passed `from` address is a real, legitimately-minted holder.
    let f = setup();
    let minter = Address::generate(&f.env);
    f.client.set_minter(&f.admin, &minter);
    let user = Address::generate(&f.env);
    grant(&f, &user);
    f.client.mint(&user, &(1_000 * SCALE));

    let impostor = Address::generate(&f.env);
    f.client.claim_yield(&impostor, &user);
}

#[test]
#[should_panic]
fn double_initialize_panics() {
    let f = setup();
    f.client.initialize(
        &f.admin,
        &f.perm.address,
        &f.underlying,
        &f.oracle.address,
        &u64::MAX,
        &String::from_str(&f.env, "Yield Token USDY"),
        &String::from_str(&f.env, "YT-USDY"),
        &7,
    );
}

#[test]
#[should_panic]
fn set_minter_twice_panics() {
    let f = setup();
    let minter = Address::generate(&f.env);
    f.client.set_minter(&f.admin, &minter);
    f.client.set_minter(&f.admin, &minter);
}

#[test]
#[should_panic]
fn set_recovery_escrow_twice_panics() {
    let f = setup();
    let escrow = Address::generate(&f.env);
    f.client.set_recovery_escrow(&f.admin, &escrow);
    f.client.set_recovery_escrow(&f.admin, &escrow);
}

#[test]
#[should_panic]
fn transfer_zero_amount_panics() {
    let f = setup();
    let minter = Address::generate(&f.env);
    f.client.set_minter(&f.admin, &minter);
    let alice = Address::generate(&f.env);
    let bob = Address::generate(&f.env);
    grant(&f, &alice);
    grant(&f, &bob);
    f.client.mint(&alice, &(100 * SCALE));
    f.client.transfer(&alice, &bob, &0);
}

#[test]
#[should_panic]
fn transfer_insufficient_balance_panics() {
    let f = setup();
    let minter = Address::generate(&f.env);
    f.client.set_minter(&f.admin, &minter);
    let alice = Address::generate(&f.env);
    let bob = Address::generate(&f.env);
    grant(&f, &alice);
    grant(&f, &bob);
    f.client.mint(&alice, &(100 * SCALE));
    f.client.transfer(&alice, &bob, &(200 * SCALE));
}

#[test]
fn approve_and_transfer_from() {
    let f = setup();
    let minter = Address::generate(&f.env);
    f.client.set_minter(&f.admin, &minter);
    let alice = Address::generate(&f.env);
    let bob = Address::generate(&f.env);
    let spender = Address::generate(&f.env);
    grant(&f, &alice);
    grant(&f, &bob);

    f.client.mint(&alice, &(500 * SCALE));
    f.client.approve(
        &alice,
        &spender,
        &(300 * SCALE),
        &(f.env.ledger().sequence() + 100),
    );
    assert_eq!(f.client.allowance(&alice, &spender), 300 * SCALE);

    f.client
        .transfer_from(&spender, &alice, &bob, &(200 * SCALE));
    assert_eq!(f.client.balance(&alice), 300 * SCALE);
    assert_eq!(f.client.balance(&bob), 200 * SCALE);
    assert_eq!(f.client.allowance(&alice, &spender), 100 * SCALE);
}

#[test]
#[should_panic]
fn transfer_from_exceeding_allowance_panics() {
    let f = setup();
    let minter = Address::generate(&f.env);
    f.client.set_minter(&f.admin, &minter);
    let alice = Address::generate(&f.env);
    let bob = Address::generate(&f.env);
    let spender = Address::generate(&f.env);
    grant(&f, &alice);
    grant(&f, &bob);

    f.client.mint(&alice, &(500 * SCALE));
    f.client.approve(
        &alice,
        &spender,
        &(100 * SCALE),
        &(f.env.ledger().sequence() + 100),
    );
    f.client
        .transfer_from(&spender, &alice, &bob, &(200 * SCALE));
}

#[test]
#[should_panic]
fn transfer_from_expired_allowance_panics() {
    let f = setup();
    let minter = Address::generate(&f.env);
    f.client.set_minter(&f.admin, &minter);
    let alice = Address::generate(&f.env);
    let bob = Address::generate(&f.env);
    let spender = Address::generate(&f.env);
    grant(&f, &alice);
    grant(&f, &bob);

    f.client.mint(&alice, &(500 * SCALE));
    let expiration = f.env.ledger().sequence() + 5;
    f.client
        .approve(&alice, &spender, &(200 * SCALE), &expiration);
    f.env
        .ledger()
        .with_mut(|li| li.sequence_number = expiration + 1);
    f.client
        .transfer_from(&spender, &alice, &bob, &(200 * SCALE));
}

#[test]
fn views_and_getters() {
    let f = setup();
    let minter = Address::generate(&f.env);
    f.client.set_minter(&f.admin, &minter);
    let escrow = Address::generate(&f.env);
    f.client.set_recovery_escrow(&f.admin, &escrow);

    assert_eq!(f.client.decimals(), 7);
    assert_eq!(
        f.client.name(),
        String::from_str(&f.env, "Yield Token USDY")
    );
    assert_eq!(f.client.symbol(), String::from_str(&f.env, "YT-USDY"));
    assert_eq!(f.client.total_supply(), 0);
    assert_eq!(f.client.maturity(), u64::MAX);
    assert_eq!(f.client.minter(), minter);
    assert_eq!(f.client.get_admin(), f.admin);
    assert_eq!(f.client.recovery_escrow(), escrow);
    assert_eq!(f.client.underlying_address(), f.underlying);
    assert_eq!(f.client.permissioning_address(), f.perm.address);
    assert_eq!(f.client.oracle_address(), f.oracle.address);

    let user = Address::generate(&f.env);
    grant(&f, &user);
    f.client.mint(&user, &(10 * SCALE));
    assert_eq!(f.client.last_claimed_index(&user), INDEX_SCALE);
    assert_eq!(f.client.pending_claim(&user), 0);
}

#[test]
#[should_panic]
fn set_minter_by_non_admin_panics() {
    let f = setup();
    let impostor = Address::generate(&f.env);
    let minter = Address::generate(&f.env);
    f.client.set_minter(&impostor, &minter);
}

#[test]
#[should_panic]
fn mint_to_account_granted_but_not_per_asset_panics() {
    let f = setup();
    let minter = Address::generate(&f.env);
    f.client.set_minter(&f.admin, &minter);
    let user = Address::generate(&f.env);
    // Account-level grant only -- never granted for this specific YT asset.
    f.perm.grant_account(&f.perm_admin, &user);
    token::StellarAssetClient::new(&f.env, &f.underlying).set_authorized(&user, &true);
    f.client.mint(&user, &(10 * SCALE));
}

#[test]
#[should_panic]
fn mint_zero_amount_panics() {
    let f = setup();
    let minter = Address::generate(&f.env);
    f.client.set_minter(&f.admin, &minter);
    let user = Address::generate(&f.env);
    grant(&f, &user);
    f.client.mint(&user, &0);
}

#[test]
#[should_panic]
fn burn_zero_amount_panics() {
    let f = setup();
    let minter = Address::generate(&f.env);
    f.client.set_minter(&f.admin, &minter);
    let user = Address::generate(&f.env);
    grant(&f, &user);
    f.client.mint(&user, &(10 * SCALE));
    f.client.burn(&user, &0);
}

#[test]
#[should_panic]
fn burn_insufficient_balance_panics() {
    let f = setup();
    let minter = Address::generate(&f.env);
    f.client.set_minter(&f.admin, &minter);
    let user = Address::generate(&f.env);
    grant(&f, &user);
    f.client.mint(&user, &(10 * SCALE));
    f.client.burn(&user, &(20 * SCALE));
}

#[test]
#[should_panic]
fn seize_rejects_zero_amount() {
    let f = setup();
    let minter = Address::generate(&f.env);
    f.client.set_minter(&f.admin, &minter);
    let escrow = Address::generate(&f.env);
    f.client.set_recovery_escrow(&f.admin, &escrow);
    let bad_actor = Address::generate(&f.env);
    grant(&f, &bad_actor);
    f.client.mint(&bad_actor, &(10 * SCALE));
    f.client.seize(&escrow, &bad_actor, &0);
}

#[test]
#[should_panic]
fn seize_cannot_exceed_target_balance() {
    let f = setup();
    let minter = Address::generate(&f.env);
    f.client.set_minter(&f.admin, &minter);
    let escrow = Address::generate(&f.env);
    f.client.set_recovery_escrow(&f.admin, &escrow);
    let bad_actor = Address::generate(&f.env);
    grant(&f, &bad_actor);
    f.client.mint(&bad_actor, &(10 * SCALE));
    f.client.seize(&escrow, &bad_actor, &(20 * SCALE));
}

#[test]
#[should_panic]
fn transfer_from_zero_amount_panics() {
    let f = setup();
    let minter = Address::generate(&f.env);
    f.client.set_minter(&f.admin, &minter);
    let alice = Address::generate(&f.env);
    let bob = Address::generate(&f.env);
    let spender = Address::generate(&f.env);
    grant(&f, &alice);
    grant(&f, &bob);
    f.client.mint(&alice, &(500 * SCALE));
    f.client.approve(
        &alice,
        &spender,
        &(300 * SCALE),
        &(f.env.ledger().sequence() + 100),
    );
    f.client.transfer_from(&spender, &alice, &bob, &0);
}

#[test]
#[should_panic]
fn transfer_from_insufficient_balance_panics() {
    let f = setup();
    let minter = Address::generate(&f.env);
    f.client.set_minter(&f.admin, &minter);
    let alice = Address::generate(&f.env);
    let bob = Address::generate(&f.env);
    let spender = Address::generate(&f.env);
    grant(&f, &alice);
    grant(&f, &bob);
    f.client.mint(&alice, &(10 * SCALE));
    f.client.approve(
        &alice,
        &spender,
        &(500 * SCALE),
        &(f.env.ledger().sequence() + 100),
    );
    f.client
        .transfer_from(&spender, &alice, &bob, &(20 * SCALE));
}

#[test]
#[should_panic]
fn approve_negative_amount_panics() {
    let f = setup();
    let alice = Address::generate(&f.env);
    let spender = Address::generate(&f.env);
    grant(&f, &alice);
    f.client
        .approve(&alice, &spender, &-1, &(f.env.ledger().sequence() + 100));
}

// --- solvency at any mint rate (regression for the r_0 != 1.0 overpayment) ---

#[test]
fn pt_plus_yt_claims_never_exceed_the_deposited_shares_at_any_mint_rate() {
    // For a position minted at rate r0 and settled at r_final, PT (N / r_final) plus every YT
    // step (N * (1/r_s - 1/r_next)) must telescope to N / r0 = the shares deposited -- for
    // r0 above, at and (in principle) below 1.0, and for any number of intermediate updates.
    for &(r0, steps) in &[
        (10_000_000_i128, 1_i128),
        (10_500_000, 1),
        (10_500_000, 7),
        (12_000_000, 30),
    ] {
        let f = setup();
        let minter = Address::generate(&f.env);
        f.client.set_minter(&f.admin, &minter);
        f.env.ledger().with_mut(|li| li.timestamp = T0 + 1);
        f.oracle
            .set_reference_value(&f.oracle_admin, &r0, &(T0 + 1));
        let user = Address::generate(&f.env);
        grant(&f, &user);
        let shares = 100 * SCALE;
        let notional = shares * r0 / SCALE;
        f.client.update_yield_index();
        f.client.mint(&user, &notional);

        let r_final = r0 * 13 / 10;
        for i in 1..=steps {
            let r = r0 + (r_final - r0) * i / steps;
            let ts = T0 + 1 + i as u64 * 10;
            f.env.ledger().with_mut(|li| li.timestamp = ts);
            f.oracle.set_reference_value(&f.oracle_admin, &r, &ts);
            f.client.update_yield_index();
        }
        let yield_underlying = f.client.claim_yield(&minter, &user);
        let pt_underlying = notional * SCALE / r_final;
        let total = pt_underlying + yield_underlying;
        assert!(
            total <= shares,
            "r0={r0} steps={steps}: claims {total} exceed {shares}"
        );
        assert!(
            shares - total < 4_000,
            "r0={r0} steps={steps}: too much dust {}",
            shares - total
        );
    }
}
