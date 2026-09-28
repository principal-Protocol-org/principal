//! Full-stack deployment helper: deploys every contract into a single `Env`, wires minters,
//! recovery-escrow registrations, the circuit breaker and the router, and grants the protocol's own
//! contract addresses the compliance standing they need -- mirroring the real deployment order in
//! `docs/DEPLOYMENT.md`.

use soroban_sdk::{
    testutils::{Address as _, IssuerFlags, Ledger as _},
    token, Address, Env, String,
};

use principal_compliance::mock_rwa::{self, MockIdentityVerifierClient, MockRwaClient};
use principal_manager::{PrincipalManagerContract, PrincipalManagerContractClient};
use principal_market_config::{MarketConfigContract, MarketConfigContractClient};
use principal_market_pool::{MarketPoolContract, MarketPoolContractClient};
use principal_oracle_adapter::{OracleAdapterContract, OracleAdapterContractClient};
use principal_permissioning::{PermissioningContract, PermissioningContractClient};
use principal_pt_token::{PTTokenContract, PTTokenContractClient};
use principal_recovery_escrow::{RecoveryEscrowContract, RecoveryEscrowContractClient};
use principal_risk_control::{RiskControlContract, RiskControlContractClient};
use principal_router::{RouterContract, RouterContractClient};
use principal_sy_wrapper::{SYWrapperContract, SYWrapperContractClient};
use principal_yt_token::{YTTokenContract, YTTokenContractClient};

pub const SCALE: i128 = 10_000_000;
/// Base ledger timestamp (> 0 so the oracle can accept its first update).
pub const T0: u64 = 1_000;
pub const DAY: u64 = 86_400;
/// A maturity comfortably inside the pool's supported range (a 365-day market from `T0`).
pub const LONG: u64 = T0 + 365 * DAY;

/// How to configure a deployed market.
#[derive(Clone)]
pub struct Config {
    pub maturity: u64,
    pub tokenization_fee_bps: u32,
    pub yt_fee_bps: u32,
    pub swap_fee_tier_bps: u32,
    pub protocol_share_bps: u32,
    pub time_stretch_years: u32,
}

impl Config {
    /// No fees at all -- the arithmetic in most tests is then exact.
    pub fn free(maturity: u64) -> Self {
        Config {
            maturity,
            tokenization_fee_bps: 0,
            yt_fee_bps: 0,
            swap_fee_tier_bps: 0,
            protocol_share_bps: 2_000,
            time_stretch_years: 4,
        }
    }

    /// The documented example fees: 5 bps tokenization, 10% YT, 0.1% swap tier, 20% protocol.
    pub fn with_example_fees(maturity: u64) -> Self {
        Config {
            tokenization_fee_bps: 5,
            yt_fee_bps: 1_000,
            swap_fee_tier_bps: 10,
            ..Self::free(maturity)
        }
    }
}

/// Which compliance model the underlying asset uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnderlyingKind {
    /// A classic Stellar asset behind its SAC (covers SEP-8 regulated assets): authorization is
    /// the trustline flag, the issuer authority is `SAC.admin()`.
    Sac,
    /// A SEP-57 (T-REX) RWA token: authorization is not-frozen + identity-verified, the issuer
    /// authority is the operator role.
    Rwa,
}

/// The RWA side of an `UnderlyingKind::Rwa` stack.
pub struct RwaParts<'a> {
    pub token: MockRwaClient<'a>,
    pub verifier: MockIdentityVerifierClient<'a>,
}

