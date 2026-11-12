//! MarketPool — the time-aware PT/SY yield-curve AMM for one market (one underlying, one
//! maturity).
//!
//! # What it trades
//! Reserves are PT and SY shares. Swaps move along the *Yield Space* constant-power-sum curve
//! `x^a + y^a = k` with `a = 1 - tau / stretch` (see [`math`]): `x` is the SY reserve valued in
//! notional (`shares x oracle_rate / SCALE`, the same rate `PrincipalManager` uses to size PT), `y`
//! is the PT reserve. As maturity approaches `a -> 1`, the curve flattens to a constant sum, and PT
//! converges to exactly one unit of SY value -- par -- with no special-case code. LPs hold a fixed
//! basket of PT and SY, so the curve sliding towards par is pure gain for them: no time-decay
//! impermanent loss.
//!
//! **Unit discipline.** The curve is always evaluated in *notional value* units. SY shares are
//! converted with the oracle rate on the way in and back on the way out, rounding in the pool's
//! favor at both boundaries. (A comparable protocol shipped a curve that mixed raw shares with
//! asset units and leaked value to traders once the SY rate moved off 1.0; the tests here sweep the
//! rate.)
//!
//! # Fees
//! Trading fee = `Fee Tier x Days to Maturity / 365`, from the market's `MarketConfig`
//! (`swap_fee_rate`), so it is largest at issuance and falls to zero at maturity. It is charged in
//! SY shares -- on the SY going in when buying PT, on the SY coming out when selling PT -- and is
//! *not* added to the reserves: it accrues to two buckets split by the config's protocol share
//! (initially 20% Principal / 80% market creator). `claim_protocol_fees` / `claim_creator_fees` pay
//! the treasury and the creator, permissionlessly. LPs earn from PT convergence, not from fees.
//!
//! # Liquidity
//! LP positions are internal balances (`lp_balance`, `transfer_lp`), priced from the pool's
//! *internal* reserves, never from live token balances, so a direct donation cannot skew
//! `add_liquidity`. The first deposit fixes the opening price (implied rate) and permanently locks
//! `MINIMUM_LIQUIDITY` LP. `remove_liquidity` works before and after maturity (the pool pause halts it only while the market is live).
//!
//! # YT trading (flash-mint / flash-redeem)
//! * **Buy YT, capital-up-front** is `Router.swap_sy_for_yt`: mint PT + YT from the user's SY, then
//!   sell the PT into this pool with `swap_pt_for_sy`; the user keeps the YT and the SY proceeds.
//!   The user must hold the full notional being tokenized, not just the net cost.
//! * **Buy YT, true flash swap** is `flash_mint_yt` here: the pool funds the mint leg from its
//!   *own* SY reserves instead of the user's, keeps the PT the mint produces (exactly as if it had
//!   just bought it via `swap_pt_for_sy`), and the user pays only the net cost up front -- the same
//!   amount `Router.swap_sy_for_yt` charges net of its SY proceeds, just advanced differently. See
//!   that function's own docs for the accounting identity and why it holds. Deliberately *not*
//!   named `swap_sy_for_yt` at this level: that name already means something different here
//!   (nothing -- it only exists at the Router level today), and giving a pool-level function the
//!   same name as a Router-level one with a different return shape would invite exactly the
//!   confusion this note is avoiding.
//! * **Sell YT** is `swap_yt_for_sy` here: the pool takes the user's YT, uses `yt_in` of its own PT
//!   reserve to `PrincipalManager.recombine` PT + YT into SY, keeps the curve price of that PT (plus
//!   fee) and pays the user the remainder -- all in one call, so nothing is ever borrowed across
//!   calls and the pool cannot be left short. It reverts if the YT is worth too little to cover the
//!   PT it consumes.
//!
//! # Compliance
//! Trading, adding/removing liquidity and holding/transferring LP all inherit the underlying's
//! compliance: the caller (and every recipient) must be authorized on the underlying (SAC
//! `authorized()` or SEP-57 frozen/identity checks, via `principal_compliance`) and pass
//! Permissioning. The pool's own address must be authorized and Permissioning-granted (also per
//! asset for PT and YT), like `PrincipalManager`. `seize_lp` lets the configured `RecoveryEscrow`
//! move a deauthorized holder's LP position.

#![no_std]
#![allow(deprecated)] // `env.events().publish` / `register_contract`: migration to `#[contractevent]` is tracked separately; event topics are kept stable for indexers.

pub mod math;

use soroban_sdk::{
    auth::{ContractContext, InvokerContractAuthEntry, SubContractInvocation},
    contract, contractclient, contracterror, contractimpl, contracttype, panic_with_error,
    symbol_short, vec, Address, Env, IntoVal, Symbol,
};

use principal_compliance as compliance;

pub const SCALE: i128 = 10_000_000;
/// Basis-point denominator, matching `MarketConfig`'s own `BPS` -- used only to replicate
/// `PrincipalManager.mint`'s tokenization-fee formula for `quote_sy_for_yt`'s preview.
pub const BPS: i128 = 10_000;
/// Fixed-point denominator of the swap fee rate returned by `MarketConfig` (1e12).
pub const FEE_SCALE: i128 = 1_000_000_000_000;
pub const SECONDS_PER_YEAR: u64 = 31_536_000;
/// LP permanently locked by the first deposit, so the pool can never be emptied to zero supply.
pub const MINIMUM_LIQUIDITY: i128 = 1_000;
/// A swap may never leave either reserve below this many raw units.
pub const MIN_RESERVE: i128 = 1_000;
const BALANCE_TTL_LEDGERS: u32 = 518_400;
/// Bisection steps for the single-sided zap split.
const ZAP_ITERATIONS: u32 = 18;

// ---------------------------------------------------------------------------
// External interfaces
// ---------------------------------------------------------------------------

/// Mirrors `PrincipalManager`'s own `MintResult`, field-for-field.
#[contracttype]
#[derive(Clone)]
pub struct MintResult {
    pub pt_minted: i128,
    pub yt_minted: i128,
    pub fee_shares: i128,
}

#[contractclient(name = "ManagerClient")]
pub trait ManagerInterface {
    fn underlying_address(env: Env) -> Address;
    fn sy_wrapper_address(env: Env) -> Address;
    fn pt_address(env: Env) -> Address;
    fn yt_address(env: Env) -> Address;
    fn oracle_address(env: Env) -> Address;
    fn config_address(env: Env) -> Address;
    fn permissioning_address(env: Env) -> Address;
    fn maturity(env: Env) -> u64;
    fn recombine(env: Env, from: Address, amount: i128) -> i128;
    fn mint(env: Env, from: Address, sy_shares: i128) -> MintResult;
}

#[contractclient(name = "SYClient")]
pub trait SYInterface {
    fn transfer(env: Env, from: Address, to: Address, amount: i128) -> i128;
}

#[contractclient(name = "TokenClient")]
pub trait TokenInterface {
    fn transfer(env: Env, from: Address, to: Address, amount: i128);
    fn balance(env: Env, account: Address) -> i128;
}

#[contractclient(name = "OracleClient")]
pub trait OracleInterface {
    fn get_reference_value(env: Env) -> i128;
    fn is_available(env: Env) -> bool;
}

#[contractclient(name = "PermClient")]
pub trait PermissioningInterface {
    fn is_allowed(env: Env, account: Address) -> bool;
}

#[contractclient(name = "ConfigClient")]
pub trait ConfigInterface {
    fn swap_fee_rate(env: Env, seconds_to_maturity: u64) -> i128;
    fn split(env: Env, fee: i128) -> (i128, i128);
    fn treasury(env: Env) -> Address;
    fn creator(env: Env) -> Address;
    fn tokenization_fee_bps(env: Env) -> u32;
}

// ---------------------------------------------------------------------------
// Errors / storage
// ---------------------------------------------------------------------------

/// Instance-storage TTL applied by `bump` (~30 days at 5 s per ledger).
const INSTANCE_TTL_LEDGERS: u32 = 518_400;

#[contracterror]
#[derive(Copy, Clone, Debug, PartialEq)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    Unauthorized = 3,
    IssuerMismatch = 4,
    Paused = 5,
    Expired = 6,
    ZeroAmount = 7,
    SlippageExceeded = 8,
    InsufficientLiquidity = 9,
    InsufficientLpBalance = 10,
    PermissionDenied = 11,
    NotAuthorizedOnSac = 12,
    OracleStale = 13,
    MaturityTooFar = 14,
    InvalidInitialRatio = 15,
    MathError = 16,
    RecoveryEscrowAlreadySet = 17,
    NotRecoveryEscrow = 18,
    NothingToClaim = 19,
    InvalidStretch = 20,
    MinimumLiquidity = 21,
}

