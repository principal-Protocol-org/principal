//! PrincipalManager — tokenization engine for the Principal Protocol.
//!
//! # Responsibilities
//! * Mint PT (Principal Token) and YT (Yield Token) when a user splits SY shares.
//! * Burn PT and YT at maturity and release the underlying SY shares to redeemers.
//! * Enforce maturity, oracle freshness, and permissioning preconditions on every operation.
//!
//! # Accounting (all values use SCALE = 1e7)
//!
//! When `n` SY shares are deposited at oracle rate `R` (USDC per underlying, scaled):
//!   notional = n * R / SCALE
//!
//! PT minted  = notional   (redeemable for `pt * SCALE / final_rate` underlying at maturity)
//! YT minted  = notional   (captures yield above initial_rate between issuance and maturity)
//!
//! At maturity, given final oracle rate `R_final` and per-user `R_initial` stored at mint:
//!   PT redemption (underlying) = floor(pt_amount * SCALE / R_final)
//!   YT redemption (underlying) = floor(yt_amount * max(0, R_final - R_initial) / R_final)
//!
//! # POC scope
//! SY share transfers are tracked internally (no actual SYWrapper cross-contract call).
//! Underlying transfers at redemption are computed and returned but not dispatched.
//! Both are Phase 2 integration milestones once Router is available.

#![no_std]

use soroban_sdk::{
    contract, contracterror, contractclient, contractimpl, contracttype, panic_with_error,
    symbol_short, Address, Env,
};

pub const SCALE: i128 = 10_000_000; // 1e7

/// Maximum seconds the oracle price may be stale at redemption.
const MAX_ORACLE_STALENESS_SECS: u64 = 3_600;

/// TTL extension applied to every persistent per-user entry (~30 days at 5 s/ledger).
const BALANCE_TTL_LEDGERS: u32 = 518_400;

// ---------------------------------------------------------------------------
// External contract interfaces (used for cross-contract calls)
// ---------------------------------------------------------------------------

/// Minimum interface required from the OracleAdapter.
#[contractclient(name = "OracleClient")]
pub trait OracleInterface {
    fn get_reference_value(env: Env) -> i128;
    fn is_fresh(env: Env, max_stale_seconds: u64) -> bool;
}

/// Minimum interface required from the Permissioning contract.
#[contractclient(name = "PermClient")]
pub trait PermissioningInterface {
    fn is_allowed(env: Env, account: Address) -> bool;
}

// ---------------------------------------------------------------------------
// Error codes
// ---------------------------------------------------------------------------

#[contracterror]
#[derive(Copy, Clone, Debug, PartialEq)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    Unauthorized = 2,
    NotInitialized = 3,
    ZeroAmount = 4,
    NotMature = 5,
    AlreadyMature = 6,
    OracleStale = 7,
    InsufficientBalance = 8,
    Paused = 9,
    PermissionDenied = 10,
}

// ---------------------------------------------------------------------------
// Storage key schema
// ---------------------------------------------------------------------------

#[contracttype]
pub enum DataKey {
    Admin,
    SYWrapper,
    Oracle,
    Permissioning,
    Maturity,     // u64 unix timestamp
    Paused,
    PTBalance(Address),
    YTBalance(Address),
    TotalPT,
    TotalYT,
    /// SY shares credited to each minter (tracked internally; Phase 2 wires to SYWrapper).
    SYDeposit(Address),
    /// Oracle rate stored at the time of this user's first mint, used for YT settlement.
    InitialRate(Address),
}

// ---------------------------------------------------------------------------
// Return types
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone)]
pub struct MintResult {
    pub pt_minted: i128,
    pub yt_minted: i128,
}

#[contracttype]
#[derive(Clone)]
pub struct RedeemResult {
    pub underlying_from_pt: i128,
    pub underlying_from_yt: i128,
}

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

#[contract]
pub struct PrincipalManagerContract;

