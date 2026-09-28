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
//! * **Buy YT** is `Router.swap_sy_for_yt`: mint PT + YT from the user's SY, then sell the PT into
//!   this pool with `swap_pt_for_sy`; the user keeps the YT and the SY proceeds.
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
    contract, contractclient, contracterror, contractimpl, contracttype, panic_with_error,
    symbol_short, Address, Env,
};

use principal_compliance as compliance;

pub const SCALE: i128 = 10_000_000;
/// Fixed-point denominator of the swap fee rate returned by `MarketConfig` (1e12).
pub const FEE_SCALE: i128 = 1_000_000_000_000;
pub const SECONDS_PER_YEAR: u64 = 31_536_000;
/// LP permanently locked by the first deposit, so the pool can never be emptied to zero supply.
pub const MINIMUM_LIQUIDITY: i128 = 1_000;
/// A swap may never leave either reserve below this many raw units.
pub const MIN_RESERVE: i128 = 1_000;
const MAX_ORACLE_STALENESS_SECS: u64 = 3_600;
const BALANCE_TTL_LEDGERS: u32 = 518_400;
/// Bisection steps for the single-sided zap split.
const ZAP_ITERATIONS: u32 = 18;

// ---------------------------------------------------------------------------
// External interfaces
// ---------------------------------------------------------------------------

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
}

#[contractclient(name = "SYClient")]
pub trait SYInterface {
    fn transfer(env: Env, from: Address, to: Address, amount: i128) -> i128;
}

#[contractclient(name = "TokenClient")]
pub trait TokenInterface {
    fn transfer(env: Env, from: Address, to: Address, amount: i128);
}

#[contractclient(name = "OracleClient")]
pub trait OracleInterface {
    fn get_reference_value(env: Env) -> i128;
    fn is_fresh(env: Env, max_stale_seconds: u64) -> bool;
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
            if sy_desired * rate / SCALE > pt_desired {
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

        Self::set_reserves(&env, pt1 + pt_used, sy1 + sy_used);
        Self::accrue_fee(&env, q.fee_shares);
        Self::credit_lp(&env, &from, lp);
        Self::set_total_lp(&env, st.total_lp + lp);

        // The user pays `split + sy_used` SY in total; the PT they "bought" never leaves the pool
        // except for `pt_bought - pt_used`, refunded here.
        let this = env.current_contract_address();
        let sy = SYClient::new(&env, &Self::addr(&env, &DataKey::SY));
        sy.transfer(&from, &this, &(split + sy_used));
        let pt_refund = pt_bought - pt_used;
        if pt_refund > 0 {
            TokenClient::new(&env, &Self::addr(&env, &DataKey::PT))
                .transfer(&this, &from, &pt_refund);
        }
        env.events().publish(
            (symbol_short!("zap_liq"),),
            (from, pt_used, split + sy_used, lp),
        );
        (pt_used, split + sy_used, lp)
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
        if !OracleClient::new(env, &Self::addr(env, &DataKey::Oracle))
            .is_fresh(&MAX_ORACLE_STALENESS_SECS)
        {
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
