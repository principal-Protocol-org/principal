use soroban_sdk::{
    testutils::{Address as _, IssuerFlags, Ledger as _},
    token, Address, Env, String,
};

use principal_permissioning::{PermissioningContract, PermissioningContractClient};

use super::{PTTokenContract, PTTokenContractClient};

struct Fixture {
    env: Env,
    client: PTTokenContractClient<'static>,
    admin: Address,
    underlying: Address,
    perm: PermissioningContractClient<'static>,
    perm_admin: Address,
    pt_id: Address,
}

fn setup() -> Fixture {
    let env = Env::default();
    env.mock_all_auths();

    let perm_id = env.register_contract(None, PermissioningContract);
    let perm = PermissioningContractClient::new(&env, &perm_id);
    let perm_admin = Address::generate(&env);
    perm.initialize(&perm_admin);

    // admin doubles as the underlying SAC's real admin, satisfying the issuer-match check.
    // RevocableFlag lets tests simulate deauthorization.
    let admin = Address::generate(&env);
    let underlying_sac = env.register_stellar_asset_contract_v2(admin.clone());
    underlying_sac.issuer().set_flag(IssuerFlags::RevocableFlag);
    let underlying = underlying_sac.address();

    let pt_id = env.register_contract(None, PTTokenContract);
    let client = PTTokenContractClient::new(&env, &pt_id);
    client.initialize(
        &admin,
        &perm_id,
        &underlying,
        &u64::MAX,
        &String::from_str(&env, "Principal Token USDY"),
        &String::from_str(&env, "PT-USDY"),
        &7,
    );

    Fixture {
        env,
        client,
        admin,
        underlying,
        perm,
        perm_admin,
        pt_id,
    }
}

fn grant(f: &Fixture, user: &Address) {
    f.perm.grant_account(&f.perm_admin, user);
    f.perm.grant_asset(&f.perm_admin, user, &f.pt_id);
    token::StellarAssetClient::new(&f.env, &f.underlying).set_authorized(user, &true);
}

#[test]
#[should_panic]
fn initialize_rejects_admin_not_matching_sac_admin() {
    let env = Env::default();
    env.mock_all_auths();
    let real_sac_admin = Address::generate(&env);
    let underlying = env
        .register_stellar_asset_contract_v2(real_sac_admin.clone())
        .address();
    let perm_id = env.register_contract(None, PermissioningContract);
    PermissioningContractClient::new(&env, &perm_id).initialize(&real_sac_admin);

    let impostor = Address::generate(&env);
    let pt_id = env.register_contract(None, PTTokenContract);
    let client = PTTokenContractClient::new(&env, &pt_id);
    client.initialize(
        &impostor,
        &perm_id,
        &underlying,
        &u64::MAX,
        &String::from_str(&env, "Principal Token USDY"),
        &String::from_str(&env, "PT-USDY"),
        &7,
    );
}

#[test]
fn mint_requires_minter_set() {
    let f = setup();
    let user = Address::generate(&f.env);
    grant(&f, &user);
    // No set_minter call yet -> mint must fail.
    let result = f.client.try_mint(&user, &100);
    assert!(result.is_err());
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
    assert_eq!(f.client.total_supply(), 1_000);
}

#[test]
#[should_panic]
fn mint_without_sac_authorization_panics() {
    let f = setup();
    let minter = Address::generate(&f.env);
    f.client.set_minter(&f.admin, &minter);

    let user = Address::generate(&f.env);
    // Granted in Permissioning but explicitly deauthorized on the underlying SAC.
    f.perm.grant_account(&f.perm_admin, &user);
    f.perm.grant_asset(&f.perm_admin, &user, &f.pt_id);
    token::StellarAssetClient::new(&f.env, &f.underlying).set_authorized(&user, &false);

    f.client.mint(&user, &100);
}

#[test]
#[should_panic]
fn set_minter_twice_panics() {
    let f = setup();
    let m1 = Address::generate(&f.env);
    let m2 = Address::generate(&f.env);
    f.client.set_minter(&f.admin, &m1);
    f.client.set_minter(&f.admin, &m2);
}

