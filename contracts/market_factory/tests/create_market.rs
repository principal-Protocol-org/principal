//! Full-stack proof that the factory's three `create_*` calls together deploy a genuinely
//! *working* second market, each call atomic, reusing the first market's shared infrastructure
//! (`OracleAdapter`, `Permissioning`, `RiskControl`) and getting a fresh `SYWrapper` -- the exact
//! two things `docs/DAPP_INTEGRATION_AND_MARKET_CREATION.md` §3b says a correct market creation
//! must do.
//!
//! Needs every contract's real Wasm built first:
//! `cargo build --workspace --release --target wasm32v1-none`.
//! Skips (does not fail) if the artifacts are absent, matching the existing convention in
//! `contracts/integration_tests/tests/budget.rs`.

use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    Address, Bytes, BytesN, Env, String,
};

use principal_integration_tests::stack::{self, Config, Stack, DAY, SCALE, T0};
use principal_market_factory::{
    EngineParams, EngineWasmHashes, MarketContracts, MarketFactoryContract,
    MarketFactoryContractClient, PoolEscrowParams, PoolEscrowWasmHashes, TokenParams,
    TokenWasmHashes,
};

const DIR: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../target/wasm32v1-none/release/"
);

struct Wasms {
    sy: Vec<u8>,
    pt: Vec<u8>,
    yt: Vec<u8>,
    config: Vec<u8>,
    manager: Vec<u8>,
    pool: Vec<u8>,
    escrow: Vec<u8>,
}

fn load() -> Option<Wasms> {
    let f = |name: &str| std::fs::read(format!("{DIR}{name}")).ok();
    Some(Wasms {
        sy: f("principal_sy_wrapper.wasm")?,
        pt: f("principal_pt_token.wasm")?,
        yt: f("principal_yt_token.wasm")?,
        config: f("principal_market_config.wasm")?,
        manager: f("principal_manager.wasm")?,
        pool: f("principal_market_pool.wasm")?,
        escrow: f("principal_recovery_escrow.wasm")?,
    })
}

struct Hashes {
    sy: BytesN<32>,
    pt: BytesN<32>,
    yt: BytesN<32>,
    config: BytesN<32>,
    manager: BytesN<32>,
    pool: BytesN<32>,
    escrow: BytesN<32>,
}

fn upload(env: &Env, w: &Wasms) -> Hashes {
    let d = env.deployer();
    Hashes {
        sy: d.upload_contract_wasm(Bytes::from_slice(env, &w.sy)),
        pt: d.upload_contract_wasm(Bytes::from_slice(env, &w.pt)),
        yt: d.upload_contract_wasm(Bytes::from_slice(env, &w.yt)),
        config: d.upload_contract_wasm(Bytes::from_slice(env, &w.config)),
        manager: d.upload_contract_wasm(Bytes::from_slice(env, &w.manager)),
        pool: d.upload_contract_wasm(Bytes::from_slice(env, &w.pool)),
        escrow: d.upload_contract_wasm(Bytes::from_slice(env, &w.escrow)),
    }
}

fn salt(env: &Env, seed: u8) -> BytesN<32> {
    let mut b = [0u8; 32];
    b[0] = seed;
    BytesN::from_array(env, &b)
}

/// Grant one address full compliance standing exactly as `docs/DEPLOYMENT.md` §11 requires for a
/// contract that custodies value: underlying-level authorization plus `Permissioning` account and
/// (where relevant) asset grants for the *new* market's PT/YT.
fn grant_contract_standing(
    s: &Stack,
    addr: &Address,
    pt: &Address,
    yt: &Address,
    needs_assets: bool,
) {
    s.sac().set_authorized(addr, &true);
    s.perm.grant_account(&s.admin, addr);
    if needs_assets {
        s.perm.grant_asset(&s.admin, addr, pt);
        s.perm.grant_asset(&s.admin, addr, yt);
    }
}

