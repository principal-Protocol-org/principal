//! Compliance adapter — one place that answers "may this account hold the underlying asset?" and
//! "is this caller the issuer's authority?" for every Principal contract.
//!
//! # Why an adapter
//! Principal's compliance model is *inheritance*: SY, PT, YT and LP positions may be held by an
//! account only if the underlying asset itself would let that account hold it, and only the
//! underlying's own issuer authority may trigger recovery or create a market. For a classic
//! Stellar Asset wrapped as a SAC (including SEP-8 regulated assets) that authority is the SAC's
//! `authorized(account)` / `admin()`. A SEP-57 (T-REX) `RWAToken` is a plain Soroban contract: it
//! has no SAC, no `admin()` and no classic trustline flag. Compliance there is expressed as a
//! frozen flag plus an `IdentityVerifier`, and privileged actions are role-gated on an `operator`.
//!
//! The adapter detects which of the two models the underlying uses once, at market creation
//! (`init`), stores the result in the *calling* contract's instance storage, and afterwards routes
//! every check through the matching rule:
//!
//! | Question                    | `Kind::Sac`                    | `Kind::Rwa` (SEP-57)                                      |
//! |-----------------------------|--------------------------------|-----------------------------------------------------------|
//! | may `account` hold it?      | `SAC.authorized(account)`      | `!is_frozen(account)` and `identity_verifier.verify_identity(account)` succeeds |
//! | is `caller` the authority?  | `caller == SAC.admin()` (live) | capability probe: idempotent `set_address_frozen` as `operator = caller` succeeds |
//!
//! Neither rule is cached beyond the *kind*: every answer is read live from the issuer's own
//! contract, so an issuer decision (deauthorize, freeze, key or role rotation) takes effect on the
//! next call with nothing to sync.

#![no_std]
#![allow(deprecated)] // `env.events().publish` / `register_contract`: migration to `#[contractevent]` is tracked separately; event topics are kept stable for indexers.

use soroban_sdk::{contractclient, contracttype, token, Address, Env};

#[cfg(any(test, feature = "testutils"))]
pub mod mock_rwa;

/// Which compliance model the underlying asset uses.
#[contracttype]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Classic Stellar asset exposed through its Stellar Asset Contract (covers SEP-8 assets).
    Sac,
    /// SEP-57 (T-REX) `RWAToken`: frozen flag + identity verifier + operator RBAC.
    Rwa,
}

#[contracttype]
enum ComplianceKey {
    /// Detected [`Kind`], stored in the calling contract's instance storage.
    Kind,
}

/// The subset of the SEP-57 `RWAToken` interface Principal reads.
#[contractclient(name = "RwaTokenClient")]
pub trait RwaTokenInterface {
    fn is_frozen(env: Env, user_address: Address) -> bool;
    fn identity_verifier(env: Env) -> Address;
    fn set_address_frozen(env: Env, user_address: Address, freeze: bool, operator: Address);
}

/// The subset of the SEP-57 `IdentityVerifier` interface Principal reads. `verify_identity`
/// returns nothing and reverts when the account is not verified.
#[contractclient(name = "IdentityVerifierClient")]
pub trait IdentityVerifierInterface {
    fn verify_identity(env: Env, user_address: Address);
}

/// Detects the compliance model of `underlying` and stores it in the caller's instance storage.
/// A contract that answers `admin()` like a SAC is `Kind::Sac`; anything else is `Kind::Rwa`.
pub fn init(env: &Env, underlying: &Address) -> Kind {
    let is_sac = matches!(
        token::StellarAssetClient::new(env, underlying).try_admin(),
        Ok(Ok(_))
    );
    let kind = if is_sac { Kind::Sac } else { Kind::Rwa };
    env.storage().instance().set(&ComplianceKey::Kind, &kind);
    kind
}

/// The stored [`Kind`]; `Kind::Sac` if `init` was never called (pre-adapter deployments).
pub fn kind(env: &Env) -> Kind {
    env.storage()
        .instance()
        .get(&ComplianceKey::Kind)
        .unwrap_or(Kind::Sac)
}

/// May `account` currently hold / move the underlying asset (and therefore every derived
/// position)? Read live from the issuer's own contract.
pub fn is_authorized(env: &Env, underlying: &Address, account: &Address) -> bool {
    match kind(env) {
        // A trap inside the SAC (e.g. no trustline for the account) is "not authorized", not a
        // panic of the calling contract.
        Kind::Sac => matches!(
            token::StellarAssetClient::new(env, underlying).try_authorized(account),
            Ok(Ok(true))
        ),
        Kind::Rwa => {
            let rwa = RwaTokenClient::new(env, underlying);
            if rwa.is_frozen(account) {
                return false;
            }
            let verifier = match rwa.try_identity_verifier() {
                Ok(Ok(v)) => v,
                _ => return false,
            };
            matches!(
                IdentityVerifierClient::new(env, &verifier).try_verify_identity(account),
                Ok(Ok(()))
            )
        }
    }
}

/// Is `caller` the issuer's current authority over the underlying? The caller must already have
/// authorized the surrounding invocation (`caller.require_auth()`).
///
/// * `Kind::Sac` — `caller` equals the SAC's `admin()`, read live.
/// * `Kind::Rwa` — SEP-57 has no `admin()`; authority is an RBAC role held by an `operator`. The
///   adapter proves the role by invoking `set_address_frozen(caller, <current value>, caller)`,
///   an idempotent call the token only accepts from an authorised operator.
pub fn is_authority(env: &Env, underlying: &Address, caller: &Address) -> bool {
    match kind(env) {
        Kind::Sac => matches!(
            token::StellarAssetClient::new(env, underlying).try_admin(),
            Ok(Ok(admin)) if admin == *caller
        ),
        Kind::Rwa => {
            let rwa = RwaTokenClient::new(env, underlying);
            let current = match rwa.try_is_frozen(caller) {
                Ok(Ok(f)) => f,
                _ => return false,
            };
            matches!(
                rwa.try_set_address_frozen(caller, &current, caller),
                Ok(Ok(()))
            )
        }
    }
}

/// The issuer authority's address where the model exposes one (`Kind::Sac`); `None` for
/// `Kind::Rwa`, whose authority is a role, not an address.
pub fn authority(env: &Env, underlying: &Address) -> Option<Address> {
    match kind(env) {
        Kind::Sac => Some(token::StellarAssetClient::new(env, underlying).admin()),
        Kind::Rwa => None,
    }
}

#[cfg(test)]
mod test;