#[contracttype]
pub enum DataKey {
    Admin,
    Manager,
    SY,
    PT,
    YT,
    Oracle,
    Config,
    Permissioning,
    Underlying,
    Maturity,
    StretchSecs,
    ReservePT,
    ReserveSY,
    TotalLp,
    Paused,
    RecoveryEscrow,
    AccruedProtocol,
    AccruedCreator,
    Lp(Address),
}

/// A point-in-time view of the pool used by every quote and swap.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct PoolState {
    pub pt_reserve: i128,
    pub sy_reserve: i128,
    pub total_lp: i128,
    /// Oracle rate (notional per underlying unit) at `SCALE`.
    pub rate: i128,
    /// Curve exponent `a` at WAD (1e18).
    pub exponent: i128,
    /// Swap fee at `FEE_SCALE` (1e12) for the current time to maturity.
    pub fee_rate: i128,
    pub seconds_to_maturity: u64,
}

/// The result of a priced trade.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct Quote {
    /// PT or SY the trader receives (for exact-out quotes: the exact amount requested).
    pub amount_out: i128,
    /// SY the trader pays in (for sells: 0).
    pub amount_in: i128,
    /// Fee, in SY shares.
    pub fee_shares: i128,
}

/// Preview of `swap_sy_for_yt`'s two legs (`PrincipalManager.mint` then `MarketPool.swap_pt_for_sy`),
/// reported separately so a caller can show the two fees instead of just a net number.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct FlashMintQuote {
    /// YT (and PT) the mint leg would produce.
    pub yt_out: i128,
    /// SY the sell-PT leg would pay back, before `max_net_cost` is applied by the caller.
    pub sy_back: i128,
    /// Tokenization fee the mint leg would charge, in SY shares.
    pub mint_fee_shares: i128,
    /// Swap fee the sell-PT leg would charge, in SY shares.
    pub swap_fee_shares: i128,
}

#[contract]
pub struct MarketPoolContract;

