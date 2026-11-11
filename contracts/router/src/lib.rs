//! Router — one-transaction user flows across the whole Principal stack.
//!
//! The Router is **stateless with respect to funds**: it never custodies tokens, never needs to
//! be authorized on the underlying, and holds no balances. It only sequences calls, always acting
//! *as the user* (`from`) -- so every downstream compliance check (underlying authorization,
//! Permissioning, per-asset PT/YT eligibility) is evaluated against the real user, never against
//! the Router. A user the issuer has deauthorized cannot use the Router to do anything they could
//! not do directly.
//!
//! # Markets
//! A market is addressed by its `MarketPool`; everything else (manager, SY, PT, YT, underlying) is
//! read from it. The protocol admin registers pools (`register_market`) after cross-checking that
//! the pool, its manager and the SY/PT/YT the pool trades all describe the same market. Every user
//! call must name a registered pool, so a user cannot be steered into an arbitrary contract.
//!
//! # Guards on every call
//! * `deadline` -- a ledger timestamp; the call reverts `DeadlineExpired` if the transaction is
//!   included later than that.
//! * `min_*_out` -- a minimum output; the call reverts `SlippageExceeded` below it (the pool and
//!   wrapper enforce their own minimums too).
//!
//! # Flows
//! | Function                  | Path                                                              |
//! |---------------------------|-------------------------------------------------------------------|
//! | `wrap_and_mint`           | underlying -> SY (`SYWrapper.deposit`) -> PT + YT (`PrincipalManager.mint`) |
//! | `unwrap`                  | SY -> underlying (`SYWrapper.withdraw`)                           |
//! | `swap_sy_for_pt` / `swap_underlying_for_pt` | SY -> PT, or underlying -> SY -> PT in one call (`MarketPool`) |
//! | `swap_pt_for_sy` / `swap_pt_for_underlying` | PT -> SY, or PT -> SY -> underlying in one call (`MarketPool`) |
//! | `swap_sy_for_yt` / `swap_underlying_for_yt` | **flash-mint**: mint PT + YT from the SY (or from underlying, wrapped first), sell the PT back to the pool, keep the YT and the SY (or underlying) proceeds |
//! | `flash_mint_yt` / `flash_mint_yt_from_underlying` | **true flash-mint**: as above, but funded from the pool's own SY reserves -- `from` pays only the net cost, not the full notional; the underlying-denominated version also returns any unused wrap budget, unwrapped |
//! | `swap_yt_for_sy` / `swap_yt_for_underlying` | **flash-redeem**: the pool recombines the YT with its own PT and pays out the difference, in SY or unwrapped to underlying |
//! | `add_liquidity` / `add_liquidity_single_sy` / `add_liquidity_single_underlying` / `add_liquidity_keep_yt` | `MarketPool` LP operations; `_single_underlying` wraps first so a holder of the underlying needs one signature, not two; `_keep_yt` mints PT+YT from underlying, deposits the PT, and lets the caller keep the YT instead of selling it |
//! | `remove_liquidity` / `remove_liquidity_to_underlying` | LP -> PT + SY, or LP -> underlying in one call (sells the PT leg, unwraps both) |
//! | `recombine`               | PT + YT -> SY before maturity                                     |
//! | `redeem_at_maturity`      | PT and/or YT -> underlying after maturity                         |
//! | `claim_yield`             | accrued YT yield -> underlying, without burning YT                |
//!
//! # `_for_underlying` / `_for_yt` from underlying: not a flash loan
//! The underlying-denominated trading functions still require `from` to hold the full amount
//! being spent (or, for `swap_underlying_for_yt`, the full `underlying_amount` being tokenized,
//! not just the eventual net cost) in their own balance before the call -- the Router wraps and
//! sequences existing calls inside one atomic transaction, it does not advance capital. A true
//! flash swap, where the pool fronts the capital and the user only ever needs to cover the net
//! cost, is a larger change to how `MarketPool`/`PrincipalManager` custody funds and is out of
//! scope here; see `docs/DAPP_SMART_CONTRACT_INTEGRATION_GUIDE.md` for the current leverage
//! limitation this implies for YT purchases.
//!
//! # Why several functions take an amount as an argument instead of deriving it mid-execution,
//! and why some need a zero pre-existing balance
//! `MarketPool`'s swap price depends on `seconds_to_maturity`, read live from the ledger, and
//! `PrincipalManager.mint`'s PT/YT output depends on the live oracle rate -- so an amount
//! computed mid-transaction from either can differ -- by a few raw units -- between a client's
//! pre-submission simulation and real execution moments later. Soroban's pre-signed
//! authorization for a *further* call must match that call's real arguments exactly, so chaining
//! such a derived amount straight into another call that needs fresh `from.require_auth()` on
//! that exact value reverts `Error(Auth, InvalidAction)` -- through no fault of the trade itself.
//! This first broke a draft of `swap_pt_for_underlying` live on Testnet (see
//! `docs/DAPP_SMART_CONTRACT_INTEGRATION_GUIDE.md`), and a later integration review found the
//! same class of bug still live in `add_liquidity_keep_yt` (its `sy_remaining` and
//! `minted.pt_minted` were both derived, mid-execution, from a deposit's live-rate-dependent
//! share count and a mint's live-rate-dependent notional) and in `swap_underlying_for_yt` (its
//! `sy_in` was the same kind of derived deposit output) -- fixed the same way below.
//!
//! Two fixes are used throughout this contract, never a derived amount passed straight through:
//! * **A caller-supplied literal, quoted off-chain before the call is built** -- `mint_amount`,
//!   now also `pt_desired`/`sy_desired`, for `add_liquidity_keep_yt` (quoted via
//!   `MarketPool.keep_yt_split` and `reserves()`/`lp_total_supply()`, the same way every other
//!   min-out in this Router is quoted); `sy_in` for `swap_underlying_for_yt` (quoted via
//!   `SYWrapper.exchange_rate()`); `sy_in` for `add_liquidity_single_underlying` (same).
//! * **A sweep that reads the balance at execution time instead of taking it as an argument** --
//!   `SYWrapper.withdraw_all` (used by `swap_pt_for_underlying`, `swap_yt_for_underlying`,
//!   `swap_underlying_for_yt`) and `MarketPool.swap_all_pt_for_sy` (used by
//!   `remove_liquidity_to_underlying`) have nothing curve- or rate-derived left to authorize --
//!   but they act on *everything* `from` holds, hence the zero-pre-existing-balance precondition
//!   (`PreexistingSyBalance`/`PreexistingPtBalance`) on every function that uses one.
//!   `SYWrapper.withdraw_gained` (used by `flash_mint_yt_from_underlying`) relaxes that
//!   precondition by taking a `baseline` -- the balance `from` held *before* the step that
//!   produced the gain, read by this Router with a plain `balance_of` call -- and withdrawing
//!   only the difference, so a caller who already holds unrelated SY does not need to clear it
//!   first; see its own docs for the much smaller residual risk that `baseline` itself carries.

