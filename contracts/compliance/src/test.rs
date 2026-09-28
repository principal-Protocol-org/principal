use soroban_sdk::{
    contract, contractimpl,
    testutils::{Address as _, IssuerFlags},
    token, Address, Env,
};

use super::*;
use crate::mock_rwa::{MockIdentityVerifierClient, MockRwaClient};

/// A thin contract exposing the adapter, so it runs inside a real contract frame with its own
/// instance storage (and a root invocation for `require_auth`), like a Principal contract.
#[contract]
struct Host;

#[contractimpl]
impl Host {
    pub fn init(env: Env, underlying: Address) -> Kind {
        init(&env, &underlying)
    }
    pub fn kind(env: Env) -> Kind {
        kind(&env)
    }
    pub fn authorized(env: Env, underlying: Address, account: Address) -> bool {
        is_authorized(&env, &underlying, &account)
    }
    pub fn is_authority(env: Env, underlying: Address, caller: Address) -> bool {
        caller.require_auth();
        is_authority(&env, &underlying, &caller)
    }
    pub fn authority(env: Env, underlying: Address) -> Option<Address> {
        authority(&env, &underlying)
    }
}

struct Sac {
    env: Env,
    host: HostClient<'static>,
    admin: Address,
    underlying: Address,
}

fn sac_setup() -> Sac {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(admin.clone());
    sac.issuer().set_flag(IssuerFlags::RevocableFlag);
    let host = HostClient::new(&env, &env.register(Host, ()));
    Sac {
        env,
        host,
        admin,
        underlying: sac.address(),
    }
}

struct Rwa {
    env: Env,
    host: HostClient<'static>,
    operator: Address,
    token: MockRwaClient<'static>,
    verifier: MockIdentityVerifierClient<'static>,
}

fn rwa_setup() -> Rwa {
    let env = Env::default();
    env.mock_all_auths();
    let operator = Address::generate(&env);
    let (token, verifier) = crate::mock_rwa::deploy(&env, &operator);
    let host = HostClient::new(&env, &env.register(Host, ()));
    Rwa {
        env,
        host,
        operator,
        token,
        verifier,
    }
}

#[test]
fn detects_sac_and_reads_authorization_live() {
    let s = sac_setup();
    let user = Address::generate(&s.env);
    assert_eq!(s.host.init(&s.underlying), Kind::Sac);
    assert_eq!(s.host.kind(), Kind::Sac);
    assert!(s.host.authorized(&s.underlying, &user));
    token::StellarAssetClient::new(&s.env, &s.underlying).set_authorized(&user, &false);
    assert!(!s.host.authorized(&s.underlying, &user));
}

#[test]
fn sac_authority_is_the_live_admin() {
    let s = sac_setup();
    let stranger = Address::generate(&s.env);
    s.host.init(&s.underlying);
    assert!(s.host.is_authority(&s.underlying, &s.admin));
    assert!(!s.host.is_authority(&s.underlying, &stranger));
    assert_eq!(s.host.authority(&s.underlying), Some(s.admin.clone()));
    // Rotating the SAC admin moves the authority immediately.
    token::StellarAssetClient::new(&s.env, &s.underlying).set_admin(&stranger);
    assert!(s.host.is_authority(&s.underlying, &stranger));
    assert!(!s.host.is_authority(&s.underlying, &s.admin));
}

#[test]
fn kind_defaults_to_sac_before_init() {
    let s = sac_setup();
    assert_eq!(s.host.kind(), Kind::Sac);
}

#[test]
fn detects_rwa_when_underlying_has_no_admin() {
    let r = rwa_setup();
    assert_eq!(r.host.init(&r.token.address), Kind::Rwa);
    assert_eq!(r.host.kind(), Kind::Rwa);
    assert_eq!(r.host.authority(&r.token.address), None);
}

#[test]
fn rwa_requires_identity_and_no_freeze() {
    let r = rwa_setup();
    let user = Address::generate(&r.env);
    r.host.init(&r.token.address);
    // Unverified identity => not authorized.
    assert!(!r.host.authorized(&r.token.address, &user));
    r.verifier.register(&user);
    assert!(r.host.authorized(&r.token.address, &user));
    // Freezing the address revokes it even though the identity is still verified.
    r.token.set_address_frozen(&user, &true, &r.operator);
    assert!(!r.host.authorized(&r.token.address, &user));
    r.token.set_address_frozen(&user, &false, &r.operator);
    r.verifier.unregister(&user);
    assert!(!r.host.authorized(&r.token.address, &user));
}

#[test]
fn rwa_without_a_verifier_is_unauthorized() {
    let r = rwa_setup();
    let user = Address::generate(&r.env);
    r.token.clear_identity_verifier();
    r.host.init(&r.token.address);
    assert!(!r.host.authorized(&r.token.address, &user));
}

#[test]
fn rwa_authority_is_proved_by_the_operator_capability_probe() {
    let r = rwa_setup();
    let stranger = Address::generate(&r.env);
    r.host.init(&r.token.address);
    assert!(r.host.is_authority(&r.token.address, &r.operator));
    assert!(!r.host.is_authority(&r.token.address, &stranger));
    // The probe is idempotent: it never changes the operator's own frozen flag.
    assert!(!r.token.is_frozen(&r.operator));
    r.token.set_address_frozen(&r.operator, &true, &r.operator);
    assert!(r.host.is_authority(&r.token.address, &r.operator));
    assert!(r.token.is_frozen(&r.operator));
}

#[test]
fn a_contract_that_is_neither_a_sac_nor_a_sep57_token_is_never_authorized_and_never_an_authority() {
    // `Host` itself answers none of `admin()`, `is_frozen()` or `identity_verifier()`.
    let s = sac_setup();
    let user = Address::generate(&s.env);
    let bogus = s.host.address.clone();
    s.host.init(&bogus);
    assert_eq!(s.host.kind(), Kind::Rwa);
    assert!(!s.host.is_authority(&bogus, &user));
}