#[test]
fn factory_creates_a_fully_functional_second_market_atomically() {
    let Some(w) = load() else {
        eprintln!("skipping: workspace Wasm not built -- run `cargo build --workspace --release --target wasm32v1-none` first");
        return;
    };

    // Market #1: deployed the ordinary way, via the Stack fixture every other test in this
    // workspace uses. This is also where the shared infrastructure (Oracle, Permissioning,
    // RiskControl) that market #2 will reuse comes from.
    let s = stack::deploy(Config::with_example_fees(T0 + 180 * DAY));
    s.env.cost_estimate().budget().reset_unlimited();

    let wasm = upload(&s.env, &w);
    let factory_id = s.env.register(MarketFactoryContract, ());
    let factory = MarketFactoryContractClient::new(&s.env, &factory_id);

    let market2_maturity = T0 + 365 * DAY;
    let s1 = salt(&s.env, 42);

    let predicted = factory.predict_market(&s1);

    // --- Phase 1: SYWrapper, PTToken, YTToken, atomically. ---
    let tokens = factory.create_tokens(
        &s.admin,
        &s1,
        &TokenWasmHashes {
            sy_wrapper: wasm.sy,
            pt_token: wasm.pt,
            yt_token: wasm.yt,
        },
        &TokenParams {
            underlying: s.underlying.clone(),
            maturity: market2_maturity,
            permissioning: s.perm.address.clone(),
            oracle: s.oracle.address.clone(),
            pt_name: String::from_str(&s.env, "Principal Token USDY (Mar)"),
            pt_symbol: String::from_str(&s.env, "PT-USDY-MAR"),
            yt_name: String::from_str(&s.env, "Yield Token USDY (Mar)"),
            yt_symbol: String::from_str(&s.env, "YT-USDY-MAR"),
            decimals: 7,
        },
    );
    assert_eq!(tokens.sy_wrapper, predicted.sy_wrapper);
    assert_eq!(tokens.pt_token, predicted.pt_token);
    assert_eq!(tokens.yt_token, predicted.yt_token);

    // --- Phase 2: MarketConfig, PrincipalManager, + minter/risk-control wiring, atomically. ---
    let engine = factory.create_engine(
        &s.admin,
        &s1,
        &EngineWasmHashes {
            market_config: wasm.config,
            principal_manager: wasm.manager,
        },
        &tokens,
        &EngineParams {
            underlying: s.underlying.clone(),
            maturity: market2_maturity,
            permissioning: s.perm.address.clone(),
            oracle: s.oracle.address.clone(),
            risk_control: s.risk.address.clone(),
            protocol_admin: s.protocol_admin.clone(),
            treasury: s.treasury.clone(),
            tokenization_fee_bps: 5,
            yt_fee_bps: 1_000,
            swap_fee_tier_bps: 10,
            protocol_share_bps: 2_000,
        },
    );
    assert_eq!(engine.market_config, predicted.market_config);
    assert_eq!(engine.principal_manager, predicted.principal_manager);

    // --- Phase 3: MarketPool, RecoveryEscrow, + recovery-escrow wiring, atomically. ---
    let pool_escrow = factory.create_pool_and_escrow(
        &s.admin,
        &s1,
        &PoolEscrowWasmHashes {
            market_pool: wasm.pool,
            recovery_escrow: wasm.escrow,
        },
        &tokens,
        &engine,
        &PoolEscrowParams {
            underlying: s.underlying.clone(),
            time_stretch_years: 4,
        },
    );
    assert_eq!(pool_escrow.market_pool, predicted.market_pool);
    assert_eq!(pool_escrow.recovery_escrow, predicted.recovery_escrow);

    let m2 = MarketContracts {
        sy_wrapper: tokens.sy_wrapper.clone(),
        pt_token: tokens.pt_token.clone(),
        yt_token: tokens.yt_token.clone(),
        market_config: engine.market_config.clone(),
        principal_manager: engine.principal_manager.clone(),
        market_pool: pool_escrow.market_pool.clone(),
        recovery_escrow: pool_escrow.recovery_escrow.clone(),
    };

    // It must be a genuinely NEW market, not an alias of market #1 -- in particular, a fresh
    // SYWrapper, which is exactly the thing that could not be reused across markets in the real
    // 4 October 2026 Testnet run (docs/TESTNET_CURRENT_DEPLOYMENT_EVIDENCE.md §6, finding 1).
    assert_ne!(m2.sy_wrapper, s.sy.address);
    assert_ne!(m2.pt_token, s.pt.address);
    assert_ne!(m2.yt_token, s.yt.address);
    assert_ne!(m2.principal_manager, s.pm.address);
    assert_ne!(m2.market_pool, s.pool.address);
    assert_ne!(m2.recovery_escrow, s.escrow.address);

    // Note on salt reuse: Soroban's host refuses to deploy a second contract at an address that
    // already has code (an already-occupied `create_contract_with_constructor` call fails), so
    // repeating any of the three calls with this same `salt` cannot silently reinitialize or
    // overwrite anything -- that is a host-level guarantee these entrypoints rely on, not
    // something they re-implement.

    // --- Finish the two steps the factory deliberately leaves to the caller (module docs). ---
    s.risk.add_consumer(&s.admin, &m2.sy_wrapper);
    s.risk.add_consumer(&s.admin, &m2.principal_manager);
    s.router.register_market(&s.admin, &m2.market_pool);

    grant_contract_standing(&s, &m2.sy_wrapper, &m2.pt_token, &m2.yt_token, false);
    grant_contract_standing(&s, &m2.principal_manager, &m2.pt_token, &m2.yt_token, false);
    grant_contract_standing(&s, &m2.market_pool, &m2.pt_token, &m2.yt_token, true);
    grant_contract_standing(&s, &m2.recovery_escrow, &m2.pt_token, &m2.yt_token, true);
    grant_contract_standing(&s, &s.treasury.clone(), &m2.pt_token, &m2.yt_token, false);

    // --- Now drive the newly-created market through mint -> trade -> redeem, using the new
    // market's own clients (constructed from the addresses the factory returned). ---
    use principal_manager::PrincipalManagerContractClient;
    use principal_market_pool::MarketPoolContractClient;
    use principal_pt_token::PTTokenContractClient;
    use principal_sy_wrapper::SYWrapperContractClient;
    use principal_yt_token::YTTokenContractClient;

    let sy2 = SYWrapperContractClient::new(&s.env, &m2.sy_wrapper);
    let pt2 = PTTokenContractClient::new(&s.env, &m2.pt_token);
    let yt2 = YTTokenContractClient::new(&s.env, &m2.yt_token);
    let pm2 = PrincipalManagerContractClient::new(&s.env, &m2.principal_manager);
    let pool2 = MarketPoolContractClient::new(&s.env, &m2.market_pool);

    let user = Address::generate(&s.env);
    s.perm.grant_account(&s.admin, &user);
    s.perm.grant_asset(&s.admin, &user, &m2.pt_token);
    s.perm.grant_asset(&s.admin, &user, &m2.yt_token);
    s.authorize(&user);
    s.mint_underlying(&user, 500 * SCALE);

    let shares = sy2.deposit(&user, &(500 * SCALE), &0);
    assert!(
        shares > 0,
        "the new market's own SYWrapper must actually accept deposits"
    );

    let minted = pm2.mint(&user, &shares);
    assert_eq!(pt2.balance(&user), minted.pt_minted);
    assert_eq!(yt2.balance(&user), minted.yt_minted);

    // Trade on the new market's own pool: seed liquidity (notional PT against a 95%-of-notional
    // SY leg, same ratio `stack::seed_pool` uses), then swap.
    let lp = Address::generate(&s.env);
    s.perm.grant_account(&s.admin, &lp);
    s.perm.grant_asset(&s.admin, &lp, &m2.pt_token);
    s.perm.grant_asset(&s.admin, &lp, &m2.yt_token);
    s.authorize(&lp);
    let notional = 100 * SCALE;
    let sy_in = 95 * SCALE;
    s.mint_underlying(&lp, notional + sy_in);
    let lp_shares = sy2.deposit(&lp, &(notional + sy_in), &0);
    let lp_minted = pm2.mint(&lp, &notional.min(lp_shares - sy_in));
    pool2.add_liquidity(&lp, &lp_minted.pt_minted, &sy_in, &0);

    let sold = pool2.swap_pt_for_sy(&user, &user, &(10 * SCALE), &0);
    assert!(sold > 0, "the new market's own pool must actually trade");

    // Settle and redeem at the new market's own, later maturity.
    s.env
        .ledger()
        .with_mut(|li| li.timestamp = market2_maturity + 1);
    s.oracle
        .set_reference_value(&s.admin, &(11 * SCALE / 10), &(market2_maturity + 1));
    pm2.settle_all();
    let remaining_pt = pt2.balance(&user);
    let remaining_yt = yt2.balance(&user);
    let redeemed = pm2.redeem(&user, &remaining_pt, &remaining_yt);
    assert!(
        redeemed.underlying_from_pt + redeemed.underlying_from_yt > 0,
        "redemption on the factory-created market must pay out"
    );

    // Market #1, untouched this whole time, must still work independently -- proving the two
    // markets are properly isolated despite sharing Oracle/Permissioning/RiskControl.
    let u1 = s.new_user();
    let shares1 = stack::deposit_sy(&s, &u1, 50 * SCALE);
    assert!(
        shares1 > 0,
        "market #1 must be unaffected by market #2's creation"
    );
}