#![no_std]
#![allow(deprecated)]
// `env.events().publish` / `register_contract`: migration to `#[contractevent]` is tracked separately; event topics are kept stable for indexers.
// Several entrypoints here deliberately compose 8-9 calls' worth of arguments into one atomic,
// one-signature transaction (see the module docs above) -- a per-function `#[allow]` does not
// reliably suppress this lint for every item `#[contractimpl]`/`#[contractargs]` generates from
// such a function, so it is allowed crate-wide instead. The per-function `#[allow]`s on those
// entrypoints are kept anyway, as documentation of exactly which ones are the intentional cases.
#![allow(clippy::too_many_arguments)]

use soroban_sdk::{
    contract, contractclient, contracterror, contractimpl, contracttype, panic_with_error,
    symbol_short, Address, Env,
};

// ---------------------------------------------------------------------------
// External interfaces (minimal, so this crate links no other contract's exports)
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone)]
pub struct MintResult {
    pub pt_minted: i128,
    pub yt_minted: i128,
    pub fee_shares: i128,
}

#[contracttype]
#[derive(Clone)]
pub struct RedeemResult {
    pub underlying_from_pt: i128,
    pub underlying_from_yt: i128,
}

#[contracttype]
#[derive(Clone)]
pub struct KeepYtResult {
    pub pt_used: i128,
    pub sy_used: i128,
    pub lp_minted: i128,
    pub yt_minted: i128,
}

/// Mirrors `MarketPool`'s own `Quote` shape (PT/SY-denominated, field-for-field) -- used for a
/// Router quote whose `amount_out` is already PT, not underlying, because only the *input* leg
/// of that flow involves a unit conversion.
#[contracttype]
#[derive(Clone)]
pub struct Quote {
    pub amount_out: i128,
    pub amount_in: i128,
    pub fee_shares: i128,
}

/// Mirrors `MarketPool`'s own `FlashMintQuote` shape, field-for-field.
#[contracttype]
#[derive(Clone)]
pub struct FlashMintQuote {
    pub yt_out: i128,
    pub sy_back: i128,
    pub mint_fee_shares: i128,
    pub swap_fee_shares: i128,
}

/// A `MarketPool`-style quote restated entirely in underlying units -- `amount_out` and the fee
/// both converted via `SYWrapper.exchange_rate()` -- for a Router flow whose final leg unwraps
/// SY back to underlying (`quote_pt_for_underlying`, `quote_yt_for_underlying`).
#[contracttype]
#[derive(Clone)]
pub struct UnderlyingQuote {
    pub amount_out: i128,
    pub amount_in: i128,
    pub fee_underlying: i128,
}

/// `FlashMintQuote` restated in underlying units, for `quote_underlying_for_yt`.
#[contracttype]
#[derive(Clone)]
pub struct UnderlyingFlashMintQuote {
    pub yt_out: i128,
    pub underlying_back: i128,
    pub mint_fee_underlying: i128,
    pub swap_fee_underlying: i128,
}

/// Preview for `add_liquidity_keep_yt`. `mint_amount`, `pt_desired` and `sy_desired` are exactly
/// what the real call's own arguments of the same names expect -- all three are now solved via
/// `MarketPool.keep_yt_split` against the pool's raw reserve ratio (the ratio `add_liquidity`'s
/// proportional deposit actually targets), not `zap_swap_amount`'s curve-swap ratio, which a
/// later integration review found recommended the wrong split and left SY stranded. `lp_estimate`
/// previews `add_liquidity`'s own `lp_minted` using the same `(pt_desired * total_lp /
/// pt_reserve).min(sy_desired * total_lp / sy_reserve)` formula -- like every quote in this
/// protocol, exact only if reserves/`total_lp` do not move between this read and the real call.
#[contracttype]
#[derive(Clone)]
pub struct KeepYtQuote {
    pub mint_amount: i128,
    pub pt_desired: i128,
    pub sy_desired: i128,
    pub lp_estimate: i128,
    pub yt_out: i128,
    pub mint_fee_underlying: i128,
}

#[contractclient(name = "PoolClient")]
pub trait PoolInterface {
    fn manager_address(env: Env) -> Address;
    fn sy_address(env: Env) -> Address;
    fn pt_address(env: Env) -> Address;
    fn yt_address(env: Env) -> Address;
    fn underlying_address(env: Env) -> Address;
    fn swap_sy_for_pt(env: Env, from: Address, to: Address, sy_in: i128, min_pt_out: i128) -> i128;
    fn swap_pt_for_sy(env: Env, from: Address, to: Address, pt_in: i128, min_sy_out: i128) -> i128;
    fn swap_yt_for_sy(env: Env, from: Address, to: Address, yt_in: i128, min_sy_out: i128) -> i128;
    fn flash_mint_yt(
        env: Env,
        from: Address,
        to: Address,
        sy_in: i128,
        min_yt_out: i128,
        max_net_cost: i128,
    ) -> (i128, i128);
    fn add_liquidity(
        env: Env,
        from: Address,
        pt_desired: i128,
        sy_desired: i128,
        min_lp_out: i128,
    ) -> (i128, i128, i128);
    fn add_liquidity_single_sy(
        env: Env,
        from: Address,
        sy_in: i128,
        min_lp_out: i128,
    ) -> (i128, i128, i128);
    fn remove_liquidity(
        env: Env,
        from: Address,
        to: Address,
        lp: i128,
        min_pt_out: i128,
        min_sy_out: i128,
    ) -> (i128, i128);
    fn swap_all_pt_for_sy(env: Env, from: Address, to: Address, min_sy_out: i128) -> i128;
    fn zap_swap_amount(env: Env, sy_in: i128) -> i128;
    fn keep_yt_split(env: Env, sy_shares: i128) -> i128;
    fn quote_sy_for_pt(env: Env, sy_in: i128) -> Quote;
    fn quote_pt_for_sy(env: Env, pt_in: i128) -> Quote;
    fn quote_yt_for_sy(env: Env, yt_in: i128) -> Quote;
    fn quote_sy_for_yt(env: Env, sy_in: i128) -> FlashMintQuote;
    fn reserves(env: Env) -> (i128, i128);
    fn lp_total_supply(env: Env) -> i128;
}

