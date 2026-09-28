use soroban_sdk::{testutils::Address as _, vec, Address, Env};

use super::{PermissioningContract, PermissioningContractClient};

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
