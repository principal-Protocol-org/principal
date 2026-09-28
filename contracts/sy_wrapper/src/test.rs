use soroban_sdk::{
    testutils::{Address as _, IssuerFlags},
    token, Address, Env,
};

use principal_permissioning::{PermissioningContract, PermissioningContractClient};

use super::{SYWrapperContract, SYWrapperContractClient, RATE_SCALE};

/// Deploy a minimal mock SAC for testing. `admin` becomes the SAC's real admin, matching
/// how `initialize` now requires SYWrapper's own admin to equal `underlying.admin()`. The
/// issuer's `RevocableFlag` is set so tests can simulate deauthorization
/// (`set_authorized(_, false)`) -- without it, deauthorizing panics with "issuer does not
/// have AUTH_REVOCABLE set" instead of exercising the check under test.
fn deploy_token(env: &Env, admin: &Address) -> Address {
    let sac = env.register_stellar_asset_contract_v2(admin.clone());
    sac.issuer().set_flag(IssuerFlags::RevocableFlag);
    sac.address()
}

struct Fixture {
    env: Env,
    client: SYWrapperContractClient<'static>,
    admin: Address,
    underlying: Address,
    perm: PermissioningContractClient<'static>,
    perm_admin: Address,
}

fn setup() -> Fixture {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let underlying = deploy_token(&env, &admin);

    let perm_id = env.register_contract(None, PermissioningContract);
    let perm = PermissioningContractClient::new(&env, &perm_id);
    let perm_admin = Address::generate(&env);
    perm.initialize(&perm_admin);

    let wrapper_id = env.register_contract(None, SYWrapperContract);
    let client = SYWrapperContractClient::new(&env, &wrapper_id);
    // admin == underlying's real SAC admin, satisfying the new issuer-match requirement.
    client.initialize(&admin, &underlying, &perm_id);

    Fixture {
        env,
        client,
        admin,
        underlying,
        perm,
        perm_admin,
    }
}

/// Grants both layers: Permissioning (Principal-specific) and SAC authorization (the
/// mandatory floor inherited from the issuer). Most tests want both cleared.
fn grant(f: &Fixture, user: &Address) {
    f.perm.grant_account(&f.perm_admin, user);
    token::StellarAssetClient::new(&f.env, &f.underlying).set_authorized(user, &true);
}

fn mint(env: &Env, token: &Address, _admin: &Address, to: &Address, amount: i128) {
    let tok = token::StellarAssetClient::new(env, token);
    tok.mint(to, &amount);
}

#[test]
#[should_panic]
fn initialize_rejects_admin_not_matching_sac_admin() {
    let env = Env::default();
    env.mock_all_auths();
    let real_sac_admin = Address::generate(&env);
    let underlying = deploy_token(&env, &real_sac_admin);
    let perm_id = env.register_contract(None, PermissioningContract);
    PermissioningContractClient::new(&env, &perm_id).initialize(&real_sac_admin);

    let impostor = Address::generate(&env);
    let wrapper_id = env.register_contract(None, SYWrapperContract);
    let client = SYWrapperContractClient::new(&env, &wrapper_id);
    // impostor is not the underlying SAC's admin -- market creation must be rejected.
    client.initialize(&impostor, &underlying, &perm_id);
}

#[test]
fn deposit_and_exchange_rate() {
    let f = setup();
    let user = Address::generate(&f.env);
    grant(&f, &user);
    mint(&f.env, &f.underlying, &f.admin, &user, 1_000_000_000);

    let shares = f.client.deposit(&user, &1_000_000_000_i128, &0);
    // First depositor: shares == underlying (1:1).
    assert_eq!(shares, 1_000_000_000);
    assert_eq!(f.client.exchange_rate(), RATE_SCALE);
    assert_eq!(f.client.balance_of(&user), 1_000_000_000);
}

#[test]
#[should_panic]
fn deposit_without_permissioning_grant_panics() {
    let f = setup();
    let user = Address::generate(&f.env);
    // SAC-authorized but never granted in Principal's own Permissioning layer.
    token::StellarAssetClient::new(&f.env, &f.underlying).set_authorized(&user, &true);
    mint(&f.env, &f.underlying, &f.admin, &user, 1_000_000_000);
    f.client.deposit(&user, &1_000_000_000_i128, &0);
}