#[contractimpl]
impl MarketPoolContract {
    /// Permissionless keeper call: extend this contract's *instance* storage (admin, config,
    /// reserves, totals, settlement rate, fee buckets) for ~30 days. Soroban does not bump instance
    /// TTL on ordinary reads or writes, so a long-dated market needs this called periodically (or
    /// an archived instance restored) -- see docs/DEPLOYMENT.md.
    pub fn bump(env: Env) {
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_TTL_LEDGERS, INSTANCE_TTL_LEDGERS);
    }

    /// Create the pool for the market run by `principal_manager`. `admin` must be the underlying's
    /// issuer authority and must authorize the call. Everything else -- underlying, SY, PT, YT,
    /// oracle, config, permissioning, maturity -- is read from the manager, so the pool cannot be
    /// wired to a mismatched set of contracts. `time_stretch_years` (1..=20) sets how gently the
    /// curve flattens; the market's remaining life must stay under 75% of it.
    pub fn initialize(
        env: Env,
        admin: Address,
        principal_manager: Address,
        time_stretch_years: u32,
    ) {
        if env.storage().instance().has(&DataKey::Admin) {
            panic_with_error!(&env, Error::AlreadyInitialized);
        }
        admin.require_auth();
        let pm = ManagerClient::new(&env, &principal_manager);
        let underlying = pm.underlying_address();
        compliance::init(&env, &underlying);
        if !compliance::is_authority(&env, &underlying, &admin) {
            panic_with_error!(&env, Error::IssuerMismatch);
        }
        if time_stretch_years == 0 || time_stretch_years > 20 {
            panic_with_error!(&env, Error::InvalidStretch);
        }
        let stretch_secs = (time_stretch_years as u64) * SECONDS_PER_YEAR;
        let maturity = pm.maturity();
        let remaining = maturity.saturating_sub(env.ledger().timestamp());
        if math::exponent(remaining, stretch_secs)
            .map(|a| a < math::MIN_EXPONENT)
            .unwrap_or(true)
        {
            panic_with_error!(&env, Error::MaturityTooFar);
        }

        let s = env.storage().instance();
        s.set(&DataKey::Admin, &admin);
        s.set(&DataKey::Manager, &principal_manager);
        s.set(&DataKey::SY, &pm.sy_wrapper_address());
        s.set(&DataKey::PT, &pm.pt_address());
        s.set(&DataKey::YT, &pm.yt_address());
        s.set(&DataKey::Oracle, &pm.oracle_address());
        s.set(&DataKey::Config, &pm.config_address());
        s.set(&DataKey::Permissioning, &pm.permissioning_address());
        s.set(&DataKey::Underlying, &underlying);
        s.set(&DataKey::Maturity, &maturity);
        s.set(&DataKey::StretchSecs, &stretch_secs);
        s.set(&DataKey::ReservePT, &0_i128);
        s.set(&DataKey::ReserveSY, &0_i128);
        s.set(&DataKey::TotalLp, &0_i128);
        s.set(&DataKey::Paused, &false);
        s.set(&DataKey::AccruedProtocol, &0_i128);
        s.set(&DataKey::AccruedCreator, &0_i128);
    }

    // ---------------------------------------------------------------- trading

    /// Buy PT with `sy_in` SY shares. `from` pays, `to` receives. Reverts `SlippageExceeded` if
    /// fewer than `min_pt_out` PT would be received.
    pub fn swap_sy_for_pt(
        env: Env,
        from: Address,
        to: Address,
        sy_in: i128,
        min_pt_out: i128,
    ) -> i128 {
        from.require_auth();
        Self::assert_active(&env);
        Self::assert_trader(&env, &from);
        let st = Self::state_fresh(&env);
        let q = Self::calc_buy_pt(&env, &st, sy_in);
        if q.amount_out < min_pt_out {
            panic_with_error!(&env, Error::SlippageExceeded);
        }
        // Effects.
        let net = sy_in - q.fee_shares;
        Self::set_reserves(&env, st.pt_reserve - q.amount_out, st.sy_reserve + net);
        Self::accrue_fee(&env, q.fee_shares);
        // Interactions.
        let this = env.current_contract_address();
        SYClient::new(&env, &Self::addr(&env, &DataKey::SY)).transfer(&from, &this, &sy_in);
        TokenClient::new(&env, &Self::addr(&env, &DataKey::PT)).transfer(&this, &to, &q.amount_out);
        env.events().publish(
            (symbol_short!("buy_pt"),),
            (from, to, sy_in, q.amount_out, q.fee_shares),
        );
        q.amount_out
    }

    /// Sell `pt_in` PT for SY. `from` pays PT, `to` receives SY. Reverts `SlippageExceeded` if less
    /// than `min_sy_out` SY (after the fee) would be received.
    pub fn swap_pt_for_sy(
        env: Env,
        from: Address,
        to: Address,
        pt_in: i128,
        min_sy_out: i128,
    ) -> i128 {
        from.require_auth();
        Self::assert_active(&env);
        Self::assert_trader(&env, &from);
        let st = Self::state_fresh(&env);
        let (q, gross) = Self::calc_sell_pt(&env, &st, pt_in);
        if q.amount_out < min_sy_out {
            panic_with_error!(&env, Error::SlippageExceeded);
        }
        Self::set_reserves(&env, st.pt_reserve + pt_in, st.sy_reserve - gross);
        Self::accrue_fee(&env, q.fee_shares);
        let this = env.current_contract_address();
        TokenClient::new(&env, &Self::addr(&env, &DataKey::PT)).transfer(&from, &this, &pt_in);
        SYClient::new(&env, &Self::addr(&env, &DataKey::SY)).transfer(&this, &to, &q.amount_out);
        env.events().publish(
            (symbol_short!("sell_pt"),),
            (from, to, pt_in, q.amount_out, q.fee_shares),
        );
        q.amount_out
    }

    /// Flash-redeem: sell `yt_in` YT for SY in one call. The pool takes the YT, recombines it with
    /// `yt_in` of its own PT reserve into SY, keeps the curve price of that PT plus the fee, and
    /// pays `to` the rest. Reverts `SlippageExceeded` if the payout would be below `min_sy_out`,
    /// and `InsufficientLiquidity` if the recombined SY does not cover the PT's price.
    pub fn swap_yt_for_sy(
        env: Env,
        from: Address,
        to: Address,
        yt_in: i128,
        min_sy_out: i128,
    ) -> i128 {
        from.require_auth();
        Self::assert_active(&env);
        Self::assert_trader(&env, &from);
        let st = Self::state_fresh(&env);
        let q = Self::calc_buy_exact_pt(&env, &st, yt_in);
        let gross_cost = q.amount_in; // SY the pool must keep: curve input + fee
        let net = gross_cost - q.fee_shares;

        // Effects: the pool consumes `yt_in` PT and keeps `net` SY in reserves.
        Self::set_reserves(&env, st.pt_reserve - yt_in, st.sy_reserve + net);
        Self::accrue_fee(&env, q.fee_shares);

        // Interactions.
        let this = env.current_contract_address();
        TokenClient::new(&env, &Self::addr(&env, &DataKey::YT)).transfer(&from, &this, &yt_in);
        let recombined =
            ManagerClient::new(&env, &Self::addr(&env, &DataKey::Manager)).recombine(&this, &yt_in);
        if recombined < gross_cost {
            panic_with_error!(&env, Error::InsufficientLiquidity);
        }
        let payout = recombined - gross_cost;
        if payout < min_sy_out {
            panic_with_error!(&env, Error::SlippageExceeded);
        }
        if payout > 0 {
            SYClient::new(&env, &Self::addr(&env, &DataKey::SY)).transfer(&this, &to, &payout);
        }
        env.events().publish(
            (symbol_short!("sell_yt"),),
            (from, to, yt_in, payout, q.fee_shares),
        );
        payout
    }

    /// **True flash-mint**: tokenize `sy_in` SY into PT + YT, keep the PT in this pool's own
    /// reserves (exactly as if the pool had just bought it via `swap_pt_for_sy`), and send `to`
    /// only the YT. `from` ends up paying just the *net* cost -- `sy_in` minus what the pool would
    /// have paid for that PT -- not the full `sy_in`, because the pool funds the rest of the mint
    /// from its own reserves instead of from `from`'s balance. `from` must authorize transferring
    /// `max_net_cost` (not `net_cost` -- see "why `max_net_cost` is pulled, not `net_cost`" below);
    /// any unused portion is refunded. Reverts `SlippageExceeded` if fewer than `min_yt_out` YT
    /// would be minted or the net cost would exceed `max_net_cost`.
    ///
    /// Returns `(yt_out, net_cost)`.
    ///
    /// # Why this is safe: the accounting identity
    /// Let `gross` be the SY value `calc_sell_pt(yt_out)` assigns the PT the mint produces (the
    /// same `gross` a real seller of that PT would face), `fee` its swap-fee component, and
    /// `sy_back = gross - fee` (what a real seller would be paid). Reserves move exactly like a
    /// sell-PT trade: `pt_reserve += yt_out`, `sy_reserve -= gross`, `fee` accrues to the fee
    /// buckets. The pool's own token balance only has to give up `sy_back` -- the fee portion was
    /// never going to leave the pool's balance either way, same as in a real `swap_pt_for_sy` where
    /// only `amount_out = gross - fee` is actually transferred out. Pulling `max_net_cost` from
    /// `from` and refunding `max_net_cost - net_cost` nets to exactly `-net_cost` on `from`'s
    /// balance and exactly `-sy_back` on the pool's -- same algebra, worked through in full in
    /// `docs/AMM_FLASH_SWAP_AND_SIDEREAL_REVIEW.md` §5, reordered into two fixed-amount transfers
    /// instead of one derived one (see below). Since `net_cost = sy_in - sy_back` by construction,
    /// `net_cost + sy_back = sy_in`: the buyer's payment plus the pool's own contribution exactly
    /// funds the mint. The existing reserves-plus-fee-buckets-equal-token-balance invariant
    /// (`pool_token_balances_always_match_internal_reserves_plus_fees`) is preserved by the same
    /// algebra that already holds for `swap_pt_for_sy`.
    ///
    /// # Why `max_net_cost` is pulled, not `net_cost`
    /// `net_cost` depends on `calc_sell_pt`, which reads `seconds_to_maturity` live -- the same
    /// curve-derived-value staleness class documented in
    /// `docs/DAPP_SMART_CONTRACT_INTEGRATION_GUIDE.md` §2's third gotcha. A first version of this
    /// function transferred `net_cost` straight from `from`, and reverted `Error(Auth,
    /// InvalidAction)` at submission (not simulation) live on Testnet: the value Soroban had
    /// `from` pre-authorize at simulation time no longer matched what real execution computed
    /// moments later. `max_net_cost` is a plain literal `from` already chooses and signs for
    /// (exactly like every other `min_*_out`/`max_*_cost` in this Router), so pulling that instead
    /// and refunding the unused remainder -- the same fix family as `add_liquidity_single_sy`'s own
    /// `Error(Auth, InvalidAction)` fix -- leaves nothing stale in the authorized invocation.
    ///
    /// # Why `net_cost` cannot be negative
    /// The curve prices PT at or below par throughout the market's life (the first deposit cannot
    /// seed it above par, and it only ever converges upward *to* par at maturity), while minting
    /// costs slightly *more* than par per unit via the tokenization fee. So `sy_back` (PT's curve
    /// value) is always below `sy_in` (what minting that same notional cost), by at least the fee
    /// -- matching `Router.swap_sy_for_yt`'s own documented economics ("YT price ~ 1 - PT price").
    /// Checked defensively anyway: panics `InsufficientLiquidity` rather than trust the inequality
    /// blindly.
    pub fn flash_mint_yt(
        env: Env,
        from: Address,
        to: Address,
        sy_in: i128,
        min_yt_out: i128,
        max_net_cost: i128,
    ) -> (i128, i128) {
        from.require_auth();
        Self::assert_active(&env);
        Self::assert_trader(&env, &from);
        if sy_in <= 0 {
            panic_with_error!(&env, Error::ZeroAmount);
        }
        let st = Self::state_fresh(&env);

        // Preview the mint leg exactly like `quote_sy_for_yt`, and the sell-PT leg exactly like
        // `swap_pt_for_sy` -- same formulas, so the real mint below is proven to match (checked
        // defensively, not just assumed).
        let fee_bps = ConfigClient::new(&env, &Self::addr(&env, &DataKey::Config))
            .tokenization_fee_bps() as i128;
        let mint_fee_shares = Self::ceil_div(sy_in * fee_bps, BPS);
        let net_shares = sy_in - mint_fee_shares;
        if net_shares <= 0 {
            panic_with_error!(&env, Error::ZeroAmount);
        }
        let yt_out = net_shares * st.rate / SCALE;
        if yt_out <= 0 {
            panic_with_error!(&env, Error::ZeroAmount);
        }
        if yt_out < min_yt_out {
            panic_with_error!(&env, Error::SlippageExceeded);
        }
        let (sell_quote, gross) = Self::calc_sell_pt(&env, &st, yt_out);
        let sy_back = sell_quote.amount_out;
        if sy_back >= sy_in {
            // Cannot happen -- see the module doc's "why net_cost cannot be negative" -- but a
            // rate/fee-schedule change this call did not anticipate must fail closed, not hand the
            // buyer a free mint funded by LP reserves.
            panic_with_error!(&env, Error::InsufficientLiquidity);
        }
        let net_cost = sy_in - sy_back;
        if net_cost > max_net_cost {
            panic_with_error!(&env, Error::SlippageExceeded);
        }

        // Effects: reserves move exactly like a sell-PT trade for `yt_out` PT (see the module doc).
        Self::set_reserves(&env, st.pt_reserve + yt_out, st.sy_reserve - gross);
        Self::accrue_fee(&env, sell_quote.fee_shares);

        // Interactions.
        let this = env.current_contract_address();
        let manager_addr = Self::addr(&env, &DataKey::Manager);
        let sy_addr = Self::addr(&env, &DataKey::SY);
        let sy = SYClient::new(&env, &sy_addr);
        // Pull `from`'s full, *fixed* `max_net_cost` up front and refund the unused remainder
        // below -- not the mid-execution-derived `net_cost` directly. `net_cost` depends on
        // `calc_sell_pt`, which reads `seconds_to_maturity` live, so it can drift by a few raw
        // units between this call's simulation and its real execution moments later; pulling it
        // straight as `from`'s authorized transfer amount would make that pre-signed authorization
        // stale by the time the transaction lands, reverting `Error(Auth, InvalidAction)` --
        // reproduced live on Testnet while this function was first written. `max_net_cost` is a
        // plain literal `from` chose and signed for, so there is nothing to go stale. Same fix
        // family as `add_liquidity_single_sy`'s own `Error(Auth, InvalidAction)` fix.
        if max_net_cost > 0 {
            sy.transfer(&from, &this, &max_net_cost);
        }
        // The pool funds the *full* `sy_in` from its own balance -- `max_net_cost` just received
        // from `from` (of which only `net_cost` is kept; the rest is refunded below), plus
        // `sy_back`'s worth it already held as part of its reserves (freed by the reserve decrease
        // above). `mint`'s own `from.require_auth()` (with `from = this`) is a *direct* call this
        // contract is making, which Soroban always considers authorized -- but `mint` in turn
        // calls `SYWrapper.transfer(this, manager, sy_in)` on *its own* frame, two hops from this
        // one, and self-authorization does not propagate past the immediately invoking frame.
        // That one nested call needs an explicit entry (the same mechanism, and the same reason,
        // documented in `docs/AMM_FLASH_SWAP_AND_SIDEREAL_REVIEW.md` §5 as the one part of a
        // competitor's design confirmed sound by their own audit). Its `sy_in` argument is the
        // same fixed literal the caller already authorized this whole call with, so it carries no
        // staleness risk of its own.
        env.authorize_as_current_contract(vec![
            &env,
            InvokerContractAuthEntry::Contract(SubContractInvocation {
                context: ContractContext {
                    contract: sy_addr.clone(),
                    fn_name: Symbol::new(&env, "transfer"),
                    args: vec![
                        &env,
                        this.into_val(&env),
                        manager_addr.clone().into_val(&env),
                        sy_in.into_val(&env),
                    ],
                },
                sub_invocations: vec![&env],
            }),
        ]);
        let minted = ManagerClient::new(&env, &manager_addr).mint(&this, &sy_in);
        if minted.yt_minted != yt_out || minted.pt_minted != yt_out {
            // Cannot happen while the manager mints against the same rate and fee this call just
            // read: under-minting would short the buyer, over-minting would leave PT/YT the
            // reserve update above never accounted for. Either way, fail closed rather than trust
            // it silently.
            panic_with_error!(&env, Error::InsufficientLiquidity);
        }
        let refund = max_net_cost - net_cost;
        if refund > 0 {
            sy.transfer(&this, &from, &refund);
        }
        TokenClient::new(&env, &Self::addr(&env, &DataKey::YT)).transfer(&this, &to, &yt_out);

        env.events().publish(
            (symbol_short!("flash_yt"),),
            (from, to, sy_in, yt_out, net_cost),
        );
        (yt_out, net_cost)
    }

    // -------------------------------------------------------------- liquidity

    /// Add liquidity with up to `pt_desired` PT and `sy_desired` SY shares. The first deposit sets
    /// the opening price (its SY value must not exceed its PT, i.e. PT priced at or below par) and
    /// locks `MINIMUM_LIQUIDITY`; later deposits are taken in the current ratio, the surplus side
    /// left with the caller. Returns `(pt_used, sy_used, lp_minted)`.
    pub fn add_liquidity(
        env: Env,
        from: Address,
        pt_desired: i128,
        sy_desired: i128,
        min_lp_out: i128,
    ) -> (i128, i128, i128) {
        from.require_auth();
        Self::assert_active(&env);
        Self::assert_trader(&env, &from);
        if pt_desired <= 0 || sy_desired <= 0 {
            panic_with_error!(&env, Error::ZeroAmount);
        }
        let (pt_res, sy_res, total_lp) = Self::reserves_raw(&env);
        let (pt_used, sy_used, lp) = if total_lp == 0 {
            Self::assert_fresh_oracle(&env);
            let rate = Self::rate(&env);
            // Round the SY leg's value *up* here (not the floor a plain `/` would give): the
            // floored value under-reports `sy_desired`'s true worth by up to one part in `SCALE`,
            // which would let a ratio a hair under par pass this guard. Every other
            // reserve-sizing computation in this contract already rounds in the pool's favor;
            // this one-time first-deposit check should too.
            if Self::ceil_div(sy_desired * rate, SCALE) > pt_desired {
                panic_with_error!(&env, Error::InvalidInitialRatio);
            }
            let root = math::isqrt(
                pt_desired
                    .checked_mul(sy_desired)
                    .unwrap_or_else(|| panic_with_error!(&env, Error::MathError)),
            );
            if root <= MINIMUM_LIQUIDITY {
                panic_with_error!(&env, Error::MinimumLiquidity);
            }
            // Lock MINIMUM_LIQUIDITY to the pool itself, forever.
            let this = env.current_contract_address();
            Self::credit_lp(&env, &this, MINIMUM_LIQUIDITY);
            (pt_desired, sy_desired, root - MINIMUM_LIQUIDITY)
        } else {
            if pt_res <= 0 || sy_res <= 0 {
                panic_with_error!(&env, Error::InsufficientLiquidity);
            }
            let lp = (pt_desired * total_lp / pt_res).min(sy_desired * total_lp / sy_res);
            if lp <= 0 {
                panic_with_error!(&env, Error::ZeroAmount);
            }
            (
                Self::ceil_div(lp * pt_res, total_lp),
                Self::ceil_div(lp * sy_res, total_lp),
                lp,
            )
        };
        if lp < min_lp_out {
            panic_with_error!(&env, Error::SlippageExceeded);
        }
        Self::set_reserves(&env, pt_res + pt_used, sy_res + sy_used);
        Self::credit_lp(&env, &from, lp);
        Self::set_total_lp(
            &env,
            total_lp + lp + if total_lp == 0 { MINIMUM_LIQUIDITY } else { 0 },
        );

        let this = env.current_contract_address();
        TokenClient::new(&env, &Self::addr(&env, &DataKey::PT)).transfer(&from, &this, &pt_used);
        SYClient::new(&env, &Self::addr(&env, &DataKey::SY)).transfer(&from, &this, &sy_used);
        env.events()
            .publish((symbol_short!("add_liq"),), (from, pt_used, sy_used, lp));
        (pt_used, sy_used, lp)
    }

    /// Add liquidity from SY alone: the pool buys just enough PT with part of `sy_in` (paying the
    /// normal swap fee) that the remainder and the PT match the post-swap pool ratio, then adds
    /// both. Any rounding leftover is returned. Returns `(pt_used, sy_used, lp_minted)`.
    ///
    /// Takes the full `sy_in` from `from` up front, at the exact value the caller authorized --
    /// not a time-dependent amount derived from the curve below -- and refunds whatever of it
    /// goes unused. Soroban's authorization for the nested `SYWrapper.transfer` must match that
    /// call's real arguments exactly; an earlier version transferred `split + sy_used`, both
    /// derived from `state_fresh()` (which reads the ledger's current timestamp), so the amount
    /// signed at simulation time could differ -- by a few raw units -- from the amount computed
    /// moments later at execution, and the stale signature was then rejected with
    /// `Error(Auth, InvalidAction)`. `sy_in` is a plain argument, identical at simulation and
    /// execution, so transferring exactly that removes the mismatch.
    pub fn add_liquidity_single_sy(
        env: Env,
        from: Address,
        sy_in: i128,
        min_lp_out: i128,
    ) -> (i128, i128, i128) {
        from.require_auth();
        Self::assert_active(&env);
        Self::assert_trader(&env, &from);
        if sy_in <= 0 {
            panic_with_error!(&env, Error::ZeroAmount);
        }
        let st = Self::state_fresh(&env);
        if st.total_lp == 0 {
            panic_with_error!(&env, Error::InsufficientLiquidity);
        }

        let this = env.current_contract_address();
        let sy = SYClient::new(&env, &Self::addr(&env, &DataKey::SY));
        sy.transfer(&from, &this, &sy_in);

        let split = Self::zap_split(&env, &st, sy_in);
        let q = Self::calc_buy_pt(&env, &st, split);
        let pt_bought = q.amount_out;
        let pt1 = st.pt_reserve - pt_bought;
        let sy1 = st.sy_reserve + (split - q.fee_shares);
        let remaining = sy_in - split;

        let lp = (pt_bought * st.total_lp / pt1).min(remaining * st.total_lp / sy1);
        if lp <= 0 {
            panic_with_error!(&env, Error::ZeroAmount);
        }
        if lp < min_lp_out {
            panic_with_error!(&env, Error::SlippageExceeded);
        }
        let pt_used = Self::ceil_div(lp * pt1, st.total_lp).min(pt_bought);
        let sy_used = Self::ceil_div(lp * sy1, st.total_lp).min(remaining);

        // The PT this zap "bought" beyond what `lp` needed is not refunded to `from` -- it is
        // left in the pool's own reserve instead, as if that dust had simply never been bought.
        // An earlier version transferred it to `from`, which reverted `PermissionDenied` for any
        // caller lacking PTToken's per-asset grant (`assert_permitted` checks it even for a
        // single raw unit, unlike SYWrapper's account-only check below) -- a real failure found
        // live by the dApp team on roughly 1 in 12 calls in their testing, since a caller using
        // this "LP only" zap specifically to avoid ever holding PT directly has no reason to have
        // requested that grant. `pt1 + pt_used + pt_refund` is algebraically `pt_reserve -
        // pt_used`: exactly as if only `pt_used` of the "bought" PT ever left the reserve ledger.
        let pt_refund = pt_bought - pt_used;
        Self::set_reserves(&env, pt1 + pt_used + pt_refund, sy1 + sy_used);
        Self::accrue_fee(&env, q.fee_shares);
        Self::credit_lp(&env, &from, lp);
        Self::set_total_lp(&env, st.total_lp + lp);

        // Refund whatever of `sy_in` the pool didn't end up using. SY carries no per-asset grant
        // (only PT/YT do), so this transfer has no equivalent compliance risk.
        let sy_total_used = split + sy_used;
        let sy_refund = sy_in - sy_total_used;
        if sy_refund > 0 {
            sy.transfer(&this, &from, &sy_refund);
        }
        env.events().publish(
            (symbol_short!("zap_liq"),),
            (from, pt_used, sy_total_used, lp),
        );
        (pt_used, sy_total_used, lp)
    }

    /// Burn `lp` and send the proportional PT and SY to `to`. Available after maturity even while
    /// paused (when it is the only way out); before maturity the pool pause applies. Reverts `SlippageExceeded` below the minimums.
    pub fn remove_liquidity(
        env: Env,
        from: Address,
        to: Address,
        lp: i128,
        min_pt_out: i128,
        min_sy_out: i128,
    ) -> (i128, i128) {
        from.require_auth();
        // The pause halts liquidity exits while the market is live, but never after maturity:
        // once trading has closed, LPs' only way out must not depend on an admin switch.
        if env.ledger().timestamp() < Self::maturity(env.clone()) {
            Self::assert_not_paused(&env);
        }
        Self::assert_trader(&env, &from);
        Self::remove_liquidity_inner(&env, &from, &to, lp, min_pt_out, min_sy_out)
    }

    // -------------------------------------------------------------- LP token

    pub fn lp_balance(env: Env, account: Address) -> i128 {
        Self::get_lp(&env, &account)
    }

    pub fn lp_total_supply(env: Env) -> i128 {
        env.storage().instance().get(&DataKey::TotalLp).unwrap_or(0)
    }

    /// Move LP between accounts. Both sides must clear the underlying's compliance and
    /// Permissioning, so an LP position can never be parked with an ineligible holder.
    pub fn transfer_lp(env: Env, from: Address, to: Address, amount: i128) {
        from.require_auth();
        Self::assert_not_paused(&env);
        Self::assert_trader(&env, &from);
        Self::assert_trader(&env, &to);
        if amount <= 0 {
            panic_with_error!(&env, Error::ZeroAmount);
        }
        let bal = Self::get_lp(&env, &from);
        if bal < amount {
            panic_with_error!(&env, Error::InsufficientLpBalance);
        }
        Self::set_lp(&env, &from, bal - amount);
        Self::credit_lp(&env, &to, amount);
        env.events()
            .publish((symbol_short!("lp_xfer"),), (from, to, amount));
    }

    /// Forced transfer of `amount` LP from `account` to the caller, without `account`'s
    /// authorization. Callable only by the configured `RecoveryEscrow`, which verifies the issuer
    /// and the target's deauthorization -- like `seize` on SY/PT/YT. Works while paused.
    pub fn seize_lp(env: Env, caller: Address, account: Address, amount: i128) -> i128 {
        caller.require_auth();
        let escrow: Address = env
            .storage()
            .instance()
            .get(&DataKey::RecoveryEscrow)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotRecoveryEscrow));
        if caller != escrow {
            panic_with_error!(&env, Error::NotRecoveryEscrow);
        }
        if amount <= 0 {
            panic_with_error!(&env, Error::ZeroAmount);
        }
        let bal = Self::get_lp(&env, &account);
        if bal < amount {
            panic_with_error!(&env, Error::InsufficientLpBalance);
        }
        Self::set_lp(&env, &account, bal - amount);
        Self::credit_lp(&env, &caller, amount);
        env.events()
            .publish((symbol_short!("lp_seize"),), (caller, account, amount));
        amount
    }

    /// Burn LP the escrow already holds and send the PT/SY to the escrow itself. Escrow-only;
    /// works while paused and after maturity.
    pub fn redeem_seized_lp(env: Env, caller: Address, lp: i128) -> (i128, i128) {
        caller.require_auth();
        let escrow: Address = env
            .storage()
            .instance()
            .get(&DataKey::RecoveryEscrow)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotRecoveryEscrow));
        if caller != escrow {
            panic_with_error!(&env, Error::NotRecoveryEscrow);
        }
        Self::remove_liquidity_inner(&env, &caller, &caller, lp, 0, 0)
    }

    // ------------------------------------------------------------------ fees

    /// Pay Principal's accrued share of swap fees, in SY shares, to the config's treasury.
    pub fn claim_protocol_fees(env: Env) -> i128 {
        Self::claim_bucket(&env, &DataKey::AccruedProtocol, true)
    }

    /// Pay the market creator's accrued share of swap fees, in SY shares, to the creator.
    pub fn claim_creator_fees(env: Env) -> i128 {
        Self::claim_bucket(&env, &DataKey::AccruedCreator, false)
    }

    /// Accrued, unclaimed swap fees in SY shares: (protocol, creator).
    pub fn accrued_fees(env: Env) -> (i128, i128) {
        let s = env.storage().instance();
        (
            s.get(&DataKey::AccruedProtocol).unwrap_or(0),
            s.get(&DataKey::AccruedCreator).unwrap_or(0),
        )
    }

    // ----------------------------------------------------------------- views

    /// (PT reserve, SY reserve).
    pub fn reserves(env: Env) -> (i128, i128) {
        let (pt, sy, _) = Self::reserves_raw(&env);
        (pt, sy)
    }

    /// The full pool state at the current ledger time (requires a readable oracle rate).
    pub fn pool_state(env: Env) -> PoolState {
        Self::state(&env)
    }

    /// Spot price of one PT in units of SY value, at `SCALE`: `(x / y)^(tau / stretch)`. Below
    /// `SCALE` while PT trades at a discount; exactly `SCALE` at maturity.
    pub fn pt_price(env: Env) -> i128 {
        let st = Self::state(&env);
        Self::spot_price(&env, &st)
    }

    /// The fixed annualized yield a PT buyer locks in at the spot price, at `SCALE`:
    /// `(1 / price - 1) x year / tau`. Zero at maturity.
    pub fn implied_rate(env: Env) -> i128 {
        let st = Self::state(&env);
        if st.seconds_to_maturity == 0 {
            return 0;
        }
        let price = Self::spot_price(&env, &st);
        if price <= 0 {
            return 0;
        }
        (SCALE * SCALE / price - SCALE) * (SECONDS_PER_YEAR as i128)
            / (st.seconds_to_maturity as i128)
    }

    /// The curve exponent `a` at WAD (1e18) for the current time; `1e18` at maturity.
    pub fn time_exponent(env: Env) -> i128 {
        let secs = Self::seconds_to_maturity(&env);
        math::exponent(secs, Self::stretch_secs(&env))
            .unwrap_or_else(|_| panic_with_error!(&env, Error::MaturityTooFar))
    }

    /// Price a PT purchase with `sy_in` SY shares.
    pub fn quote_sy_for_pt(env: Env, sy_in: i128) -> Quote {
        let st = Self::state(&env);
        Self::calc_buy_pt(&env, &st, sy_in)
    }

    /// Price a PT sale of `pt_in` PT (`amount_out` is the SY after the fee).
    pub fn quote_pt_for_sy(env: Env, pt_in: i128) -> Quote {
        let st = Self::state(&env);
        Self::calc_sell_pt(&env, &st, pt_in).0
    }

    /// Price buying exactly `pt_out` PT: `amount_in` is the SY required, fee included.
    pub fn quote_buy_exact_pt(env: Env, pt_out: i128) -> Quote {
        let st = Self::state(&env);
        Self::calc_buy_exact_pt(&env, &st, pt_out)
    }

    /// How much of `sy_in` a single-sided deposit swaps into PT.
    pub fn zap_swap_amount(env: Env, sy_in: i128) -> i128 {
        let st = Self::state(&env);
        Self::zap_split(&env, &st, sy_in)
    }

    /// Preview `add_liquidity_single_sy`'s own `(pt_used, sy_used, lp)` exactly -- found missing
    /// in a dApp-integration review ("no quote for the two new USDY LP functions"), since no
    /// quote for this zap existed anywhere in this protocol, not even for the plain SY-denominated
    /// call this previews, let alone `Router.add_liquidity_single_underlying`, which composes it.
    /// Replicates the real call's math read-only (same `zap_split`/`calc_buy_pt` bisection, same
    /// rounding) rather than calling it, so this stays a view. Like every quote in this protocol,
    /// exact only if reserves/`total_lp` do not move between this read and the real call.
    pub fn quote_add_liquidity_single_sy(env: Env, sy_in: i128) -> (i128, i128, i128) {
        if sy_in <= 0 {
            panic_with_error!(&env, Error::ZeroAmount);
        }
        let st = Self::state(&env);
        if st.total_lp == 0 {
            panic_with_error!(&env, Error::InsufficientLiquidity);
        }
        let split = Self::zap_split(&env, &st, sy_in);
        let q = Self::calc_buy_pt(&env, &st, split);
        let pt_bought = q.amount_out;
        let pt1 = st.pt_reserve - pt_bought;
        let sy1 = st.sy_reserve + (split - q.fee_shares);
        let remaining = sy_in - split;
        let lp = (pt_bought * st.total_lp / pt1).min(remaining * st.total_lp / sy1);
        if lp <= 0 {
            panic_with_error!(&env, Error::ZeroAmount);
        }
        let pt_used = Self::ceil_div(lp * pt1, st.total_lp).min(pt_bought);
        let sy_used = Self::ceil_div(lp * sy1, st.total_lp).min(remaining);
        (pt_used, split + sy_used, lp)
    }

    /// Preview the full `Router.remove_liquidity_to_underlying` round trip's SY-denominated
    /// legs: `remove_liquidity`'s own proportional split for `lp_in`, then selling the PT leg
    /// *against the reserves as they stand after that removal* -- not the reserves before it.
    /// Found needed building the dApp-integration-requested quote for that function: a first
    /// version priced the PT sale with the plain `quote_pt_for_sy` against the pre-removal
    /// reserves, which overstates the real payout by exactly this removal's own price impact
    /// (the real multi-call sequence necessarily sells the PT *after* `remove_liquidity` has
    /// already shrunk the reserves). Returns `(pt_out, sy_out, pt_sold_for_sy, swap_fee_shares)`;
    /// the last two are `0` if `pt_out` rounds to `0` (tiny `lp_in`), matching
    /// `swap_gained_pt_for_sy`'s own zero-gain tolerance rather than reverting `ZeroAmount`.
    pub fn quote_lp_exit(env: Env, lp_in: i128) -> (i128, i128, i128, i128) {
        let st = Self::state(&env);
        Self::assert_seeded(&env, &st);
        if lp_in <= 0 {
            panic_with_error!(&env, Error::ZeroAmount);
        }
        let pt_out = lp_in * st.pt_reserve / st.total_lp;
        let sy_out = lp_in * st.sy_reserve / st.total_lp;
        if pt_out <= 0 {
            return (pt_out, sy_out, 0, 0);
        }
        let mut st_after = st.clone();
        st_after.pt_reserve = st.pt_reserve - pt_out;
        st_after.sy_reserve = st.sy_reserve - sy_out;
        let (q, _gross) = Self::calc_sell_pt(&env, &st_after, pt_out);
        (pt_out, sy_out, q.amount_out, q.fee_shares)
    }

    /// How much of `sy_shares` `Router.add_liquidity_keep_yt` should mint (rather than swap) so
    /// that the minted PT and the remaining SY land in this pool's current raw reserve ratio --
    /// the exact ratio `add_liquidity`'s own proportional deposit targets.
    ///
    /// Deliberately **not** `zap_swap_amount`: that helper solves a different problem (buying PT
    /// from this pool at the curve price, for `add_liquidity_single_sy`'s zap), and reusing it
    /// here was found, during a later integration review, to recommend the wrong split --
    /// `PrincipalManager.mint` issues PT 1:1 with the net SY shares valued at the *oracle* rate,
    /// not at the curve's swap price, so balancing against the curve-swap ratio generally leaves
    /// SY stranded in the subsequent `add_liquidity` call instead of being fully used.
    pub fn keep_yt_split(env: Env, sy_shares: i128) -> i128 {
        if sy_shares <= 0 {
            panic_with_error!(&env, Error::ZeroAmount);
        }
        let st = Self::state(&env);
        Self::assert_seeded(&env, &st);
        let fee_bps = ConfigClient::new(&env, &Self::addr(&env, &DataKey::Config))
            .tokenization_fee_bps() as i128;
        let mut lo: i128 = 0;
        let mut hi: i128 = sy_shares;
        let mut i = 0;
        while i < ZAP_ITERATIONS && hi - lo > 1 {
            let mid = (lo + hi) / 2;
            let fee = Self::ceil_div(mid * fee_bps, BPS);
            let pt_minted = Self::value_of(mid - fee, st.rate);
            let sy_remaining = sy_shares - mid;
            // Target: pt_minted / sy_remaining == pt_reserve / sy_reserve (raw), i.e.
            // pt_minted * sy_reserve ?= sy_remaining * pt_reserve. Monotonically increasing in
            // `mid`, so a plain bisection suffices -- same technique as `zap_split` above.
            let f = pt_minted * st.sy_reserve - sy_remaining * st.pt_reserve;
            if f < 0 {
                lo = mid;
            } else {
                hi = mid;
            }
            i += 1;
        }
        if lo <= 0 {
            panic_with_error!(&env, Error::ZeroAmount);
        }
        lo
    }

    /// Sell `from`'s **entire** PT balance for SY, crediting `to`. Mirrors `swap_pt_for_sy`
    /// exactly except for reading the amount instead of taking it as an argument -- the same
    /// reason `SYWrapper.withdraw_all` exists: a caller composing this after a call whose PT
    /// payout is derived mid-transaction (e.g. `remove_liquidity`) cannot safely pass that
    /// payout as an explicit, pre-signed `pt_in` argument to a further `require_auth`'d call
    /// without risking `Error(Auth, InvalidAction)` if reserves move between simulation and
    /// real execution; reading the balance at execution time removes that argument entirely.
    /// Safe only when the caller is certain `from` holds no other PT it wants to keep separate.
    pub fn swap_all_pt_for_sy(env: Env, from: Address, to: Address, min_sy_out: i128) -> i128 {
        let pt_in = TokenClient::new(&env, &Self::addr(&env, &DataKey::PT)).balance(&from);
        Self::swap_pt_for_sy(env, from, to, pt_in, min_sy_out)
    }

    /// Sell only the PT `from` gained *since* `baseline` -- the PT equivalent of
    /// `SYWrapper.withdraw_gained`, for the same reason: a caller composing this after a call
    /// whose PT payout is derived mid-transaction (e.g. `remove_liquidity`) can safely use this
    /// without first clearing out PT it already held for an unrelated reason, unlike
    /// `swap_all_pt_for_sy` which acts on the *entire* balance. Found worth building during a
    /// later integration review: `withdraw_gained` existed but was only wired into one Router
    /// flow, while others still used the zero-pre-existing-balance idiom and failed for any
    /// caller who, for instance, simply already held PT from an earlier trade.
    ///
    /// Tolerates a zero gain gracefully (no PT bought, or none left after a prior leg) rather
    /// than reverting `ZeroAmount` the way a direct `swap_pt_for_sy(0, ...)` would -- composed
    /// callers cannot always guarantee a nonzero gain (e.g. `remove_liquidity` producing no PT
    /// leg). Returns `0` in that case, but still enforces `min_sy_out`: a zero gain is only
    /// silently fine if the caller did not actually require a nonzero `sy_out`.
    pub fn swap_gained_pt_for_sy(
        env: Env,
        from: Address,
        to: Address,
        baseline: i128,
        min_sy_out: i128,
    ) -> i128 {
        // No separate `from.require_auth()` here -- `swap_pt_for_sy` below already does it
        // exactly once for this same invocation (not a cross-contract call, so a second,
        // redundant check here would authorize the same address twice for one logical call,
        // which the test environment's auth mock rejects outright as `Error(Auth,
        // ExistingValue)`; real Soroban's own authorization model has no use for it either).
        // The zero/negative-gain path below is a pure no-op or an outright panic, so it is
        // harmless to leave unauthorized: nothing it does ever persists.
        let pt_in =
            TokenClient::new(&env, &Self::addr(&env, &DataKey::PT)).balance(&from) - baseline;
        if pt_in <= 0 {
            if min_sy_out > 0 {
                panic_with_error!(&env, Error::SlippageExceeded);
            }
            return 0;
        }
        Self::swap_pt_for_sy(env, from, to, pt_in, min_sy_out)
    }

    /// Preview `swap_yt_for_sy` (the flash-redeem behind `Router.swap_yt_for_sy` /
    /// `swap_yt_for_underlying`): recombining `yt_in` YT with `yt_in` PT pulled from this pool's
    /// own reserve, at the live oracle rate, minus the curve-priced cost of pulling that PT.
    /// Matches `swap_yt_for_sy`'s own math exactly -- see `contracts/market_factory/tests/
    /// create_market.rs`-style regression tests that assert the real swap's payout equals this.
    pub fn quote_yt_for_sy(env: Env, yt_in: i128) -> Quote {
        let st = Self::state(&env);
        let q = Self::calc_buy_exact_pt(&env, &st, yt_in);
        let recombined = yt_in * SCALE / st.rate;
        let payout = recombined - q.amount_in;
        if payout < 0 {
            panic_with_error!(&env, Error::InsufficientLiquidity);
        }
        Quote {
            amount_out: payout,
            amount_in: 0,
            fee_shares: q.fee_shares,
        }
    }

    /// Preview `PrincipalManager.mint` followed by `MarketPool.swap_pt_for_sy` together (the
    /// flash-mint behind `Router.swap_sy_for_yt` / `swap_underlying_for_yt`): tokenizing `sy_in`
    /// SY into PT+YT at the live oracle rate and `MarketConfig`'s tokenization fee, then selling
    /// the minted PT back into this same pool at the live curve price. Replicates
    /// `PrincipalManager.mint`'s exact formula (ceil-rounded fee, floor-rounded notional) rather
    /// than calling it, since the quote must stay read-only.
    pub fn quote_sy_for_yt(env: Env, sy_in: i128) -> FlashMintQuote {
        if sy_in <= 0 {
            panic_with_error!(&env, Error::ZeroAmount);
        }
        let st = Self::state(&env);
        let fee_bps = ConfigClient::new(&env, &Self::addr(&env, &DataKey::Config))
            .tokenization_fee_bps() as i128;
        let mint_fee_shares = Self::ceil_div(sy_in * fee_bps, BPS);
        let net_shares = sy_in - mint_fee_shares;
        if net_shares <= 0 {
            panic_with_error!(&env, Error::ZeroAmount);
        }
        let yt_out = net_shares * st.rate / SCALE;
        if yt_out <= 0 {
            panic_with_error!(&env, Error::ZeroAmount);
        }
        let (sell_quote, _gross) = Self::calc_sell_pt(&env, &st, yt_out);
        FlashMintQuote {
            yt_out,
            sy_back: sell_quote.amount_out,
            mint_fee_shares,
            swap_fee_shares: sell_quote.fee_shares,
        }
    }

    pub fn maturity(env: Env) -> u64 {
        env.storage()
            .instance()
            .get(&DataKey::Maturity)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized))
    }

    pub fn manager_address(env: Env) -> Address {
        Self::addr(&env, &DataKey::Manager)
    }

    pub fn sy_address(env: Env) -> Address {
        Self::addr(&env, &DataKey::SY)
    }

    pub fn pt_address(env: Env) -> Address {
        Self::addr(&env, &DataKey::PT)
    }

    pub fn yt_address(env: Env) -> Address {
        Self::addr(&env, &DataKey::YT)
    }

    pub fn underlying_address(env: Env) -> Address {
        Self::addr(&env, &DataKey::Underlying)
    }

    pub fn config_address(env: Env) -> Address {
        Self::addr(&env, &DataKey::Config)
    }

    pub fn is_paused(env: Env) -> bool {
        env.storage()
            .instance()
            .get(&DataKey::Paused)
            .unwrap_or(false)
    }

    pub fn get_admin(env: Env) -> Address {
        Self::addr(&env, &DataKey::Admin)
    }

    // ----------------------------------------------------------------- admin

    pub fn set_paused(env: Env, caller: Address, paused: bool) {
        Self::assert_admin(&env, &caller);
        env.storage().instance().set(&DataKey::Paused, &paused);
        env.events().publish((symbol_short!("paused"),), paused);
    }

    pub fn transfer_admin(env: Env, current_admin: Address, new_admin: Address) {
        Self::assert_admin(&env, &current_admin);
        env.storage().instance().set(&DataKey::Admin, &new_admin);
        env.events()
            .publish((symbol_short!("adm_xfer"),), (current_admin, new_admin));
    }

    /// One-time wiring of the `RecoveryEscrow` allowed to call `seize_lp`.
    pub fn set_recovery_escrow(env: Env, admin: Address, escrow: Address) {
        Self::assert_admin(&env, &admin);
        if env.storage().instance().has(&DataKey::RecoveryEscrow) {
            panic_with_error!(&env, Error::RecoveryEscrowAlreadySet);
        }
        env.storage()
            .instance()
            .set(&DataKey::RecoveryEscrow, &escrow);
        env.events().publish((symbol_short!("esc_set"),), escrow);
    }

    pub fn recovery_escrow(env: Env) -> Option<Address> {
        env.storage().instance().get(&DataKey::RecoveryEscrow)
    }
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

