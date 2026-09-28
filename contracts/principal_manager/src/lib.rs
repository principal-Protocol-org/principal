//! PrincipalManager — tokenization engine for the Principal Protocol.
//!
//! # Responsibilities
//! * Mint PT (Principal Token) and YT (Yield Token) when a user splits SY shares.
//! * Recombine equal PT + YT back into SY before maturity.
//! * Freeze the market's settlement rate at maturity (`settle_all`) and, from then on, burn PT/YT
//!   and release the underlying to redeemers.
//! * Charge and account the market's tokenization and YT fees.
//! * Enforce maturity, oracle freshness, permissioning and the circuit breaker on every operation.
//!
//! # Accounting (all values use SCALE = 1e7)
//!
//! When `n` SY shares are deposited at oracle rate `R` (USDC per underlying, scaled) and the
//! market's tokenization fee is `f` bps:
//!   fee_shares = ceil(n * f / 10_000)           (kept in custody, split protocol / creator)
//!   notional   = (n - fee_shares) * R / SCALE
//!
//! PT minted  = notional   (redeemable for `pt * SCALE / final_rate` underlying at maturity)
//! YT minted  = notional   (captures yield above the rate at issuance, via YTToken's own index)
//!
//! At maturity, given the frozen settlement rate `R_final` (see `settle_all`):
//!   PT redemption (underlying) = floor(pt_amount * SCALE / R_final)
//!   YT redemption (underlying) = whatever `YTToken.claim_yield` settles and returns, less the YT
//!                                fee (see below)
//!
//! # Fees
//! Rates come from the market's `MarketConfig`, so the underlying's issuer can retune them. All
//! fees are taken in SY shares and stay in this contract's custody until claimed:
//! * **Tokenization fee** -- on the SY tokenized at `mint`.
//! * **YT fee** -- `yt_fee_bps` of every yield payout (`claim_yield` and the YT leg of `redeem`).
//!
//! Each fee is split at accrual time by the config's protocol share (initially 20% Principal /
//! 80% market creator). `claim_protocol_fees` pays the treasury; `claim_creator_fees` pays the
//! creator, i.e. the underlying's *current* issuer authority. Both are permissionless: the payee
//! is fixed by configuration, so nobody can redirect them by calling.
//!
//! # Maturity settlement
//! `settle_all` is permissionless. At or after maturity it advances `YTToken`'s yield index one
//! last time from the fresh oracle -- which freezes it -- and records that rate as the settlement
//! rate. Every redemption, by every holder, then uses that one number, so PT and YT are always
//! settled consistently and the first redeemer is not advantaged. `redeem` settles implicitly if
//! nobody called `settle_all` yet.
//!
//! # Recombination
//! Before maturity, `recombine(amount)` burns `amount` PT and `amount` YT and returns
//! `amount * SCALE / R` SY shares at the *current* oracle rate `R`. The shortfall against the
//! shares originally deposited is exactly the yield the YT position already accrued, which stays
//! claimable through `claim_yield`.
//!
//! # Why YT redemption delegates to YTToken instead of computing its own formula
//! An earlier version of this contract computed YT's payout itself, from a per-user rate
//! recorded at mint time. That is numerically right for one unbroken holding period, but
//! `YTToken` is backed by its own continuously-compounding index, and two independent payers for
//! one claim can both pay for the same accrued yield. Redemption therefore calls
//! `YTToken.update_yield_index` then `claim_yield` and treats its return value as authoritative.
//!
//! # `claim_yield` — accrual without redemption
//! `YTToken.claim_yield` is minter-gated (only `PrincipalManager` can call it), so this contract
//! is the sole place that turns a settled claim into a real payment. Its own `claim_yield` lets a
//! holder collect accrued yield without burning YT or waiting for maturity, paying via
//! `SYWrapper.withdraw` in the same call. Found during audit review (H-03).
//!
//! # Circuit breaker
//! Once `set_risk_control` is called, `mint` reports the underlying value it tokenizes to
//! `RiskControl.check_deposit`; an over-limit mint reverts inside the same transaction.
//!
//! # Compliance — authorization inheritance and market creation
//! `mint`, `redeem`, `recombine` and `claim_yield` check both the underlying's own authorization
//! (via `principal_compliance`: SAC `authorized()` for classic/SEP-8 assets, frozen flag + identity
//! verifier for SEP-57 RWA tokens) and `Permissioning.is_allowed`. `initialize` requires `admin`
//! to be the underlying's issuer authority, so a market can only be created with the issuer's
//! participation -- see `SYWrapper`'s module docs.
//!
//! # Integration scope
//! `mint` takes real custody of the caller's SY shares via `SYWrapper.transfer` and mints real
//! `PTToken`/`YTToken` balances. `redeem` burns those real balances and releases real underlying
//! via `SYWrapper.withdraw`, self-authorizing as this contract's own address. This contract's own
//! address must itself be authorized on the underlying and Permissioning-granted before deployment
//! is usable, since it is a genuine SY-share holder between mint and redemption.
//!
//! SY share custody is converted to/from underlying amounts via `SYWrapper.exchange_rate()`
//! (shares-to-underlying), a *different* rate from the oracle's USDC-per-underlying feed used for
//! the PT/YT notional split. For a price-appreciating asset like USDY, where holding the token
//! doesn't change its own balance, `SYWrapper`'s exchange rate stays at 1.0 and that conversion is
//! a no-op; reconciling the two for a balance-rebasing asset is out of scope until one is onboarded.

