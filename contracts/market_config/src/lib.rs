//! MarketConfig — the per-market configuration and fee-split rules.
//!
//! One `MarketConfig` exists per market (one underlying asset and one maturity). It is the single
//! place that says *how much* a market charges and *who gets it*:
//!
//! | Parameter               | Set by                                   | Meaning                                              |
//! |-------------------------|------------------------------------------|------------------------------------------------------|
//! | `tokenization_fee_bps`  | underlying's issuer authority            | fee on SY tokenized into PT/YT (e.g. 5 bps)          |
//! | `yt_fee_bps`            | underlying's issuer authority            | share of the yield a YT holder claims (e.g. 1000 = 10%) |
//! | `swap_fee_tier_bps`     | underlying's issuer authority            | swap Fee Tier (e.g. 10 bps = 0.1%)                   |
//! | `protocol_share_bps`    | Principal (`protocol_admin`)             | Principal's cut of every fee (initially 2000 = 20%)  |
//! | `treasury`              | Principal (`protocol_admin`)             | where Principal's cut is paid                        |
//!
//! The remainder of every fee (initially 80%) belongs to the **market creator**, defined as the
//! underlying asset's *current* issuer authority: for a Stellar Asset that is `SAC.admin()`, read
//! live at payout time, so a SAC admin rotation redirects the creator share with nothing to
//! update here. (A SEP-57 RWA token has no `admin()`; its creator share is paid to an explicit
//! `creator_payee` that the operator sets.)
//!
//! # Who may configure fees
//! Fees are configured by whoever *currently* controls the underlying — never by a key stored in
//! this contract. `initialize` and `set_fees` both run the compliance adapter's authority check
//! against the live issuer, exactly like market creation on `SYWrapper`/`PrincipalManager`. The
//! protocol share is deliberately *not* settable by the creator, or a creator could zero out
//! Principal's cut; it belongs to `protocol_admin`.
//!
//! # Swap fee schedule
//! `Trading Fee = Fee Tier x Days to Maturity / 365`, with days measured to the second
//! (`seconds_to_maturity / 86_400`). It is highest at issuance and falls linearly to zero at
//! maturity; `swap_fee_rate` exposes it at `FEE_SCALE = 1e12` and caps it at `MAX_SWAP_FEE_RATE`.

#![no_std]
#![allow(deprecated)]
// `env.events().publish` / `register_contract`: migration to `#[contractevent]` is tracked separately; event topics are kept stable for indexers.
#![allow(clippy::too_many_arguments)] // Soroban `initialize` entrypoints wire many contracts by design.

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, panic_with_error, symbol_short, Address,
    Env,
};

/// Fixed-point denominator shared with every other Principal contract.
pub const SCALE: i128 = 10_000_000;
pub const BPS: i128 = 10_000;
/// Fixed-point denominator of the *swap fee rate* (1e12). Finer than `SCALE` so the time-decayed
/// fee stays accurate in the last days before maturity, when it is a few parts per million.
pub const FEE_SCALE: i128 = 1_000_000_000_000;
pub const SECONDS_PER_DAY: i128 = 86_400;

/// Hard caps, so a compromised or careless issuer key cannot configure a confiscatory market.
pub const MAX_TOKENIZATION_FEE_BPS: u32 = 100; // 1%
pub const MAX_YT_FEE_BPS: u32 = 5_000; // 50% of claimed yield
pub const MAX_SWAP_FEE_TIER_BPS: u32 = 500; // 5%
/// Cap on the *effective* swap fee after the time factor (10%, at `FEE_SCALE`).
pub const MAX_SWAP_FEE_RATE: i128 = FEE_SCALE / 10;

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
    FeeTooHigh = 5,
    InvalidShare = 6,
    NotApplicable = 7,
}

#[contracttype]
pub enum DataKey {
    Underlying,
    Maturity,
    ProtocolAdmin,
    Treasury,
    TokenizationFeeBps,
    YtFeeBps,
    SwapFeeTierBps,
    ProtocolShareBps,
    /// Explicit creator payee; only used when the underlying has no `admin()` (SEP-57).
    CreatorPayee,
}

#[contract]
pub struct MarketConfigContract;

