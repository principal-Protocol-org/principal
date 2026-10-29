//! MarketFactory — atomic, one-transaction deployment of a Principal Protocol market.
//!
//! # Why this exists
//! Before this contract, "creating a market" meant an operator running ~15 separate CLI
//! transactions by hand (`docs/DEPLOYMENT.md`): deploy `SYWrapper`, initialize it, deploy `PTToken`,
//! initialize it, ... deploy `RecoveryEscrow`, wire it into four other contracts. If any step failed
//! partway, every earlier contract was left deployed and half-wired on-chain -- exactly what
//! happened to the first two markets in the 4 October 2026 Testnet run
//! (`docs/TESTNET_CURRENT_DEPLOYMENT_EVIDENCE.md` §3, "Superseded" rows). `docs/TRANCHE_1_AUDIT.md`
//! names this directly: deploy-time atomicity via a factory was flagged as the correct fix and
//! scoped to a later tranche. This contract is that fix.
//!
//! Three entrypoints deploy all seven per-market contracts (`SYWrapper`, `PTToken`, `YTToken`,
//! `MarketConfig`, `PrincipalManager`, `MarketPool`, `RecoveryEscrow`) from already-uploaded Wasm,
//! initialize each one, and wire the minter/risk-control/recovery-escrow references between them:
//!
//! 1. [`MarketFactoryContract::create_tokens`] -- `SYWrapper`, `PTToken`, `YTToken`.
//! 2. [`MarketFactoryContract::create_engine`] -- `MarketConfig`, `PrincipalManager`, plus wiring
//!    the minter on PT/YT and the circuit breaker on SY/PrincipalManager.
//! 3. [`MarketFactoryContract::create_pool_and_escrow`] -- `MarketPool`, `RecoveryEscrow`, plus
//!    wiring the recovery escrow on SY/PT/YT/Pool.
//!
//! Soroban transactions are all-or-nothing, so *within* each of these three calls, nothing is ever
//! observed half-wired: if any step panics (a bad topology, a stale oracle, an issuer mismatch),
//! every earlier deploy and write in that same call rolls back too. **This is three transactions,
//! not one** -- deploying and initializing all seven contracts in a single call was the original
//! design, but it was measured to exceed a real Soroban transaction's memory budget on Stellar
//! Testnet itself (not just a test-harness artifact): calling the one-shot version live failed with
//! `Error(Budget, ExceededLimit)` / `Memory(OutOfBoundsGrowth)` partway through deploying the sixth
//! of seven contracts. Splitting into three calls, each comfortably within budget, was verified the
//! same way -- see `docs/MARKET_FACTORY.md` for both runs' transaction hashes. Three transactions is
//! still a large reduction from the ~15-20 an operator runs by hand today (`docs/DEPLOYMENT.md`),
//! and each of the three remains internally atomic.
//!
//! # What this contract deliberately does *not* do
//! - **It holds no admin key and no storage of its own.** Every one of the four `deploy_v2` /
//!   `initialize` checks below is gated by the *child* contract's own compliance check
//!   (`compliance::is_authority`, reading the underlying's live issuer authority), exactly as it is
//!   today when those calls are made by hand. This contract does not loosen that gate by one bit --
//!   it only removes the multi-transaction window between steps. A squatted or misconfigured
//!   factory cannot stand up a market over the issuer's objection, for the same reason a squatted
//!   `OracleAdapter`/`Permissioning` cannot (`docs/TRANCHE_1_AUDIT.md`, finding on deploy-time
//!   atomicity).
//! - **It does not deploy or own the shared infrastructure** (`OracleAdapter`, `Permissioning`,
//!   `RiskControl`, `Router`). Those are not maturity-bound and are meant to be reused across
//!   markets (`docs/DAPP_INTEGRATION_AND_MARKET_CREATION.md` §3b); their addresses are passed in.
//! - **It does not call `RiskControl.add_consumer` or `Router.register_market`.** Both are gated by
//!   *that* contract's own stored admin (Principal's shared-infra operator), which is generally a
//!   different key from the issuer signing this call. Folding them in would force every market
//!   creation to require two co-signers in one transaction; leaving them out keeps this contract
//!   single-signer (the issuer alone) and the gap is harmless by design: an unregistered consumer
//!   makes `SYWrapper.deposit` / `PrincipalManager.mint` revert closed, never open
//!   (`docs/TESTNET_CURRENT_DEPLOYMENT_EVIDENCE.md` §6, finding 1). The caller finishes those two
//!   calls, plus granting compliance standing (`docs/DEPLOYMENT.md` §11), right after.
//! - **It does not manage Wasm hashes.** The caller passes the hash of each already-uploaded
//!   contract Wasm on every call, rather than this contract storing a configurable "approved
//!   version" registry. A stored, admin-settable registry would reintroduce exactly the kind of
//!   mutable, front-runnable admin state this design avoids having at all.
//!
//! # Determinism
//! The seven child contracts are deployed with `env.deployer().with_current_contract(salt)`, so
//! their addresses are derived from *this* contract's own address plus a salt -- not from the
//! issuer or the underlying. The *same* `salt` must be passed to all three calls for one market.
//! [`MarketFactoryContract::predict_market`] derives the same seven sub-salts from one
//! caller-supplied `salt` and returns the addresses the three calls together will produce, before
//! anything is deployed. Reusing a `salt` that already deployed a contract fails the deployment
//! outright (Soroban refuses to redeploy at an occupied address), so a caller should derive a fresh
//! salt per market (for example, `sha256(underlying ++ maturity)`).

