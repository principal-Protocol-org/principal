//! SYWrapper — standardized yield wrapper for a single underlying yield-bearing asset.
//!
//! # Design
//! Users deposit the underlying asset (e.g. BENJI, USDY) and receive SY shares in return.
//! The exchange rate (underlying per share) increases over time as the underlying accrues yield.
//! The PrincipalManager reads the exchange rate to compute PT and YT amounts when splitting.
//!
//! # Exchange-rate invariant
//!   exchange_rate = total_underlying / total_shares   (scaled by RATE_SCALE = 1e7)
//!
//! On deposit of `u` underlying units:
//!   shares_minted = u * RATE_SCALE / exchange_rate
//!
//! On withdrawal of `s` shares:
//!   underlying_returned = s * exchange_rate / RATE_SCALE
//!
//! # Compliance — authorization inheritance
//! Most Stellar RWAs are issued as Stellar Assets with native authorization and clawback
//! controls exposed through their Stellar Asset Contract (SAC): `authorized(address) -> bool`
//! and `admin() -> Address`, both public, no-auth-required view functions. Those controls apply
//! to the underlying asset but do not automatically extend to SY shares — a separate Soroban
//! position. Without inheritance, an investor deauthorized on the underlying asset could still
//! hold or transfer SY.
//!
//! `deposit` and `withdraw` therefore check **both** layers on every affected account:
//! `underlying_SAC.authorized(account)` -- routed through `principal_compliance`, which reads the
//! SAC for classic/SEP-8 assets and the frozen flag + identity verifier for a SEP-57 RWA token
//! (the mandatory floor — inherited live from the actual
//! issuer, with no separate registry that could drift out of sync with the issuer's own
//! decisions) and `Permissioning.is_allowed(account)` (an optional, Principal-specific
//! additional layer, narrower than but never looser than the SAC's own authorization).
//!
//! # Slippage protection, deposit cap, circuit breaker
//! * `deposit(from, amount, min_shares_out)` and `withdraw(from, shares, to, min_underlying_out)`
//!   revert with `SlippageExceeded` if the output would be below the caller's minimum, so a rate
//!   move between simulation and inclusion cannot silently cost the user. Pass `0` to opt out.
//! * A **per-address deposit cap** (`set_deposit_cap`) bounds the underlying any one address may
//!   have deposited, net of what it has withdrawn (`net_deposited`). `0` disables it.
//! * Once `set_risk_control` is called, every `deposit` invokes `RiskControl.check_deposit` for the
//!   deposited amount *inside the same transaction*, so a deposit that would breach the
//!   protocol-wide or per-asset circuit-breaker limit (or hit the global pause) reverts by itself.
//!
//! # Market creation
//! `initialize` requires the caller to be the underlying SAC's actual admin (`admin()`, read
//! live), so only the entity that controls a regulated asset's authorization and clawback can
//! stand up a market on it. This is checked once, at market creation; day-to-day operational
//! admin can be transferred afterward via `transfer_admin`.
//!
//! # Compliance recovery — seize
//! The native clawback function of a Stellar Asset only applies to the underlying asset's own
//! balance; it cannot reach SY shares directly, since they are a separate Soroban position.
//! `seize` lets a pre-configured `RecoveryEscrow` contract forcibly move a restricted holder's
//! SY balance to itself, without the holder's authorization -- a forced transfer, not a burn.
//! The escrow then unwraps that SY into the underlying asset via a normal `withdraw` call (the
//! escrow is itself pre-authorized to hold the underlying, same as `SYWrapper`), leaving the
//! escrow holding raw underlying, ready for the issuer's native SAC `clawback`. `SYWrapper`
//! itself does not verify *why* a seizure is happening or authenticate the real issuer directly
//! -- it only trusts calls from its one configured `RecoveryEscrow` address. All of that
//! verification (issuer admin signature, target already deauthorized) lives once in the escrow,
//! shared across SY/PT/YT, instead of being duplicated per contract.

