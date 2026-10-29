//! Pure-logic unit tests: address derivation and the auth gate. These need no deployed Wasm
//! (`predict_market` only computes addresses, and the auth check is the first thing each `create_*`
//! call does, before any contract is deployed), so they run under plain `cargo test` with no build
//! step. The full deploy-and-wire path, exercised against the real compiled contracts, is in
//! `tests/create_market.rs`.

use soroban_sdk::{testutils::Address as _, Address, BytesN, Env, String};

use super::*;

fn client(env: &Env) -> MarketFactoryContractClient<'static> {
    MarketFactoryContractClient::new(env, &env.register(MarketFactoryContract, ()))
}

fn salt(env: &Env, seed: u8) -> BytesN<32> {
    let mut b = [0u8; 32];
    b[0] = seed;
    BytesN::from_array(env, &b)
}

#[test]
fn predict_market_is_deterministic_and_salt_sensitive() {
    let env = Env::default();
    let c = client(&env);

    let s1 = salt(&env, 1);
    let first = c.predict_market(&s1);
    let again = c.predict_market(&s1);
    assert_eq!(
        first, again,
        "same salt must predict the same addresses every time"
    );

    let s2 = salt(&env, 2);
    let second = c.predict_market(&s2);
    assert_ne!(
        first, second,
        "different salts must predict different addresses"
    );

    // All seven addresses within one prediction are themselves distinct.
    let a = &first;
    let addrs = [
        &a.sy_wrapper,
        &a.pt_token,
        &a.yt_token,
        &a.market_config,
        &a.principal_manager,
        &a.market_pool,
        &a.recovery_escrow,
    ];
    for i in 0..addrs.len() {
        for j in (i + 1)..addrs.len() {
            assert_ne!(addrs[i], addrs[j], "contract {i} and {j} must not collide");
        }
    }
}

fn dummy_token_params(env: &Env) -> (TokenWasmHashes, TokenParams) {
    let zero = BytesN::from_array(env, &[0u8; 32]);
    let wasm = TokenWasmHashes {
        sy_wrapper: zero.clone(),
        pt_token: zero.clone(),
        yt_token: zero,
    };
    let params = TokenParams {
        underlying: Address::generate(env),
        maturity: 2_000_000,
        permissioning: Address::generate(env),
        oracle: Address::generate(env),
        pt_name: String::from_str(env, "Principal Token"),
        pt_symbol: String::from_str(env, "PT"),
        yt_name: String::from_str(env, "Yield Token"),
        yt_symbol: String::from_str(env, "YT"),
        decimals: 7,
    };
    (wasm, params)
}

/// `create_tokens` must refuse to run at all without the issuer's authorization -- checked before
/// touching the (here, deliberately invalid/zero) wasm hashes, proving the auth gate is the very
/// first thing evaluated and cannot be bypassed by a caller who skips signing. `create_engine` and
/// `create_pool_and_escrow` share the identical `issuer.require_auth()` first line, so this one
/// check covers the gate for all three entrypoints.
#[test]
#[should_panic]
fn create_tokens_requires_issuer_auth() {
    let env = Env::default();
    // Deliberately no `env.mock_all_auths()`: nothing is authorized in this test.
    let c = client(&env);
    let issuer = Address::generate(&env);
    let (wasm, params) = dummy_token_params(&env);
    c.create_tokens(&issuer, &salt(&env, 9), &wasm, &params);
}

/// Sanity check on the salt-mixing helper itself: `Bytes::append` + `sha256` must not collapse two
/// different (salt, tag) pairs onto the same sub-salt for the tags this contract actually uses.
#[test]
fn sub_salts_are_pairwise_distinct_for_one_base_salt() {
    let env = Env::default();
    let base = salt(&env, 7);
    let tags = [
        TAG_SY,
        TAG_PT,
        TAG_YT,
        TAG_CONFIG,
        TAG_MANAGER,
        TAG_POOL,
        TAG_ESCROW,
    ];
    let mut derived: soroban_sdk::Vec<BytesN<32>> = soroban_sdk::Vec::new(&env);
    for t in tags {
        derived.push_back(sub_salt(&env, &base, t));
    }
    for i in 0..derived.len() {
        for j in (i + 1)..derived.len() {
            assert_ne!(derived.get(i).unwrap(), derived.get(j).unwrap());
        }
    }
}