#![no_std]
#![allow(deprecated)] // `env.events().publish`: same tracked `#[contractevent]` migration as the rest of the workspace.
#![allow(clippy::too_many_arguments)] // one entrypoint wires seven contracts by design.

use soroban_sdk::{
    contract, contractimpl, contracttype, symbol_short, Address, Bytes, BytesN, Env, String,
};

/// Wasm hashes for `create_tokens` (`SYWrapper`, `PTToken`, `YTToken`), already uploaded to the
/// network (`stellar contract upload` or `Deployer::upload_contract_wasm`).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TokenWasmHashes {
    pub sy_wrapper: BytesN<32>,
    pub pt_token: BytesN<32>,
    pub yt_token: BytesN<32>,
}

/// Parameters for `create_tokens`. `permissioning` and `oracle` are shared, non-maturity-bound
/// infrastructure -- pass existing deployments; see the crate-level docs.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TokenParams {
    pub underlying: Address,
    pub maturity: u64,
    pub permissioning: Address,
    pub oracle: Address,
    pub pt_name: String,
    pub pt_symbol: String,
    pub yt_name: String,
    pub yt_symbol: String,
    pub decimals: u32,
}

/// The three contracts `create_tokens` deploys and wires. Pass this struct straight into
/// `create_engine` and `create_pool_and_escrow`.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarketTokens {
    pub sy_wrapper: Address,
    pub pt_token: Address,
    pub yt_token: Address,
}

/// Wasm hashes for `create_engine` (`MarketConfig`, `PrincipalManager`).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EngineWasmHashes {
    pub market_config: BytesN<32>,
    pub principal_manager: BytesN<32>,
}

/// Parameters for `create_engine`.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EngineParams {
    pub underlying: Address,
    pub maturity: u64,
    pub permissioning: Address,
    pub oracle: Address,
    pub risk_control: Address,
    pub protocol_admin: Address,
    pub treasury: Address,
    pub tokenization_fee_bps: u32,
    pub yt_fee_bps: u32,
    pub swap_fee_tier_bps: u32,
    pub protocol_share_bps: u32,
}

/// The two contracts `create_engine` deploys and wires. Pass this struct straight into
/// `create_pool_and_escrow`.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarketEngine {
    pub market_config: Address,
    pub principal_manager: Address,
}

/// Wasm hashes for `create_pool_and_escrow` (`MarketPool`, `RecoveryEscrow`).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PoolEscrowWasmHashes {
    pub market_pool: BytesN<32>,
    pub recovery_escrow: BytesN<32>,
}

/// Parameters for `create_pool_and_escrow`.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PoolEscrowParams {
    pub underlying: Address,
    pub time_stretch_years: u32,
}

