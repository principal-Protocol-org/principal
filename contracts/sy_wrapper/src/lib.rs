//! SYWrapper — standardized yield wrapper for a single underlying yield-bearing asset.
//!
//! # Design
//! Users deposit the underlying asset (e.g. USDY) and receive SY shares in return.
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
//! # Compliance
//! `deposit` and `withdraw` both check `Permissioning.is_allowed()` (added after the SCF #44
//! resubmission audit found this contract had no eligibility gate at all — only
//! `PrincipalManager.mint()` did). `remediate()` is Clawback Propagation: it lets the admin
//! (expected in production to be an issuer-authorized compliance role, not the routine protocol
//! admin key) burn a single flagged account's own SY balance and release the equivalent
//! underlying, without touching any other depositor's share of the pool. See
//! PHASE2_DESIGN.md §2-3 for the full rationale.

#![no_std]

use soroban_sdk::{
    contract, contracterror, contractclient, contractimpl, contracttype, panic_with_error,
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
}

#[contracttype]
pub enum DataKey {
    Admin,
    Underlying,   // Address of the underlying token contract
    Permissioning,
    TotalUnderlying,
    TotalShares,
    Balance(Address), // SY share balance per holder
    Paused,
}

#[contract]
pub struct SYWrapperContract;

#[contractimpl]
impl SYWrapperContract {
    /// Initialize with the admin address, the underlying token contract address, and the
    /// Permissioning registry used to gate deposits/withdrawals.
    pub fn initialize(env: Env, admin: Address, underlying: Address, permissioning: Address) {
        if env.storage().instance().has(&DataKey::Admin) {
            panic_with_error!(&env, Error::AlreadyInitialized);
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::Underlying, &underlying);
        env.storage()
            .instance()
            .set(&DataKey::Permissioning, &permissioning);
        env.storage().instance().set(&DataKey::TotalUnderlying, &0_i128);
        env.storage().instance().set(&DataKey::TotalShares, &0_i128);
        env.storage().instance().set(&DataKey::Paused, &false);
    }

    // --- deposit / withdraw ---

    /// Deposit `amount` of the underlying asset; returns shares minted to `from`.
    pub fn deposit(env: Env, from: Address, amount: i128) -> i128 {
        from.require_auth();
        Self::assert_not_paused(&env);
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

        // Effects before interaction (SECURITY.md's stated invariant for this contract, which
        // this function previously violated: the external transfer ran before these updates,
        // leaving a window where a malicious `underlying` token contract could reenter deposit
        // with total_underlying/total_shares still at their pre-call values). Updating state
        // first means a reentrant call sees this deposit already accounted for.
        let total_u: i128 = env.storage().instance().get(&DataKey::TotalUnderlying).unwrap_or(0);
        let total_s: i128 = env.storage().instance().get(&DataKey::TotalShares).unwrap_or(0);
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
        // Both sides are checked, not just `to`: if only the recipient were gated, a flagged
        // account could self-withdraw the instant it suspected remediation was coming,
        // completely defeating remediate() (Clawback Propagation) by cashing out first. An
        // account whose eligibility is revoked is frozen on both the sending and receiving
        // side until remediated, not just blocked from directing funds to new destinations.
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
        let total_u: i128 = env.storage().instance().get(&DataKey::TotalUnderlying).unwrap_or(0);
        let total_s: i128 = env.storage().instance().get(&DataKey::TotalShares).unwrap_or(0);
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
        let total_s: i128 = env.storage().instance().get(&DataKey::TotalShares).unwrap_or(0);
        if total_s == 0 {
            return RATE_SCALE; // 1:1 at inception
        }
        let total_u: i128 = env.storage().instance().get(&DataKey::TotalUnderlying).unwrap_or(0);
        total_u * RATE_SCALE / total_s
    }

    pub fn total_underlying(env: Env) -> i128 {
        env.storage().instance().get(&DataKey::TotalUnderlying).unwrap_or(0)
    }

    pub fn total_shares(env: Env) -> i128 {
        env.storage().instance().get(&DataKey::TotalShares).unwrap_or(0)
    }

    pub fn balance_of(env: Env, account: Address) -> i128 {
        Self::get_balance(&env, &account)
    }

    pub fn underlying_address(env: Env) -> Address {
        Self::get_underlying(&env)
    }

    // --- admin ---