#[test]
#[should_panic]
fn deposit_without_sac_authorization_panics() {
    // Granted in Principal's own Permissioning, but explicitly deauthorized on the
    // underlying SAC itself (e.g. the issuer never cleared them, or revoked them) -- the
    // mandatory floor inherited from the issuer must still block this regardless of
    // Principal's own Permissioning state. (A freshly-registered test SAC defaults every
    // address to authorized=true, matching real unrestricted-asset semantics, so this test
    // explicitly deauthorizes rather than relying on an unset default.)
    let f = setup();
    let user = Address::generate(&f.env);
    f.perm.grant_account(&f.perm_admin, &user);
    token::StellarAssetClient::new(&f.env, &f.underlying).set_authorized(&user, &false);
    mint(&f.env, &f.underlying, &f.admin, &user, 1_000_000_000);
    f.client.deposit(&user, &1_000_000_000_i128, &0);
}

#[test]
#[should_panic]
fn deauthorized_on_sac_cannot_front_run_seizure_by_self_withdrawing() {
    // If withdraw only checked `to`, an investor the issuer just deauthorized on the SAC
    // could cash out the instant they suspected a seizure was coming.
    let f = setup();
    let user = Address::generate(&f.env);
    grant(&f, &user);
    mint(&f.env, &f.underlying, &f.admin, &user, 500_000_000);
    let shares = f.client.deposit(&user, &500_000_000_i128, &0);

    // Issuer deauthorizes the account directly on the underlying SAC.
    token::StellarAssetClient::new(&f.env, &f.underlying).set_authorized(&user, &false);
    f.client.withdraw(&user, &shares, &user, &0);
}

#[test]
fn withdraw_returns_underlying() {
    let f = setup();
    let user = Address::generate(&f.env);
    grant(&f, &user);
    mint(&f.env, &f.underlying, &f.admin, &user, 500_000_000);

    let shares = f.client.deposit(&user, &500_000_000_i128, &0);
    let out = f.client.withdraw(&user, &shares, &user, &0);
    assert_eq!(out, 500_000_000);
    assert_eq!(f.client.total_shares(), 0);
}

#[test]
#[should_panic]
fn withdraw_to_unpermitted_recipient_panics() {
    let f = setup();
    let user = Address::generate(&f.env);
    let stranger = Address::generate(&f.env);
    grant(&f, &user);
    mint(&f.env, &f.underlying, &f.admin, &user, 500_000_000);

    let shares = f.client.deposit(&user, &500_000_000_i128, &0);
    f.client.withdraw(&user, &shares, &stranger, &0); // stranger never granted
}

#[test]
#[should_panic]
fn withdraw_more_than_balance_panics() {
    let f = setup();
    let user = Address::generate(&f.env);
    grant(&f, &user);
    mint(&f.env, &f.underlying, &f.admin, &user, 100_000_000);
    f.client.deposit(&user, &100_000_000_i128, &0);
    f.client.withdraw(&user, &200_000_000_i128, &user, &0);
}

#[test]
#[should_panic]
fn deposit_while_paused_panics() {
    let f = setup();
    let user = Address::generate(&f.env);
    grant(&f, &user);
    mint(&f.env, &f.underlying, &f.admin, &user, 100_000_000);
    f.client.set_paused(&f.admin, &true);
    f.client.deposit(&user, &100_000_000_i128, &0);
}

#[test]
fn unpause_re_enables_deposits() {
    let f = setup();
    let user = Address::generate(&f.env);
    grant(&f, &user);
    mint(&f.env, &f.underlying, &f.admin, &user, 200_000_000);

    f.client.set_paused(&f.admin, &true);
    f.client.set_paused(&f.admin, &false); // unpause
    let shares = f.client.deposit(&user, &200_000_000_i128, &0);
    assert!(shares > 0);
}

#[test]
fn exchange_rate_stays_at_inception_rate() {
    let f = setup();
    let user = Address::generate(&f.env);
    grant(&f, &user);
    mint(&f.env, &f.underlying, &f.admin, &user, 1_000_000_000);

    let shares = f.client.deposit(&user, &1_000_000_000_i128, &0);
    assert_eq!(f.client.exchange_rate(), RATE_SCALE); // 1:1 at inception

    let user2 = Address::generate(&f.env);
    grant(&f, &user2);
    mint(&f.env, &f.underlying, &f.admin, &user2, 500_000_000);
    f.client.deposit(&user2, &500_000_000_i128, &0);
    assert_eq!(f.client.exchange_rate(), RATE_SCALE); // still 1:1

    assert_eq!(f.client.total_underlying(), 1_500_000_000);
    assert_eq!(f.client.total_shares(), 1_500_000_000);
    let _ = shares;
}