/// Every deployed contract in one `Env`, wired together the way a real deployment would be.
/// `admin` is the underlying SAC's real, live admin -- the market creator -- and is also used as the
/// oracle/permissioning/risk-control admin for simplicity.
pub struct Stack<'a> {
    pub env: Env,
    pub admin: Address,
    pub protocol_admin: Address,
    pub treasury: Address,
    pub underlying: Address,
    pub oracle: OracleAdapterContractClient<'a>,
    pub perm: PermissioningContractClient<'a>,
    pub risk: RiskControlContractClient<'a>,
    pub config: MarketConfigContractClient<'a>,
    pub sy: SYWrapperContractClient<'a>,
    pub pt: PTTokenContractClient<'a>,
    pub yt: YTTokenContractClient<'a>,
    pub pm: PrincipalManagerContractClient<'a>,
    pub pool: MarketPoolContractClient<'a>,
    pub router: RouterContractClient<'a>,
    pub escrow: RecoveryEscrowContractClient<'a>,
    pub maturity: u64,
    pub kind: UnderlyingKind,
    pub rwa: Option<RwaParts<'a>>,
}

impl<'a> Stack<'a> {
    pub fn underlying_client(&self) -> token::Client<'_> {
        token::Client::new(&self.env, &self.underlying)
    }

    pub fn sac(&self) -> token::StellarAssetClient<'_> {
        token::StellarAssetClient::new(&self.env, &self.underlying)
    }

    /// Move the ledger clock to `ts` and publish the oracle value `rate` at that time, so the
    /// oracle is fresh there. `rate` must not be below the previous value.
    pub fn advance(&self, ts: u64, rate: i128) {
        self.env.ledger().with_mut(|li| li.timestamp = ts);
        self.oracle.set_reference_value(&self.admin, &rate, &ts);
    }

    /// Move the ledger clock to `ts` and republish the *current* oracle rate at that time.
    pub fn advance_flat(&self, ts: u64) {
        let rate = self.oracle.get_reference_value();
        self.advance(ts, rate);
    }

    /// Mint `amount` of the underlying to `to` (issuer action).
    pub fn mint_underlying(&self, to: &Address, amount: i128) {
        match &self.rwa {
            None => self.sac().mint(to, &amount),
            Some(r) => r.token.mint(to, &amount),
        }
    }

    /// Give `user` the underlying-level authorization: SAC `set_authorized(true)`, or a verified
    /// identity for an RWA token.
    pub fn authorize(&self, user: &Address) {
        match &self.rwa {
            None => self.sac().set_authorized(user, &true),
            Some(r) => {
                r.verifier.register(user);
                r.token.set_address_frozen(user, &false, &self.admin);
            }
        }
    }

    /// The issuer deauthorizes `user`: SAC `set_authorized(false)`, or freezing the RWA address.
    pub fn deauthorize(&self, user: &Address) {
        match &self.rwa {
            None => self.sac().set_authorized(user, &false),
            Some(r) => r.token.set_address_frozen(user, &true, &self.admin),
        }
    }

    pub fn new_user(&self) -> Address {
        let u = Address::generate(&self.env);
        grant_user(self, &u);
        u
    }
}

/// Deploy with no fees and the default 4-year time-stretch.
pub fn deploy_stack(maturity: u64) -> Stack<'static> {
    deploy(Config::free(maturity))
}

pub fn deploy(cfg: Config) -> Stack<'static> {
    deploy_kind(cfg, UnderlyingKind::Sac)
}

/// Deploy the whole stack over either a classic-asset (SAC) or a SEP-57 RWA underlying.
pub fn deploy_kind(cfg: Config, kind: UnderlyingKind) -> Stack<'static> {
    deploy_inner(cfg, kind, None)
}

/// Deploy with the `MarketPool` registered from its compiled WASM instead of native Rust, so that
/// the guest's CPU instructions are metered like they would be on the network. Every other contract
/// stays native. Budget enforcement is left at the network default.
pub fn deploy_with_pool_wasm(cfg: Config, pool_wasm: &[u8]) -> Stack<'static> {
    deploy_inner(cfg, UnderlyingKind::Sac, Some(pool_wasm))
}

