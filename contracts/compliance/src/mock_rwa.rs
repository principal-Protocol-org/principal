//! Minimal SEP-57 (T-REX) style RWA token and identity verifier, for tests only. Implements just
//! the surface Principal touches: SEP-41 `transfer`/`balance`, the frozen flag, the identity
//! verifier hookup and operator-gated `set_address_frozen`.

use soroban_sdk::{contract, contractimpl, contracttype, panic_with_error, Address, Env};

use crate::IdentityVerifierClient;

#[soroban_sdk::contracterror]
#[derive(Copy, Clone, Debug, PartialEq)]
#[repr(u32)]
pub enum MockError {
    NotOperator = 1,
    Frozen = 2,
    NotVerified = 3,
    InsufficientBalance = 4,
    NoVerifier = 5,
}

#[contracttype]
enum Key {
    Operator,
    Verifier,
    Balance(Address),
    Frozen(Address),
    Verified(Address),
}

#[contract]
pub struct MockIdentityVerifier;

#[contractimpl]
impl MockIdentityVerifier {
    pub fn initialize(env: Env, operator: Address) {
        env.storage().instance().set(&Key::Operator, &operator);
    }

    pub fn register(env: Env, user: Address) {
        let op: Address = env.storage().instance().get(&Key::Operator).unwrap();
        op.require_auth();
        env.storage().persistent().set(&Key::Verified(user), &true);
    }

    pub fn unregister(env: Env, user: Address) {
        let op: Address = env.storage().instance().get(&Key::Operator).unwrap();
        op.require_auth();
        env.storage().persistent().set(&Key::Verified(user), &false);
    }

    /// SEP-57 `verify_identity`: returns nothing, reverts when the account is not verified.
    pub fn verify_identity(env: Env, user_address: Address) {
        let ok: bool = env
            .storage()
            .persistent()
            .get(&Key::Verified(user_address))
            .unwrap_or(false);
        if !ok {
            panic_with_error!(&env, MockError::NotVerified);
        }
    }
}

#[contract]
pub struct MockRwa;

#[contractimpl]
impl MockRwa {
    pub fn initialize(env: Env, operator: Address, verifier: Address) {
        env.storage().instance().set(&Key::Operator, &operator);
        env.storage().instance().set(&Key::Verifier, &verifier);
    }

    pub fn clear_identity_verifier(env: Env) {
        env.storage().instance().remove(&Key::Verifier);
    }

    pub fn mint(env: Env, to: Address, amount: i128) {
        let op: Address = env.storage().instance().get(&Key::Operator).unwrap();
        op.require_auth();
        Self::credit(&env, &to, amount);
    }

    pub fn balance(env: Env, id: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&Key::Balance(id))
            .unwrap_or(0)
    }

    /// SEP-41 transfer, gated the T-REX way: neither side frozen, both identity-verified.
    pub fn transfer(env: Env, from: Address, to: Address, amount: i128) {
        from.require_auth();
        for a in [&from, &to] {
            if Self::is_frozen(env.clone(), a.clone()) {
                panic_with_error!(&env, MockError::Frozen);
            }
            let verifier = Self::identity_verifier(env.clone());
            IdentityVerifierClient::new(&env, &verifier).verify_identity(a);
        }
        let bal = Self::balance(env.clone(), from.clone());
        if bal < amount {
            panic_with_error!(&env, MockError::InsufficientBalance);
        }
        env.storage()
            .persistent()
            .set(&Key::Balance(from), &(bal - amount));
        Self::credit(&env, &to, amount);
    }

    pub fn is_frozen(env: Env, user_address: Address) -> bool {
        env.storage()
            .persistent()
            .get(&Key::Frozen(user_address))
            .unwrap_or(false)
    }

    pub fn identity_verifier(env: Env) -> Address {
        env.storage()
            .instance()
            .get(&Key::Verifier)
            .unwrap_or_else(|| panic_with_error!(&env, MockError::NoVerifier))
    }

    /// SEP-57 `set_address_frozen`; only the operator role may call it.
    pub fn set_address_frozen(env: Env, user_address: Address, freeze: bool, operator: Address) {
        operator.require_auth();
        let op: Address = env.storage().instance().get(&Key::Operator).unwrap();
        if operator != op {
            panic_with_error!(&env, MockError::NotOperator);
        }
        env.storage()
            .persistent()
            .set(&Key::Frozen(user_address), &freeze);
    }

    fn credit(env: &Env, to: &Address, amount: i128) {
        let bal = Self::balance(env.clone(), to.clone());
        env.storage()
            .persistent()
            .set(&Key::Balance(to.clone()), &(bal + amount));
    }
}

/// Convenience for tests: a fully wired RWA token + verifier pair, returning their clients.
pub fn deploy<'a>(
    env: &Env,
    operator: &Address,
) -> (MockRwaClient<'a>, MockIdentityVerifierClient<'a>) {
    let verifier_id = env.register(MockIdentityVerifier, ());
    let verifier = MockIdentityVerifierClient::new(env, &verifier_id);
    verifier.initialize(operator);
    let token_id = env.register(MockRwa, ());
    let token = MockRwaClient::new(env, &token_id);
    token.initialize(operator, &verifier_id);
    (token, verifier)
}