/// The two contracts `create_pool_and_escrow` deploys and wires. The market is fully live once
/// this call returns.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarketPoolAndEscrow {
    pub market_pool: Address,
    pub recovery_escrow: Address,
}

/// The seven contracts of one market, in deployment order.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarketContracts {
    pub sy_wrapper: Address,
    pub pt_token: Address,
    pub yt_token: Address,
    pub market_config: Address,
    pub principal_manager: Address,
    pub market_pool: Address,
    pub recovery_escrow: Address,
}

// ---------------------------------------------------------------------------
// Minimal client interfaces for the contracts this factory wires together.
// Declared locally rather than depending on each sibling crate, matching this
// workspace's existing convention (see e.g. `contracts/recovery_escrow`).
// ---------------------------------------------------------------------------

use soroban_sdk::contractclient;

#[contractclient(name = "SYWrapperClient")]
pub trait SYWrapperInterface {
    fn initialize(env: Env, admin: Address, underlying: Address, permissioning: Address);
    fn set_risk_control(env: Env, admin: Address, risk_control: Address);
    fn set_recovery_escrow(env: Env, admin: Address, escrow: Address);
}

#[contractclient(name = "PTTokenClient")]
pub trait PTTokenInterface {
    fn initialize(
        env: Env,
        admin: Address,
        permissioning: Address,
        underlying: Address,
        maturity: u64,
        name: String,
        symbol: String,
        decimals: u32,
    );
    fn set_minter(env: Env, admin: Address, minter: Address);
    fn set_recovery_escrow(env: Env, admin: Address, escrow: Address);
}

#[contractclient(name = "YTTokenClient")]
pub trait YTTokenInterface {
    fn initialize(
        env: Env,
        admin: Address,
        permissioning: Address,
        underlying: Address,
        oracle: Address,
        maturity: u64,
        name: String,
        symbol: String,
        decimals: u32,
    );
    fn set_minter(env: Env, admin: Address, minter: Address);
    fn set_recovery_escrow(env: Env, admin: Address, escrow: Address);
}

#[contractclient(name = "MarketConfigClient")]
pub trait MarketConfigInterface {
    fn initialize(
        env: Env,
        admin: Address,
        underlying: Address,
        maturity: u64,
        protocol_admin: Address,
        treasury: Address,
        tokenization_fee_bps: u32,
        yt_fee_bps: u32,
        swap_fee_tier_bps: u32,
        protocol_share_bps: u32,
    );
}

#[contractclient(name = "PrincipalManagerClient")]
pub trait PrincipalManagerInterface {
    fn initialize(
        env: Env,
        admin: Address,
        sy_wrapper: Address,
        pt_token: Address,
        yt_token: Address,
        oracle: Address,
        permissioning: Address,
        underlying: Address,
        maturity: u64,
        market_config: Address,
    );
    fn set_risk_control(env: Env, admin: Address, risk_control: Address);
}

#[contractclient(name = "MarketPoolClient")]
pub trait MarketPoolInterface {
    fn initialize(env: Env, admin: Address, principal_manager: Address, time_stretch_years: u32);
    fn set_recovery_escrow(env: Env, admin: Address, escrow: Address);
}

#[contractclient(name = "RecoveryEscrowClient")]
pub trait RecoveryEscrowInterface {
    fn initialize(
        env: Env,
        underlying: Address,
        sy_wrapper: Address,
        pt_token: Address,
        yt_token: Address,
        principal_manager: Address,
        market_pool: Address,
    );
}

/// One distinct tag per deployed contract, mixed into the caller's salt so one `salt` input
/// yields seven distinct, deterministic sub-addresses.
const TAG_SY: u8 = 1;
const TAG_PT: u8 = 2;
const TAG_YT: u8 = 3;
const TAG_CONFIG: u8 = 4;
const TAG_MANAGER: u8 = 5;
const TAG_POOL: u8 = 6;
const TAG_ESCROW: u8 = 7;