#[test]
#[should_panic]
fn zero_deposit_panics() {
    let f = setup();
    let user = Address::generate(&f.env);
    grant(&f, &user);
    f.client.deposit(&user, &0_i128, &0);
}

#[test]
fn balance_of_returns_correct_shares() {
    let f = setup();
    let user = Address::generate(&f.env);
    grant(&f, &user);
    mint(&f.env, &f.underlying, &f.admin, &user, 300_000_000);
    f.client.deposit(&user, &300_000_000_i128, &0);
    assert_eq!(f.client.balance_of(&user), 300_000_000);
}

#[test]
fn admin_transfer() {
    let f = setup();
    let new_admin = Address::generate(&f.env);
    f.client.transfer_admin(&f.admin, &new_admin);
    assert_eq!(f.client.get_admin(), new_admin);
}

// --- share transfer ---

#[test]
fn transfer_moves_shares_between_eligible_accounts() {
    let f = setup();
    let user = Address::generate(&f.env);
    let recipient = Address::generate(&f.env);
    grant(&f, &user);
    grant(&f, &recipient);
    mint(&f.env, &f.underlying, &f.admin, &user, 500_000_000);
    let shares = f.client.deposit(&user, &500_000_000_i128, &0);

    let moved = f.client.transfer(&user, &recipient, &shares);
    assert_eq!(moved, shares);
    assert_eq!(f.client.balance_of(&user), 0);
    assert_eq!(f.client.balance_of(&recipient), shares);
    // Pure balance move: total shares/underlying unaffected.
    assert_eq!(f.client.total_shares(), shares);
}

#[test]
#[should_panic]
fn transfer_to_unpermitted_recipient_panics() {
    let f = setup();
    let user = Address::generate(&f.env);
    let stranger = Address::generate(&f.env);
    grant(&f, &user);
    mint(&f.env, &f.underlying, &f.admin, &user, 500_000_000);
    let shares = f.client.deposit(&user, &500_000_000_i128, &0);

    f.client.transfer(&user, &stranger, &shares); // stranger never granted
}

#[test]
#[should_panic]
fn transfer_more_than_balance_panics() {
    let f = setup();
    let user = Address::generate(&f.env);
    let recipient = Address::generate(&f.env);
    grant(&f, &user);
    grant(&f, &recipient);
    mint(&f.env, &f.underlying, &f.admin, &user, 100_000_000);
    f.client.deposit(&user, &100_000_000_i128, &0);

    f.client.transfer(&user, &recipient, &200_000_000_i128);
}

// --- seize (compliance recovery) ---

#[test]
fn set_recovery_escrow_once() {
    let f = setup();
    let escrow = Address::generate(&f.env);
    f.client.set_recovery_escrow(&f.admin, &escrow);
    assert_eq!(f.client.recovery_escrow(), escrow);
}

#[test]
#[should_panic]
fn set_recovery_escrow_twice_panics() {
    let f = setup();
    let escrow1 = Address::generate(&f.env);
    let escrow2 = Address::generate(&f.env);
    f.client.set_recovery_escrow(&f.admin, &escrow1);
    f.client.set_recovery_escrow(&f.admin, &escrow2);
}

#[test]
fn seize_moves_balance_to_escrow_without_holder_auth() {
    let f = setup();
    let escrow = Address::generate(&f.env);
    f.client.set_recovery_escrow(&f.admin, &escrow);

    let bad_actor = Address::generate(&f.env);
    let innocent = Address::generate(&f.env);
    grant(&f, &bad_actor);
    grant(&f, &innocent);
    mint(&f.env, &f.underlying, &f.admin, &bad_actor, 1_000_000_000);
    mint(&f.env, &f.underlying, &f.admin, &innocent, 1_000_000_000);
    f.client.deposit(&bad_actor, &1_000_000_000_i128, &0);
    f.client.deposit(&innocent, &1_000_000_000_i128, &0);

    let seized = f.client.seize(&escrow, &bad_actor, &1_000_000_000_i128);
    assert_eq!(seized, 1_000_000_000);

    // Balance moved to the escrow; total shares unaffected (forced transfer, not a burn);
    // innocent depositor untouched.
    assert_eq!(f.client.balance_of(&bad_actor), 0);
    assert_eq!(f.client.balance_of(&escrow), 1_000_000_000);
    assert_eq!(f.client.balance_of(&innocent), 1_000_000_000);
    assert_eq!(f.client.total_shares(), 2_000_000_000);
}

