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
//! `underlying_SAC.authorized(account)` (the mandatory floor — inherited live from the actual
//! issuer, with no separate registry that could drift out of sync with the issuer's own
//! decisions) and `Permissioning.is_allowed(account)` (an optional, Principal-specific
//! additional layer, narrower than but never looser than the SAC's own authorization).
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

use soroban_sdk::{
    contract, contractclient, contracterror, contractimpl, contracttype, panic_with_error,
    symbol_short, token, Address, Env,
};

pub const RATE_SCALE: i128 = 10_000_000; // 1e7

/// TTL extension applied to every persistent per-user balance entry (~30 days at 5 s/ledger).
const BALANCE_TTL_LEDGERS: u32 = 518_400;

#[contractclient(name = "PermClient")]
pub trait PermissioningInterface {
    fn is_allowed(env: Env, account: Address) -> bool;
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
        let sac_admin = token::StellarAssetClient::new(&env, &underlying).admin();
        if admin != sac_admin {
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
        Self::assert_not_paused(&env);
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

    /// Deposit `amount` of the underlying asset; returns shares minted to `from`.
    pub fn deposit(env: Env, from: Address, amount: i128) -> i128 {
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
        token::Client::new(&env, &underlying).transfer(
            &from,
            &env.current_contract_address(),
            &amount,
        );

        env.events()
            .publish((symbol_short!("deposit"),), (from, amount, shares));
        shares
    }

    /// Burn `shares` and return the equivalent underlying amount to `to`.
    pub fn withdraw(env: Env, from: Address, shares: i128, to: Address) -> i128 {
        from.require_auth();
        Self::assert_not_paused(&env);
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
        if !token::StellarAssetClient::new(env, &underlying).authorized(account) {
            panic_with_error!(env, Error::NotAuthorizedOnSac);
        }
    }
}

#[cfg(test)]
mod test {
    use soroban_sdk::{
        testutils::{Address as _, IssuerFlags},
        token, Address, Env,
    };

    use principal_permissioning::{PermissioningContract, PermissioningContractClient};

    use super::{SYWrapperContract, SYWrapperContractClient, RATE_SCALE};

    /// Deploy a minimal mock SAC for testing. `admin` becomes the SAC's real admin, matching
    /// how `initialize` now requires SYWrapper's own admin to equal `underlying.admin()`. The
    /// issuer's `RevocableFlag` is set so tests can simulate deauthorization
    /// (`set_authorized(_, false)`) -- without it, deauthorizing panics with "issuer does not
    /// have AUTH_REVOCABLE set" instead of exercising the check under test.
    fn deploy_token(env: &Env, admin: &Address) -> Address {
        let sac = env.register_stellar_asset_contract_v2(admin.clone());
        sac.issuer().set_flag(IssuerFlags::RevocableFlag);
        sac.address()
    }

    struct Fixture {
        env: Env,
        client: SYWrapperContractClient<'static>,
        admin: Address,
        underlying: Address,
        perm: PermissioningContractClient<'static>,
        perm_admin: Address,
    }

    fn setup() -> Fixture {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let underlying = deploy_token(&env, &admin);

        let perm_id = env.register_contract(None, PermissioningContract);
        let perm = PermissioningContractClient::new(&env, &perm_id);
        let perm_admin = Address::generate(&env);
        perm.initialize(&perm_admin);

        let wrapper_id = env.register_contract(None, SYWrapperContract);
        let client = SYWrapperContractClient::new(&env, &wrapper_id);
        // admin == underlying's real SAC admin, satisfying the new issuer-match requirement.
        client.initialize(&admin, &underlying, &perm_id);

        Fixture {
            env,
            client,
            admin,
            underlying,
            perm,
            perm_admin,
        }
    }

    /// Grants both layers: Permissioning (Principal-specific) and SAC authorization (the
    /// mandatory floor inherited from the issuer). Most tests want both cleared.
    fn grant(f: &Fixture, user: &Address) {
        f.perm.grant_account(&f.perm_admin, user);
        token::StellarAssetClient::new(&f.env, &f.underlying).set_authorized(user, &true);
    }

    fn mint(env: &Env, token: &Address, _admin: &Address, to: &Address, amount: i128) {
        let tok = token::StellarAssetClient::new(env, token);
        tok.mint(to, &amount);
    }

