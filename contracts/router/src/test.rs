//! Direct, lightweight unit tests for `RouterContract` -- deployed straight into a fresh `Env`,
//! with no other contract in the stack (unlike `contracts/integration_tests`, which deploys the
//! whole protocol and always calls `env.mock_all_auths()` before anything else).
//!
//! That blanket `mock_all_auths()` is exactly why a real bug -- `initialize` missing its
//! `admin.require_auth()` call entirely -- went undetected until a senior-level audit ahead of
//! mainnet launch: `mock_all_auths()` approves *every* address's signature unconditionally, so it
//! can prove a caller-identity mismatch is rejected (`assert_admin`'s `caller != admin` check)
//! but can never prove a `require_auth()` call is actually *present*, since there is nothing left
//! for a missing call to fail. `initialize_requires_the_admins_own_signature` below is this
//! codebase's first test built specifically to catch that gap: it mocks no auths at all for the
//! failing call, then mocks the admin's signature for exactly one invocation for the succeeding
//! call, instead of mocking every signature for the whole test.

use soroban_sdk::{
    contract, contractimpl,
    testutils::{Address as _, MockAuth, MockAuthInvoke},
    Address, Env, IntoVal,
};

use super::{RouterContract, RouterContractClient};

// Minimal stand-ins for `PoolInterface`/`ManagerInterface` -- only the five view functions
// `register_market` actually calls -- used to construct a deliberately mismatched pool/manager
// pair cheaply. A real `MarketPool`/`PrincipalManager` pair can't easily be made to disagree:
// each already cross-checks its own topology at `initialize`, which is exactly why this gap went
// untested (per the audit finding this regression test closes).
#[contract]
struct MockManager;

#[contractimpl]
impl MockManager {
    pub fn init(env: Env, sy: Address, pt: Address, yt: Address, underlying: Address) {
        env.storage()
            .instance()
            .set(&0u32, &(sy, pt, yt, underlying));
    }
    pub fn sy_wrapper_address(env: Env) -> Address {
        let (sy, _, _, _): (Address, Address, Address, Address) =
            env.storage().instance().get(&0u32).unwrap();
        sy
    }
    pub fn pt_address(env: Env) -> Address {
        let (_, pt, _, _): (Address, Address, Address, Address) =
            env.storage().instance().get(&0u32).unwrap();
        pt
    }
    pub fn yt_address(env: Env) -> Address {
        let (_, _, yt, _): (Address, Address, Address, Address) =
            env.storage().instance().get(&0u32).unwrap();
        yt
    }
    pub fn underlying_address(env: Env) -> Address {
        let (_, _, _, u): (Address, Address, Address, Address) =
            env.storage().instance().get(&0u32).unwrap();
        u
    }
}

#[contract]
struct MockPool;

#[contractimpl]
impl MockPool {
    pub fn init(
        env: Env,
        manager: Address,
        sy: Address,
        pt: Address,
        yt: Address,
        underlying: Address,
    ) {
        env.storage()
            .instance()
            .set(&0u32, &(manager, sy, pt, yt, underlying));
    }
    pub fn manager_address(env: Env) -> Address {
        let (m, ..): (Address, Address, Address, Address, Address) =
            env.storage().instance().get(&0u32).unwrap();
        m
    }
    pub fn sy_address(env: Env) -> Address {
        let (_, sy, ..): (Address, Address, Address, Address, Address) =
            env.storage().instance().get(&0u32).unwrap();
        sy
    }
    pub fn pt_address(env: Env) -> Address {
        let (_, _, pt, _, _): (Address, Address, Address, Address, Address) =
            env.storage().instance().get(&0u32).unwrap();
        pt
    }
    pub fn yt_address(env: Env) -> Address {
        let (_, _, _, yt, _): (Address, Address, Address, Address, Address) =
            env.storage().instance().get(&0u32).unwrap();
        yt
    }
    pub fn underlying_address(env: Env) -> Address {
        let (_, _, _, _, u): (Address, Address, Address, Address, Address) =
            env.storage().instance().get(&0u32).unwrap();
        u
    }
}

fn setup() -> (Env, RouterContractClient<'static>, Address) {
    let env = Env::default();
    let id = env.register(RouterContract, ());
    let client = RouterContractClient::new(&env, &id);
    let admin = Address::generate(&env);
    (env, client, admin)
}