#[contractimpl]
impl PrincipalManagerContract {
    /// One-time initialization.
    ///
    /// * `sy_wrapper`    — address of the SYWrapper contract
    /// * `oracle`        — address of the OracleAdapter contract
    /// * `permissioning` — address of the Permissioning contract
    /// * `maturity`      — Unix timestamp at which PT and YT can be redeemed
    pub fn initialize(
        env: Env,
        admin: Address,
        sy_wrapper: Address,
        oracle: Address,
        permissioning: Address,
        maturity: u64,
    ) {
        if env.storage().instance().has(&DataKey::Admin) {
            panic_with_error!(&env, Error::AlreadyInitialized);
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::SYWrapper, &sy_wrapper);
        env.storage().instance().set(&DataKey::Oracle, &oracle);
        env.storage().instance().set(&DataKey::Permissioning, &permissioning);
        env.storage().instance().set(&DataKey::Maturity, &maturity);
        env.storage().instance().set(&DataKey::Paused, &false);
        env.storage().instance().set(&DataKey::TotalPT, &0_i128);
        env.storage().instance().set(&DataKey::TotalYT, &0_i128);
    }

    // --- core protocol operations ---

    /// Split `sy_shares` into PT + YT. The caller must already hold these shares in the
    /// SYWrapper and must authorize the transfer to this contract.
    ///
    /// Returns the number of PT and YT minted (equal at issuance).
    pub fn mint(env: Env, from: Address, sy_shares: i128) -> MintResult {
        from.require_auth();
        Self::assert_not_paused(&env);
        Self::assert_not_mature(&env);
        if sy_shares <= 0 {
            panic_with_error!(&env, Error::ZeroAmount);
        }

        // Verify the caller is on the permissioning allow-list.
        Self::assert_permitted(&env, &from);

        // Read the oracle rate at mint time; store it for this user's YT settlement.
        // If the user mints again, we keep their first recorded rate so that each
        // unit of YT in their balance is settled from the same baseline.
        // Production should track per-batch rates when multiple mints are supported.
        let initial_rate = Self::get_oracle_rate(&env);
        let rate_key = DataKey::InitialRate(from.clone());
        if !env.storage().persistent().has(&rate_key) {
            env.storage().persistent().set(&rate_key, &initial_rate);
            env.storage()
                .persistent()
                .extend_ttl(&rate_key, BALANCE_TTL_LEDGERS, BALANCE_TTL_LEDGERS);
        }

        // Track SY shares deposited (Phase 2 replaces this with an actual SYWrapper transfer).
        let deposit: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::SYDeposit(from.clone()))
            .unwrap_or(0);
        let sy_key = DataKey::SYDeposit(from.clone());
        env.storage()
            .persistent()
            .set(&sy_key, &(deposit + sy_shares));
        env.storage()
            .persistent()
            .extend_ttl(&sy_key, BALANCE_TTL_LEDGERS, BALANCE_TTL_LEDGERS);

        // Compute notional principal: sy_shares valued at oracle rate.
        let notional = sy_shares * initial_rate / SCALE;

        // Mint PT and YT (1:1 with notional).
        Self::add_pt_balance(&env, &from, notional);
        Self::add_yt_balance(&env, &from, notional);

        let total_pt: i128 = env.storage().instance().get(&DataKey::TotalPT).unwrap_or(0);
        let total_yt: i128 = env.storage().instance().get(&DataKey::TotalYT).unwrap_or(0);
        env.storage()
            .instance()
            .set(&DataKey::TotalPT, &(total_pt + notional));
        env.storage()
            .instance()
            .set(&DataKey::TotalYT, &(total_yt + notional));

        env.events()
            .publish((symbol_short!("mint"),), (from, sy_shares, notional));