impl MarketPoolContract {
    // ---- pricing ----

    /// SY shares -> notional value, floor.
    fn value_of(shares: i128, rate: i128) -> i128 {
        shares * rate / SCALE
    }

    fn ceil_div(a: i128, b: i128) -> i128 {
        (a + b - 1) / b
    }

    fn math_err<T>(env: &Env, r: Result<T, math::MathError>) -> T {
        match r {
            Ok(v) => v,
            Err(math::MathError::Insufficient) => {
                panic_with_error!(env, Error::InsufficientLiquidity)
            }
            Err(_) => panic_with_error!(env, Error::MathError),
        }
    }

    /// Exact-in: buy PT with `sy_in` SY shares.
    fn calc_buy_pt(env: &Env, st: &PoolState, sy_in: i128) -> Quote {
        if sy_in <= 0 {
            panic_with_error!(env, Error::ZeroAmount);
        }
        Self::assert_seeded(env, st);
        let fee = Self::ceil_div(sy_in * st.fee_rate, FEE_SCALE);
        let net = sy_in - fee;
        let net_value = Self::value_of(net, st.rate);
        if net_value <= 0 {
            panic_with_error!(env, Error::ZeroAmount);
        }
        let x = Self::value_of(st.sy_reserve, st.rate);
        let y = st.pt_reserve;
        let y_new = Self::math_err(env, math::solve(x, y, x + net_value, st.exponent));
        // `solve` already rounds `y_new` up (against the trader) by a magnitude-scaled pad.
        let pt_out = y - y_new;
        if pt_out <= 0 || y - pt_out < MIN_RESERVE {
            panic_with_error!(env, Error::InsufficientLiquidity);
        }
        Quote {
            amount_out: pt_out,
            amount_in: sy_in,
            fee_shares: fee,
        }
    }

