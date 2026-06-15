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

#![no_std]

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, panic_with_error, symbol_short, token,
    Address, Env,
};

pub const RATE_SCALE: i128 = 10_000_000; // 1e7

/// TTL extension applied to every persistent per-user balance entry (~30 days at 5 s/ledger).
const BALANCE_TTL_LEDGERS: u32 = 518_400;

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
}

#[contracttype]
pub enum DataKey {
    Admin,
    Underlying,   // Address of the underlying token contract
    TotalUnderlying,
    TotalShares,
    Balance(Address), // SY share balance per holder
    Paused,
}

#[contract]
pub struct SYWrapperContract;

#[contractimpl]
impl SYWrapperContract {
    /// Initialize with the admin address and the underlying token contract address.
    pub fn initialize(env: Env, admin: Address, underlying: Address) {
        if env.storage().instance().has(&DataKey::Admin) {
            panic_with_error!(&env, Error::AlreadyInitialized);
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::Underlying, &underlying);
        env.storage().instance().set(&DataKey::TotalUnderlying, &0_i128);
        env.storage().instance().set(&DataKey::TotalShares, &0_i128);
        env.storage().instance().set(&DataKey::Paused, &false);
    }

    // --- deposit / withdraw ---

    /// Deposit `amount` of the underlying asset; returns shares minted to `from`.
    pub fn deposit(env: Env, from: Address, amount: i128) -> i128 {
        from.require_auth();
        Self::assert_not_paused(&env);
        if amount <= 0 {
            panic_with_error!(&env, Error::ZeroAmount);
        }

        // Transfer underlying from depositor to this contract.
        let underlying = Self::get_underlying(&env);
        token::Client::new(&env, &underlying).transfer(
            &from,
            &env.current_contract_address(),
            &amount,
        );

        // Compute shares to mint at current exchange rate.
        let shares = Self::underlying_to_shares(&env, amount);
        if shares <= 0 {
            panic_with_error!(&env, Error::ZeroAmount);
        }

        // Update state.
        let total_u: i128 = env.storage().instance().get(&DataKey::TotalUnderlying).unwrap_or(0);
        let total_s: i128 = env.storage().instance().get(&DataKey::TotalShares).unwrap_or(0);
        env.storage()
            .instance()
            .set(&DataKey::TotalUnderlying, &(total_u + amount));
        env.storage()
            .instance()
            .set(&DataKey::TotalShares, &(total_s + shares));
        Self::add_balance(&env, &from, shares);

        env.events()
            .publish((symbol_short!("deposit"),), (from, amount, shares));
        shares
    }

    /// Burn `shares` and return the equivalent underlying amount to `to`.
    pub fn withdraw(env: Env, from: Address, shares: i128, to: Address) -> i128 {
        from.require_auth();
        Self::assert_not_paused(&env);
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
}

#[cfg(test)]
mod test {
    use soroban_sdk::{
        testutils::{Address as _, MockAuth, MockAuthInvoke},
        token, Address, Env, IntoVal,
    };

    use super::{SYWrapperContract, SYWrapperContractClient, RATE_SCALE};

    /// Deploy a minimal mock token for testing without a full SAC.
    fn deploy_token(env: &Env, admin: &Address) -> Address {
        let token_id = env.register_stellar_asset_contract_v2(admin.clone());
        token_id.address()
    }

    fn setup() -> (
        Env,
        SYWrapperContractClient<'static>,
        Address, // admin
        Address, // underlying token
    ) {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let underlying = deploy_token(&env, &admin);
        let wrapper_id = env.register_contract(None, SYWrapperContract);
        let client = SYWrapperContractClient::new(&env, &wrapper_id);
        client.initialize(&admin, &underlying);
        (env, client, admin, underlying)
    }

    fn mint(env: &Env, token: &Address, admin: &Address, to: &Address, amount: i128) {
        let tok = token::StellarAssetClient::new(env, token);
        tok.mint(to, &amount);
    }