fn sub_salt(env: &Env, salt: &BytesN<32>, tag: u8) -> BytesN<32> {
    let mut bytes = Bytes::new(env);
    bytes.append(&Bytes::from_array(env, &salt.to_array()));
    bytes.append(&Bytes::from_array(env, &[tag]));
    env.crypto().sha256(&bytes).to_bytes()
}

#[contract]
pub struct MarketFactoryContract;

#[contractimpl]
impl MarketFactoryContract {
    /// Derive the seven contract addresses the three `create_*` calls with this exact `salt` will
    /// produce. Pure computation -- no deployment, no storage read, no auth. Safe to call before
    /// the issuer commits to signing anything, so a dapp can show the addresses in a confirmation
    /// screen first.
    pub fn predict_market(env: Env, salt: BytesN<32>) -> MarketContracts {
        let addr_for = |tag: u8| -> Address {
            env.deployer()
                .with_current_contract(sub_salt(&env, &salt, tag))
                .deployed_address()
        };
        MarketContracts {
            sy_wrapper: addr_for(TAG_SY),
            pt_token: addr_for(TAG_PT),
            yt_token: addr_for(TAG_YT),
            market_config: addr_for(TAG_CONFIG),
            principal_manager: addr_for(TAG_MANAGER),
            market_pool: addr_for(TAG_POOL),
            recovery_escrow: addr_for(TAG_ESCROW),
        }
    }

    /// Phase 1 of 3. Deploy and initialize `SYWrapper`, `PTToken` and `YTToken`, atomically.
    /// `issuer` must be the underlying's live issuer authority and must authorize this call; each
    /// child `initialize` independently re-checks that live, exactly as it does today when called
    /// by hand. Pass the returned [`MarketTokens`] into `create_engine`.
    pub fn create_tokens(
        env: Env,
        issuer: Address,
        salt: BytesN<32>,
        wasm: TokenWasmHashes,
        params: TokenParams,
    ) -> MarketTokens {
        issuer.require_auth();

        let deploy = |tag: u8, hash: BytesN<32>| -> Address {
            env.deployer()
                .with_current_contract(sub_salt(&env, &salt, tag))
                .deploy_v2(hash, ())
        };

        let sy_addr = deploy(TAG_SY, wasm.sy_wrapper);
        SYWrapperClient::new(&env, &sy_addr).initialize(
            &issuer,
            &params.underlying,
            &params.permissioning,
        );

        let pt_addr = deploy(TAG_PT, wasm.pt_token);
        PTTokenClient::new(&env, &pt_addr).initialize(
            &issuer,
            &params.permissioning,
            &params.underlying,
            &params.maturity,
            &params.pt_name,
            &params.pt_symbol,
            &params.decimals,
        );

        let yt_addr = deploy(TAG_YT, wasm.yt_token);
        YTTokenClient::new(&env, &yt_addr).initialize(
            &issuer,
            &params.permissioning,
            &params.underlying,
            &params.oracle,
            &params.maturity,
            &params.yt_name,
            &params.yt_symbol,
            &params.decimals,
        );

        let tokens = MarketTokens {
            sy_wrapper: sy_addr,
            pt_token: pt_addr,
            yt_token: yt_addr,
        };
        env.events().publish(
            (symbol_short!("mkt_tok"),),
            (salt, tokens.sy_wrapper.clone()),
        );
        tokens
    }