#[test]
#[should_panic]
fn seize_requires_configured_escrow_caller() {
    let f = setup();
    let escrow = Address::generate(&f.env);
    let impostor = Address::generate(&f.env);
    f.client.set_recovery_escrow(&f.admin, &escrow);

    let bad_actor = Address::generate(&f.env);
    grant(&f, &bad_actor);
    mint(&f.env, &f.underlying, &f.admin, &bad_actor, 500_000_000);
    f.client.deposit(&bad_actor, &500_000_000_i128, &0);

    f.client.seize(&impostor, &bad_actor, &500_000_000_i128);
}

#[test]
#[should_panic]
fn seize_cannot_exceed_target_balance() {
    let f = setup();
    let escrow = Address::generate(&f.env);
    f.client.set_recovery_escrow(&f.admin, &escrow);

    let bad_actor = Address::generate(&f.env);
    grant(&f, &bad_actor);
    mint(&f.env, &f.underlying, &f.admin, &bad_actor, 500_000_000);
    f.client.deposit(&bad_actor, &500_000_000_i128, &0);

    f.client.seize(&escrow, &bad_actor, &600_000_000_i128);
}

#[test]
fn seize_works_while_paused() {
    let f = setup();
    let escrow = Address::generate(&f.env);
    f.client.set_recovery_escrow(&f.admin, &escrow);

    let bad_actor = Address::generate(&f.env);
    grant(&f, &bad_actor);
    mint(&f.env, &f.underlying, &f.admin, &bad_actor, 500_000_000);
    f.client.deposit(&bad_actor, &500_000_000_i128, &0);

    f.client.set_paused(&f.admin, &true);
    let seized = f.client.seize(&escrow, &bad_actor, &500_000_000_i128);
    assert_eq!(seized, 500_000_000);
}

#[test]
fn escrow_can_unwrap_seized_sy_via_normal_withdraw() {
    // End-to-end: seize, then the escrow -- pre-authorized on both layers, same as any
    // legitimate holder -- unwraps into raw underlying via the ordinary withdraw path.
    let f = setup();
    let escrow = Address::generate(&f.env);
    f.client.set_recovery_escrow(&f.admin, &escrow);
    grant(&f, &escrow); // issuer + Principal both pre-authorize the escrow

    let bad_actor = Address::generate(&f.env);
    grant(&f, &bad_actor);
    mint(&f.env, &f.underlying, &f.admin, &bad_actor, 500_000_000);
    f.client.deposit(&bad_actor, &500_000_000_i128, &0);

    f.client.seize(&escrow, &bad_actor, &500_000_000_i128);
    let unwrapped = f.client.withdraw(&escrow, &500_000_000_i128, &escrow, &0);
    assert_eq!(unwrapped, 500_000_000);

    let underlying_client = token::Client::new(&f.env, &f.underlying);
    assert_eq!(underlying_client.balance(&escrow), 500_000_000);
}

#[test]
#[should_panic]
fn double_initialize_panics() {
    let f = setup();
    f.client
        .initialize(&f.admin, &f.underlying, &f.perm.address);
}

#[test]
#[should_panic]
fn seize_rejects_zero_shares() {
    let f = setup();
    let escrow = Address::generate(&f.env);
    f.client.set_recovery_escrow(&f.admin, &escrow);
    let bad_actor = Address::generate(&f.env);
    grant(&f, &bad_actor);
    f.client.seize(&escrow, &bad_actor, &0_i128);
}

#[test]
fn exchange_rate_before_any_deposit_is_rate_scale() {
    let f = setup();
    assert_eq!(f.client.exchange_rate(), RATE_SCALE);
    assert_eq!(f.client.total_underlying(), 0);
    assert_eq!(f.client.total_shares(), 0);
}

#[test]
#[should_panic]
fn non_admin_cannot_set_paused() {
    let f = setup();
    let impostor = Address::generate(&f.env);
    f.client.set_paused(&impostor, &true);
}

#[test]
fn admin_transfer_and_getters() {
    let f = setup();
    let escrow = Address::generate(&f.env);
    f.client.set_recovery_escrow(&f.admin, &escrow);
    assert_eq!(f.client.get_admin(), f.admin);
    assert_eq!(f.client.underlying_address(), f.underlying);
    assert_eq!(f.client.permissioning_address(), f.perm.address);
    assert_eq!(f.client.recovery_escrow(), escrow);

    let new_admin = Address::generate(&f.env);
    f.client.transfer_admin(&f.admin, &new_admin);
    assert_eq!(f.client.get_admin(), new_admin);
}