#[contractclient(name = "ManagerClient")]
pub trait ManagerInterface {
    fn underlying_address(env: Env) -> Address;
    fn sy_wrapper_address(env: Env) -> Address;
    fn pt_address(env: Env) -> Address;
    fn yt_address(env: Env) -> Address;
    fn mint(env: Env, from: Address, sy_shares: i128) -> MintResult;
    fn recombine(env: Env, from: Address, amount: i128) -> i128;
    fn redeem(env: Env, from: Address, pt_amount: i128, yt_amount: i128) -> RedeemResult;
    fn claim_yield(env: Env, from: Address) -> i128;
}

#[contractclient(name = "SYClient")]
pub trait SYInterface {
    fn deposit(env: Env, from: Address, amount: i128, min_shares_out: i128) -> i128;
    fn withdraw(
        env: Env,
        from: Address,
        shares: i128,
        to: Address,
        min_underlying_out: i128,
    ) -> i128;
    fn withdraw_all(env: Env, from: Address, to: Address, min_underlying_out: i128) -> i128;
    fn withdraw_gained(
        env: Env,
        from: Address,
        baseline: i128,
        to: Address,
        min_underlying_out: i128,
    ) -> i128;
    fn balance_of(env: Env, account: Address) -> i128;
    fn exchange_rate(env: Env) -> i128;
}

#[contractclient(name = "PtClient")]
pub trait PtInterface {
    fn balance(env: Env, account: Address) -> i128;
}

// ---------------------------------------------------------------------------
// Errors / storage
// ---------------------------------------------------------------------------

/// Instance-storage TTL applied by `bump` (~30 days at 5 s per ledger).
const INSTANCE_TTL_LEDGERS: u32 = 518_400;
/// TTL applied to a market listing each time it is used or registered.
const MARKET_TTL_LEDGERS: u32 = 518_400;
/// Matches `SYWrapper::RATE_SCALE` -- `exchange_rate()` is underlying units per share at this
/// scale, used to convert a quote's SY-denominated legs to/from underlying.
const RATE_SCALE: i128 = 10_000_000;

#[contracterror]
#[derive(Copy, Clone, Debug, PartialEq)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    Unauthorized = 3,
    MarketNotRegistered = 4,
    DeadlineExpired = 5,
    SlippageExceeded = 6,
    TopologyMismatch = 7,
    AlreadyRegistered = 8,
    /// Returned by the `_for_underlying` functions when `from` already holds SY before the call:
    /// they withdraw `from`'s *entire* SY balance at the end (see `SYWrapper.withdraw_all`'s
    /// docs), which would sweep away SY the caller did not mean to touch.
    PreexistingSyBalance = 9,
    /// Returned by `remove_liquidity_to_underlying` when `from` already holds PT before the
    /// call: it sells `from`'s *entire* PT balance at the end (see `MarketPool.swap_all_pt_for_sy`),
    /// which would sweep away PT the caller did not mean to touch.
    PreexistingPtBalance = 10,
}

/// Everything the Router needs to know about one market, resolved from its pool at registration.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct MarketInfo {
    pub pool: Address,
    pub manager: Address,
    pub sy: Address,
    pub pt: Address,
    pub yt: Address,
    pub underlying: Address,
}

#[contracttype]
pub enum DataKey {
    Admin,
    Market(Address),
}

#[contract]
pub struct RouterContract;