    /// Exact-in: sell `pt_in` PT. Returns the quote (`amount_out` after fee) and the gross SY
    /// leaving the reserves.
    fn calc_sell_pt(env: &Env, st: &PoolState, pt_in: i128) -> (Quote, i128) {
        if pt_in <= 0 {
            panic_with_error!(env, Error::ZeroAmount);
        }
        Self::assert_seeded(env, st);
        let x = Self::value_of(st.sy_reserve, st.rate);
        let y = st.pt_reserve;
        let x_new = Self::math_err(env, math::solve(y, x, y + pt_in, st.exponent));
        let value_out = x - x_new;
        if value_out <= 0 {
            panic_with_error!(env, Error::InsufficientLiquidity);
        }
        let gross = value_out * SCALE / st.rate;
        if gross <= 0 || st.sy_reserve - gross < MIN_RESERVE {
            panic_with_error!(env, Error::InsufficientLiquidity);
        }
        let fee = Self::ceil_div(gross * st.fee_rate, FEE_SCALE);
        (
            Quote {
                amount_out: gross - fee,
                amount_in: 0,
                fee_shares: fee,
            },
            gross,
        )
    }

    /// Exact-out: buy exactly `pt_out` PT. `amount_in` is the SY required, fee included.
    fn calc_buy_exact_pt(env: &Env, st: &PoolState, pt_out: i128) -> Quote {
        if pt_out <= 0 {
            panic_with_error!(env, Error::ZeroAmount);
        }
        Self::assert_seeded(env, st);
        if st.pt_reserve - pt_out < MIN_RESERVE {
            panic_with_error!(env, Error::InsufficientLiquidity);
        }
        let x = Self::value_of(st.sy_reserve, st.rate);
        let y = st.pt_reserve;
        let x_new = Self::math_err(env, math::solve(y, x, y - pt_out, st.exponent));
        // Cost in value (`x_new` is already padded up), then to shares (up), then grossed up for
        // the fee so that `gross - fee(gross) >= net`.
        let cost_value = x_new - x + 1;
        let net_shares = Self::ceil_div(cost_value * SCALE, st.rate);
        let gross = Self::ceil_div(net_shares * FEE_SCALE, FEE_SCALE - st.fee_rate);
        Quote {
            amount_out: pt_out,
            amount_in: gross,
            fee_shares: gross - net_shares,
        }
    }