#[test]
fn transfer_between_eligible_accounts() {
    let f = setup();
    let minter = Address::generate(&f.env);
    f.client.set_minter(&f.admin, &minter);

    let alice = Address::generate(&f.env);
    let bob = Address::generate(&f.env);
    grant(&f, &alice);
    grant(&f, &bob);

    f.client.mint(&alice, &500);
    f.client.transfer(&alice, &bob, &200);

    assert_eq!(f.client.balance(&alice), 300);
    assert_eq!(f.client.balance(&bob), 200);
}

#[test]
#[should_panic]
fn transfer_to_account_without_asset_grant_panics() {
    // Bob is on the global allow-list but was never granted PT specifically —
    // this is the asymmetric-permissioning enforcement path.
    let f = setup();
    let minter = Address::generate(&f.env);
    f.client.set_minter(&f.admin, &minter);

    let alice = Address::generate(&f.env);
    let bob = Address::generate(&f.env);
    grant(&f, &alice);
    f.perm.grant_account(&f.perm_admin, &bob); // account-level only, no PT asset grant
    token::StellarAssetClient::new(&f.env, &f.underlying).set_authorized(&bob, &true);

    f.client.mint(&alice, &500);
    f.client.transfer(&alice, &bob, &100);
}

#[test]
#[should_panic]
fn revoked_holder_cannot_dump_pt_before_seizure() {
    // If transfer only checked `to`, a revoked account could freely move its PT to any
    // still-eligible party the instant it suspected a seizure was coming.
    let f = setup();
    let minter = Address::generate(&f.env);
    f.client.set_minter(&f.admin, &minter);

    let alice = Address::generate(&f.env);
    let bob = Address::generate(&f.env);
    grant(&f, &alice);
    grant(&f, &bob);
    f.client.mint(&alice, &500);

    f.perm.revoke_account(&f.perm_admin, &alice);
    f.client.transfer(&alice, &bob, &100); // bob is still fully eligible
}

#[test]
#[should_panic]
fn deauthorized_on_sac_cannot_dump_pt_before_seizure() {
    let f = setup();
    let minter = Address::generate(&f.env);
    f.client.set_minter(&f.admin, &minter);

    let alice = Address::generate(&f.env);
    let bob = Address::generate(&f.env);
    grant(&f, &alice);
    grant(&f, &bob);
    f.client.mint(&alice, &500);

    // Issuer deauthorizes directly on the SAC, not via Principal's own Permissioning.
    token::StellarAssetClient::new(&f.env, &f.underlying).set_authorized(&alice, &false);
    f.client.transfer(&alice, &bob, &100);
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

    f.client.mint(&alice, &100);
    f.client.transfer(&alice, &bob, &200);
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

    f.client.mint(&alice, &500);
    f.client
        .approve(&alice, &spender, &300, &(f.env.ledger().sequence() + 100));
    f.client.transfer_from(&spender, &alice, &bob, &200);

    assert_eq!(f.client.balance(&alice), 300);
    assert_eq!(f.client.balance(&bob), 200);
    assert_eq!(f.client.allowance(&alice, &spender), 100);
}

#[test]
fn burn_reduces_supply() {
    let f = setup();
    let minter = Address::generate(&f.env);
    f.client.set_minter(&f.admin, &minter);

    let user = Address::generate(&f.env);
    grant(&f, &user);
    f.client.mint(&user, &1_000);
    f.client.burn(&user, &400);

    assert_eq!(f.client.balance(&user), 600);
    assert_eq!(f.client.total_supply(), 600);
}

// --- seize (compliance recovery) ---

