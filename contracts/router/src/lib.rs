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
//! | `swap_yt_for_sy` / `swap_yt_for_underlying` | **flash-redeem**: the pool recombines the YT with its own PT and pays out the difference, in SY or unwrapped to underlying |
//! | `add_liquidity` / `add_liquidity_single_sy` / `add_liquidity_keep_yt` / `remove_liquidity` | `MarketPool` LP operations; `add_liquidity_keep_yt` mints PT+YT from underlying, deposits the PT, and lets the caller keep the YT instead of selling it |
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
//! # Why `swap_pt_for_underlying` / `swap_yt_for_underlying` / `swap_underlying_for_yt` need a
//! zero SY balance, and `add_liquidity_keep_yt` takes `mint_amount` as an argument
//! `MarketPool`'s swap price depends on `seconds_to_maturity`, read live from the ledger, so the
//! exact amount a swap pays out can differ -- by a few raw units -- between a client's
//! pre-submission simulation and real execution moments later. Soroban's pre-signed
//! authorization for a *further* call must match that call's real arguments exactly, so chaining
//! a swap's payout straight into another call that needs fresh `from.require_auth()` on that
//! exact amount reverted `Error(Auth, InvalidAction)` live on Testnet while this Router was first
//! written (see `docs/DAPP_SMART_CONTRACT_INTEGRATION_GUIDE.md`) -- through no fault of the
//! trade itself. The fix used here is `SYWrapper.withdraw_all` (reads `from`'s balance at
//! execution instead of taking it as an argument, so there is nothing curve-derived left to
//! authorize -- but it withdraws *everything*, hence the zero-balance precondition) for the three
//! `_for_underlying`/`_for_yt` swap functions, and a caller-supplied `mint_amount` (quoted via
//! `MarketPool.zap_swap_amount` before the call is built, the same way every other min-out in
//! this Router is quoted) instead of deriving the split mid-execution, for `add_liquidity_keep_yt`.

#![no_std]
#![allow(deprecated)] // `env.events().publish` / `register_contract`: migration to `#[contractevent]` is tracked separately; event topics are kept stable for indexers.

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
    fn zap_swap_amount(env: Env, sy_in: i128) -> i128;
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
    fn balance_of(env: Env, account: Address) -> i128;
}

// ---------------------------------------------------------------------------
// Errors / storage
// ---------------------------------------------------------------------------

/// Instance-storage TTL applied by `bump` (~30 days at 5 s per ledger).
const INSTANCE_TTL_LEDGERS: u32 = 518_400;
/// TTL applied to a market listing each time it is used or registered.
const MARKET_TTL_LEDGERS: u32 = 518_400;

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
    /// The final unwrap uses `withdraw_all` and so requires `from` to hold no SY before this call
    /// (reverts `PreexistingSyBalance` otherwise) -- see `swap_pt_for_underlying`'s docs for why.
    /// Everything `deposit` credits to `from` is fully consumed by `mint` a few lines later, so by
    /// the time `withdraw_all` runs, `from`'s SY balance is exactly the PT sale's proceeds and
    /// nothing else, as long as that precondition held at the start.
    pub fn swap_underlying_for_yt(
        env: Env,
        from: Address,
        pool: Address,
        underlying_amount: i128,
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
        let sy_in = sy.deposit(&from, &underlying_amount, &0);
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

    /// Pendle's "keep YT" deposit: `underlying_amount` of underlying becomes LP *and* YT, with no
    /// swap and so no price-impact slippage (the only guard that applies is `min_lp_out`, exactly
    /// as `add_liquidity`'s own proportional deposit already works). Wraps to SY, mints PT+YT from
    /// `mint_amount` of it, then adds that PT and the remaining SY as a normal proportional
    /// deposit -- keeping the YT instead of selling it, which is the one difference from
    /// `add_liquidity_single_sy`.
    ///
    /// `mint_amount` is a plain argument, computed off-chain (quote `MarketPool.zap_swap_amount`
    /// for the split that balances the pool's current ratio before building this call, the same
    /// way every other min-out in this Router is quoted beforehand) rather than something this
    /// function derives mid-execution. That distinction matters: `zap_swap_amount`, like every
    /// other curve read, depends on `seconds_to_maturity` taken live from the ledger, so a value
    /// this function computed itself and then passed as `mint`'s `sy_shares` argument could
    /// differ -- by a few raw units -- between a client's pre-submission simulation and real
    /// execution moments later, reverting `Error(Auth, InvalidAction)` on `mint`'s own
    /// authorization for no reason related to the trade itself (the exact failure
    /// `swap_pt_for_underlying` hit this same way; see its docs and
    /// `docs/DAPP_SMART_CONTRACT_INTEGRATION_GUIDE.md`). A quote taken moments earlier and signed
    /// as a literal argument has no such drift to go stale.
    ///
    /// A `mint_amount` that doesn't match the pool's current ratio (stale quote, or just a
    /// round number the caller chose) may leave a small PT or SY remainder unused: that's fine,
    /// `add_liquidity`'s proportional deposit only ever pulls the `pt_used`/`sy_used` it actually
    /// needs (see its implementation) and leaves the rest in `from`'s own balance untouched --
    /// there is nothing to refund and nothing gets stranded.
    pub fn add_liquidity_keep_yt(
        env: Env,
        from: Address,
        pool: Address,
        underlying_amount: i128,
        mint_amount: i128,
        min_lp_out: i128,
        deadline: u64,
    ) -> KeepYtResult {
        from.require_auth();
        Self::assert_deadline(&env, deadline);
        let info = Self::info(&env, &pool);
        let pool_client = PoolClient::new(&env, &pool);
        let shares = SYClient::new(&env, &info.sy).deposit(&from, &underlying_amount, &0);
        let minted = ManagerClient::new(&env, &info.manager).mint(&from, &mint_amount);
        let sy_remaining = shares - mint_amount;
        let (pt_used, sy_used, lp_minted) =
            pool_client.add_liquidity(&from, &minted.pt_minted, &sy_remaining, &min_lp_out);
        KeepYtResult {
            pt_used,
            sy_used,
            lp_minted,
            yt_minted: minted.yt_minted,
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