#[contractimpl]
impl MarketConfigContract {
    /// Permissionless keeper call: extend this contract's *instance* storage (admin, config,
    /// reserves, totals, settlement rate, fee buckets) for ~30 days. Soroban does not bump instance
    /// TTL on ordinary reads or writes, so a long-dated market needs this called periodically (or
    /// an archived instance restored) -- see docs/DEPLOYMENT.md.
    pub fn bump(env: Env) {
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_TTL_LEDGERS, INSTANCE_TTL_LEDGERS);
    }

    /// Create the market configuration. `admin` must be the underlying's issuer authority (for a
    /// SAC: `admin()`, read live) and must authorize this call.
    #[allow(clippy::too_many_arguments)]
    pub fn initialize(
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
    ) {
        if env.storage().instance().has(&DataKey::Underlying) {
            panic_with_error!(&env, Error::AlreadyInitialized);
        }
        admin.require_auth();
        principal_compliance::init(&env, &underlying);
        if !principal_compliance::is_authority(&env, &underlying, &admin) {
            panic_with_error!(&env, Error::IssuerMismatch);
        }
        Self::assert_fee_caps(&env, tokenization_fee_bps, yt_fee_bps, swap_fee_tier_bps);
        Self::assert_share(&env, protocol_share_bps);

        let s = env.storage().instance();
        s.set(&DataKey::Underlying, &underlying);
        s.set(&DataKey::Maturity, &maturity);
        s.set(&DataKey::ProtocolAdmin, &protocol_admin);
        s.set(&DataKey::Treasury, &treasury);
        s.set(&DataKey::TokenizationFeeBps, &tokenization_fee_bps);
        s.set(&DataKey::YtFeeBps, &yt_fee_bps);
        s.set(&DataKey::SwapFeeTierBps, &swap_fee_tier_bps);
        s.set(&DataKey::ProtocolShareBps, &protocol_share_bps);
        if principal_compliance::authority(&env, &underlying).is_none() {
            s.set(&DataKey::CreatorPayee, &admin);
        }
        env.events().publish(
            (symbol_short!("mkt_new"),),
            (
                underlying,
                maturity,
                tokenization_fee_bps,
                yt_fee_bps,
                swap_fee_tier_bps,
                protocol_share_bps,
            ),
        );
    }

    // --- market creator (issuer authority) ---

    /// Reconfigure the three market fees. Only the underlying's *current* issuer authority.
    pub fn set_fees(
        env: Env,
        caller: Address,
        tokenization_fee_bps: u32,
        yt_fee_bps: u32,
        swap_fee_tier_bps: u32,
    ) {
        Self::assert_issuer(&env, &caller);
        Self::assert_fee_caps(&env, tokenization_fee_bps, yt_fee_bps, swap_fee_tier_bps);
        let s = env.storage().instance();
        s.set(&DataKey::TokenizationFeeBps, &tokenization_fee_bps);
        s.set(&DataKey::YtFeeBps, &yt_fee_bps);
        s.set(&DataKey::SwapFeeTierBps, &swap_fee_tier_bps);
        env.events().publish(
            (symbol_short!("fees_set"),),
            (caller, tokenization_fee_bps, yt_fee_bps, swap_fee_tier_bps),
        );
    }

    /// Set the explicit creator payee. Only meaningful (and only allowed) when the underlying has
    /// no `admin()` to read live (SEP-57 RWA tokens).
    pub fn set_creator_payee(env: Env, caller: Address, payee: Address) {
        Self::assert_issuer(&env, &caller);
        if !env.storage().instance().has(&DataKey::CreatorPayee) {
            panic_with_error!(&env, Error::NotApplicable);
        }
        env.storage().instance().set(&DataKey::CreatorPayee, &payee);
        env.events()
            .publish((symbol_short!("payee_set"),), (caller, payee));
    }

    // --- protocol (Principal) ---

    pub fn set_protocol_share(env: Env, caller: Address, protocol_share_bps: u32) {
        Self::assert_protocol_admin(&env, &caller);
        Self::assert_share(&env, protocol_share_bps);
        env.storage()
            .instance()
            .set(&DataKey::ProtocolShareBps, &protocol_share_bps);
        env.events()
            .publish((symbol_short!("share_set"),), (caller, protocol_share_bps));
    }

    pub fn set_treasury(env: Env, caller: Address, treasury: Address) {
        Self::assert_protocol_admin(&env, &caller);
        env.storage().instance().set(&DataKey::Treasury, &treasury);
        env.events()
            .publish((symbol_short!("treas_set"),), (caller, treasury));
    }

    pub fn transfer_protocol_admin(env: Env, caller: Address, new_admin: Address) {
        Self::assert_protocol_admin(&env, &caller);
        env.storage()
            .instance()
            .set(&DataKey::ProtocolAdmin, &new_admin);
        env.events()
            .publish((symbol_short!("padm_xfer"),), (caller, new_admin));
    }

    // --- views ---

    pub fn underlying(env: Env) -> Address {
        Self::get(&env, &DataKey::Underlying)
    }

    pub fn maturity(env: Env) -> u64 {
        Self::get(&env, &DataKey::Maturity)
    }

    pub fn tokenization_fee_bps(env: Env) -> u32 {
        Self::get(&env, &DataKey::TokenizationFeeBps)
    }

    pub fn yt_fee_bps(env: Env) -> u32 {
        Self::get(&env, &DataKey::YtFeeBps)
    }

    pub fn swap_fee_tier_bps(env: Env) -> u32 {
        Self::get(&env, &DataKey::SwapFeeTierBps)
    }

    pub fn protocol_share_bps(env: Env) -> u32 {
        Self::get(&env, &DataKey::ProtocolShareBps)
    }

    pub fn treasury(env: Env) -> Address {
        Self::get(&env, &DataKey::Treasury)
    }

    pub fn protocol_admin(env: Env) -> Address {
        Self::get(&env, &DataKey::ProtocolAdmin)
    }

    /// The market creator's payout address: the underlying's *current* `admin()` (live) for a SAC,
    /// or the configured payee for a SEP-57 token.
    pub fn creator(env: Env) -> Address {
        let underlying: Address = Self::get(&env, &DataKey::Underlying);
        match principal_compliance::authority(&env, &underlying) {
            Some(admin) => admin,
            None => Self::get(&env, &DataKey::CreatorPayee),
        }
    }

    /// Split `fee` into (protocol share, creator share). The protocol share is floored, so any
    /// rounding dust goes to the creator and the two always sum to `fee`.
    pub fn split(env: Env, fee: i128) -> (i128, i128) {
        let share: u32 = Self::get(&env, &DataKey::ProtocolShareBps);
        let protocol = fee * (share as i128) / BPS;
        (protocol, fee - protocol)
    }

    /// Effective swap fee at `FEE_SCALE` (1e12) for a trade `seconds_to_maturity` before maturity:
    /// `Fee Tier x (seconds_to_maturity / 86_400) / 365`, capped at `MAX_SWAP_FEE_RATE`.
    pub fn swap_fee_rate(env: Env, seconds_to_maturity: u64) -> i128 {
        let tier: u32 = Self::get(&env, &DataKey::SwapFeeTierBps);
        let rate = (tier as i128) * FEE_SCALE * (seconds_to_maturity as i128)
            / (BPS * SECONDS_PER_DAY * 365);
        if rate > MAX_SWAP_FEE_RATE {
            MAX_SWAP_FEE_RATE
        } else {
            rate
        }
    }

    // --- internal helpers ---

    fn get<T: soroban_sdk::TryFromVal<Env, soroban_sdk::Val>>(env: &Env, key: &DataKey) -> T {
        env.storage()
            .instance()
            .get(key)
            .unwrap_or_else(|| panic_with_error!(env, Error::NotInitialized))
    }

    fn assert_fee_caps(env: &Env, tokenization: u32, yt: u32, swap_tier: u32) {
        if tokenization > MAX_TOKENIZATION_FEE_BPS
            || yt > MAX_YT_FEE_BPS
            || swap_tier > MAX_SWAP_FEE_TIER_BPS
        {
            panic_with_error!(env, Error::FeeTooHigh);
        }
    }

    fn assert_share(env: &Env, share: u32) {
        if share as i128 > BPS {
            panic_with_error!(env, Error::InvalidShare);
        }
    }

    fn assert_issuer(env: &Env, caller: &Address) {
        caller.require_auth();
        let underlying: Address = Self::get(env, &DataKey::Underlying);
        if !principal_compliance::is_authority(env, &underlying, caller) {
            panic_with_error!(env, Error::Unauthorized);
        }
    }

    fn assert_protocol_admin(env: &Env, caller: &Address) {
        caller.require_auth();
        let admin: Address = Self::get(env, &DataKey::ProtocolAdmin);
        if *caller != admin {
            panic_with_error!(env, Error::Unauthorized);
        }
    }
}

#[cfg(test)]
mod test;