fn deploy_inner(cfg: Config, kind: UnderlyingKind, pool_wasm: Option<&[u8]>) -> Stack<'static> {
    let env = Env::default();
    env.mock_all_auths();
    env.cost_estimate().budget().reset_unlimited();
    env.ledger().with_mut(|li| {
        li.timestamp = T0;
        li.sequence_number = 100;
    });

    let admin = Address::generate(&env);
    let protocol_admin = Address::generate(&env);
    let treasury = Address::generate(&env);
    let maturity = cfg.maturity;

    let oracle_id = env.register(OracleAdapterContract, ());
    let oracle = OracleAdapterContractClient::new(&env, &oracle_id);
    oracle.initialize(&admin);
    oracle.set_reference_value(&admin, &SCALE, &T0);

    let perm_id = env.register(PermissioningContract, ());
    let perm = PermissioningContractClient::new(&env, &perm_id);
    perm.initialize(&admin);

    let risk_id = env.register(RiskControlContract, ());
    let risk = RiskControlContractClient::new(&env, &risk_id);
    risk.initialize(&admin, &0_i128);

    let (underlying, rwa) = match kind {
        UnderlyingKind::Sac => {
            let sac = env.register_stellar_asset_contract_v2(admin.clone());
            sac.issuer().set_flag(IssuerFlags::RevocableFlag);
            sac.issuer().set_flag(IssuerFlags::ClawbackEnabledFlag);
            (sac.address(), None)
        }
        UnderlyingKind::Rwa => {
            // `admin` is the RWA token's operator: the issuer authority.
            let (token, verifier) = mock_rwa::deploy(&env, &admin);
            (token.address.clone(), Some(RwaParts { token, verifier }))
        }
    };

    let sy_id = env.register(SYWrapperContract, ());
    let sy = SYWrapperContractClient::new(&env, &sy_id);
    sy.initialize(&admin, &underlying, &perm_id);
    sy.set_risk_control(&admin, &risk_id);
    risk.add_consumer(&admin, &sy_id);

    let pt_id = env.register(PTTokenContract, ());
    let pt = PTTokenContractClient::new(&env, &pt_id);
    pt.initialize(
        &admin,
        &perm_id,
        &underlying,
        &maturity,
        &String::from_str(&env, "Principal Token USDY"),
        &String::from_str(&env, "PT-USDY"),
        &7,
    );

    let yt_id = env.register(YTTokenContract, ());
    let yt = YTTokenContractClient::new(&env, &yt_id);
    yt.initialize(
        &admin,
        &perm_id,
        &underlying,
        &oracle_id,
        &maturity,
        &String::from_str(&env, "Yield Token USDY"),
        &String::from_str(&env, "YT-USDY"),
        &7,
    );

    let config_id = env.register(MarketConfigContract, ());
    let config = MarketConfigContractClient::new(&env, &config_id);
    config.initialize(
        &admin,
        &underlying,
        &maturity,
        &protocol_admin,
        &treasury,
        &cfg.tokenization_fee_bps,
        &cfg.yt_fee_bps,
        &cfg.swap_fee_tier_bps,
        &cfg.protocol_share_bps,
    );

    let pm_id = env.register(PrincipalManagerContract, ());
    let pm = PrincipalManagerContractClient::new(&env, &pm_id);
    pm.initialize(
        &admin,
        &sy_id,
        &pt_id,
        &yt_id,
        &oracle_id,
        &perm_id,
        &underlying,
        &maturity,
        &config_id,
    );
    pm.set_risk_control(&admin, &risk_id);
    risk.add_consumer(&admin, &pm_id);

    pt.set_minter(&admin, &pm_id);
    yt.set_minter(&admin, &pm_id);

    let pool_id = match pool_wasm {
        Some(wasm) => env.register(wasm, ()),
        None => env.register(MarketPoolContract, ()),
    };
    let pool = MarketPoolContractClient::new(&env, &pool_id);
    pool.initialize(&admin, &pm_id, &cfg.time_stretch_years);

    let escrow_id = env.register(RecoveryEscrowContract, ());
    let escrow = RecoveryEscrowContractClient::new(&env, &escrow_id);
    escrow.initialize(&underlying, &sy_id, &pt_id, &yt_id, &pm_id, &pool_id);
    sy.set_recovery_escrow(&admin, &escrow_id);
    pt.set_recovery_escrow(&admin, &escrow_id);
    yt.set_recovery_escrow(&admin, &escrow_id);
    pool.set_recovery_escrow(&admin, &escrow_id);

    let router_id = env.register(RouterContract, ());
    let router = RouterContractClient::new(&env, &router_id);
    router.initialize(&admin);
    router.register_market(&admin, &pool_id);

    // Every protocol contract that ever holds SY/PT/YT needs both compliance layers, exactly like
    // any other participant: PrincipalManager (SY custody), MarketPool (reserves), RecoveryEscrow
    // (seized positions). The Router holds nothing and needs no standing at all.
    let mut stack = Stack {
        env,
        admin,
        protocol_admin,
        treasury,
        underlying,
        oracle,
        perm,
        risk,
        config,
        sy,
        pt,
        yt,
        pm,
        pool,
        router,
        escrow,
        maturity,
        kind,
        rwa,
    };
    let s = &stack;
    // SYWrapper custodies the underlying itself, so it needs underlying-level standing too (for an
    // RWA token: a verified identity, just like an investor).
    s.perm.grant_account(&s.admin, &sy_id);
    s.authorize(&sy_id);
    for holder in [&pm_id, &pool_id, &escrow_id] {
        s.perm.grant_account(&s.admin, holder);
        s.perm.grant_asset(&s.admin, holder, &pt_id);
        s.perm.grant_asset(&s.admin, holder, &yt_id);
        s.authorize(holder);
    }
    // Fee recipients receive SY when fees are claimed. (The SAC issuer account is always
    // authorized on its own asset; an RWA operator still needs a verified identity.)
    for recipient in [&s.admin, &s.treasury] {
        s.perm.grant_account(&s.admin, recipient);
        s.authorize(recipient);
    }
    let _ = &mut stack;
    stack
}