#![no_std]
#![allow(deprecated)] // `env.events().publish` / `register_contract`: migration to `#[contractevent]` is tracked separately; event topics are kept stable for indexers.

use soroban_sdk::{
    contract, contractclient, contracterror, contractimpl, contracttype, panic_with_error,
    symbol_short, token, Address, Env,
};

use principal_compliance as compliance;

pub const RATE_SCALE: i128 = 10_000_000; // 1e7

/// TTL extension applied to every persistent per-user balance entry (~30 days at 5 s/ledger).
const BALANCE_TTL_LEDGERS: u32 = 518_400;

#[contractclient(name = "PermClient")]
pub trait PermissioningInterface {
    fn is_allowed(env: Env, account: Address) -> bool;
}

/// Minimum interface required from RiskControl.
#[contractclient(name = "RiskControlClient")]
pub trait RiskControlInterface {
    fn check_deposit(env: Env, caller: Address, asset: Address, amount: i128);
}

#[contracterror]
#[derive(Copy, Clone, Debug, PartialEq)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    Unauthorized = 2,
    NotInitialized = 3,
    ZeroAmount = 4,
    InsufficientShares = 5,
    Paused = 6,
    ArithmeticOverflow = 7,
    PermissionDenied = 8,
    NotAuthorizedOnSac = 9,
    RecoveryEscrowAlreadySet = 10,
    NotRecoveryEscrow = 11,
    IssuerMismatch = 12,
    SlippageExceeded = 13,
    DepositCapExceeded = 14,
    InvalidCap = 15,
}

#[contracttype]
pub enum DataKey {
    Admin,
    Underlying, // Address of the underlying SAC/token contract
    Permissioning,
    RecoveryEscrow, // absent until set_recovery_escrow
    TotalUnderlying,
    TotalShares,
    Balance(Address), // SY share balance per holder
    Paused,
    RiskControl,           // absent until set_risk_control
    DepositCap,            // per-address cap on net deposited underlying (0 / absent = none)
    NetDeposited(Address), // underlying deposited by an address, net of what it withdrew
}

#[contract]
pub struct SYWrapperContract;