    #[test]
    fn deposit_and_exchange_rate() {
        let (env, client, admin, underlying) = setup();
        let user = Address::generate(&env);
        mint(&env, &underlying, &admin, &user, 1_000_000_000);

        let shares = client.deposit(&user, &1_000_000_000_i128);
        // First depositor: shares == underlying (1:1).
        assert_eq!(shares, 1_000_000_000);
        assert_eq!(client.exchange_rate(), RATE_SCALE);
        assert_eq!(client.balance_of(&user), 1_000_000_000);
    }

    #[test]
    fn withdraw_returns_underlying() {
        let (env, client, admin, underlying) = setup();
        let user = Address::generate(&env);
        mint(&env, &underlying, &admin, &user, 500_000_000);

        let shares = client.deposit(&user, &500_000_000_i128);
        let out = client.withdraw(&user, &shares, &user);
        assert_eq!(out, 500_000_000);
        assert_eq!(client.total_shares(), 0);
    }

    #[test]
    #[should_panic]
    fn withdraw_more_than_balance_panics() {
        let (env, client, admin, underlying) = setup();
        let user = Address::generate(&env);
        mint(&env, &underlying, &admin, &user, 100_000_000);
        client.deposit(&user, &100_000_000_i128);
        client.withdraw(&user, &200_000_000_i128, &user);
    }

    #[test]
    #[should_panic]
    fn deposit_while_paused_panics() {
        let (env, client, admin, underlying) = setup();
        let user = Address::generate(&env);
        mint(&env, &underlying, &admin, &user, 100_000_000);
        client.set_paused(&admin, &true);
        client.deposit(&user, &100_000_000_i128);
    }

    #[test]
    fn unpause_re_enables_deposits() {
        let (env, client, admin, underlying) = setup();
        let user = Address::generate(&env);
        mint(&env, &underlying, &admin, &user, 200_000_000);

        client.set_paused(&admin, &true);
        client.set_paused(&admin, &false); // unpause
        // Deposit should succeed after unpause.
        let shares = client.deposit(&user, &200_000_000_i128);
        assert!(shares > 0);
    }

    #[test]
    fn exchange_rate_stays_at_inception_rate() {
        // In the POC, yield accrual is not simulated — the exchange rate is always
        // determined by deposits/withdrawals which maintain the ratio.
        let (env, client, admin, underlying) = setup();
        let user = Address::generate(&env);
        mint(&env, &underlying, &admin, &user, 1_000_000_000);

        let shares = client.deposit(&user, &1_000_000_000_i128);
        assert_eq!(client.exchange_rate(), RATE_SCALE); // 1:1 at inception

        // Second depositor at the same 1:1 rate.
        let user2 = Address::generate(&env);
        mint(&env, &underlying, &admin, &user2, 500_000_000);
        client.deposit(&user2, &500_000_000_i128);
        assert_eq!(client.exchange_rate(), RATE_SCALE); // still 1:1

        assert_eq!(client.total_underlying(), 1_500_000_000);
        assert_eq!(client.total_shares(), 1_500_000_000);
        let _ = shares;
    }

    #[test]
    #[should_panic]
    fn zero_deposit_panics() {
        let (env, client, _admin, _underlying) = setup();
        let user = Address::generate(&env);
        client.deposit(&user, &0_i128);
    }

    #[test]
    fn balance_of_returns_correct_shares() {
        let (env, client, admin, underlying) = setup();
        let user = Address::generate(&env);
        mint(&env, &underlying, &admin, &user, 300_000_000);
        client.deposit(&user, &300_000_000_i128);
        assert_eq!(client.balance_of(&user), 300_000_000);
    }

    #[test]
    fn admin_transfer() {
        let (env, client, admin, _) = setup();
        let new_admin = Address::generate(&env);
        client.transfer_admin(&admin, &new_admin);
        assert_eq!(client.get_admin(), new_admin);
    }
}
