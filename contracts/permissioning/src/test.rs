use soroban_sdk::{
    testutils::{Address as _, MockAuth, MockAuthInvoke},
    vec, Address, Env, IntoVal,
};

use super::{PermissioningContract, PermissioningContractClient};

/// Regression for a follow-up finding (PRINCIPAL-004): `initialize` must bind to the nominated
/// admin's own signature, not just record whatever address is passed in. `setup()`'s blanket
/// `env.mock_all_auths()` cannot distinguish "checks `require_auth()`" from "never calls it at
/// all", so this test deliberately mocks no auths for the failing call and only the admin's own
/// signature, for exactly one invocation, for the succeeding call -- mirroring
/// `router`'s identical `initialize_requires_the_admins_own_signature` regression test.
#[test]
fn initialize_requires_the_admins_own_signature() {
    let env = Env::default();
    let id = env.register_contract(None, PermissioningContract);
    let client = PermissioningContractClient::new(&env, &id);
    let admin = Address::generate(&env);

    assert!(client.try_initialize(&admin).is_err());

    client
        .mock_auths(&[MockAuth {
            address: &admin,
            invoke: &MockAuthInvoke {
                contract: &client.address,
                fn_name: "initialize",
                args: (&admin,).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .initialize(&admin);
}

fn setup() -> (Env, PermissioningContractClient<'static>, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let id = env.register_contract(None, PermissioningContract);
    let client = PermissioningContractClient::new(&env, &id);
    let admin = Address::generate(&env);
    client.initialize(&admin);
    (env, client, admin)
}

#[test]
fn grant_and_check_account() {
    let (env, client, admin) = setup();
    let user = Address::generate(&env);
    assert!(!client.is_allowed(&user));
    client.grant_account(&admin, &user);
    assert!(client.is_allowed(&user));
}

#[test]
fn revoke_account() {
    let (env, client, admin) = setup();
    let user = Address::generate(&env);
    client.grant_account(&admin, &user);
    client.revoke_account(&admin, &user);
    assert!(!client.is_allowed(&user));
}

#[test]
fn grant_and_revoke_asset() {
    let (env, client, admin) = setup();
    let user = Address::generate(&env);
    let asset = Address::generate(&env);
    client.grant_asset(&admin, &user, &asset);
    assert!(client.is_allowed_for_asset(&user, &asset));
    client.revoke_asset(&admin, &user, &asset);
    assert!(!client.is_allowed_for_asset(&user, &asset));
}

#[test]
fn batch_grant_accounts() {
    let (env, client, admin) = setup();
    let u1 = Address::generate(&env);
    let u2 = Address::generate(&env);
    let u3 = Address::generate(&env);
    client.grant_accounts(&admin, &vec![&env, u1.clone(), u2.clone(), u3.clone()]);
    assert!(client.is_allowed(&u1));
    assert!(client.is_allowed(&u2));
    assert!(client.is_allowed(&u3));
}

#[test]
#[should_panic]
fn unauthorized_grant_panics() {
    let (env, client, _admin) = setup();
    let attacker = Address::generate(&env);
    client.grant_account(&attacker, &Address::generate(&env));
}

#[test]
#[should_panic]
fn unauthorized_revoke_asset_panics() {
    let (env, client, admin) = setup();
    let attacker = Address::generate(&env);
    let user = Address::generate(&env);
    let asset = Address::generate(&env);
    client.grant_asset(&admin, &user, &asset);
    client.revoke_asset(&attacker, &user, &asset);
}

#[test]
#[should_panic]
fn unauthorized_grant_accounts_panics() {
    let (env, client, _admin) = setup();
    let attacker = Address::generate(&env);
    let u1 = Address::generate(&env);
    client.grant_accounts(&attacker, &vec![&env, u1]);
}

#[test]
#[should_panic]
fn unauthorized_transfer_admin_panics() {
    let (env, client, _admin) = setup();
    let attacker = Address::generate(&env);
    let new_admin = Address::generate(&env);
    client.transfer_admin(&attacker, &new_admin);
}

#[test]
fn admin_transfer() {
    let (env, client, admin) = setup();
    let new_admin = Address::generate(&env);
    client.transfer_admin(&admin, &new_admin);
    assert_eq!(client.get_admin(), new_admin);
}

#[test]
#[should_panic]
fn double_initialize_panics() {
    let (_env, client, admin) = setup();
    client.initialize(&admin);
}