    /// The PT/SY swap size that leaves a single-sided deposit exactly balanced (bisection).
    fn zap_split(env: &Env, st: &PoolState, sy_in: i128) -> i128 {
        Self::assert_seeded(env, st);
        let mut lo: i128 = 0;
        let mut hi: i128 = sy_in;
        let mut i = 0;
        while i < ZAP_ITERATIONS && hi - lo > 1 {
            let mid = (lo + hi) / 2;
            // Skip splits too small/large to price; they are on the wrong side of the root anyway.
            let fee = Self::ceil_div(mid * st.fee_rate, FEE_SCALE);
            let net_value = Self::value_of(mid - fee, st.rate);
            let x = Self::value_of(st.sy_reserve, st.rate);
            let bought = if net_value <= 0 {
                0
            } else {
                match math::solve(x, st.pt_reserve, x + net_value, st.exponent) {
                    Ok(y_new) => (st.pt_reserve - y_new).max(0),
                    Err(_) => {
                        hi = mid;
                        i += 1;
                        continue;
                    }
                }
            };
            let pt1 = st.pt_reserve - bought;
            let sy1 = st.sy_reserve + (mid - fee);
            // Remaining SY vs bought PT, in the post-swap ratio: f > 0 => swap more.
            let f = (sy_in - mid) * pt1 - bought * sy1;
            if f > 0 {
                lo = mid;
            } else {
                hi = mid;
            }
            i += 1;
        }
        if lo <= 0 {
            panic_with_error!(env, Error::ZeroAmount);
        }
        lo
    }