#[contractimpl]
impl RouterContract {
    /// Permissionless keeper call: extend this contract's *instance* storage (admin, config,
    /// reserves, totals, settlement rate, fee buckets) for ~30 days. Soroban does not bump instance
    /// TTL on ordinary reads or writes, so a long-dated market needs this called periodically (or
    /// an archived instance restored) -- see docs/DEPLOYMENT.md.
    pub fn bump(env: Env) {
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_TTL_LEDGERS, INSTANCE_TTL_LEDGERS);
    }

    pub fn initialize(env: Env, admin: Address) {
        if env.storage().instance().has(&DataKey::Admin) {
            panic_with_error!(&env, Error::AlreadyInitialized);
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
    }

    // --------------------------------------------------------------- registry

    /// Register a market by its pool, after checking that the pool, its manager and the tokens the
    /// pool trades all describe the same market. Admin-only.
    pub fn register_market(env: Env, caller: Address, pool: Address) -> MarketInfo {
        Self::assert_admin(&env, &caller);
        if env
            .storage()
            .persistent()
            .has(&DataKey::Market(pool.clone()))
        {
            panic_with_error!(&env, Error::AlreadyRegistered);
        }
        let p = PoolClient::new(&env, &pool);
        let manager = p.manager_address();
        let m = ManagerClient::new(&env, &manager);
        let info = MarketInfo {
            pool: pool.clone(),
            manager: manager.clone(),
            sy: m.sy_wrapper_address(),
            pt: m.pt_address(),
            yt: m.yt_address(),
            underlying: m.underlying_address(),
        };
        if p.sy_address() != info.sy
            || p.pt_address() != info.pt
            || p.yt_address() != info.yt
            || p.underlying_address() != info.underlying
        {
            panic_with_error!(&env, Error::TopologyMismatch);
        }
        env.storage()
            .persistent()
            .set(&DataKey::Market(pool.clone()), &info);
        env.events().publish((symbol_short!("mkt_reg"),), pool);
        info
    }

    /// Remove a market from the registry (a mistaken or retired listing). Admin-only; users can no
    /// longer route through it, existing positions are unaffected.
    pub fn unregister_market(env: Env, caller: Address, pool: Address) {
        Self::assert_admin(&env, &caller);
        if !env
            .storage()
            .persistent()
            .has(&DataKey::Market(pool.clone()))
        {
            panic_with_error!(&env, Error::MarketNotRegistered);
        }
        env.storage()
            .persistent()
            .remove(&DataKey::Market(pool.clone()));
        env.events().publish((symbol_short!("mkt_unreg"),), pool);
    }

    pub fn is_registered(env: Env, pool: Address) -> bool {
        env.storage().persistent().has(&DataKey::Market(pool))
    }

    pub fn market(env: Env, pool: Address) -> MarketInfo {
        Self::info(&env, &pool)
    }

    pub fn get_admin(env: Env) -> Address {
        Self::require_admin(&env)
    }

    pub fn transfer_admin(env: Env, current_admin: Address, new_admin: Address) {
        Self::assert_admin(&env, &current_admin);
        env.storage().instance().set(&DataKey::Admin, &new_admin);
        env.events()
            .publish((symbol_short!("adm_xfer"),), (current_admin, new_admin));
    }

    // ---------------------------------------------------------------- wrapping

    /// Underlying -> SY -> PT + YT in one transaction. Reverts if fewer than `min_pt_out` PT are
    /// minted (the tokenization fee is withheld before PT/YT are sized).
    pub fn wrap_and_mint(
        env: Env,
        from: Address,
        pool: Address,
        amount: i128,
        min_pt_out: i128,
        deadline: u64,
    ) -> MintResult {
        from.require_auth();
        Self::assert_deadline(&env, deadline);
        let info = Self::info(&env, &pool);
        let shares = SYClient::new(&env, &info.sy).deposit(&from, &amount, &0);
        let result = ManagerClient::new(&env, &info.manager).mint(&from, &shares);
        if result.pt_minted < min_pt_out {
            panic_with_error!(&env, Error::SlippageExceeded);
        }
        result
    }

    /// SY -> underlying.
    pub fn unwrap(
        env: Env,
        from: Address,
        pool: Address,
        shares: i128,
        min_underlying_out: i128,
        deadline: u64,
    ) -> i128 {
        from.require_auth();
        Self::assert_deadline(&env, deadline);
        let info = Self::info(&env, &pool);
        SYClient::new(&env, &info.sy).withdraw(&from, &shares, &from, &min_underlying_out)
    }

    // ----------------------------------------------------------------- trading

    pub fn swap_sy_for_pt(
        env: Env,
        from: Address,
        pool: Address,
        sy_in: i128,
        min_pt_out: i128,
        deadline: u64,
    ) -> i128 {
        from.require_auth();
        Self::assert_deadline(&env, deadline);
        Self::info(&env, &pool);
        PoolClient::new(&env, &pool).swap_sy_for_pt(&from, &from, &sy_in, &min_pt_out)
    }

    pub fn swap_pt_for_sy(
        env: Env,
        from: Address,
        pool: Address,
        pt_in: i128,
        min_sy_out: i128,
        deadline: u64,
    ) -> i128 {
        from.require_auth();
        Self::assert_deadline(&env, deadline);
        Self::info(&env, &pool);
        PoolClient::new(&env, &pool).swap_pt_for_sy(&from, &from, &pt_in, &min_sy_out)
    }

    /// Underlying -> SY (`SYWrapper.deposit`) -> PT (`MarketPool`), in one transaction and one
    /// signature, for a user who holds the underlying rather than SY. The wrap leg is unguarded
    /// (`min_shares_out = 0`); `min_pt_out` protects the whole round trip, matching
    /// `wrap_and_mint`'s existing pattern of guarding only the final output.
    pub fn swap_underlying_for_pt(
        env: Env,
        from: Address,
        pool: Address,
        underlying_amount: i128,
        min_pt_out: i128,
        deadline: u64,
    ) -> i128 {
        from.require_auth();
        Self::assert_deadline(&env, deadline);
        let info = Self::info(&env, &pool);
        let shares = SYClient::new(&env, &info.sy).deposit(&from, &underlying_amount, &0);
        PoolClient::new(&env, &pool).swap_sy_for_pt(&from, &from, &shares, &min_pt_out)
    }

    /// PT -> SY (`MarketPool`) -> underlying (`SYWrapper.withdraw_all`), in one transaction. The
    /// swap leg is unguarded; `min_underlying_out` protects the whole round trip at the final,
    /// underlying-denominated output -- the same reasoning `swap_underlying_for_pt` uses in
    /// reverse.
    ///
    /// Uses `withdraw_all`, not `withdraw`, and so requires `from` to hold **no** SY before this
    /// call (reverts `PreexistingSyBalance` otherwise): the swap's SY payout is a curve-derived
    /// amount that can drift by a few raw units between simulation and real execution (see
    /// `SYWrapper.withdraw_all`'s docs), so passing it as an explicit, pre-signed `shares`
    /// argument to a plain `withdraw` reverted `Error(Auth, InvalidAction)` live on Testnet while
    /// this function was first written -- confirmed in `docs/DAPP_SMART_CONTRACT_INTEGRATION_GUIDE.md`.
    /// `withdraw_all` reads the balance at execution time instead, so there is nothing
    /// curve-derived left in the authorized invocation; the tradeoff is that it must be *all* of
    /// `from`'s SY, which is why any pre-existing balance is rejected rather than swept along with it.
    pub fn swap_pt_for_underlying(
        env: Env,
        from: Address,
        pool: Address,
        pt_in: i128,
        min_underlying_out: i128,
        deadline: u64,
    ) -> i128 {
        from.require_auth();
        Self::assert_deadline(&env, deadline);
        let info = Self::info(&env, &pool);
        let sy = SYClient::new(&env, &info.sy);
        if sy.balance_of(&from) != 0 {
            panic_with_error!(&env, Error::PreexistingSyBalance);
        }
        PoolClient::new(&env, &pool).swap_pt_for_sy(&from, &from, &pt_in, &0);
        sy.withdraw_all(&from, &from, &min_underlying_out)
    }

    /// **Flash-mint** buy of YT. Tokenizes `sy_in` SY into PT + YT, sells all of the PT back into
    /// the pool in the same transaction, and leaves the user with the YT plus the SY the PT
    /// fetched. Net cost of the YT is `sy_in - sy_back` SY. Returns `(yt_out, sy_back)`.
    ///
    /// Two guards: `min_yt_out` on the YT minted (fixed by the oracle rate and the tokenization
    /// fee -- it protects against a fee or rate change between quote and inclusion) and
    /// `max_net_cost` on the **price** the PT sale achieves: the call reverts `SlippageExceeded` if
    /// `sy_in - sy_back > max_net_cost`. Without the second guard the pool leg would run
    /// unprotected and could be sandwiched. No loan is involved: it is two synchronous calls in one
    /// atomic transaction, both made as the user.
    pub fn swap_sy_for_yt(
        env: Env,
        from: Address,
        pool: Address,
        sy_in: i128,
        min_yt_out: i128,
        max_net_cost: i128,
        deadline: u64,
    ) -> (i128, i128) {
        from.require_auth();
        Self::assert_deadline(&env, deadline);
        let info = Self::info(&env, &pool);
        let minted = ManagerClient::new(&env, &info.manager).mint(&from, &sy_in);
        if minted.yt_minted < min_yt_out {
            panic_with_error!(&env, Error::SlippageExceeded);
        }
        // The PT must fetch at least `sy_in - max_net_cost` SY, or the YT costs more than allowed.
        let min_sy_back = if max_net_cost >= sy_in {
            0
        } else {
            sy_in - max_net_cost
        };
        let sy_back = PoolClient::new(&env, &pool).swap_pt_for_sy(
            &from,
            &from,
            &minted.pt_minted,
            &min_sy_back,
        );
        (minted.yt_minted, sy_back)
    }

    /// Same flash-mint as `swap_sy_for_yt`, but for a caller who holds the underlying rather than
    /// SY: wraps `underlying_amount` into SY first, runs the identical flash-mint, then unwraps
    /// the SY the PT sale fetched back into underlying, so the whole round trip -- pay underlying,
    /// receive YT plus underlying change -- is one transaction and one signature. The pool never
    /// advances anything: `from` still needs `underlying_amount` in hand up front, exactly as
    /// `swap_sy_for_yt` needs `sy_in` up front (see the Router's module docs on why this is not a
    /// true flash loan).
    ///
    /// `sy_in` is a caller-supplied literal (quote `SYWrapper.exchange_rate()` off-chain:
    /// `underlying_amount * RATE_SCALE / exchange_rate()`, matching `SYWrapper.deposit`'s own
    /// rounding), **not** derived from `deposit`'s return value -- a later integration review
    /// found that chaining `deposit`'s live-rate-dependent output straight into `mint`'s
    /// authorized call had exactly the `Error(Auth, InvalidAction)` drift risk the Router's
    /// module docs describe, just not yet fixed here. If `sy_in` is larger than what `deposit`
    /// actually credited, `mint`'s own transfer simply fails on insufficient balance (an ordinary
    /// revert, not an auth error); if smaller, the shortfall is still recovered by the final
    /// `withdraw_all` sweep below, same as a generous `mint_amount` leaves a recoverable surplus
    /// with `add_liquidity_keep_yt`.
    ///
    /// The final unwrap uses `withdraw_all` and so requires `from` to hold no SY before this call
    /// (reverts `PreexistingSyBalance` otherwise) -- see `swap_pt_for_underlying`'s docs for why.
    #[allow(clippy::too_many_arguments)] // one entrypoint composing a wrap, mint and sell, by design.
    pub fn swap_underlying_for_yt(
        env: Env,
        from: Address,
        pool: Address,
        underlying_amount: i128,
        sy_in: i128,
        min_yt_out: i128,
        max_net_cost: i128,
        deadline: u64,
    ) -> (i128, i128) {
        from.require_auth();
        Self::assert_deadline(&env, deadline);
        let info = Self::info(&env, &pool);
        let sy = SYClient::new(&env, &info.sy);
        if sy.balance_of(&from) != 0 {
            panic_with_error!(&env, Error::PreexistingSyBalance);
        }
        sy.deposit(&from, &underlying_amount, &0);
        let minted = ManagerClient::new(&env, &info.manager).mint(&from, &sy_in);
        if minted.yt_minted < min_yt_out {
            panic_with_error!(&env, Error::SlippageExceeded);
        }
        let min_sy_back = if max_net_cost >= sy_in {
            0
        } else {
            sy_in - max_net_cost
        };
        PoolClient::new(&env, &pool).swap_pt_for_sy(&from, &from, &minted.pt_minted, &min_sy_back);
        let underlying_back = sy.withdraw_all(&from, &from, &0);
        (minted.yt_minted, underlying_back)
    }

    /// **True flash-mint** of YT: forwards to `MarketPool.flash_mint_yt`, which funds the mint
    /// leg from the pool's own SY reserves instead of `from`'s balance. `from` pays only the net
    /// cost (`MarketPool.flash_mint_yt`'s own docs explain the accounting identity), not the full
    /// `sy_in` notional `swap_sy_for_yt` above requires. Returns `(yt_out, net_cost)`.
    pub fn flash_mint_yt(
        env: Env,
        from: Address,
        pool: Address,
        sy_in: i128,
        min_yt_out: i128,
        max_net_cost: i128,
        deadline: u64,
    ) -> (i128, i128) {
        from.require_auth();
        Self::assert_deadline(&env, deadline);
        Self::info(&env, &pool);
        PoolClient::new(&env, &pool).flash_mint_yt(&from, &from, &sy_in, &min_yt_out, &max_net_cost)
    }

    /// Same true flash-mint as `flash_mint_yt`, for a caller who holds the underlying rather than
    /// SY. Wraps `underlying_budget` of underlying into SY first -- not the full `sy_in` notional,
    /// since the whole point of this function is that the caller never needs the full notional --
    /// so `from`'s SY balance covers whatever the real net cost turns out to be, as long as the
    /// caller chose `underlying_budget` generously enough to cover `max_net_cost` SY shares at the
    /// current `SYWrapper.exchange_rate()` (a plain off-chain conversion, the same kind every other
    /// `_for_underlying` caller already does). `max_net_cost` is SY-denominated and passed through
    /// to the pool unchanged -- kept as its own parameter, not derived from `underlying_budget`,
    /// because the two are only numerically equal while `exchange_rate()` is exactly 1.0 (true for
    /// every `SYWrapper` deployed today, not guaranteed forever; see `docs/ROUTER_QUOTES.md` §2).
    ///
    /// Any surplus (`underlying_budget` wrapped minus the real net cost) is unwrapped back to
    /// underlying before this call returns, via `SYWrapper.withdraw_gained` with a `baseline`
    /// read right before the wrap leg -- so, unlike the other `_for_underlying` functions, a
    /// caller who already holds unrelated SY does **not** need to clear it first (no
    /// `PreexistingSyBalance` precondition here). An earlier version left the surplus as plain SY
    /// in `from`'s balance, which a later integration review flagged as a real cost: any further
    /// same-transaction call using `withdraw_all` (every other `_for_underlying` flow) would then
    /// revert `PreexistingSyBalance` against that very surplus, needing an extra signature just
    /// to clear it first. Returns `(yt_out, net_cost, underlying_back)`.
    #[allow(clippy::too_many_arguments)] // one entrypoint composing a wrap plus a flash-mint, by design.
    pub fn flash_mint_yt_from_underlying(
        env: Env,
        from: Address,
        pool: Address,
        sy_in: i128,
        min_yt_out: i128,
        max_net_cost: i128,
        underlying_budget: i128,
        min_underlying_back: i128,
        deadline: u64,
    ) -> (i128, i128, i128) {
        from.require_auth();
        Self::assert_deadline(&env, deadline);
        let info = Self::info(&env, &pool);
        let sy = SYClient::new(&env, &info.sy);
        let baseline = sy.balance_of(&from);
        if underlying_budget > 0 {
            sy.deposit(&from, &underlying_budget, &0);
        }
        let (yt_out, net_cost) = PoolClient::new(&env, &pool).flash_mint_yt(
            &from,
            &from,
            &sy_in,
            &min_yt_out,
            &max_net_cost,
        );
        let underlying_back = sy.withdraw_gained(&from, &baseline, &from, &min_underlying_back);
        (yt_out, net_cost, underlying_back)
    }

    /// **Flash-redeem** sale of YT: the pool recombines the YT with PT from its own reserve and
    /// pays the difference in SY. Returns the SY received; reverts below `min_sy_out`.
    pub fn swap_yt_for_sy(
        env: Env,
        from: Address,
        pool: Address,
        yt_in: i128,
        min_sy_out: i128,
        deadline: u64,
    ) -> i128 {
        from.require_auth();
        Self::assert_deadline(&env, deadline);
        Self::info(&env, &pool);
        PoolClient::new(&env, &pool).swap_yt_for_sy(&from, &from, &yt_in, &min_sy_out)
    }

    /// Same flash-redeem as `swap_yt_for_sy`, but unwraps the SY proceeds into underlying in the
    /// same transaction, for a caller who wants to receive the underlying directly. Uses
    /// `withdraw_all` and so requires `from` to hold no SY before the call -- see
    /// `swap_pt_for_underlying`'s docs for why.
    pub fn swap_yt_for_underlying(
        env: Env,
        from: Address,
        pool: Address,
        yt_in: i128,
        min_underlying_out: i128,
        deadline: u64,
    ) -> i128 {
        from.require_auth();
        Self::assert_deadline(&env, deadline);
        let info = Self::info(&env, &pool);
        let sy = SYClient::new(&env, &info.sy);
        if sy.balance_of(&from) != 0 {
            panic_with_error!(&env, Error::PreexistingSyBalance);
        }
        PoolClient::new(&env, &pool).swap_yt_for_sy(&from, &from, &yt_in, &0);
        sy.withdraw_all(&from, &from, &min_underlying_out)
    }

    // --------------------------------------------------------------- liquidity

    pub fn add_liquidity(
        env: Env,
        from: Address,
        pool: Address,
        pt_in: i128,
        sy_in: i128,
        min_lp_out: i128,
        deadline: u64,
    ) -> (i128, i128, i128) {
        from.require_auth();
        Self::assert_deadline(&env, deadline);
        Self::info(&env, &pool);
        PoolClient::new(&env, &pool).add_liquidity(&from, &pt_in, &sy_in, &min_lp_out)
    }

    pub fn add_liquidity_single_sy(
        env: Env,
        from: Address,
        pool: Address,
        sy_in: i128,
        min_lp_out: i128,
        deadline: u64,
    ) -> (i128, i128, i128) {
        from.require_auth();
        Self::assert_deadline(&env, deadline);
        Self::info(&env, &pool);
        PoolClient::new(&env, &pool).add_liquidity_single_sy(&from, &sy_in, &min_lp_out)
    }

    /// Underlying -> SY (`SYWrapper.deposit`) -> single-sided LP deposit (`MarketPool
    /// .add_liquidity_single_sy`), in one transaction and one signature, for a user who holds
    /// the underlying rather than SY -- found missing in an integration review (single-sided LP
    /// from underlying previously needed a separate `deposit` first, two signatures).
    ///
    /// `sy_in` is a caller-supplied literal (quote `SYWrapper.exchange_rate()` off-chain, the
    /// same `underlying_amount * RATE_SCALE / exchange_rate()` used throughout this Router), not
    /// derived from `deposit`'s return value, for the same reason `swap_underlying_for_yt` takes
    /// `sy_in` as an argument rather than deriving it -- see the module docs. The wrap leg is
    /// unguarded (`min_shares_out = 0`); `min_lp_out` protects the whole round trip.
    pub fn add_liquidity_single_underlying(
        env: Env,
        from: Address,
        pool: Address,
        underlying_amount: i128,
        sy_in: i128,
        min_lp_out: i128,
        deadline: u64,
    ) -> (i128, i128, i128) {
        from.require_auth();
        Self::assert_deadline(&env, deadline);
        let info = Self::info(&env, &pool);
        SYClient::new(&env, &info.sy).deposit(&from, &underlying_amount, &0);
        PoolClient::new(&env, &pool).add_liquidity_single_sy(&from, &sy_in, &min_lp_out)
    }

    /// Pendle's "keep YT" deposit: `underlying_amount` of underlying becomes LP *and* YT, with no
    /// swap and so no price-impact slippage (the only guard that applies is `min_lp_out`, exactly
    /// as `add_liquidity`'s own proportional deposit already works). Wraps to SY, mints PT+YT from
    /// `mint_amount` of it, then adds PT and SY as a normal proportional deposit -- keeping the YT
    /// instead of selling it, which is the one difference from `add_liquidity_single_sy`.
    ///
    /// `mint_amount`, `pt_desired` and `sy_desired` are all plain arguments, computed off-chain
    /// (quote `MarketPool.keep_yt_split` and `reserves()`/`lp_total_supply()` -- or just call
    /// `quote_add_liquidity_keep_yt`, which does exactly that -- before building this call, the
    /// same way every other min-out in this Router is quoted beforehand) rather than anything
    /// this function derives mid-execution. That distinction matters: an earlier version derived
    /// `sy_remaining` from `deposit`'s live-exchange-rate-dependent output and `pt_desired` from
    /// `mint`'s live-oracle-rate-dependent output, then passed both straight into `add_liquidity`'s
    /// own authorized call -- exactly the `Error(Auth, InvalidAction)` drift risk the module docs
    /// describe, found live in a later integration review. Three literal, pre-quoted arguments
    /// have no such drift to go stale.
    ///
    /// Values that don't match the pool's current state (a stale quote, or just a round number
    /// the caller chose) may leave a small PT or SY remainder unused, or make `mint`'s own SY
    /// transfer revert on insufficient balance (an ordinary revert, not an auth error) if
    /// `mint_amount` was quoted too high against what `deposit` actually credited: that's fine,
    /// `add_liquidity`'s proportional deposit only ever pulls the `pt_used`/`sy_used` it actually
    /// needs (see its implementation) and leaves the rest in `from`'s own balance untouched --
    /// there is nothing to refund and nothing gets stranded.
    #[allow(clippy::too_many_arguments)] // one entrypoint composing a wrap, mint and LP deposit, by design.
    pub fn add_liquidity_keep_yt(
        env: Env,
        from: Address,
        pool: Address,
        underlying_amount: i128,
        mint_amount: i128,
        pt_desired: i128,
        sy_desired: i128,
        min_lp_out: i128,
        deadline: u64,
    ) -> KeepYtResult {
        from.require_auth();
        Self::assert_deadline(&env, deadline);
        let info = Self::info(&env, &pool);
        let pool_client = PoolClient::new(&env, &pool);
        SYClient::new(&env, &info.sy).deposit(&from, &underlying_amount, &0);
        let minted = ManagerClient::new(&env, &info.manager).mint(&from, &mint_amount);
        let (pt_used, sy_used, lp_minted) =
            pool_client.add_liquidity(&from, &pt_desired, &sy_desired, &min_lp_out);
        KeepYtResult {
            pt_used,
            sy_used,
            lp_minted,
            yt_minted: minted.yt_minted,
        }
    }

    // ------------------------------------------------- quotes (underlying-denominated flows)
    //
    // Previewing any of the five `_for_underlying`/`_for_yt`/`add_liquidity_keep_yt` flows above
    // used to mean calling `MarketPool`'s own SY-denominated quote and separately calling
    // `SYWrapper.exchange_rate()` to convert by hand. These five compose the two calls so a
    // caller gets one underlying-denominated (or, for `quote_underlying_for_pt`, already
    // PT-denominated) answer. Each replicates the exact rounding of its real call -- same
    // formula, same single `exchange_rate()` read -- so the only drift against the real
    // transaction is the same unavoidable one every quote in this protocol already has: time
    // passing between simulating the quote and the real call landing (MarketPool's curve is
    // `seconds_to_maturity`-dependent; see `docs/DAPP_SMART_CONTRACT_INTEGRATION_GUIDE.md` §2's
    // third gotcha and §5.4's live-measured example of it).

    /// Preview `swap_underlying_for_pt`. `amount_out` is PT (unaffected by the wrap leg);
    /// `fee_shares` is the AMM swap fee, in SY, exactly as `MarketPool.quote_sy_for_pt` reports it
    /// -- there is nothing to convert on the output side, only the input (`underlying_amount` ->
    /// SY shares) feeds into the pool quote.
    pub fn quote_underlying_for_pt(env: Env, pool: Address, underlying_amount: i128) -> Quote {
        let info = Self::info(&env, &pool);
        let rate = SYClient::new(&env, &info.sy).exchange_rate();
        let shares = underlying_amount * RATE_SCALE / rate;
        PoolClient::new(&env, &pool).quote_sy_for_pt(&shares)
    }

    /// Preview `swap_pt_for_underlying`: `MarketPool.quote_pt_for_sy`'s SY-denominated
    /// `amount_out`/`fee_shares`, both converted to underlying via the same `exchange_rate()`
    /// read `SYWrapper.withdraw_all` would apply to the swap's real SY payout.
    pub fn quote_pt_for_underlying(env: Env, pool: Address, pt_in: i128) -> UnderlyingQuote {
        let info = Self::info(&env, &pool);
        let q = PoolClient::new(&env, &pool).quote_pt_for_sy(&pt_in);
        let rate = SYClient::new(&env, &info.sy).exchange_rate();
        UnderlyingQuote {
            amount_out: q.amount_out * rate / RATE_SCALE,
            amount_in: 0,
            fee_underlying: q.fee_shares * rate / RATE_SCALE,
        }
    }

    /// Preview `swap_underlying_for_yt`: wraps `underlying_amount` to SY shares, previews the
    /// flash-mint via `MarketPool.quote_sy_for_yt`, then converts `sy_back` and both fee legs to
    /// underlying the same way the real call's final `withdraw_all` would.
    pub fn quote_underlying_for_yt(
        env: Env,
        pool: Address,
        underlying_amount: i128,
    ) -> UnderlyingFlashMintQuote {
        let info = Self::info(&env, &pool);
        let rate = SYClient::new(&env, &info.sy).exchange_rate();
        let sy_in = underlying_amount * RATE_SCALE / rate;
        let flash = PoolClient::new(&env, &pool).quote_sy_for_yt(&sy_in);
        UnderlyingFlashMintQuote {
            yt_out: flash.yt_out,
            underlying_back: flash.sy_back * rate / RATE_SCALE,
            mint_fee_underlying: flash.mint_fee_shares * rate / RATE_SCALE,
            swap_fee_underlying: flash.swap_fee_shares * rate / RATE_SCALE,
        }
    }

    /// Preview `swap_yt_for_underlying`: `MarketPool.quote_yt_for_sy`'s SY-denominated payout and
    /// fee, converted to underlying.
    pub fn quote_yt_for_underlying(env: Env, pool: Address, yt_in: i128) -> UnderlyingQuote {
        let info = Self::info(&env, &pool);
        let q = PoolClient::new(&env, &pool).quote_yt_for_sy(&yt_in);
        let rate = SYClient::new(&env, &info.sy).exchange_rate();
        UnderlyingQuote {
            amount_out: q.amount_out * rate / RATE_SCALE,
            amount_in: 0,
            fee_underlying: q.fee_shares * rate / RATE_SCALE,
        }
    }

    /// Preview `add_liquidity_keep_yt`. Returns the exact `mint_amount` to pass into that call
    /// (the SY split `MarketPool.zap_swap_amount` recommends for `underlying_amount`'s worth of
    /// wrapped shares), the SY left over for the proportional deposit leg, and the YT the mint
    /// alone would produce (via `MarketPool.quote_sy_for_yt`'s mint leg -- its sell-PT leg is
    /// irrelevant here since this flow deposits the minted PT as liquidity instead of selling it).
    pub fn quote_add_liquidity_keep_yt(
        env: Env,
        pool: Address,
        underlying_amount: i128,
    ) -> KeepYtQuote {
        let info = Self::info(&env, &pool);
        let rate = SYClient::new(&env, &info.sy).exchange_rate();
        let shares = underlying_amount * RATE_SCALE / rate;
        let pool_client = PoolClient::new(&env, &pool);
        let mint_amount = pool_client.keep_yt_split(&shares);
        let sy_desired = shares - mint_amount;
        let flash = pool_client.quote_sy_for_yt(&mint_amount);
        let pt_desired = flash.yt_out; // mint's PT/YT output is 1:1 (same notional)
        let (pt_res, sy_res) = pool_client.reserves();
        let total_lp = pool_client.lp_total_supply();
        let lp_estimate = if pt_res > 0 && sy_res > 0 {
            (pt_desired * total_lp / pt_res).min(sy_desired * total_lp / sy_res)
        } else {
            0
        };
        KeepYtQuote {
            mint_amount,
            pt_desired,
            sy_desired,
            lp_estimate,
            yt_out: flash.yt_out,
            mint_fee_underlying: flash.mint_fee_shares * rate / RATE_SCALE,
        }
    }

    pub fn remove_liquidity(
        env: Env,
        from: Address,
        pool: Address,
        lp_in: i128,
        min_pt_out: i128,
        min_sy_out: i128,
        deadline: u64,
    ) -> (i128, i128) {
        from.require_auth();
        Self::assert_deadline(&env, deadline);
        Self::info(&env, &pool);
        PoolClient::new(&env, &pool).remove_liquidity(
            &from,
            &from,
            &lp_in,
            &min_pt_out,
            &min_sy_out,
        )
    }

    /// LP -> PT + SY (`MarketPool.remove_liquidity`) -> sell the PT leg for SY
    /// (`MarketPool.swap_all_pt_for_sy`) -> unwrap everything to underlying
    /// (`SYWrapper.withdraw_all`), in one transaction and one signature -- found missing in an
    /// integration review (landing in underlying previously needed 2-3 separate calls: remove,
    /// sell the PT, unwrap).
    ///
    /// Both intermediate legs are swept with the balance-reading, no-amount-argument primitives
    /// (`swap_all_pt_for_sy`, `withdraw_all`) rather than chaining `remove_liquidity`'s PT/SY
    /// payout straight into further authorized calls as explicit arguments -- see the module
    /// docs. That means this call requires `from` to hold **no** PT and **no** SY beforehand
    /// (reverts `PreexistingPtBalance`/`PreexistingSyBalance` otherwise): both sweeps act on
    /// `from`'s entire balance of their respective token, not just what this call itself
    /// produces. The `remove_liquidity`/swap legs are unguarded; `min_underlying_out` protects
    /// the whole round trip at the final, underlying-denominated output.
    pub fn remove_liquidity_to_underlying(
        env: Env,
        from: Address,
        pool: Address,
        lp_in: i128,
        min_underlying_out: i128,
        deadline: u64,
    ) -> i128 {
        from.require_auth();
        Self::assert_deadline(&env, deadline);
        let info = Self::info(&env, &pool);
        let pt = PtClient::new(&env, &info.pt);
        let sy = SYClient::new(&env, &info.sy);
        if pt.balance(&from) != 0 {
            panic_with_error!(&env, Error::PreexistingPtBalance);
        }
        if sy.balance_of(&from) != 0 {
            panic_with_error!(&env, Error::PreexistingSyBalance);
        }
        let pool_client = PoolClient::new(&env, &pool);
        pool_client.remove_liquidity(&from, &from, &lp_in, &0, &0);
        if pt.balance(&from) > 0 {
            pool_client.swap_all_pt_for_sy(&from, &from, &0);
        }
        sy.withdraw_all(&from, &from, &min_underlying_out)
    }

    // -------------------------------------------------------------- redemption

    /// PT + YT -> SY before maturity.
    pub fn recombine(
        env: Env,
        from: Address,
        pool: Address,
        amount: i128,
        min_sy_out: i128,
        deadline: u64,
    ) -> i128 {
        from.require_auth();
        Self::assert_deadline(&env, deadline);
        let info = Self::info(&env, &pool);
        let shares = ManagerClient::new(&env, &info.manager).recombine(&from, &amount);
        if shares < min_sy_out {
            panic_with_error!(&env, Error::SlippageExceeded);
        }
        shares
    }

    /// PT and/or YT -> underlying after maturity.
    pub fn redeem_at_maturity(
        env: Env,
        from: Address,
        pool: Address,
        pt_amount: i128,
        yt_amount: i128,
    ) -> RedeemResult {
        from.require_auth();
        let info = Self::info(&env, &pool);
        ManagerClient::new(&env, &info.manager).redeem(&from, &pt_amount, &yt_amount)
    }

    /// Claim accrued YT yield as underlying without burning YT.
    pub fn claim_yield(env: Env, from: Address, pool: Address) -> i128 {
        from.require_auth();
        let info = Self::info(&env, &pool);
        ManagerClient::new(&env, &info.manager).claim_yield(&from)
    }
}

impl RouterContract {
    fn info(env: &Env, pool: &Address) -> MarketInfo {
        let key = DataKey::Market(pool.clone());
        let info: MarketInfo = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or_else(|| panic_with_error!(env, Error::MarketNotRegistered));
        // A listing that is being used stays alive.
        env.storage()
            .persistent()
            .extend_ttl(&key, MARKET_TTL_LEDGERS, MARKET_TTL_LEDGERS);
        info
    }

    fn assert_deadline(env: &Env, deadline: u64) {
        if env.ledger().timestamp() > deadline {
            panic_with_error!(env, Error::DeadlineExpired);
        }
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
}