#[contractimpl]
impl SYWrapperContract {
    /// Initialize with the admin address, the underlying SAC address, and the Permissioning
    /// registry used as an additional eligibility layer. `admin` must be the underlying SAC's
    /// actual admin (`admin()`, read live) and must authorize this call -- this is what ties
    /// market creation to the entity that actually controls the regulated asset, rather than
    /// letting any third party stand up a market for someone else's asset.
    pub fn initialize(env: Env, admin: Address, underlying: Address, permissioning: Address) {
        if env.storage().instance().has(&DataKey::Admin) {
            panic_with_error!(&env, Error::AlreadyInitialized);
        }
        admin.require_auth();
        compliance::init(&env, &underlying);
        if !compliance::is_authority(&env, &underlying, &admin) {
            panic_with_error!(&env, Error::IssuerMismatch);
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage()
            .instance()
            .set(&DataKey::Underlying, &underlying);
        env.storage()
            .instance()
            .set(&DataKey::Permissioning, &permissioning);
        env.storage()
            .instance()
            .set(&DataKey::TotalUnderlying, &0_i128);
        env.storage().instance().set(&DataKey::TotalShares, &0_i128);
        env.storage().instance().set(&DataKey::Paused, &false);
    }

    // --- share transfer ---

    /// Move `amount` shares from `from` to `to`, both still subject to the same two-layer
    /// compliance check as `deposit`/`withdraw`. This is what lets `PrincipalManager` take
    /// custody of a user's SY shares when splitting them into PT + YT, and is otherwise a plain
    /// SEP-41-style balance move: no change to `TotalUnderlying`/`TotalShares`, no external
    /// token call, so there is no reentrancy surface here the way there is in deposit/withdraw.
    pub fn transfer(env: Env, from: Address, to: Address, amount: i128) -> i128 {
        from.require_auth();
        Self::assert_not_paused_unless_escrow(&env, &[&from, &to]);
        Self::assert_sac_authorized(&env, &from);
        Self::assert_sac_authorized(&env, &to);
        Self::assert_permitted(&env, &from);
        Self::assert_permitted(&env, &to);
        if amount <= 0 {
            panic_with_error!(&env, Error::ZeroAmount);
        }

        let balance = Self::get_balance(&env, &from);
        if balance < amount {
            panic_with_error!(&env, Error::InsufficientShares);
        }

        Self::sub_balance(&env, &from, amount);
        Self::add_balance(&env, &to, amount);

        env.events()
            .publish((symbol_short!("sy_xfer"),), (from, to, amount));
        amount
    }

    // --- deposit / withdraw ---

    /// Deposit `amount` of the underlying asset; returns shares minted to `from`. Reverts
    /// `SlippageExceeded` if fewer than `min_shares_out` shares would be minted,
    /// `DepositCapExceeded` if `from`'s net deposits would pass the per-address cap, and -- when
    /// a RiskControl is wired -- `CircuitBreakerTripped`/`Paused` from RiskControl itself.
    pub fn deposit(env: Env, from: Address, amount: i128, min_shares_out: i128) -> i128 {
        from.require_auth();
        Self::assert_not_paused(&env);
        Self::assert_sac_authorized(&env, &from);
        Self::assert_permitted(&env, &from);
        if amount <= 0 {
            panic_with_error!(&env, Error::ZeroAmount);
        }

        // Compute shares to mint at the exchange rate observed before this deposit's own
        // effects are applied.
        let shares = Self::underlying_to_shares(&env, amount);
        if shares <= 0 {
            panic_with_error!(&env, Error::ZeroAmount);
        }
        if shares < min_shares_out {
            panic_with_error!(&env, Error::SlippageExceeded);
        }
        Self::charge_deposit_cap(&env, &from, amount);

        // Circuit breaker: registered as a consumer of RiskControl, this contract reports every
        // deposit to it; a breach reverts this whole call. The nested call is authorized by this
        // contract's own address (a contract auto-authorizes calls it makes directly).
        if let Some(rc) = env
            .storage()
            .instance()
            .get::<_, Address>(&DataKey::RiskControl)
        {
            RiskControlClient::new(&env, &rc).check_deposit(
                &env.current_contract_address(),
                &Self::get_underlying(&env),
                &amount,
            );
        }

        // Effects before interaction (checks-effects-interactions): update state first so a
        // reentrant call from a malicious `underlying` token would see this deposit already
        // accounted for.
        let total_u: i128 = env
            .storage()
            .instance()
            .get(&DataKey::TotalUnderlying)
            .unwrap_or(0);
        let total_s: i128 = env
            .storage()
            .instance()
            .get(&DataKey::TotalShares)
            .unwrap_or(0);
        env.storage()
            .instance()
            .set(&DataKey::TotalUnderlying, &(total_u + amount));
        env.storage()
            .instance()
            .set(&DataKey::TotalShares, &(total_s + shares));
        Self::add_balance(&env, &from, shares);

        // Interaction last: transfer underlying from depositor to this contract. If this
        // fails, the whole transaction (including the state updates above) reverts atomically.
        let underlying = Self::get_underlying(&env);
        let this = env.current_contract_address();
        token::Client::new(&env, &underlying).transfer(&from, &this, &amount);

        env.events()
            .publish((symbol_short!("deposit"),), (from, amount, shares));
        shares
    }

    /// Burn `shares` and return the equivalent underlying amount to `to`. Reverts
    /// `SlippageExceeded` if less than `min_underlying_out` would be returned.
    pub fn withdraw(
        env: Env,
        from: Address,
        shares: i128,
        to: Address,
        min_underlying_out: i128,
    ) -> i128 {
        from.require_auth();
        Self::assert_not_paused_unless_escrow(&env, &[&from]);
        // Both sides are checked, not just `to`: if only the recipient were gated, a
        // deauthorized account could self-withdraw the instant it suspected a seizure was
        // coming. Frozen means frozen on both the sending and receiving side.
        Self::assert_sac_authorized(&env, &from);
        Self::assert_sac_authorized(&env, &to);
        Self::assert_permitted(&env, &from);
        Self::assert_permitted(&env, &to);
        if shares <= 0 {
            panic_with_error!(&env, Error::ZeroAmount);
        }

        let balance = Self::get_balance(&env, &from);
        if balance < shares {
            panic_with_error!(&env, Error::InsufficientShares);
        }

        let underlying_out = Self::shares_to_underlying(&env, shares);
        if underlying_out <= 0 {
            panic_with_error!(&env, Error::ZeroAmount);
        }
        if underlying_out < min_underlying_out {
            panic_with_error!(&env, Error::SlippageExceeded);
        }

        // Update state before external call (checks-effects-interactions).
        let total_u: i128 = env
            .storage()
            .instance()
            .get(&DataKey::TotalUnderlying)
            .unwrap_or(0);
        let total_s: i128 = env
            .storage()
            .instance()
            .get(&DataKey::TotalShares)
            .unwrap_or(0);
        env.storage()
            .instance()
            .set(&DataKey::TotalUnderlying, &(total_u - underlying_out));
        env.storage()
            .instance()
            .set(&DataKey::TotalShares, &(total_s - shares));
        Self::sub_balance(&env, &from, shares);
        Self::release_deposit_cap(&env, &from, underlying_out);

        // Transfer underlying to recipient.
        let underlying = Self::get_underlying(&env);
        token::Client::new(&env, &underlying).transfer(
            &env.current_contract_address(),
            &to,
            &underlying_out,
        );

        env.events()
            .publish((symbol_short!("withdraw"),), (from, shares, underlying_out));
        underlying_out
    }

    // --- views ---

    /// Current exchange rate: underlying units per share, scaled by RATE_SCALE.
    pub fn exchange_rate(env: Env) -> i128 {
        let total_s: i128 = env
            .storage()
            .instance()
            .get(&DataKey::TotalShares)
            .unwrap_or(0);
        if total_s == 0 {
            return RATE_SCALE; // 1:1 at inception
        }
        let total_u: i128 = env
            .storage()
            .instance()
            .get(&DataKey::TotalUnderlying)
            .unwrap_or(0);
        total_u * RATE_SCALE / total_s
    }

    pub fn total_underlying(env: Env) -> i128 {
        env.storage()
            .instance()
            .get(&DataKey::TotalUnderlying)
            .unwrap_or(0)
    }

    pub fn total_shares(env: Env) -> i128 {
        env.storage()
            .instance()
            .get(&DataKey::TotalShares)
            .unwrap_or(0)
    }

    pub fn balance_of(env: Env, account: Address) -> i128 {
        Self::get_balance(&env, &account)
    }

    pub fn underlying_address(env: Env) -> Address {
        Self::get_underlying(&env)
    }

    pub fn recovery_escrow(env: Env) -> Address {
        Self::require_escrow(&env)
    }

    pub fn permissioning_address(env: Env) -> Address {
        env.storage()
            .instance()
            .get(&DataKey::Permissioning)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized))
    }

    /// The per-address deposit cap in underlying units (`0` = disabled).
    pub fn deposit_cap(env: Env) -> i128 {
        env.storage()
            .instance()
            .get(&DataKey::DepositCap)
            .unwrap_or(0)
    }

    /// Underlying `account` has deposited, net of what it withdrew (floored at 0). This is the
    /// quantity the per-address cap applies to.
    pub fn net_deposited(env: Env, account: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::NetDeposited(account))
            .unwrap_or(0)
    }

    pub fn is_paused(env: Env) -> bool {
        env.storage()
            .instance()
            .get(&DataKey::Paused)
            .unwrap_or(false)
    }

    // --- admin ---

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

    /// Set the per-address deposit cap (underlying units); `0` disables it. Existing balances
    /// above a newly lowered cap are not clawed back -- the cap only gates further deposits.
    pub fn set_deposit_cap(env: Env, admin: Address, cap: i128) {
        Self::assert_admin(&env, &admin);
        if cap < 0 {
            panic_with_error!(&env, Error::InvalidCap);
        }
        env.storage().instance().set(&DataKey::DepositCap, &cap);
        env.events().publish((symbol_short!("cap_set"),), cap);
    }

    /// Wire the RiskControl this wrapper reports deposits to. The wrapper must also be registered
    /// as a consumer on that RiskControl (`RiskControl.add_consumer`) or every deposit reverts.
    pub fn set_risk_control(env: Env, admin: Address, risk_control: Address) {
        Self::assert_admin(&env, &admin);
        env.storage()
            .instance()
            .set(&DataKey::RiskControl, &risk_control);
        env.events()
            .publish((symbol_short!("rc_set"),), risk_control);
    }

    pub fn risk_control(env: Env) -> Option<Address> {
        env.storage().instance().get(&DataKey::RiskControl)
    }

    pub fn get_admin(env: Env) -> Address {
        Self::require_admin(&env)
    }

    /// One-time wiring of the RecoveryEscrow contract authorized to call `seize`. Mirrors the
    /// `set_minter` pattern used on PTToken/YTToken: settable once, never reassigned, breaking
    /// the circular dependency between SYWrapper and RecoveryEscrow at deployment time.
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

    // --- compliance recovery (seize) ---

    /// Forcibly move `shares` from `account`'s SY balance to the caller's own balance, without
    /// `account`'s authorization. Callable only by the configured `RecoveryEscrow` -- this
    /// contract does not itself verify the issuer's admin signature or check whether `account`
    /// has actually been deauthorized; that verification happens once, in the escrow, shared
    /// across every position type instead of duplicated here. A pure forced transfer, not a
    /// burn: `TotalShares`/`TotalUnderlying` are unaffected, since the value stays inside the
    /// wrapper (now credited to the escrow) until the escrow separately calls `withdraw` to
    /// unwrap it into raw underlying.
    ///
    /// Deliberately callable while paused: `set_paused` blocks ordinary user activity, but a
    /// seizure already gated on caller-is-the-escrow shouldn't become unusable during an
    /// operational pause (e.g. triggered by the same incident that justifies a legal freeze).
    pub fn seize(env: Env, caller: Address, account: Address, shares: i128) -> i128 {
        caller.require_auth();
        let escrow = Self::require_escrow(&env);
        if caller != escrow {
            panic_with_error!(&env, Error::NotRecoveryEscrow);
        }
        if shares <= 0 {
            panic_with_error!(&env, Error::ZeroAmount);
        }

        let balance = Self::get_balance(&env, &account);
        if balance < shares {
            panic_with_error!(&env, Error::InsufficientShares);
        }

        Self::sub_balance(&env, &account, shares);
        Self::add_balance(&env, &caller, shares);

        env.events()
            .publish((symbol_short!("seize"),), (caller, account, shares));
        shares
    }

    // --- internal helpers ---

    fn underlying_to_shares(env: &Env, amount: i128) -> i128 {
        let total_s: i128 = env
            .storage()
            .instance()
            .get(&DataKey::TotalShares)
            .unwrap_or(0);
        if total_s == 0 {
            return amount; // first depositor: 1:1
        }
        let rate = SYWrapperContract::exchange_rate(env.clone());
        amount * RATE_SCALE / rate
    }

    fn shares_to_underlying(env: &Env, shares: i128) -> i128 {
        let rate = SYWrapperContract::exchange_rate(env.clone());
        shares * rate / RATE_SCALE
    }

    fn get_balance(env: &Env, account: &Address) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::Balance(account.clone()))
            .unwrap_or(0)
    }

    fn add_balance(env: &Env, account: &Address, delta: i128) {
        let key = DataKey::Balance(account.clone());
        let bal: i128 = env.storage().persistent().get(&key).unwrap_or(0);
        env.storage().persistent().set(&key, &(bal + delta));
        env.storage()
            .persistent()
            .extend_ttl(&key, BALANCE_TTL_LEDGERS, BALANCE_TTL_LEDGERS);
    }

    fn sub_balance(env: &Env, account: &Address, delta: i128) {
        let key = DataKey::Balance(account.clone());
        let bal: i128 = env.storage().persistent().get(&key).unwrap_or(0);
        env.storage().persistent().set(&key, &(bal - delta));
        env.storage()
            .persistent()
            .extend_ttl(&key, BALANCE_TTL_LEDGERS, BALANCE_TTL_LEDGERS);
    }

    fn get_underlying(env: &Env) -> Address {
        env.storage()
            .instance()
            .get(&DataKey::Underlying)
            .unwrap_or_else(|| panic_with_error!(env, Error::NotInitialized))
    }

    fn require_admin(env: &Env) -> Address {
        env.storage()
            .instance()
            .get(&DataKey::Admin)
            .unwrap_or_else(|| panic_with_error!(env, Error::NotInitialized))
    }

    fn require_escrow(env: &Env) -> Address {
        env.storage()
            .instance()
            .get(&DataKey::RecoveryEscrow)
            .unwrap_or_else(|| panic_with_error!(env, Error::NotRecoveryEscrow))
    }

    fn assert_admin(env: &Env, caller: &Address) {
        caller.require_auth();
        let admin = Self::require_admin(env);
        if *caller != admin {
            panic_with_error!(env, Error::Unauthorized);
        }
    }

    /// The pause blocks ordinary user activity, but not the configured `RecoveryEscrow` acting on
    /// its own balance: `seize_sy` unwraps through `withdraw`, and `seize_lp` receives SY through
    /// `transfer`, and a recovery must not become impossible during the very incident that
    /// justifies a legal freeze. Only calls where the escrow is a party are exempt.
    fn assert_not_paused_unless_escrow(env: &Env, parties: &[&Address]) {
        if let Some(escrow) = env
            .storage()
            .instance()
            .get::<_, Address>(&DataKey::RecoveryEscrow)
        {
            if parties.iter().any(|p| **p == escrow) {
                return;
            }
        }
        Self::assert_not_paused(env);
    }

    fn assert_not_paused(env: &Env) {
        let paused: bool = env
            .storage()
            .instance()
            .get(&DataKey::Paused)
            .unwrap_or(false);
        if paused {
            panic_with_error!(env, Error::Paused);
        }
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
        let underlying = Self::get_underlying(env);
        if !compliance::is_authorized(env, &underlying, account) {
            panic_with_error!(env, Error::NotAuthorizedOnSac);
        }
    }

    /// Enforce the per-address cap on `account`'s net deposits and record `amount` against it.
    fn charge_deposit_cap(env: &Env, account: &Address, amount: i128) {
        let cap: i128 = env
            .storage()
            .instance()
            .get(&DataKey::DepositCap)
            .unwrap_or(0);
        let key = DataKey::NetDeposited(account.clone());
        let current: i128 = env.storage().persistent().get(&key).unwrap_or(0);
        let updated = current
            .checked_add(amount)
            .unwrap_or_else(|| panic_with_error!(env, Error::ArithmeticOverflow));
        if cap > 0 && updated > cap {
            panic_with_error!(env, Error::DepositCapExceeded);
        }
        env.storage().persistent().set(&key, &updated);
        env.storage()
            .persistent()
            .extend_ttl(&key, BALANCE_TTL_LEDGERS, BALANCE_TTL_LEDGERS);
    }

    /// Credit `amount` withdrawn by `account` back against its net deposits (floored at zero).
    fn release_deposit_cap(env: &Env, account: &Address, amount: i128) {
        let key = DataKey::NetDeposited(account.clone());
        let current: i128 = env.storage().persistent().get(&key).unwrap_or(0);
        if current > 0 {
            let updated = if amount >= current {
                0
            } else {
                current - amount
            };
            env.storage().persistent().set(&key, &updated);
            env.storage()
                .persistent()
                .extend_ttl(&key, BALANCE_TTL_LEDGERS, BALANCE_TTL_LEDGERS);
        }
    }
}

#[cfg(test)]
mod test;