    #[test]
    #[should_panic]
    fn initialize_rejects_admin_not_matching_sac_admin() {
        let env = Env::default();
        env.mock_all_auths();
        let real_sac_admin = Address::generate(&env);
        let underlying = deploy_token(&env, &real_sac_admin);
        let perm_id = env.register_contract(None, PermissioningContract);
        PermissioningContractClient::new(&env, &perm_id).initialize(&real_sac_admin);

        let impostor = Address::generate(&env);
        let wrapper_id = env.register_contract(None, SYWrapperContract);
        let client = SYWrapperContractClient::new(&env, &wrapper_id);
        // impostor is not the underlying SAC's admin -- market creation must be rejected.
        client.initialize(&impostor, &underlying, &perm_id);
    }

    #[test]
    fn deposit_and_exchange_rate() {
        let f = setup();
        let user = Address::generate(&f.env);
        grant(&f, &user);
        mint(&f.env, &f.underlying, &f.admin, &user, 1_000_000_000);

        let shares = f.client.deposit(&user, &1_000_000_000_i128);
        // First depositor: shares == underlying (1:1).
        assert_eq!(shares, 1_000_000_000);
        assert_eq!(f.client.exchange_rate(), RATE_SCALE);
        assert_eq!(f.client.balance_of(&user), 1_000_000_000);
    }

    #[test]
    #[should_panic]
    fn deposit_without_permissioning_grant_panics() {
        let f = setup();
        let user = Address::generate(&f.env);
        // SAC-authorized but never granted in Principal's own Permissioning layer.
        token::StellarAssetClient::new(&f.env, &f.underlying).set_authorized(&user, &true);
        mint(&f.env, &f.underlying, &f.admin, &user, 1_000_000_000);
        f.client.deposit(&user, &1_000_000_000_i128);
    }

    #[test]
    #[should_panic]
    fn deposit_without_sac_authorization_panics() {
        // Granted in Principal's own Permissioning, but explicitly deauthorized on the
        // underlying SAC itself (e.g. the issuer never cleared them, or revoked them) -- the
        // mandatory floor inherited from the issuer must still block this regardless of
        // Principal's own Permissioning state. (A freshly-registered test SAC defaults every
        // address to authorized=true, matching real unrestricted-asset semantics, so this test
        // explicitly deauthorizes rather than relying on an unset default.)
        let f = setup();
        let user = Address::generate(&f.env);
        f.perm.grant_account(&f.perm_admin, &user);
        token::StellarAssetClient::new(&f.env, &f.underlying).set_authorized(&user, &false);
        mint(&f.env, &f.underlying, &f.admin, &user, 1_000_000_000);
        f.client.deposit(&user, &1_000_000_000_i128);
    }

    #[test]
    #[should_panic]
    fn deauthorized_on_sac_cannot_front_run_seizure_by_self_withdrawing() {
        // If withdraw only checked `to`, an investor the issuer just deauthorized on the SAC
        // could cash out the instant they suspected a seizure was coming.
        let f = setup();
        let user = Address::generate(&f.env);
        grant(&f, &user);
        mint(&f.env, &f.underlying, &f.admin, &user, 500_000_000);
        let shares = f.client.deposit(&user, &500_000_000_i128);

        // Issuer deauthorizes the account directly on the underlying SAC.
        token::StellarAssetClient::new(&f.env, &f.underlying).set_authorized(&user, &false);
        f.client.withdraw(&user, &shares, &user);
    }

    #[test]
    fn withdraw_returns_underlying() {
        let f = setup();
        let user = Address::generate(&f.env);
        grant(&f, &user);
        mint(&f.env, &f.underlying, &f.admin, &user, 500_000_000);

        let shares = f.client.deposit(&user, &500_000_000_i128);
        let out = f.client.withdraw(&user, &shares, &user);
        assert_eq!(out, 500_000_000);
        assert_eq!(f.client.total_shares(), 0);
    }

    #[test]
    #[should_panic]
    fn withdraw_to_unpermitted_recipient_panics() {
        let f = setup();
        let user = Address::generate(&f.env);
        let stranger = Address::generate(&f.env);
        grant(&f, &user);
        mint(&f.env, &f.underlying, &f.admin, &user, 500_000_000);

        let shares = f.client.deposit(&user, &500_000_000_i128);
        f.client.withdraw(&user, &shares, &stranger); // stranger never granted
    }

    #[test]
    #[should_panic]
    fn withdraw_more_than_balance_panics() {
        let f = setup();
        let user = Address::generate(&f.env);
        grant(&f, &user);
        mint(&f.env, &f.underlying, &f.admin, &user, 100_000_000);
        f.client.deposit(&user, &100_000_000_i128);
        f.client.withdraw(&user, &200_000_000_i128, &user);
    }