    fn spot_price(env: &Env, st: &PoolState) -> i128 {
        if st.pt_reserve <= 0 || st.sy_reserve <= 0 {
            panic_with_error!(env, Error::InsufficientLiquidity);
        }
        let x = Self::value_of(st.sy_reserve, st.rate);
        if x <= 0 {
            panic_with_error!(env, Error::InsufficientLiquidity);
        }
        let ratio = Self::math_err(
            env,
            (x).checked_mul(math::WAD)
                .map(|v| v / st.pt_reserve)
                .ok_or(math::MathError::Overflow),
        );
        let one_minus_a = math::WAD - st.exponent;
        let p = Self::math_err(env, math::pow(ratio, one_minus_a));
        p / (math::WAD / SCALE)
    }

    fn assert_seeded(env: &Env, st: &PoolState) {
        if st.total_lp == 0 || st.pt_reserve <= 0 || st.sy_reserve <= 0 {
            panic_with_error!(env, Error::InsufficientLiquidity);
        }
    }

    // ---- state ----

    fn seconds_to_maturity(env: &Env) -> u64 {
        Self::maturity(env.clone()).saturating_sub(env.ledger().timestamp())
    }

    fn stretch_secs(env: &Env) -> u64 {
        env.storage()
            .instance()
            .get(&DataKey::StretchSecs)
            .unwrap_or_else(|| panic_with_error!(env, Error::NotInitialized))
    }