#[test]
fn seize_moves_balance_to_escrow_without_holder_auth() {
    let f = setup();
    let minter = Address::generate(&f.env);
    f.client.set_minter(&f.admin, &minter);
    let escrow = Address::generate(&f.env);
    f.client.set_recovery_escrow(&f.admin, &escrow);

    let bad_actor = Address::generate(&f.env);
    grant(&f, &bad_actor);
    f.client.mint(&bad_actor, &500);

    let seized = f.client.seize(&escrow, &bad_actor, &500);
    assert_eq!(seized, 500);
    assert_eq!(f.client.balance(&bad_actor), 0);
    assert_eq!(f.client.balance(&escrow), 500);
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
    f.client.mint(&bad_actor, &500);

    f.client.seize(&impostor, &bad_actor, &500);
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
#[should_panic]
fn double_initialize_panics() {
    let f = setup();
    f.client.initialize(
        &f.admin,
        &f.perm.address,
        &f.underlying,
        &u64::MAX,
        &String::from_str(&f.env, "Principal Token USDY"),
        &String::from_str(&f.env, "PT-USDY"),
        &7,
    );
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
    f.client.mint(&alice, &100);
    f.client.transfer(&alice, &bob, &0);
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

    f.client.mint(&alice, &500);
    f.client
        .approve(&alice, &spender, &100, &(f.env.ledger().sequence() + 100));
    f.client.transfer_from(&spender, &alice, &bob, &200);
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

    f.client.mint(&alice, &500);
    let expiration = f.env.ledger().sequence() + 5;
    f.client.approve(&alice, &spender, &200, &expiration);
    f.env
        .ledger()
        .with_mut(|li| li.sequence_number = expiration + 1);
    f.client.transfer_from(&spender, &alice, &bob, &200);
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
        String::from_str(&f.env, "Principal Token USDY")
    );
    assert_eq!(f.client.symbol(), String::from_str(&f.env, "PT-USDY"));
    assert_eq!(f.client.total_supply(), 0);
    assert_eq!(f.client.maturity(), u64::MAX);
    assert_eq!(f.client.minter(), minter);
    assert_eq!(f.client.get_admin(), f.admin);
    assert_eq!(f.client.recovery_escrow(), escrow);
    assert_eq!(f.client.underlying_address(), f.underlying);
    assert_eq!(f.client.permissioning_address(), f.perm.address);
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
fn transfer_from_zero_amount_panics() {
    let f = setup();
    let minter = Address::generate(&f.env);
    f.client.set_minter(&f.admin, &minter);
    let alice = Address::generate(&f.env);
    let bob = Address::generate(&f.env);
    let spender = Address::generate(&f.env);
    grant(&f, &alice);
    grant(&f, &bob);
    f.client.mint(&alice, &500);
    f.client
        .approve(&alice, &spender, &300, &(f.env.ledger().sequence() + 100));
    f.client.transfer_from(&spender, &alice, &bob, &0);
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

#[test]
#[should_panic]
fn mint_zero_amount_panics() {
    let f = setup();
    let minter = Address::generate(&f.env);
    f.client.set_minter(&f.admin, &minter);
    let alice = Address::generate(&f.env);
    grant(&f, &alice);
    f.client.mint(&alice, &0);
}

#[test]
#[should_panic]
fn burn_zero_amount_panics() {
    let f = setup();
    let minter = Address::generate(&f.env);
    f.client.set_minter(&f.admin, &minter);
    let alice = Address::generate(&f.env);
    grant(&f, &alice);
    f.client.mint(&alice, &100);
    f.client.burn(&alice, &0);
}

#[test]
#[should_panic]
fn burn_insufficient_balance_panics() {
    let f = setup();
    let minter = Address::generate(&f.env);
    f.client.set_minter(&f.admin, &minter);
    let alice = Address::generate(&f.env);
    grant(&f, &alice);
    f.client.mint(&alice, &100);
    f.client.burn(&alice, &200);
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
    f.client.mint(&bad_actor, &100);
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
    f.client.mint(&bad_actor, &100);
    f.client.seize(&escrow, &bad_actor, &200);
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

    f.client.mint(&alice, &100);
    f.client
        .approve(&alice, &spender, &500, &(f.env.ledger().sequence() + 100));
    f.client.transfer_from(&spender, &alice, &bob, &200);
}