    pub fn set_paused(env: Env, caller: Address, paused: bool) {
        Self::assert_admin(&env, &caller);
        env.storage().instance().set(&DataKey::Paused, &paused);
        env.events()
            .publish((symbol_short!("paused"),), paused);
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

    // --- compliance remediation (Clawback Propagation) ---

    /// Burn exactly `shares` from `account`'s own SY balance and release the equivalent
    /// underlying to `caller`. Never touches any other depositor's balance — `shares` can be
    /// at most `account`'s own holding, so a single flagged account's remediation can't
    /// haircut the shared pool the way a native issuer clawback against this contract's
    /// pooled balance would. `caller` must be this contract's admin; in production that role
    /// is expected to be an issuer-authorized compliance signer, not the routine protocol
    /// admin key — the contract enforces *who* can call this, not *why* they're calling it.
    pub fn remediate(env: Env, caller: Address, account: Address, shares: i128) -> i128 {
        Self::assert_admin(&env, &caller);
        if shares <= 0 {
            panic_with_error!(&env, Error::ZeroAmount);
        }

        let balance = Self::get_balance(&env, &account);
        if balance < shares {
            panic_with_error!(&env, Error::InsufficientShares);
        }

        let underlying_out = Self::shares_to_underlying(&env, shares);
        if underlying_out <= 0 {
            panic_with_error!(&env, Error::ZeroAmount);
        }

        let total_u: i128 = env.storage().instance().get(&DataKey::TotalUnderlying).unwrap_or(0);
        let total_s: i128 = env.storage().instance().get(&DataKey::TotalShares).unwrap_or(0);
        env.storage()
            .instance()
            .set(&DataKey::TotalUnderlying, &(total_u - underlying_out));
        env.storage()
            .instance()
            .set(&DataKey::TotalShares, &(total_s - shares));
        Self::sub_balance(&env, &account, shares);

        let underlying = Self::get_underlying(&env);
        token::Client::new(&env, &underlying).transfer(
            &env.current_contract_address(),
            &caller,
            &underlying_out,
        );

        env.events().publish(
            (symbol_short!("remediate"),),
            (caller, account, shares, underlying_out),
        );
        underlying_out
    }

    // --- internal helpers ---

    fn underlying_to_shares(env: &Env, amount: i128) -> i128 {
        let total_s: i128 = env.storage().instance().get(&DataKey::TotalShares).unwrap_or(0);
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

    fn assert_admin(env: &Env, caller: &Address) {
        caller.require_auth();
        let admin = Self::require_admin(env);
        if *caller != admin {
            panic_with_error!(env, Error::Unauthorized);
        }
    }

    fn assert_not_paused(env: &Env) {
        let paused: bool = env.storage().instance().get(&DataKey::Paused).unwrap_or(false);
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
}

#[cfg(test)]
mod test {
    use soroban_sdk::{testutils::Address as _, token, Address, Env};

    use principal_permissioning::{PermissioningContract, PermissioningContractClient};

    use super::{SYWrapperContract, SYWrapperContractClient, RATE_SCALE};

    /// Deploy a minimal mock token for testing without a full SAC.
    fn deploy_token(env: &Env, admin: &Address) -> Address {
        let token_id = env.register_stellar_asset_contract_v2(admin.clone());
        token_id.address()
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

    fn grant(f: &Fixture, user: &Address) {
        f.perm.grant_account(&f.perm_admin, user);
    }

    fn mint(env: &Env, token: &Address, _admin: &Address, to: &Address, amount: i128) {
        let tok = token::StellarAssetClient::new(env, token);
        tok.mint(to, &amount);
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
        // Not granted.
        mint(&f.env, &f.underlying, &f.admin, &user, 1_000_000_000);
        f.client.deposit(&user, &1_000_000_000_i128);
    }

    #[test]
    #[should_panic]
    fn revoked_holder_cannot_front_run_remediation_by_self_withdrawing() {
        // If withdraw only checked `to`, a flagged account could cash out the instant it
        // suspected remediation was coming, defeating remediate() entirely. Revocation must
        // freeze the account on the sending side too, not just block new destinations.
        let f = setup();
        let user = Address::generate(&f.env);
        grant(&f, &user);
        mint(&f.env, &f.underlying, &f.admin, &user, 500_000_000);
        let shares = f.client.deposit(&user, &500_000_000_i128);

        f.perm.revoke_account(&f.perm_admin, &user);
        f.client.withdraw(&user, &shares, &user); // to == from, both now revoked
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

    // --- Clawback Propagation ---

    #[test]
    fn remediate_burns_only_flagged_account_share() {
        let f = setup();
        let bad_actor = Address::generate(&f.env);
        let innocent = Address::generate(&f.env);
        grant(&f, &bad_actor);
        grant(&f, &innocent);

        mint(&f.env, &f.underlying, &f.admin, &bad_actor, 1_000_000_000);
        mint(&f.env, &f.underlying, &f.admin, &innocent, 1_000_000_000);
        f.client.deposit(&bad_actor, &1_000_000_000_i128);
        f.client.deposit(&innocent, &1_000_000_000_i128);

        let released = f.client.remediate(&f.admin, &bad_actor, &1_000_000_000_i128);
        assert_eq!(released, 1_000_000_000);

        // Bad actor's SY balance is gone; innocent depositor's share is untouched.
        assert_eq!(f.client.balance_of(&bad_actor), 0);
        assert_eq!(f.client.balance_of(&innocent), 1_000_000_000);
        assert_eq!(f.client.total_shares(), 1_000_000_000);
    }

    #[test]
    #[should_panic]
    fn remediate_cannot_exceed_flagged_account_balance() {
        let f = setup();
        let bad_actor = Address::generate(&f.env);
        grant(&f, &bad_actor);
        mint(&f.env, &f.underlying, &f.admin, &bad_actor, 500_000_000);
        f.client.deposit(&bad_actor, &500_000_000_i128);

        // Attempting to remediate more than the account holds must fail, not spill into
        // the shared pool.
        f.client.remediate(&f.admin, &bad_actor, &600_000_000_i128);
    }

    #[test]
    #[should_panic]
    fn remediate_requires_admin() {
        let f = setup();
        let bad_actor = Address::generate(&f.env);
        let not_admin = Address::generate(&f.env);
        grant(&f, &bad_actor);
        mint(&f.env, &f.underlying, &f.admin, &bad_actor, 500_000_000);
        f.client.deposit(&bad_actor, &500_000_000_i128);

        f.client.remediate(&not_admin, &bad_actor, &500_000_000_i128);
    }
}