    /// Phase 2 of 3. Deploy and initialize `MarketConfig` and `PrincipalManager`, wire the minter
    /// on PT/YT and the circuit breaker on SY/PrincipalManager -- atomically. `tokens` must be the
    /// exact value `create_tokens` returned for this `salt`. Pass the returned [`MarketEngine`]
    /// into `create_pool_and_escrow`.
    pub fn create_engine(
        env: Env,
        issuer: Address,
        salt: BytesN<32>,
        wasm: EngineWasmHashes,
        tokens: MarketTokens,
        params: EngineParams,
    ) -> MarketEngine {
        issuer.require_auth();

        let deploy = |tag: u8, hash: BytesN<32>| -> Address {
            env.deployer()
                .with_current_contract(sub_salt(&env, &salt, tag))
                .deploy_v2(hash, ())
        };

        let config_addr = deploy(TAG_CONFIG, wasm.market_config);
        MarketConfigClient::new(&env, &config_addr).initialize(
            &issuer,
            &params.underlying,
            &params.maturity,
            &params.protocol_admin,
            &params.treasury,
            &params.tokenization_fee_bps,
            &params.yt_fee_bps,
            &params.swap_fee_tier_bps,
            &params.protocol_share_bps,
        );

        let manager_addr = deploy(TAG_MANAGER, wasm.principal_manager);
        PrincipalManagerClient::new(&env, &manager_addr).initialize(
            &issuer,
            &tokens.sy_wrapper,
            &tokens.pt_token,
            &tokens.yt_token,
            &params.oracle,
            &params.permissioning,
            &params.underlying,
            &params.maturity,
            &config_addr,
        );

        PTTokenClient::new(&env, &tokens.pt_token).set_minter(&issuer, &manager_addr);
        YTTokenClient::new(&env, &tokens.yt_token).set_minter(&issuer, &manager_addr);
        SYWrapperClient::new(&env, &tokens.sy_wrapper)
            .set_risk_control(&issuer, &params.risk_control);
        PrincipalManagerClient::new(&env, &manager_addr)
            .set_risk_control(&issuer, &params.risk_control);

        let engine = MarketEngine {
            market_config: config_addr,
            principal_manager: manager_addr,
        };
        env.events().publish(
            (symbol_short!("mkt_eng"),),
            (salt, engine.principal_manager.clone()),
        );
        engine
    }

    /// Phase 3 of 3. Deploy and initialize `MarketPool` and `RecoveryEscrow`, wire the recovery
    /// escrow into SY/PT/YT/Pool -- atomically. `tokens` and `engine` must be the exact values
    /// `create_tokens`/`create_engine` returned for this `salt`. The market is fully live once
    /// this call returns -- except the two steps this contract never does for any phase (see the
    /// module docs): `RiskControl.add_consumer` for SY/PrincipalManager, `Router.register_market`
    /// for the pool, and granting every one of these five addresses (plus the treasury and the
    /// creator payee) compliance standing (`docs/DEPLOYMENT.md` §11).
    pub fn create_pool_and_escrow(
        env: Env,
        issuer: Address,
        salt: BytesN<32>,
        wasm: PoolEscrowWasmHashes,
        tokens: MarketTokens,
        engine: MarketEngine,
        params: PoolEscrowParams,
    ) -> MarketPoolAndEscrow {
        issuer.require_auth();

        let deploy = |tag: u8, hash: BytesN<32>| -> Address {
            env.deployer()
                .with_current_contract(sub_salt(&env, &salt, tag))
                .deploy_v2(hash, ())
        };

        let pool_addr = deploy(TAG_POOL, wasm.market_pool);
        MarketPoolClient::new(&env, &pool_addr).initialize(
            &issuer,
            &engine.principal_manager,
            &params.time_stretch_years,
        );

        let escrow_addr = deploy(TAG_ESCROW, wasm.recovery_escrow);
        RecoveryEscrowClient::new(&env, &escrow_addr).initialize(
            &params.underlying,
            &tokens.sy_wrapper,
            &tokens.pt_token,
            &tokens.yt_token,
            &engine.principal_manager,
            &pool_addr,
        );

        SYWrapperClient::new(&env, &tokens.sy_wrapper).set_recovery_escrow(&issuer, &escrow_addr);
        PTTokenClient::new(&env, &tokens.pt_token).set_recovery_escrow(&issuer, &escrow_addr);
        YTTokenClient::new(&env, &tokens.yt_token).set_recovery_escrow(&issuer, &escrow_addr);
        MarketPoolClient::new(&env, &pool_addr).set_recovery_escrow(&issuer, &escrow_addr);

        let result = MarketPoolAndEscrow {
            market_pool: pool_addr,
            recovery_escrow: escrow_addr,
        };
        env.events().publish(
            (symbol_short!("mkt_new"),),
            (
                salt,
                result.market_pool.clone(),
                result.recovery_escrow.clone(),
            ),
        );
        result
    }
}

#[cfg(test)]
mod test;