#![no_std]
#![allow(deprecated)]
// `env.events().publish` / `register_contract`: migration to `#[contractevent]` is tracked separately; event topics are kept stable for indexers.
#![allow(clippy::too_many_arguments)] // Soroban `initialize` entrypoints wire many contracts by design.

use soroban_sdk::{
    contract, contractclient, contracterror, contractimpl, contracttype, panic_with_error,
    symbol_short, Address, Env,
};

use principal_compliance as compliance;

pub const SCALE: i128 = 10_000_000; // 1e7
pub const BPS: i128 = 10_000;

/// Maximum seconds the oracle price may be stale at mint, claim and settlement.
const MAX_ORACLE_STALENESS_SECS: u64 = 3_600;

// ---------------------------------------------------------------------------
// External contract interfaces (used for cross-contract calls)
// ---------------------------------------------------------------------------

/// Minimum interface required from the OracleAdapter.
#[contractclient(name = "OracleClient")]
pub trait OracleInterface {
    fn get_reference_value(env: Env) -> i128;
    fn is_fresh(env: Env, max_stale_seconds: u64) -> bool;
}

/// Minimum interface required from the Permissioning contract.
#[contractclient(name = "PermClient")]
pub trait PermissioningInterface {
    fn is_allowed(env: Env, account: Address) -> bool;
}

/// Minimum interface required from SYWrapper.
#[contractclient(name = "SYWrapperClient")]
pub trait SYWrapperInterface {
    fn transfer(env: Env, from: Address, to: Address, amount: i128) -> i128;
    fn withdraw(
        env: Env,
        from: Address,
        shares: i128,
        to: Address,
        min_underlying_out: i128,
    ) -> i128;
    fn exchange_rate(env: Env) -> i128;
    fn underlying_address(env: Env) -> Address;
    fn permissioning_address(env: Env) -> Address;
}

/// Minimum interface required from PTToken.
#[contractclient(name = "PTTokenClient")]
pub trait PTTokenInterface {
    fn mint(env: Env, to: Address, amount: i128);
    fn burn(env: Env, from: Address, amount: i128);
    fn balance(env: Env, account: Address) -> i128;
    fn total_supply(env: Env) -> i128;
    fn underlying_address(env: Env) -> Address;
    fn permissioning_address(env: Env) -> Address;
    fn maturity(env: Env) -> u64;
}

/// Minimum interface required from YTToken.
#[contractclient(name = "YTTokenClient")]
pub trait YTTokenInterface {
    fn mint(env: Env, to: Address, amount: i128);
    fn burn(env: Env, from: Address, amount: i128);
    fn update_yield_index(env: Env);
    fn claim_yield(env: Env, caller: Address, from: Address) -> i128;
    fn balance(env: Env, account: Address) -> i128;
    fn total_supply(env: Env) -> i128;
    fn underlying_address(env: Env) -> Address;
    fn permissioning_address(env: Env) -> Address;
    fn oracle_address(env: Env) -> Address;
    fn maturity(env: Env) -> u64;
    fn last_oracle_rate(env: Env) -> i128;
}

/// Minimum interface required from MarketConfig.
#[contractclient(name = "ConfigClient")]
pub trait MarketConfigInterface {
    fn underlying(env: Env) -> Address;
    fn maturity(env: Env) -> u64;
    fn tokenization_fee_bps(env: Env) -> u32;
    fn yt_fee_bps(env: Env) -> u32;
    fn treasury(env: Env) -> Address;
    fn creator(env: Env) -> Address;
    fn split(env: Env, fee: i128) -> (i128, i128);
}

