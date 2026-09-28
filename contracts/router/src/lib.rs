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
//! | `swap_sy_for_pt`          | SY -> PT (`MarketPool`)                                           |
//! | `swap_pt_for_sy`          | PT -> SY (`MarketPool`)                                           |
//! | `swap_sy_for_yt`          | **flash-mint**: mint PT + YT from the SY, sell the PT back to the pool, keep the YT and the SY proceeds |
//! | `swap_yt_for_sy`          | **flash-redeem**: the pool recombines the YT with its own PT and pays out the difference |
//! | `add_liquidity` / `add_liquidity_single_sy` / `remove_liquidity` | `MarketPool` LP operations |
//! | `recombine`               | PT + YT -> SY before maturity                                     |
//! | `redeem_at_maturity`      | PT and/or YT -> underlying after maturity                         |
//! | `claim_yield`             | accrued YT yield -> underlying, without burning YT                |

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
}

// ---------------------------------------------------------------------------
// Errors / storage
// ---------------------------------------------------------------------------

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

    /// **Flash-mint** buy of YT. Tokenizes `sy_in` SY into PT + YT, sells all of the PT back into
    /// the pool in the same transaction, and leaves the user with the YT plus the SY the PT
    /// fetched. Net cost of the YT is `sy_in - sy_back` SY. Returns `(yt_out, sy_back)`; reverts
    /// `SlippageExceeded` if `yt_out < min_yt_out`. No loan is involved: it is two synchronous
    /// calls in one atomic transaction, both made as the user.
    pub fn swap_sy_for_yt(
        env: Env,
        from: Address,
        pool: Address,
        sy_in: i128,
        min_yt_out: i128,
        deadline: u64,
    ) -> (i128, i128) {
        from.require_auth();
        Self::assert_deadline(&env, deadline);
        let info = Self::info(&env, &pool);
        let minted = ManagerClient::new(&env, &info.manager).mint(&from, &sy_in);
        if minted.yt_minted < min_yt_out {
            panic_with_error!(&env, Error::SlippageExceeded);
        }
        let sy_back =
            PoolClient::new(&env, &pool).swap_pt_for_sy(&from, &from, &minted.pt_minted, &0);
        (minted.yt_minted, sy_back)
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
        env.storage()
            .persistent()
            .get(&DataKey::Market(pool.clone()))
            .unwrap_or_else(|| panic_with_error!(env, Error::MarketNotRegistered))
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