/// The novel regression test: proves `initialize` actually calls `admin.require_auth()`, not
/// just that `caller == admin` is checked. No `mock_all_auths()` anywhere in this test.
#[test]
fn initialize_requires_the_admins_own_signature() {
    let (env, client, admin) = setup();

    // No auths mocked at all -- the host has nothing to approve `admin.require_auth()` against,
    // so this must fail. If `initialize` had never called `require_auth()` on `admin` (the exact
    // bug this test guards against), it would succeed here instead.
    assert!(client.try_initialize(&admin).is_err());
    // And it must not have taken effect.
    assert!(client.try_get_admin().is_err());

    // The admin's own signature, mocked for exactly this one invocation, is enough.
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
    assert_eq!(client.get_admin(), admin);
}

/// A second signature mocked for a *different* address does not substitute for the admin's own --
/// `mock_auths` only approves the invocations it explicitly lists.
#[test]
fn initialize_is_not_satisfied_by_someone_elses_signature() {
    let (env, client, admin) = setup();
    let stranger = Address::generate(&env);

    assert!(client
        .mock_auths(&[MockAuth {
            address: &stranger,
            invoke: &MockAuthInvoke {
                contract: &client.address,
                fn_name: "initialize",
                args: (&admin,).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .try_initialize(&admin)
        .is_err());
}

/// Calling `initialize` twice fails even with the admin's signature -- ordinary double-init
/// rejection, covered elsewhere in spirit by every other `test.rs` in this codebase, included here
/// too since this is the Router's only unit-test file.
#[test]
fn double_initialize_fails() {
    let (env, client, admin) = setup();
    env.mock_all_auths();
    client.initialize(&admin);
    assert!(client.try_initialize(&admin).is_err());
}

/// `register_market`/`unregister_market` reject any caller but the admin. `assert_admin` runs
/// before either function ever touches the named pool address, so this holds even though no real
/// `MarketPool`/`PrincipalManager` is deployed at `pool` in this lightweight test.
#[test]
fn register_and_unregister_market_are_admin_only() {
    let (env, client, admin) = setup();
    env.mock_all_auths();
    client.initialize(&admin);

    let stranger = Address::generate(&env);
    let pool = Address::generate(&env);
    assert!(client.try_register_market(&stranger, &pool).is_err());
    assert!(client.try_unregister_market(&stranger, &pool).is_err());
}

/// `register_market` cross-checks the pool's own self-reported `sy`/`pt`/`yt`/`underlying`
/// addresses against what its manager reports for the same market, and rejects a mismatch --
/// found untested in an audit review (every other assertion about this check was only in prose).
/// A real `MarketPool`/`PrincipalManager` pair can't easily be coerced into disagreeing (each
/// already validates its own topology at `initialize`), so this uses the minimal mock pair above
/// to construct a deliberately inconsistent one cheaply.
#[test]
fn register_market_rejects_a_pool_whose_self_reported_topology_disagrees_with_its_manager() {
    let (env, client, admin) = setup();
    env.mock_all_auths();
    client.initialize(&admin);

    let manager_id = env.register(MockManager, ());
    let manager_client = MockManagerClient::new(&env, &manager_id);
    let sy = Address::generate(&env);
    let pt = Address::generate(&env);
    let yt = Address::generate(&env);
    let underlying = Address::generate(&env);
    manager_client.init(&sy, &pt, &yt, &underlying);

    // A pool that agrees with its manager on everything: registers fine.
    let good_pool_id = env.register(MockPool, ());
    let good_pool = MockPoolClient::new(&env, &good_pool_id);
    good_pool.init(&manager_id, &sy, &pt, &yt, &underlying);
    client.register_market(&admin, &good_pool_id);
    assert!(client.is_registered(&good_pool_id));

    // A pool that reports a different `pt` than its own manager does: rejected, not registered.
    let wrong_pt = Address::generate(&env);
    let bad_pool_id = env.register(MockPool, ());
    let bad_pool = MockPoolClient::new(&env, &bad_pool_id);
    bad_pool.init(&manager_id, &sy, &wrong_pt, &yt, &underlying);
    assert!(client.try_register_market(&admin, &bad_pool_id).is_err());
    assert!(!client.is_registered(&bad_pool_id));
}

/// `transfer_admin` moves admin rights to a new address, and only the current admin can do it.
#[test]
fn transfer_admin_moves_rights_and_is_admin_only() {
    let (env, client, admin) = setup();
    env.mock_all_auths();
    client.initialize(&admin);

    let stranger = Address::generate(&env);
    let new_admin = Address::generate(&env);
    assert!(client.try_transfer_admin(&stranger, &new_admin).is_err());

    client.transfer_admin(&admin, &new_admin);
    assert_eq!(client.get_admin(), new_admin);
    // The old admin no longer has rights.
    assert!(client
        .try_transfer_admin(&admin, &Address::generate(&env))
        .is_err());
}