/// Minimum interface required from RiskControl.
#[contractclient(name = "RiskControlClient")]
pub trait RiskControlInterface {
    fn check_deposit(env: Env, caller: Address, asset: Address, amount: i128);
}

// ---------------------------------------------------------------------------
// Error codes
// ---------------------------------------------------------------------------

#[contracterror]
#[derive(Copy, Clone, Debug, PartialEq)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    Unauthorized = 2,
    NotInitialized = 3,
    ZeroAmount = 4,
    NotMature = 5,
    AlreadyMature = 6,
    OracleStale = 7,
    Paused = 9,
    PermissionDenied = 10,
    NotAuthorizedOnSac = 11,
    IssuerMismatch = 12,
    TopologyMismatch = 13,
    NothingToClaim = 14,
    ArithmeticOverflow = 15,
}

// ---------------------------------------------------------------------------
// Storage key schema
// ---------------------------------------------------------------------------

#[contracttype]
pub enum DataKey {
    Admin,
    SYWrapper,
    PTToken,
    YTToken,
    Oracle,
    Permissioning,
    Underlying, // Address of the underlying asset, used for authorization inheritance
    Maturity,   // u64 unix timestamp
    Paused,
    Config,          // MarketConfig address
    RiskControl,     // absent until set_risk_control
    SettledRate,     // i128, absent until settle_all / first redeem after maturity
    AccruedProtocol, // i128 SY shares owed to the treasury
    AccruedCreator,  // i128 SY shares owed to the market creator
}

// ---------------------------------------------------------------------------
// Return types
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone)]
pub struct MintResult {
    pub pt_minted: i128,
    pub yt_minted: i128,
    /// SY shares withheld as the tokenization fee.
    pub fee_shares: i128,
}

#[contracttype]
#[derive(Clone)]
pub struct RedeemResult {
    pub underlying_from_pt: i128,
    pub underlying_from_yt: i128,
}

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

#[contract]
pub struct PrincipalManagerContract;

#[contractimpl]
impl PrincipalManagerContract {
    /// One-time initialization.
    ///
    /// * `sy_wrapper`    — address of the SYWrapper contract
    /// * `pt_token`      — address of the PTToken contract (this contract must later be
    ///                     registered as its minter via `PTToken.set_minter`)
    /// * `yt_token`      — address of the YTToken contract (same two-phase pattern)
    /// * `oracle`        — address of the OracleAdapter contract
    /// * `permissioning` — address of the Permissioning contract
    /// * `underlying`    — address of the underlying asset; `admin` must be its issuer authority
    /// * `maturity`      — Unix timestamp at which PT and YT can be redeemed
    /// * `market_config` — the market's MarketConfig (fees and fee split); its underlying and
    ///                     maturity must match
    ///
    /// `admin` must be the underlying's issuer authority (for a SAC: `admin()`, read live) and
    /// must authorize this call -- creating a new market (even a new maturity on an existing
    /// asset) requires the issuer's participation, matching `SYWrapper`'s market-creation gate.
    #[allow(clippy::too_many_arguments)]
    pub fn initialize(
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
    ) {
        if env.storage().instance().has(&DataKey::Admin) {
            panic_with_error!(&env, Error::AlreadyInitialized);
        }
        admin.require_auth();
        compliance::init(&env, &underlying);
        if !compliance::is_authority(&env, &underlying, &admin) {
            panic_with_error!(&env, Error::IssuerMismatch);
        }
        Self::assert_topology_matches(
            &env,
            &sy_wrapper,
            &pt_token,
            &yt_token,
            &oracle,
            &permissioning,
            &underlying,
            maturity,
            &market_config,
        );
        let s = env.storage().instance();
        s.set(&DataKey::Admin, &admin);
        s.set(&DataKey::SYWrapper, &sy_wrapper);
        s.set(&DataKey::PTToken, &pt_token);
        s.set(&DataKey::YTToken, &yt_token);
        s.set(&DataKey::Oracle, &oracle);
        s.set(&DataKey::Permissioning, &permissioning);
        s.set(&DataKey::Underlying, &underlying);
        s.set(&DataKey::Maturity, &maturity);
        s.set(&DataKey::Config, &market_config);
        s.set(&DataKey::Paused, &false);
        s.set(&DataKey::AccruedProtocol, &0_i128);
        s.set(&DataKey::AccruedCreator, &0_i128);
    }

    // --- core protocol operations ---