/// Grants `user` both compliance layers (Permissioning account + per-asset PT/YT, and underlying
/// authorization) needed to hold and move SY, PT, YT and LP.
pub fn grant_user(s: &Stack, user: &Address) {
    s.perm.grant_account(&s.admin, user);
    s.perm.grant_asset(&s.admin, user, &s.pt.address);
    s.perm.grant_asset(&s.admin, user, &s.yt.address);
    s.authorize(user);
}

/// Mints `amount` of the underlying to `user` and deposits it into SYWrapper, returning the SY
/// shares received. `user` must already be granted.
pub fn deposit_sy(s: &Stack, user: &Address, amount: i128) -> i128 {
    s.mint_underlying(user, amount);
    s.sy.deposit(user, &amount, &0)
}

/// Seed the pool the way a liquidity provider would: tokenize `notional` of underlying into PT +
/// YT, then deposit the resulting PT with `sy_in` fresh SY. `sy_in` (valued at the oracle rate)
/// against `notional` PT sets the opening PT price -- e.g. `sy_in = 95%` of `notional` opens at
/// roughly a 5% discount. Returns `(lp_minted, lp_provider)`; the provider keeps the YT.
pub fn seed_pool(s: &Stack, notional: i128, sy_in: i128) -> (i128, Address) {
    let lp = s.new_user();
    let shares = deposit_sy(s, &lp, notional + sy_in);
    let minted = s.pm.mint(&lp, &notional.min(shares - sy_in));
    let (_, _, lp_out) = s.pool.add_liquidity(&lp, &minted.pt_minted, &sy_in, &0);
    (lp_out, lp)
}

/// Absolute difference helper for tolerance assertions.
pub fn abs_diff(a: i128, b: i128) -> i128 {
    (a - b).abs()
}

/// What a `try_*` call returns for the contract-defined error `code`.
pub fn err_code(code: u32) -> Option<Result<soroban_sdk::Error, soroban_sdk::InvokeError>> {
    Some(Ok(soroban_sdk::Error::from_contract_error(code)))
}

/// Raw token balance of `who` in the underlying.
pub fn underlying_balance(s: &Stack, who: &Address) -> i128 {
    s.underlying_client().balance(who)
}