    fn rate(env: &Env) -> i128 {
        OracleClient::new(env, &Self::addr(env, &DataKey::Oracle)).get_reference_value()
    }

    fn assert_fresh_oracle(env: &Env) {
        if !OracleClient::new(env, &Self::addr(env, &DataKey::Oracle)).is_available() {
            panic_with_error!(env, Error::OracleStale);
        }
    }

    fn reserves_raw(env: &Env) -> (i128, i128, i128) {
        let s = env.storage().instance();
        (
            s.get(&DataKey::ReservePT).unwrap_or(0),
            s.get(&DataKey::ReserveSY).unwrap_or(0),
            s.get(&DataKey::TotalLp).unwrap_or(0),
        )
    }

    fn set_reserves(env: &Env, pt: i128, sy: i128) {
        let s = env.storage().instance();
        s.set(&DataKey::ReservePT, &pt);
        s.set(&DataKey::ReserveSY, &sy);
    }

    fn set_total_lp(env: &Env, total: i128) {
        env.storage().instance().set(&DataKey::TotalLp, &total);
    }

    fn state(env: &Env) -> PoolState {
        let (pt_reserve, sy_reserve, total_lp) = Self::reserves_raw(env);
        let secs = Self::seconds_to_maturity(env);
        let exponent = math::exponent(secs, Self::stretch_secs(env))
            .unwrap_or_else(|_| panic_with_error!(env, Error::MaturityTooFar));
        let fee_rate =
            ConfigClient::new(env, &Self::addr(env, &DataKey::Config)).swap_fee_rate(&secs);
        PoolState {
            pt_reserve,
            sy_reserve,
            total_lp,
            rate: Self::rate(env),
            exponent,
            fee_rate,
            seconds_to_maturity: secs,
        }
    }

    /// `state` plus the oracle freshness check every state-changing trade requires.
    fn state_fresh(env: &Env) -> PoolState {
        Self::assert_fresh_oracle(env);
        Self::state(env)
    }

    // ---- checks ----

    fn addr(env: &Env, key: &DataKey) -> Address {
        env.storage()
            .instance()
            .get(key)
            .unwrap_or_else(|| panic_with_error!(env, Error::NotInitialized))
    }

    fn assert_admin(env: &Env, caller: &Address) {
        caller.require_auth();
        if *caller != Self::addr(env, &DataKey::Admin) {
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

    /// Not paused and not yet matured: the gate for everything that trades or adds liquidity.
    fn assert_active(env: &Env) {
        Self::assert_not_paused(env);
        if env.ledger().timestamp() >= Self::maturity(env.clone()) {
            panic_with_error!(env, Error::Expired);
        }
    }

    /// Both compliance layers for one account.
    fn assert_trader(env: &Env, account: &Address) {
        let underlying = Self::addr(env, &DataKey::Underlying);
        if !compliance::is_authorized(env, &underlying, account) {
            panic_with_error!(env, Error::NotAuthorizedOnSac);
        }
        let perm = Self::addr(env, &DataKey::Permissioning);
        if !PermClient::new(env, &perm).is_allowed(account) {
            panic_with_error!(env, Error::PermissionDenied);
        }
    }

    // ---- LP ledger ----

    fn get_lp(env: &Env, account: &Address) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::Lp(account.clone()))
            .unwrap_or(0)
    }

    fn set_lp(env: &Env, account: &Address, amount: i128) {
        let key = DataKey::Lp(account.clone());
        env.storage().persistent().set(&key, &amount);
        env.storage()
            .persistent()
            .extend_ttl(&key, BALANCE_TTL_LEDGERS, BALANCE_TTL_LEDGERS);
    }

    fn credit_lp(env: &Env, account: &Address, amount: i128) {
        let bal = Self::get_lp(env, account);
        Self::set_lp(env, account, bal + amount);
    }

    fn remove_liquidity_inner(
        env: &Env,
        from: &Address,
        to: &Address,
        lp: i128,
        min_pt_out: i128,
        min_sy_out: i128,
    ) -> (i128, i128) {
        if lp <= 0 {
            panic_with_error!(env, Error::ZeroAmount);
        }
        let bal = Self::get_lp(env, from);
        if bal < lp {
            panic_with_error!(env, Error::InsufficientLpBalance);
        }
        let (pt_res, sy_res, total_lp) = Self::reserves_raw(env);
        let pt_out = lp * pt_res / total_lp;
        let sy_out = lp * sy_res / total_lp;
        if pt_out < min_pt_out || sy_out < min_sy_out {
            panic_with_error!(env, Error::SlippageExceeded);
        }
        Self::set_lp(env, from, bal - lp);
        Self::set_total_lp(env, total_lp - lp);
        Self::set_reserves(env, pt_res - pt_out, sy_res - sy_out);

        let this = env.current_contract_address();
        if pt_out > 0 {
            TokenClient::new(env, &Self::addr(env, &DataKey::PT)).transfer(&this, to, &pt_out);
        }
        if sy_out > 0 {
            SYClient::new(env, &Self::addr(env, &DataKey::SY)).transfer(&this, to, &sy_out);
        }
        env.events().publish(
            (symbol_short!("rem_liq"),),
            (from.clone(), to.clone(), lp, pt_out, sy_out),
        );
        (pt_out, sy_out)
    }

    // ---- fees ----

    fn accrue_fee(env: &Env, fee_shares: i128) {
        if fee_shares <= 0 {
            return;
        }
        let (protocol, creator) =
            ConfigClient::new(env, &Self::addr(env, &DataKey::Config)).split(&fee_shares);
        let s = env.storage().instance();
        let p: i128 = s.get(&DataKey::AccruedProtocol).unwrap_or(0);
        let c: i128 = s.get(&DataKey::AccruedCreator).unwrap_or(0);
        s.set(&DataKey::AccruedProtocol, &(p + protocol));
        s.set(&DataKey::AccruedCreator, &(c + creator));
    }

    fn claim_bucket(env: &Env, key: &DataKey, protocol: bool) -> i128 {
        let owed: i128 = env.storage().instance().get(key).unwrap_or(0);
        if owed <= 0 {
            panic_with_error!(env, Error::NothingToClaim);
        }
        env.storage().instance().set(key, &0_i128);
        let config = ConfigClient::new(env, &Self::addr(env, &DataKey::Config));
        let payee = if protocol {
            config.treasury()
        } else {
            config.creator()
        };
        SYClient::new(env, &Self::addr(env, &DataKey::SY)).transfer(
            &env.current_contract_address(),
            &payee,
            &owed,
        );
        env.events()
            .publish((symbol_short!("fee_claim"),), (payee, owed, protocol));
        owed
    }
}