    /// Split `sy_shares` into PT + YT. The caller must already hold these shares in the
    /// SYWrapper and must authorize both this call and the resulting SYWrapper transfer. The
    /// tokenization fee is withheld from `sy_shares` before the PT/YT notional is computed.
    ///
    /// Returns the PT and YT minted (equal at issuance) and the fee withheld.
    pub fn mint(env: Env, from: Address, sy_shares: i128) -> MintResult {
        from.require_auth();
        Self::assert_not_paused(&env);
        Self::assert_not_mature(&env);
        Self::assert_oracle_fresh(&env);
        if sy_shares <= 0 {
            panic_with_error!(&env, Error::ZeroAmount);
        }

        // Verify the caller clears both compliance layers: the mandatory floor inherited from
        // the underlying, and Principal's own optional additional narrowing.
        Self::assert_sac_authorized(&env, &from);
        Self::assert_permitted(&env, &from);

        let sy_wrapper = Self::get_sy_wrapper(&env);
        let sy_client = SYWrapperClient::new(&env, &sy_wrapper);
        let this_contract = env.current_contract_address();

        // Circuit breaker: report the underlying value being tokenized. A breach reverts here.
        if let Some(rc) = env
            .storage()
            .instance()
            .get::<_, Address>(&DataKey::RiskControl)
        {
            let underlying_value = sy_shares * sy_client.exchange_rate() / SCALE;
            let amount = if underlying_value > 0 {
                underlying_value
            } else {
                sy_shares
            };
            RiskControlClient::new(&env, &rc).check_deposit(
                &this_contract,
                &Self::underlying_of(&env),
                &amount,
            );
        }

        // Tokenization fee, rounded up so it is never zero on a non-zero fee rate.
        let fee_bps = Self::config(&env).tokenization_fee_bps() as i128;
        let fee_shares = (sy_shares * fee_bps + BPS - 1) / BPS;
        let net_shares = sy_shares - fee_shares;

        // Compute notional principal: net shares valued at the current oracle rate.
        let rate = Self::get_oracle_rate(&env);
        let notional = net_shares * rate / SCALE;
        if notional <= 0 {
            panic_with_error!(&env, Error::ZeroAmount);
        }

        // Take custody of ALL the caller's SY shares (net + fee) -- this contract holds them
        // until redemption / fee claim. `from` already authorized this call above; that covers
        // this nested SYWrapper invocation for the same address in the same transaction.
        sy_client.transfer(&from, &this_contract, &sy_shares);
        Self::accrue_fee(&env, fee_shares);

        // Mint real PT and YT (1:1 with notional) -- this contract must already be the
        // registered minter on both (set via `set_minter` after this contract is deployed).
        let pt_token = Self::get_pt_token(&env);
        let yt_token = Self::get_yt_token(&env);
        let yt_client = YTTokenClient::new(&env, &yt_token);
        // Bring the global yield factor current BEFORE crediting the new balance, so a fresh mint
        // can never retroactively earn a PRIOR factor movement. Found during audit review (H-01).
        yt_client.update_yield_index();
        PTTokenClient::new(&env, &pt_token).mint(&from, &notional);
        yt_client.mint(&from, &notional);

        env.events().publish(
            (symbol_short!("mint"),),
            (from, sy_shares, notional, fee_shares),
        );

        MintResult {
            pt_minted: notional,
            yt_minted: notional,
            fee_shares,
        }
    }

    /// Before maturity: burn `amount` PT and `amount` YT and return `amount * SCALE / rate` SY
    /// shares at the current oracle rate. Yield the YT already accrued stays claimable.
    pub fn recombine(env: Env, from: Address, amount: i128) -> i128 {
        from.require_auth();
        Self::assert_not_paused(&env);
        Self::assert_not_mature(&env);
        Self::assert_oracle_fresh(&env);
        if amount <= 0 {
            panic_with_error!(&env, Error::ZeroAmount);
        }
        Self::assert_sac_authorized(&env, &from);
        Self::assert_permitted(&env, &from);

        let rate = Self::get_oracle_rate(&env);
        let shares = amount * SCALE / rate;
        if shares <= 0 {
            panic_with_error!(&env, Error::ZeroAmount);
        }

        let yt_client = YTTokenClient::new(&env, &Self::get_yt_token(&env));
        yt_client.update_yield_index();
        // Burn settles the account's pending YT yield first, so it survives as a claim.
        yt_client.burn(&from, &amount);
        PTTokenClient::new(&env, &Self::get_pt_token(&env)).burn(&from, &amount);

        SYWrapperClient::new(&env, &Self::get_sy_wrapper(&env)).transfer(
            &env.current_contract_address(),
            &from,
            &shares,
        );

        env.events()
            .publish((symbol_short!("recombine"),), (from, amount, shares));
        shares
    }