        MintResult {
            pt_minted: notional,
            yt_minted: notional,
        }
    }

    /// Redeem PT and/or YT after maturity. Both can be supplied in any combination.
    ///
    /// * `pt_amount` — PT tokens to burn (0 = skip PT redemption)
    /// * `yt_amount` — YT tokens to burn (0 = skip YT redemption)
    ///
    /// Returns underlying units released for each token type.
    /// Note: actual transfer of underlying to the caller is a Phase 2 milestone.
    pub fn redeem(env: Env, from: Address, pt_amount: i128, yt_amount: i128) -> RedeemResult {
        from.require_auth();
        Self::assert_not_paused(&env);
        Self::assert_mature(&env);
        Self::assert_oracle_fresh(&env);

        if pt_amount == 0 && yt_amount == 0 {
            panic_with_error!(&env, Error::ZeroAmount);
        }

        let final_rate = Self::get_oracle_rate(&env);

        let mut from_pt = 0_i128;
        let mut from_yt = 0_i128;

        if pt_amount > 0 {
            let bal = Self::get_pt_balance(&env, &from);
            if bal < pt_amount {
                panic_with_error!(&env, Error::InsufficientBalance);
            }
            // PT: notional units → underlying = floor(pt_amount * SCALE / final_rate)
            from_pt = pt_amount * SCALE / final_rate;
            Self::sub_pt_balance(&env, &from, pt_amount);

            let total_pt: i128 = env.storage().instance().get(&DataKey::TotalPT).unwrap_or(0);
            env.storage()
                .instance()
                .set(&DataKey::TotalPT, &(total_pt - pt_amount));
        }

        if yt_amount > 0 {
            let bal = Self::get_yt_balance(&env, &from);
            if bal < yt_amount {
                panic_with_error!(&env, Error::InsufficientBalance);
            }
            // YT: captures yield accrued above the rate at this user's mint time.
            // yield_delta = max(0, final_rate - initial_rate)
            // underlying  = floor(yt_amount * yield_delta / final_rate)
            let initial_rate: i128 = env
                .storage()
                .persistent()
                .get(&DataKey::InitialRate(from.clone()))
                .unwrap_or(SCALE);
            let yield_delta = if final_rate > initial_rate {
                final_rate - initial_rate
            } else {
                0
            };
            from_yt = yt_amount * yield_delta / final_rate;
            Self::sub_yt_balance(&env, &from, yt_amount);

            let total_yt: i128 = env.storage().instance().get(&DataKey::TotalYT).unwrap_or(0);
            env.storage()
                .instance()
                .set(&DataKey::TotalYT, &(total_yt - yt_amount));
        }

        env.events().publish(
            (symbol_short!("redeem"),),
            (from, pt_amount, yt_amount, from_pt, from_yt),
        );

        RedeemResult {
            underlying_from_pt: from_pt,
            underlying_from_yt: from_yt,
        }
    }

    // --- views ---

    pub fn pt_balance(env: Env, account: Address) -> i128 {
        Self::get_pt_balance(&env, &account)
    }

    pub fn yt_balance(env: Env, account: Address) -> i128 {
        Self::get_yt_balance(&env, &account)
    }

    pub fn total_pt(env: Env) -> i128 {
        env.storage().instance().get(&DataKey::TotalPT).unwrap_or(0)
    }

    pub fn total_yt(env: Env) -> i128 {
        env.storage().instance().get(&DataKey::TotalYT).unwrap_or(0)
    }

    pub fn maturity(env: Env) -> u64 {
        env.storage()
            .instance()
            .get(&DataKey::Maturity)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized))
    }

    pub fn is_mature(env: Env) -> bool {
        let mat: u64 = env.storage().instance().get(&DataKey::Maturity).unwrap_or(u64::MAX);
        env.ledger().timestamp() >= mat
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

    // --- internal helpers ---

    fn get_oracle_rate(env: &Env) -> i128 {
        let oracle_addr: Address = env
            .storage()
            .instance()
            .get(&DataKey::Oracle)
            .unwrap_or_else(|| panic_with_error!(env, Error::NotInitialized));
        OracleClient::new(env, &oracle_addr).get_reference_value()
    }

    fn assert_oracle_fresh(env: &Env) {
        let oracle_addr: Address = env
            .storage()
            .instance()
            .get(&DataKey::Oracle)
            .unwrap_or_else(|| panic_with_error!(env, Error::NotInitialized));
        if !OracleClient::new(env, &oracle_addr).is_fresh(&MAX_ORACLE_STALENESS_SECS) {
            panic_with_error!(env, Error::OracleStale);
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

    fn get_pt_balance(env: &Env, account: &Address) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::PTBalance(account.clone()))
            .unwrap_or(0)
    }

    fn get_yt_balance(env: &Env, account: &Address) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::YTBalance(account.clone()))
            .unwrap_or(0)
    }

    fn add_pt_balance(env: &Env, account: &Address, delta: i128) {
        let key = DataKey::PTBalance(account.clone());
        let bal: i128 = env.storage().persistent().get(&key).unwrap_or(0);
        env.storage().persistent().set(&key, &(bal + delta));
        env.storage()
            .persistent()
            .extend_ttl(&key, BALANCE_TTL_LEDGERS, BALANCE_TTL_LEDGERS);
    }

    fn add_yt_balance(env: &Env, account: &Address, delta: i128) {
        let key = DataKey::YTBalance(account.clone());
        let bal: i128 = env.storage().persistent().get(&key).unwrap_or(0);
        env.storage().persistent().set(&key, &(bal + delta));
        env.storage()
            .persistent()
            .extend_ttl(&key, BALANCE_TTL_LEDGERS, BALANCE_TTL_LEDGERS);
    }

    fn sub_pt_balance(env: &Env, account: &Address, delta: i128) {
        let key = DataKey::PTBalance(account.clone());
        let bal: i128 = env.storage().persistent().get(&key).unwrap_or(0);
        env.storage().persistent().set(&key, &(bal - delta));
        env.storage()
            .persistent()
            .extend_ttl(&key, BALANCE_TTL_LEDGERS, BALANCE_TTL_LEDGERS);
    }

    fn sub_yt_balance(env: &Env, account: &Address, delta: i128) {
        let key = DataKey::YTBalance(account.clone());
        let bal: i128 = env.storage().persistent().get(&key).unwrap_or(0);
        env.storage().persistent().set(&key, &(bal - delta));
        env.storage()
            .persistent()
            .extend_ttl(&key, BALANCE_TTL_LEDGERS, BALANCE_TTL_LEDGERS);
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

    fn assert_not_paused(env: &Env) {
        if env.storage().instance().get(&DataKey::Paused).unwrap_or(false) {
            panic_with_error!(env, Error::Paused);
        }
    }

    fn assert_mature(env: &Env) {
        let mat: u64 = env.storage().instance().get(&DataKey::Maturity).unwrap_or(u64::MAX);
        if env.ledger().timestamp() < mat {
            panic_with_error!(env, Error::NotMature);
        }
    }

    fn assert_not_mature(env: &Env) {
        let mat: u64 = env.storage().instance().get(&DataKey::Maturity).unwrap_or(u64::MAX);
        if env.ledger().timestamp() >= mat {
            panic_with_error!(env, Error::AlreadyMature);
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod test {
    use soroban_sdk::{
        testutils::{Address as _, Ledger as _},
        Address, Env,
    };

    use principal_oracle_adapter::{OracleAdapterContract, OracleAdapterContractClient};
    use principal_permissioning::{PermissioningContract, PermissioningContractClient};

    use super::{PrincipalManagerContract, PrincipalManagerContractClient, MAX_ORACLE_STALENESS_SECS, SCALE};

    /// Base ledger timestamp (> 0 so the oracle can accept its first update).
    const T0: u64 = 1_000;

    /// All contracts deployed into the same Env, returned together so tests can
    /// create addresses, advance ledger time, and update the oracle after setup.
    struct TestFixture {
        env: Env,
        client: PrincipalManagerContractClient<'static>,
        pm_admin: Address,
        oracle: OracleAdapterContractClient<'static>,
        oracle_admin: Address,
        perm: PermissioningContractClient<'static>,
        perm_admin: Address,
    }

    /// Deploy oracle + permissioning + PrincipalManager into a single Env.
    /// Oracle rate is seeded at SCALE (1.0) at ledger timestamp T0.
    /// No users are pre-granted — tests call `grant_user` explicitly.
    fn setup(maturity: u64) -> TestFixture {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().with_mut(|li| li.timestamp = T0);

        // OracleAdapter: rate = 1.0 at T0.
        let oracle_id = env.register_contract(None, OracleAdapterContract);
        let oracle = OracleAdapterContractClient::new(&env, &oracle_id);
        let oracle_admin = Address::generate(&env);
        oracle.initialize(&oracle_admin);
        oracle.set_reference_value(&oracle_admin, &SCALE, &T0);

        // Permissioning (no accounts granted yet).
        let perm_id = env.register_contract(None, PermissioningContract);
        let perm = PermissioningContractClient::new(&env, &perm_id);
        let perm_admin = Address::generate(&env);
        perm.initialize(&perm_admin);

        // PrincipalManager.
        let pm_id = env.register_contract(None, PrincipalManagerContract);
        let client = PrincipalManagerContractClient::new(&env, &pm_id);
        let pm_admin = Address::generate(&env);
        let sy_wrapper = Address::generate(&env);
        client.initialize(&pm_admin, &sy_wrapper, &oracle_id, &perm_id, &maturity);

        TestFixture { env, client, pm_admin, oracle, oracle_admin, perm, perm_admin }
    }

    fn grant_user(f: &TestFixture, user: &Address) {
        f.perm.grant_account(&f.perm_admin, user);
    }

    // --- tests ---

    #[test]
    fn mint_before_maturity() {
        let f = setup(u64::MAX);
        let user = Address::generate(&f.env);
        grant_user(&f, &user);

        let result = f.client.mint(&user, &(100_i128 * SCALE));
        // Oracle rate = SCALE → notional = 100 * SCALE * SCALE / SCALE = 100 * SCALE.
        assert_eq!(result.pt_minted, 100_i128 * SCALE);
        assert_eq!(result.yt_minted, 100_i128 * SCALE);
        assert_eq!(f.client.pt_balance(&user), 100_i128 * SCALE);
        assert_eq!(f.client.yt_balance(&user), 100_i128 * SCALE);
    }

    #[test]
    #[should_panic]
    fn mint_after_maturity_panics() {
        // maturity = T0 means the contract is already mature at ledger time T0.
        let f = setup(T0);
        let user = Address::generate(&f.env);
        grant_user(&f, &user);
        f.client.mint(&user, &(100_i128 * SCALE));
    }

    #[test]
    #[should_panic]
    fn redeem_before_maturity_panics() {
        let f = setup(u64::MAX);
        let user = Address::generate(&f.env);
        grant_user(&f, &user);
        f.client.mint(&user, &(10_i128 * SCALE));
        f.client.redeem(&user, &(10_i128 * SCALE), &0_i128);
    }

    #[test]
    fn total_supply_tracks_mints() {
        let f = setup(u64::MAX);
        let u1 = Address::generate(&f.env);
        let u2 = Address::generate(&f.env);
        grant_user(&f, &u1);
        grant_user(&f, &u2);

        f.client.mint(&u1, &(30_i128 * SCALE));
        f.client.mint(&u2, &(70_i128 * SCALE));
        assert_eq!(f.client.total_pt(), 100_i128 * SCALE);
        assert_eq!(f.client.total_yt(), 100_i128 * SCALE);
    }

    #[test]
    fn total_supply_decrements_after_redeem() {
        let maturity = T0 + 500;
        let f = setup(maturity);
        let user = Address::generate(&f.env);
        grant_user(&f, &user);

        let result = f.client.mint(&user, &(100_i128 * SCALE));
        let pt = result.pt_minted;
        let yt = result.yt_minted;
        assert_eq!(f.client.total_pt(), pt);
        assert_eq!(f.client.total_yt(), yt);

        // Advance past maturity; oracle stays fresh (T0+501 − T0 = 501 < 3600).
        f.env.ledger().with_mut(|li| li.timestamp = maturity + 1);

        f.client.redeem(&user, &pt, &0_i128);
        assert_eq!(f.client.total_pt(), 0);
        assert_eq!(f.client.total_yt(), yt); // YT supply unchanged

        // YT with no rate change → yield_delta = 0 → 0 returned.
        f.client.redeem(&user, &0_i128, &yt);
        assert_eq!(f.client.total_yt(), 0);
    }

    #[test]
    fn redeem_pt_correct_formula() {
        let maturity = T0 + 500;
        let f = setup(maturity);
        let user = Address::generate(&f.env);
        grant_user(&f, &user);

        // Mint at rate = SCALE (1.0).
        let result = f.client.mint(&user, &(100_i128 * SCALE));
        let pt = result.pt_minted; // = 100 * SCALE

        // Advance to maturity; update oracle to 1.03.
        f.env.ledger().with_mut(|li| li.timestamp = maturity + 1);
        let final_rate: i128 = 10_300_000;
        f.oracle.set_reference_value(&f.oracle_admin, &final_rate, &(maturity + 1));

        let r = f.client.redeem(&user, &pt, &0_i128);
        let expected = pt * SCALE / final_rate;
        assert_eq!(r.underlying_from_pt, expected);
        assert_eq!(r.underlying_from_yt, 0);
    }

    #[test]
    fn redeem_yt_correct_formula_with_yield() {
        let maturity = T0 + 500;
        let f = setup(maturity);
        let user = Address::generate(&f.env);
        grant_user(&f, &user);

        // Mint at rate = SCALE (1.0); initial_rate stored = SCALE.
        let result = f.client.mint(&user, &(100_i128 * SCALE));
        let yt = result.yt_minted; // = 100 * SCALE

        // Advance to maturity; oracle → 1.03.
        f.env.ledger().with_mut(|li| li.timestamp = maturity + 1);
        let final_rate: i128 = 10_300_000;
        f.oracle.set_reference_value(&f.oracle_admin, &final_rate, &(maturity + 1));

        let r = f.client.redeem(&user, &0_i128, &yt);
        // yield_delta = 10_300_000 − 10_000_000 = 300_000
        // underlying  = floor(yt * 300_000 / 10_300_000)
        let yield_delta = final_rate - SCALE;
        let expected = yt * yield_delta / final_rate;
        assert_eq!(r.underlying_from_yt, expected);
        assert_eq!(r.underlying_from_pt, 0);
    }

    #[test]
    fn redeem_yt_zero_when_no_yield() {
        let maturity = T0 + 500;
        let f = setup(maturity);
        let user = Address::generate(&f.env);
        grant_user(&f, &user);

        let result = f.client.mint(&user, &(100_i128 * SCALE));
        let yt = result.yt_minted;

        // Oracle set at T0; ledger at T0+501 → delta = 501 < 3600 → fresh.
        f.env.ledger().with_mut(|li| li.timestamp = maturity + 1);

        let r = f.client.redeem(&user, &0_i128, &yt);
        assert_eq!(r.underlying_from_yt, 0); // final_rate == initial_rate
    }

    #[test]
    #[should_panic]
    fn oracle_stale_blocks_redeem() {
        let maturity = T0 + 500;
        let f = setup(maturity);
        let user = Address::generate(&f.env);
        grant_user(&f, &user);
        f.client.mint(&user, &(10_i128 * SCALE));

        // Advance past maturity AND past the 1-hour staleness window.
        // Oracle set at T0=1000; ledger → 1000+3601=4601 → delta=3601 > 3600 → stale.
        f.env
            .ledger()
            .with_mut(|li| li.timestamp = T0 + MAX_ORACLE_STALENESS_SECS + 1);
        f.client.redeem(&user, &(10_i128 * SCALE), &0_i128);
    }

    #[test]
    #[should_panic]
    fn unpermissioned_user_cannot_mint() {
        let f = setup(u64::MAX);
        // stranger was never granted — must be rejected.
        let stranger = Address::generate(&f.env);
        f.client.mint(&stranger, &(10_i128 * SCALE));
    }

    #[test]
    fn admin_transfer() {
        let f = setup(u64::MAX);
        let new_admin = Address::generate(&f.env);
        f.client.transfer_admin(&f.pm_admin, &new_admin);
        assert_eq!(f.client.get_admin(), new_admin);
    }
}