    #[test]
    #[should_panic]
    fn deposit_while_paused_panics() {
        let f = setup();
        let user = Address::generate(&f.env);
        grant(&f, &user);
        mint(&f.env, &f.underlying, &f.admin, &user, 100_000_000);
        f.client.set_paused(&f.admin, &true);
        f.client.deposit(&user, &100_000_000_i128);
    }

    #[test]
    fn unpause_re_enables_deposits() {
        let f = setup();
        let user = Address::generate(&f.env);
        grant(&f, &user);
        mint(&f.env, &f.underlying, &f.admin, &user, 200_000_000);

        f.client.set_paused(&f.admin, &true);
        f.client.set_paused(&f.admin, &false); // unpause
        let shares = f.client.deposit(&user, &200_000_000_i128);
        assert!(shares > 0);
    }

    #[test]
    fn exchange_rate_stays_at_inception_rate() {
        let f = setup();
        let user = Address::generate(&f.env);
        grant(&f, &user);
        mint(&f.env, &f.underlying, &f.admin, &user, 1_000_000_000);

        let shares = f.client.deposit(&user, &1_000_000_000_i128);
        assert_eq!(f.client.exchange_rate(), RATE_SCALE); // 1:1 at inception

        let user2 = Address::generate(&f.env);
        grant(&f, &user2);
        mint(&f.env, &f.underlying, &f.admin, &user2, 500_000_000);
        f.client.deposit(&user2, &500_000_000_i128);
        assert_eq!(f.client.exchange_rate(), RATE_SCALE); // still 1:1

        assert_eq!(f.client.total_underlying(), 1_500_000_000);
        assert_eq!(f.client.total_shares(), 1_500_000_000);
        let _ = shares;
    }

    #[test]
    #[should_panic]
    fn zero_deposit_panics() {
        let f = setup();
        let user = Address::generate(&f.env);
        grant(&f, &user);
        f.client.deposit(&user, &0_i128);
    }

    #[test]
    fn balance_of_returns_correct_shares() {
        let f = setup();
        let user = Address::generate(&f.env);
        grant(&f, &user);
        mint(&f.env, &f.underlying, &f.admin, &user, 300_000_000);
        f.client.deposit(&user, &300_000_000_i128);
        assert_eq!(f.client.balance_of(&user), 300_000_000);
    }

    #[test]
    fn admin_transfer() {
        let f = setup();
        let new_admin = Address::generate(&f.env);
        f.client.transfer_admin(&f.admin, &new_admin);
        assert_eq!(f.client.get_admin(), new_admin);
    }

    // --- share transfer ---

    #[test]
    fn transfer_moves_shares_between_eligible_accounts() {
        let f = setup();
        let user = Address::generate(&f.env);
        let recipient = Address::generate(&f.env);
        grant(&f, &user);
        grant(&f, &recipient);
        mint(&f.env, &f.underlying, &f.admin, &user, 500_000_000);
        let shares = f.client.deposit(&user, &500_000_000_i128);

        let moved = f.client.transfer(&user, &recipient, &shares);
        assert_eq!(moved, shares);
        assert_eq!(f.client.balance_of(&user), 0);
        assert_eq!(f.client.balance_of(&recipient), shares);
        // Pure balance move: total shares/underlying unaffected.
        assert_eq!(f.client.total_shares(), shares);
    }

    #[test]
    #[should_panic]
    fn transfer_to_unpermitted_recipient_panics() {
        let f = setup();
        let user = Address::generate(&f.env);
        let stranger = Address::generate(&f.env);
        grant(&f, &user);
        mint(&f.env, &f.underlying, &f.admin, &user, 500_000_000);
        let shares = f.client.deposit(&user, &500_000_000_i128);

        f.client.transfer(&user, &stranger, &shares); // stranger never granted
    }

    #[test]
    #[should_panic]
    fn transfer_more_than_balance_panics() {
        let f = setup();
        let user = Address::generate(&f.env);
        let recipient = Address::generate(&f.env);
        grant(&f, &user);
        grant(&f, &recipient);
        mint(&f.env, &f.underlying, &f.admin, &user, 100_000_000);
        f.client.deposit(&user, &100_000_000_i128);

        f.client.transfer(&user, &recipient, &200_000_000_i128);
    }

    // --- seize (compliance recovery) ---

    #[test]
    fn set_recovery_escrow_once() {
        let f = setup();
        let escrow = Address::generate(&f.env);
        f.client.set_recovery_escrow(&f.admin, &escrow);
        assert_eq!(f.client.recovery_escrow(), escrow);
    }