    /// Freeze the market's settlement rate. Permissionless; callable once the market has matured
    /// and the oracle is fresh. Advances (and thereby freezes) `YTToken`'s index and records its
    /// final rate as the rate every redemption uses. Idempotent: later calls return the recorded
    /// rate.
    pub fn settle_all(env: Env) -> i128 {
        Self::assert_mature(&env);
        Self::settle(&env)
    }

    /// Redeem PT and/or YT after maturity. Both can be supplied in any combination.
    ///
    /// * `pt_amount` — PT tokens to burn (0 = skip PT redemption)
    /// * `yt_amount` — YT tokens to burn (0 = skip YT redemption)
    ///
    /// Burns the caller's real PT/YT balances and releases real underlying via SYWrapper at the
    /// frozen settlement rate. Returns the underlying units actually transferred for each type.
    pub fn redeem(env: Env, from: Address, pt_amount: i128, yt_amount: i128) -> RedeemResult {
        from.require_auth();
        Self::assert_not_paused(&env);
        Self::assert_mature(&env);
        // Same two-layer check as mint(): the underlying's own authorization is the mandatory
        // floor, Permissioning is an optional additional layer.
        Self::assert_sac_authorized(&env, &from);
        Self::assert_permitted(&env, &from);

        if pt_amount < 0 || yt_amount < 0 || (pt_amount == 0 && yt_amount == 0) {
            panic_with_error!(&env, Error::ZeroAmount);
        }

        let final_rate = Self::settle(&env);
        let sy_wrapper = Self::get_sy_wrapper(&env);
        let sy_client = SYWrapperClient::new(&env, &sy_wrapper);
        let this_contract = env.current_contract_address();

        let mut from_pt = 0_i128;
        let mut from_yt = 0_i128;

        if pt_amount > 0 {
            let pt_token = Self::get_pt_token(&env);
            PTTokenClient::new(&env, &pt_token).burn(&from, &pt_amount);

            // PT: notional units → underlying = floor(pt_amount * SCALE / final_rate)
            let desired_underlying = pt_amount * SCALE / final_rate;
            let shares = Self::underlying_to_shares(sy_client.exchange_rate(), desired_underlying);
            if shares > 0 {
                from_pt = sy_client.withdraw(&this_contract, &shares, &from, &0);
            }
        }

        if yt_amount > 0 {
            let yt_token = Self::get_yt_token(&env);
            let yt_client = YTTokenClient::new(&env, &yt_token);

            // Bring the index current (a no-op once frozen), then burn (settling pending yield)
            // and claim it. See this module's doc comment for why redemption delegates to
            // YTToken's own accrual/claim mechanism.
            yt_client.update_yield_index();
            yt_client.burn(&from, &yt_amount);
            let gross = yt_client.claim_yield(&this_contract, &from);
            from_yt = Self::pay_yield(&env, &from, gross);
        }

        env.events().publish(
            (symbol_short!("redeem"),),
            (from, pt_amount, yt_amount, from_pt, from_yt),
        );

        RedeemResult {
            underlying_from_pt: from_pt,
            underlying_from_yt: from_yt,
        }
    }

    /// Claim accrued YT yield without redeeming (burning) the underlying YT position. Available
    /// before maturity -- continuous accrual is the point of holding YT ahead of redemption --
    /// unlike `redeem`, this does not require the market to have matured. The YT fee is deducted
    /// from the payout. Returns the underlying paid to `from`.
    pub fn claim_yield(env: Env, from: Address) -> i128 {
        from.require_auth();
        Self::assert_not_paused(&env);
        Self::assert_oracle_fresh_unless_frozen(&env);
        Self::assert_sac_authorized(&env, &from);
        Self::assert_permitted(&env, &from);

        let yt_client = YTTokenClient::new(&env, &Self::get_yt_token(&env));
        let this_contract = env.current_contract_address();

        yt_client.update_yield_index();
        let gross = yt_client.claim_yield(&this_contract, &from);
        let paid = Self::pay_yield(&env, &from, gross);

        env.events()
            .publish((symbol_short!("yt_claim"),), (from, gross, paid));
        paid
    }

    // --- fees ---

    /// Pay Principal's accrued share of the market's fees, in SY shares, to the config's
    /// treasury. Permissionless -- the payee is fixed by configuration. Returns the shares paid.
    pub fn claim_protocol_fees(env: Env) -> i128 {
        Self::claim_fee_bucket(&env, &DataKey::AccruedProtocol, true)
    }

    /// Pay the market creator's accrued share, in SY shares, to the creator (the underlying's
    /// current issuer authority). Permissionless. Returns the shares paid.
    pub fn claim_creator_fees(env: Env) -> i128 {
        Self::claim_fee_bucket(&env, &DataKey::AccruedCreator, false)
    }

    // --- views ---

    pub fn pt_balance(env: Env, account: Address) -> i128 {
        let pt_token = Self::get_pt_token(&env);
        PTTokenClient::new(&env, &pt_token).balance(&account)
    }

    pub fn yt_balance(env: Env, account: Address) -> i128 {
        let yt_token = Self::get_yt_token(&env);
        YTTokenClient::new(&env, &yt_token).balance(&account)
    }

    pub fn total_pt(env: Env) -> i128 {
        let pt_token = Self::get_pt_token(&env);
        PTTokenClient::new(&env, &pt_token).total_supply()
    }

    pub fn total_yt(env: Env) -> i128 {
        let yt_token = Self::get_yt_token(&env);
        YTTokenClient::new(&env, &yt_token).total_supply()
    }

    pub fn maturity(env: Env) -> u64 {
        env.storage()
            .instance()
            .get(&DataKey::Maturity)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized))
    }

    pub fn is_mature(env: Env) -> bool {
        let mat: u64 = env
            .storage()
            .instance()
            .get(&DataKey::Maturity)
            .unwrap_or(u64::MAX);
        env.ledger().timestamp() >= mat
    }

    pub fn underlying_address(env: Env) -> Address {
        Self::underlying_of(&env)
    }

    pub fn sy_wrapper_address(env: Env) -> Address {
        Self::get_sy_wrapper(&env)
    }

    pub fn pt_address(env: Env) -> Address {
        Self::get_pt_token(&env)
    }

    pub fn yt_address(env: Env) -> Address {
        Self::get_yt_token(&env)
    }

    pub fn oracle_address(env: Env) -> Address {
        env.storage()
            .instance()
            .get(&DataKey::Oracle)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized))
    }

    pub fn permissioning_address(env: Env) -> Address {
        env.storage()
            .instance()
            .get(&DataKey::Permissioning)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized))
    }

    pub fn config_address(env: Env) -> Address {
        env.storage()
            .instance()
            .get(&DataKey::Config)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized))
    }

    pub fn risk_control(env: Env) -> Option<Address> {
        env.storage().instance().get(&DataKey::RiskControl)
    }

    /// The frozen settlement rate, once `settle_all` (or the first redemption) has run.
    pub fn settled_rate(env: Env) -> Option<i128> {
        env.storage().instance().get(&DataKey::SettledRate)
    }

    /// SY shares accrued but not yet claimed: (protocol, creator).
    pub fn accrued_fees(env: Env) -> (i128, i128) {
        let s = env.storage().instance();
        (
            s.get(&DataKey::AccruedProtocol).unwrap_or(0),
            s.get(&DataKey::AccruedCreator).unwrap_or(0),
        )
    }

    // --- admin ---

    pub fn set_paused(env: Env, caller: Address, paused: bool) {
        Self::assert_admin(&env, &caller);
        env.storage().instance().set(&DataKey::Paused, &paused);
        env.events().publish((symbol_short!("paused"),), paused);
    }

    /// Wire the RiskControl this market reports mints to. This contract must also be registered
    /// as a consumer on that RiskControl or every mint reverts.
    pub fn set_risk_control(env: Env, admin: Address, risk_control: Address) {
        Self::assert_admin(&env, &admin);
        env.storage()
            .instance()
            .set(&DataKey::RiskControl, &risk_control);
        env.events()
            .publish((symbol_short!("rc_set"),), risk_control);
    }

    pub fn transfer_admin(env: Env, current_admin: Address, new_admin: Address) {
        Self::assert_admin(&env, &current_admin);
        env.storage().instance().set(&DataKey::Admin, &new_admin);
        env.events()
            .publish((symbol_short!("adm_xfer"),), (current_admin, new_admin));
    }

    pub fn get_admin(env: Env) -> Address {
        Self::require_admin(&env)
    }

    // --- internal helpers ---

    /// Verifies the configured SY/PT/YT/config contracts actually belong together before this
    /// market ever accepts a deposit: same underlying, same permissioning contract, PT/YT/config
    /// maturity matching this market's own, and YTToken's oracle matching this market's own.
    /// Found during audit review (M-01).
    #[allow(clippy::too_many_arguments)]
    fn assert_topology_matches(
        env: &Env,
        sy_wrapper: &Address,
        pt_token: &Address,
        yt_token: &Address,
        oracle: &Address,
        permissioning: &Address,
        underlying: &Address,
        maturity: u64,
        market_config: &Address,
    ) {
        let sy_client = SYWrapperClient::new(env, sy_wrapper);
        let pt_client = PTTokenClient::new(env, pt_token);
        let yt_client = YTTokenClient::new(env, yt_token);
        let config = ConfigClient::new(env, market_config);

        if sy_client.underlying_address() != *underlying
            || pt_client.underlying_address() != *underlying
            || yt_client.underlying_address() != *underlying
            || config.underlying() != *underlying
        {
            panic_with_error!(env, Error::TopologyMismatch);
        }

        if sy_client.permissioning_address() != *permissioning
            || pt_client.permissioning_address() != *permissioning
            || yt_client.permissioning_address() != *permissioning
        {
            panic_with_error!(env, Error::TopologyMismatch);
        }

        if pt_client.maturity() != maturity
            || yt_client.maturity() != maturity
            || config.maturity() != maturity
        {
            panic_with_error!(env, Error::TopologyMismatch);
        }

        if yt_client.oracle_address() != *oracle {
            panic_with_error!(env, Error::TopologyMismatch);
        }
    }

    /// Record (once) and return the frozen settlement rate. Requires maturity to have passed.
    fn settle(env: &Env) -> i128 {
        if let Some(rate) = env
            .storage()
            .instance()
            .get::<_, i128>(&DataKey::SettledRate)
        {
            return rate;
        }
        Self::assert_oracle_fresh(env);
        let yt_client = YTTokenClient::new(env, &Self::get_yt_token(env));
        // The first update at/after maturity advances the index a final time and freezes it.
        yt_client.update_yield_index();
        let rate = yt_client.last_oracle_rate();
        env.storage().instance().set(&DataKey::SettledRate, &rate);
        env.events().publish((symbol_short!("settled"),), rate);
        rate
    }

    /// Turn a gross yield entitlement (underlying units) into a payment to `to`: withhold the YT
    /// fee in SY shares, withdraw the rest. Returns the underlying actually paid.
    fn pay_yield(env: &Env, to: &Address, gross_underlying: i128) -> i128 {
        if gross_underlying <= 0 {
            return 0;
        }
        let sy_client = SYWrapperClient::new(env, &Self::get_sy_wrapper(env));
        let gross_shares = Self::underlying_to_shares(sy_client.exchange_rate(), gross_underlying);
        let fee_bps = Self::config(env).yt_fee_bps() as i128;
        let fee_shares = (gross_shares * fee_bps + BPS - 1) / BPS;
        let net_shares = gross_shares - fee_shares;
        Self::accrue_fee(env, fee_shares);
        if net_shares <= 0 {
            return 0;
        }
        sy_client.withdraw(&env.current_contract_address(), &net_shares, to, &0)
    }

    /// Split `fee_shares` by the market's protocol share and add to the two accrual buckets.
    fn accrue_fee(env: &Env, fee_shares: i128) {
        if fee_shares <= 0 {
            return;
        }
        let (protocol, creator) = Self::config(env).split(&fee_shares);
        let s = env.storage().instance();
        let p: i128 = s.get(&DataKey::AccruedProtocol).unwrap_or(0);
        let c: i128 = s.get(&DataKey::AccruedCreator).unwrap_or(0);
        s.set(&DataKey::AccruedProtocol, &(p + protocol));
        s.set(&DataKey::AccruedCreator, &(c + creator));
    }

    fn claim_fee_bucket(env: &Env, key: &DataKey, protocol: bool) -> i128 {
        let owed: i128 = env.storage().instance().get(key).unwrap_or(0);
        if owed <= 0 {
            panic_with_error!(env, Error::NothingToClaim);
        }
        // Effects before interaction.
        env.storage().instance().set(key, &0_i128);
        let config = Self::config(env);
        let payee = if protocol {
            config.treasury()
        } else {
            config.creator()
        };
        SYWrapperClient::new(env, &Self::get_sy_wrapper(env)).transfer(
            &env.current_contract_address(),
            &payee,
            &owed,
        );
        env.events()
            .publish((symbol_short!("fee_claim"),), (payee, owed, protocol));
        owed
    }

    fn config(env: &Env) -> ConfigClient<'_> {
        let addr: Address = env
            .storage()
            .instance()
            .get(&DataKey::Config)
            .unwrap_or_else(|| panic_with_error!(env, Error::NotInitialized));
        ConfigClient::new(env, &addr)
    }

    fn underlying_of(env: &Env) -> Address {
        env.storage()
            .instance()
            .get(&DataKey::Underlying)
            .unwrap_or_else(|| panic_with_error!(env, Error::NotInitialized))
    }

    fn get_oracle_rate(env: &Env) -> i128 {
        let oracle_addr: Address = env
            .storage()
            .instance()
            .get(&DataKey::Oracle)
            .unwrap_or_else(|| panic_with_error!(env, Error::NotInitialized));
        OracleClient::new(env, &oracle_addr).get_reference_value()
    }

    fn assert_oracle_fresh(env: &Env) {
        let oracle_addr: Address = env
            .storage()
            .instance()
            .get(&DataKey::Oracle)
            .unwrap_or_else(|| panic_with_error!(env, Error::NotInitialized));
        if !OracleClient::new(env, &oracle_addr).is_fresh(&MAX_ORACLE_STALENESS_SECS) {
            panic_with_error!(env, Error::OracleStale);
        }
    }

    /// After settlement the YT index is frozen and no longer reads the oracle, so a stale oracle
    /// must not block holders from claiming what they already earned.
    fn assert_oracle_fresh_unless_frozen(env: &Env) {
        if env.storage().instance().has(&DataKey::SettledRate) {
            return;
        }
        Self::assert_oracle_fresh(env);
    }

    fn assert_permitted(env: &Env, account: &Address) {
        let perm_addr: Address = env
            .storage()
            .instance()
            .get(&DataKey::Permissioning)
            .unwrap_or_else(|| panic_with_error!(env, Error::NotInitialized));
        if !PermClient::new(env, &perm_addr).is_allowed(account) {
            panic_with_error!(env, Error::PermissionDenied);
        }
    }

    fn assert_sac_authorized(env: &Env, account: &Address) {
        let underlying = Self::underlying_of(env);
        if !compliance::is_authorized(env, &underlying, account) {
            panic_with_error!(env, Error::NotAuthorizedOnSac);
        }
    }

    fn get_sy_wrapper(env: &Env) -> Address {
        env.storage()
            .instance()
            .get(&DataKey::SYWrapper)
            .unwrap_or_else(|| panic_with_error!(env, Error::NotInitialized))
    }

    fn get_pt_token(env: &Env) -> Address {
        env.storage()
            .instance()
            .get(&DataKey::PTToken)
            .unwrap_or_else(|| panic_with_error!(env, Error::NotInitialized))
    }

    fn get_yt_token(env: &Env) -> Address {
        env.storage()
            .instance()
            .get(&DataKey::YTToken)
            .unwrap_or_else(|| panic_with_error!(env, Error::NotInitialized))
    }

    /// Convert a desired underlying payout into the SY shares needed to withdraw it, at
    /// SYWrapper's current exchange rate (a different rate from the Oracle's pricing feed --
    /// see this module's doc comment). Inverts SYWrapper's own `shares * rate / RATE_SCALE`.
    fn underlying_to_shares(exchange_rate: i128, underlying: i128) -> i128 {
        underlying * SCALE / exchange_rate
    }

    fn require_admin(env: &Env) -> Address {
        env.storage()
            .instance()
            .get(&DataKey::Admin)
            .unwrap_or_else(|| panic_with_error!(env, Error::NotInitialized))
    }

    fn assert_admin(env: &Env, caller: &Address) {
        caller.require_auth();
        if *caller != Self::require_admin(env) {
            panic_with_error!(env, Error::Unauthorized);
        }
    }

    fn assert_not_paused(env: &Env) {
        if env
            .storage()
            .instance()
            .get(&DataKey::Paused)
            .unwrap_or(false)
        {
            panic_with_error!(env, Error::Paused);
        }
    }

    fn assert_mature(env: &Env) {
        let mat: u64 = env
            .storage()
            .instance()
            .get(&DataKey::Maturity)
            .unwrap_or(u64::MAX);
        if env.ledger().timestamp() < mat {
            panic_with_error!(env, Error::NotMature);
        }
    }

    fn assert_not_mature(env: &Env) {
        let mat: u64 = env
            .storage()
            .instance()
            .get(&DataKey::Maturity)
            .unwrap_or(u64::MAX);
        if env.ledger().timestamp() >= mat {
            panic_with_error!(env, Error::AlreadyMature);
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod test;