    #[test]
    #[should_panic]
    fn set_recovery_escrow_twice_panics() {
        let f = setup();
        let escrow1 = Address::generate(&f.env);
        let escrow2 = Address::generate(&f.env);
        f.client.set_recovery_escrow(&f.admin, &escrow1);
        f.client.set_recovery_escrow(&f.admin, &escrow2);
    }

    #[test]
    fn seize_moves_balance_to_escrow_without_holder_auth() {
        let f = setup();
        let escrow = Address::generate(&f.env);
        f.client.set_recovery_escrow(&f.admin, &escrow);

        let bad_actor = Address::generate(&f.env);
        let innocent = Address::generate(&f.env);
        grant(&f, &bad_actor);
        grant(&f, &innocent);
        mint(&f.env, &f.underlying, &f.admin, &bad_actor, 1_000_000_000);
        mint(&f.env, &f.underlying, &f.admin, &innocent, 1_000_000_000);
        f.client.deposit(&bad_actor, &1_000_000_000_i128);
        f.client.deposit(&innocent, &1_000_000_000_i128);

        let seized = f.client.seize(&escrow, &bad_actor, &1_000_000_000_i128);
        assert_eq!(seized, 1_000_000_000);

        // Balance moved to the escrow; total shares unaffected (forced transfer, not a burn);
        // innocent depositor untouched.
        assert_eq!(f.client.balance_of(&bad_actor), 0);
        assert_eq!(f.client.balance_of(&escrow), 1_000_000_000);
        assert_eq!(f.client.balance_of(&innocent), 1_000_000_000);
        assert_eq!(f.client.total_shares(), 2_000_000_000);
    }

    #[test]
    #[should_panic]
    fn seize_requires_configured_escrow_caller() {
        let f = setup();
        let escrow = Address::generate(&f.env);
        let impostor = Address::generate(&f.env);
        f.client.set_recovery_escrow(&f.admin, &escrow);

        let bad_actor = Address::generate(&f.env);
        grant(&f, &bad_actor);
        mint(&f.env, &f.underlying, &f.admin, &bad_actor, 500_000_000);
        f.client.deposit(&bad_actor, &500_000_000_i128);

        f.client.seize(&impostor, &bad_actor, &500_000_000_i128);
    }

    #[test]
    #[should_panic]
    fn seize_cannot_exceed_target_balance() {
        let f = setup();
        let escrow = Address::generate(&f.env);
        f.client.set_recovery_escrow(&f.admin, &escrow);

        let bad_actor = Address::generate(&f.env);
        grant(&f, &bad_actor);
        mint(&f.env, &f.underlying, &f.admin, &bad_actor, 500_000_000);
        f.client.deposit(&bad_actor, &500_000_000_i128);

        f.client.seize(&escrow, &bad_actor, &600_000_000_i128);
    }

    #[test]
    fn seize_works_while_paused() {
        let f = setup();
        let escrow = Address::generate(&f.env);
        f.client.set_recovery_escrow(&f.admin, &escrow);

        let bad_actor = Address::generate(&f.env);
        grant(&f, &bad_actor);
        mint(&f.env, &f.underlying, &f.admin, &bad_actor, 500_000_000);
        f.client.deposit(&bad_actor, &500_000_000_i128);

        f.client.set_paused(&f.admin, &true);
        let seized = f.client.seize(&escrow, &bad_actor, &500_000_000_i128);
        assert_eq!(seized, 500_000_000);
    }

    #[test]
    fn escrow_can_unwrap_seized_sy_via_normal_withdraw() {
        // End-to-end: seize, then the escrow -- pre-authorized on both layers, same as any
        // legitimate holder -- unwraps into raw underlying via the ordinary withdraw path.
        let f = setup();
        let escrow = Address::generate(&f.env);
        f.client.set_recovery_escrow(&f.admin, &escrow);
        grant(&f, &escrow); // issuer + Principal both pre-authorize the escrow

        let bad_actor = Address::generate(&f.env);
        grant(&f, &bad_actor);
        mint(&f.env, &f.underlying, &f.admin, &bad_actor, 500_000_000);
        f.client.deposit(&bad_actor, &500_000_000_i128);

        f.client.seize(&escrow, &bad_actor, &500_000_000_i128);
        let unwrapped = f.client.withdraw(&escrow, &500_000_000_i128, &escrow);
        assert_eq!(unwrapped, 500_000_000);

        let underlying_client = token::Client::new(&f.env, &f.underlying);
        assert_eq!(underlying_client.balance(&escrow), 500_000_000);
    }
}
