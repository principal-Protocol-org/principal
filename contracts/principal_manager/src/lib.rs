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
//! YT minted  = notional   (captures yield above the rate at issuance, via YTToken's own index)
//!
//! At maturity, given final oracle rate `R_final`:
//!   PT redemption (underlying) = floor(pt_amount * SCALE / R_final)
//!   YT redemption (underlying) = whatever `YTToken.claim_yield` settles and returns (see below)
//!
//! # Why YT redemption delegates to YTToken instead of computing its own formula
//! An earlier version of this contract computed YT's payout itself, from a per-user rate
//! recorded at mint time: `yt_amount * max(0, R_final - R_initial) / R_final`. That formula is
//! numerically correct for a single, unbroken holding period -- but `YTToken.claim_yield` is a
//! separate, publicly callable entrypoint (`from.require_auth()` only, no permissioning around
//! *when* it can be called) backed by its own continuously-compounding index
//! (`update_yield_index`/`settle`, see YTToken's module docs). If this contract paid out its own
//! independently-computed amount at redemption *and* a holder could also call
//! `YTToken.claim_yield` directly at any point beforehand, the two paths could both pay out for
//! the same accrued yield. Rather than have two payers for one claim, redemption calls
//! `YTToken.update_yield_index` then `claim_yield` and treats its return value as authoritative --
//! it is already expressed in underlying units (verified: for a single price movement it is
//! numerically identical to the formula above; for multiple intermediate oracle updates it
//! compounds per-step, which is the standard, defensible approach and the one actually reachable
//! by any YT holder today).
//!
//! # Compliance — authorization inheritance and market creation
//! `mint` and `redeem` check both `underlying_SAC.authorized(from)` (the mandatory floor,
//! inherited live from the actual issuer) and `Permissioning.is_allowed(from)` (an optional,
//! Principal-specific additional layer). `initialize` requires `admin` to equal the underlying
//! SAC's actual `admin()`, so a market can only be created with the issuer's participation --
//! see `SYWrapper`'s module docs for the full rationale, which applies identically here.
//!
//! # Integration scope
//! `mint` takes real custody of the caller's SY shares via `SYWrapper.transfer` and mints real
//! `PTToken`/`YTToken` balances. `redeem` burns those real balances and releases real underlying
//! via `SYWrapper.withdraw`, self-authorizing as this contract's own address the same way
//! `RecoveryEscrow` does when unwrapping a seizure. This contract's own address must itself be
//! SAC-authorized and Permissioning-granted before deployment is usable, since it is now a
//! genuine SY-share holder between mint and redemption -- see DEPLOYMENT.md.
//!
//! SY share custody is converted to/from underlying amounts via `SYWrapper.exchange_rate()`
//! (shares-to-underlying), which is a *different* rate from the Oracle's USDC-per-underlying
//! price feed used for the PT/YT notional split above. For a price-appreciating asset like USDY,
//! where holding the token doesn't change its own balance, `SYWrapper`'s exchange rate stays at
//! 1.0 and this conversion is a no-op; for a balance-rebasing asset it would not be, and that
//! reconciliation is out of scope until a second asset type is actually onboarded.

#![no_std]

use soroban_sdk::{
    contract, contractclient, contracterror, contractimpl, contracttype, panic_with_error,
    symbol_short, token, Address, Env,
};

pub const SCALE: i128 = 10_000_000; // 1e7

/// Maximum seconds the oracle price may be stale at redemption.
const MAX_ORACLE_STALENESS_SECS: u64 = 3_600;

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

/// Minimum interface required from SYWrapper.
#[contractclient(name = "SYWrapperClient")]
pub trait SYWrapperInterface {
    fn transfer(env: Env, from: Address, to: Address, amount: i128) -> i128;
    fn withdraw(env: Env, from: Address, shares: i128, to: Address) -> i128;
    fn exchange_rate(env: Env) -> i128;
}

/// Minimum interface required from PTToken.
#[contractclient(name = "PTTokenClient")]
pub trait PTTokenInterface {
    fn mint(env: Env, to: Address, amount: i128);
    fn burn(env: Env, from: Address, amount: i128);
    fn balance(env: Env, account: Address) -> i128;
    fn total_supply(env: Env) -> i128;
}

/// Minimum interface required from YTToken.
#[contractclient(name = "YTTokenClient")]
pub trait YTTokenInterface {
    fn mint(env: Env, to: Address, amount: i128);
    fn burn(env: Env, from: Address, amount: i128);
    fn update_yield_index(env: Env);
    fn claim_yield(env: Env, from: Address) -> i128;
    fn balance(env: Env, account: Address) -> i128;
    fn total_supply(env: Env) -> i128;
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
    Paused = 9,
    PermissionDenied = 10,
    NotAuthorizedOnSac = 11,
    IssuerMismatch = 12,
}

// ---------------------------------------------------------------------------
// Storage key schema
// ---------------------------------------------------------------------------

#[contracttype]
pub enum DataKey {
    Admin,
    SYWrapper,
    PTToken,
    YTToken,
    Oracle,
    Permissioning,
    Underlying, // Address of the underlying SAC, used for authorization inheritance
    Maturity,   // u64 unix timestamp
    Paused,
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
    /// * `pt_token`      — address of the PTToken contract (this contract must later be
    ///                     registered as its minter via `PTToken.set_minter`)
    /// * `yt_token`      — address of the YTToken contract (same two-phase pattern)
    /// * `oracle`        — address of the OracleAdapter contract
    /// * `permissioning` — address of the Permissioning contract
    /// * `underlying`    — address of the underlying SAC; `admin` must equal its `admin()`
    /// * `maturity`      — Unix timestamp at which PT and YT can be redeemed
    ///
    /// `admin` must be the underlying SAC's actual admin and must authorize this call --
    /// creating a new market (even a new maturity on an existing asset) requires the issuer's
    /// participation, matching `SYWrapper`'s market-creation gate.
    pub fn initialize(
        env: Env,
        admin: Address,
        sy_wrapper: Address,
        pt_token: Address,
        yt_token: Address,
        oracle: Address,
        permissioning: Address,
        underlying: Address,
        maturity: u64,
    ) {
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
            .set(&DataKey::SYWrapper, &sy_wrapper);
        env.storage().instance().set(&DataKey::PTToken, &pt_token);
        env.storage().instance().set(&DataKey::YTToken, &yt_token);
        env.storage().instance().set(&DataKey::Oracle, &oracle);
        env.storage()
            .instance()
            .set(&DataKey::Permissioning, &permissioning);
        env.storage()
            .instance()
            .set(&DataKey::Underlying, &underlying);
        env.storage().instance().set(&DataKey::Maturity, &maturity);
        env.storage().instance().set(&DataKey::Paused, &false);
    }

    // --- core protocol operations ---

    /// Split `sy_shares` into PT + YT. The caller must already hold these shares in the
    /// SYWrapper and must authorize both this call and the resulting SYWrapper transfer.
    ///
    /// Returns the number of PT and YT minted (equal at issuance).
    pub fn mint(env: Env, from: Address, sy_shares: i128) -> MintResult {
        from.require_auth();
        Self::assert_not_paused(&env);
        Self::assert_not_mature(&env);
        if sy_shares <= 0 {
            panic_with_error!(&env, Error::ZeroAmount);
        }

        // Verify the caller clears both compliance layers: the mandatory floor inherited from
        // the underlying SAC, and Principal's own optional additional narrowing.
        Self::assert_sac_authorized(&env, &from);
        Self::assert_permitted(&env, &from);

        // Compute notional principal: sy_shares valued at the current oracle rate.
        let rate = Self::get_oracle_rate(&env);
        let notional = sy_shares * rate / SCALE;

        // Take custody of the caller's SY shares -- this contract now holds them until
        // redemption. `from` already authorized this call above; that authorization covers
        // this nested SYWrapper invocation for the same address in the same transaction.
        let sy_wrapper = Self::get_sy_wrapper(&env);
        SYWrapperClient::new(&env, &sy_wrapper).transfer(
            &from,
            &env.current_contract_address(),
            &sy_shares,
        );

        // Mint real PT and YT (1:1 with notional) -- this contract must already be the
        // registered minter on both (set via `set_minter` after this contract is deployed).
        let pt_token = Self::get_pt_token(&env);
        let yt_token = Self::get_yt_token(&env);
        PTTokenClient::new(&env, &pt_token).mint(&from, &notional);
        YTTokenClient::new(&env, &yt_token).mint(&from, &notional);

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
    /// Burns the caller's real PT/YT balances and releases real underlying via SYWrapper.
    /// Returns the underlying units actually transferred for each token type.
    pub fn redeem(env: Env, from: Address, pt_amount: i128, yt_amount: i128) -> RedeemResult {
        from.require_auth();
        Self::assert_not_paused(&env);
        Self::assert_mature(&env);
        Self::assert_oracle_fresh(&env);
        // Same two-layer check as mint(): the SAC's own authorization is the mandatory floor,
        // Permissioning is an optional additional layer. See COMPLIANT_SETTLEMENT_DESIGN.md.
        Self::assert_sac_authorized(&env, &from);
        Self::assert_permitted(&env, &from);

        if pt_amount == 0 && yt_amount == 0 {
            panic_with_error!(&env, Error::ZeroAmount);
        }

        let final_rate = Self::get_oracle_rate(&env);
        let sy_wrapper = Self::get_sy_wrapper(&env);
        let sy_client = SYWrapperClient::new(&env, &sy_wrapper);
        let this_contract = env.current_contract_address();

        let mut from_pt = 0_i128;
        let mut from_yt = 0_i128;

        if pt_amount > 0 {
            let pt_token = Self::get_pt_token(&env);
            PTTokenClient::new(&env, &pt_token).burn(&from, &pt_amount);

            // PT: notional units → underlying = floor(pt_amount * SCALE / final_rate)
            let desired_underlying = pt_amount * SCALE / final_rate;
            let shares = Self::underlying_to_shares(sy_client.exchange_rate(), desired_underlying);
            from_pt = sy_client.withdraw(&this_contract, &shares, &from);
        }

        if yt_amount > 0 {
            let yt_token = Self::get_yt_token(&env);
            let yt_client = YTTokenClient::new(&env, &yt_token);

            // Bring the index current, then burn (settling pending yield) and claim it. See
            // this module's doc comment for why redemption delegates to YTToken's own
            // accrual/claim mechanism instead of computing a second, independent payout here.
            yt_client.update_yield_index();
            yt_client.burn(&from, &yt_amount);
            let desired_underlying = yt_client.claim_yield(&from);
            if desired_underlying > 0 {
                let shares =
                    Self::underlying_to_shares(sy_client.exchange_rate(), desired_underlying);
                from_yt = sy_client.withdraw(&this_contract, &shares, &from);
            }
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
        let pt_token = Self::get_pt_token(&env);
        PTTokenClient::new(&env, &pt_token).balance(&account)
    }

    pub fn yt_balance(env: Env, account: Address) -> i128 {
        let yt_token = Self::get_yt_token(&env);
        YTTokenClient::new(&env, &yt_token).balance(&account)
    }

    pub fn total_pt(env: Env) -> i128 {
        let pt_token = Self::get_pt_token(&env);
        PTTokenClient::new(&env, &pt_token).total_supply()
    }

    pub fn total_yt(env: Env) -> i128 {
        let yt_token = Self::get_yt_token(&env);
        YTTokenClient::new(&env, &yt_token).total_supply()
    }

    pub fn maturity(env: Env) -> u64 {
        env.storage()
            .instance()
            .get(&DataKey::Maturity)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized))
    }

    pub fn is_mature(env: Env) -> bool {
        let mat: u64 = env
            .storage()
            .instance()
            .get(&DataKey::Maturity)
            .unwrap_or(u64::MAX);
        env.ledger().timestamp() >= mat
    }

    pub fn underlying_address(env: Env) -> Address {
        env.storage()
            .instance()
            .get(&DataKey::Underlying)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized))
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

    fn assert_sac_authorized(env: &Env, account: &Address) {
        let underlying: Address = env
            .storage()
            .instance()
            .get(&DataKey::Underlying)
            .unwrap_or_else(|| panic_with_error!(env, Error::NotInitialized));
        if !token::StellarAssetClient::new(env, &underlying).authorized(account) {
            panic_with_error!(env, Error::NotAuthorizedOnSac);
        }
    }

    fn get_sy_wrapper(env: &Env) -> Address {
        env.storage()
            .instance()
            .get(&DataKey::SYWrapper)
            .unwrap_or_else(|| panic_with_error!(env, Error::NotInitialized))
    }

    fn get_pt_token(env: &Env) -> Address {
        env.storage()
            .instance()
            .get(&DataKey::PTToken)
            .unwrap_or_else(|| panic_with_error!(env, Error::NotInitialized))
    }

    fn get_yt_token(env: &Env) -> Address {
        env.storage()
            .instance()
            .get(&DataKey::YTToken)
            .unwrap_or_else(|| panic_with_error!(env, Error::NotInitialized))
    }

    /// Convert a desired underlying payout into the SY shares needed to withdraw it, at
    /// SYWrapper's current exchange rate (a different rate from the Oracle's pricing feed --
    /// see this module's doc comment). Inverts SYWrapper's own `shares * rate / RATE_SCALE`.
    fn underlying_to_shares(exchange_rate: i128, underlying: i128) -> i128 {
        underlying * SCALE / exchange_rate
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
        if env
            .storage()
            .instance()
            .get(&DataKey::Paused)
            .unwrap_or(false)
        {
            panic_with_error!(env, Error::Paused);
        }
    }

    fn assert_mature(env: &Env) {
        let mat: u64 = env
            .storage()
            .instance()
            .get(&DataKey::Maturity)
            .unwrap_or(u64::MAX);
        if env.ledger().timestamp() < mat {
            panic_with_error!(env, Error::NotMature);
        }
    }

    fn assert_not_mature(env: &Env) {
        let mat: u64 = env
            .storage()
            .instance()
            .get(&DataKey::Maturity)
            .unwrap_or(u64::MAX);
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
        testutils::{Address as _, IssuerFlags, Ledger as _},
        token, Address, Env, String,
    };

    use principal_oracle_adapter::{OracleAdapterContract, OracleAdapterContractClient};
    use principal_permissioning::{PermissioningContract, PermissioningContractClient};
    use principal_pt_token::{PTTokenContract, PTTokenContractClient};
    use principal_sy_wrapper::{SYWrapperContract, SYWrapperContractClient};
    use principal_yt_token::{YTTokenContract, YTTokenContractClient};

    use super::{
        PrincipalManagerContract, PrincipalManagerContractClient, MAX_ORACLE_STALENESS_SECS, SCALE,
    };

    /// Base ledger timestamp (> 0 so the oracle can accept its first update).
    const T0: u64 = 1_000;

    /// All contracts deployed into the same Env, returned together so tests can
    /// create addresses, advance ledger time, and update the oracle/mint SY after setup.
    struct TestFixture {
        env: Env,
        client: PrincipalManagerContractClient<'static>,
        pm_id: Address,
        pm_admin: Address,
        underlying: Address,
        oracle: OracleAdapterContractClient<'static>,
        oracle_admin: Address,
        perm: PermissioningContractClient<'static>,
        perm_admin: Address,
        sy: SYWrapperContractClient<'static>,
        pt: PTTokenContractClient<'static>,
        yt: YTTokenContractClient<'static>,
    }

    /// Deploy the full contract set (oracle, permissioning, an underlying SAC, SYWrapper,
    /// PTToken, YTToken, PrincipalManager) into a single Env, and wire PrincipalManager as the
    /// registered minter on both token contracts -- mirroring the real two-phase deployment
    /// order in DEPLOYMENT.md. Oracle rate is seeded at SCALE (1.0) at ledger timestamp T0.
    /// `pm_admin` is also the underlying SAC's real admin, satisfying the market-creation gate
    /// on every contract. No users are pre-granted -- tests call `grant_user` explicitly.
    fn setup(maturity: u64) -> TestFixture {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().with_mut(|li| li.timestamp = T0);

        let oracle_id = env.register_contract(None, OracleAdapterContract);
        let oracle = OracleAdapterContractClient::new(&env, &oracle_id);
        let oracle_admin = Address::generate(&env);
        oracle.initialize(&oracle_admin);
        oracle.set_reference_value(&oracle_admin, &SCALE, &T0);

        let perm_id = env.register_contract(None, PermissioningContract);
        let perm = PermissioningContractClient::new(&env, &perm_id);
        let perm_admin = Address::generate(&env);
        perm.initialize(&perm_admin);

        let pm_admin = Address::generate(&env);
        let underlying_sac = env.register_stellar_asset_contract_v2(pm_admin.clone());
        underlying_sac.issuer().set_flag(IssuerFlags::RevocableFlag);
        let underlying = underlying_sac.address();

        let sy_id = env.register_contract(None, SYWrapperContract);
        let sy = SYWrapperContractClient::new(&env, &sy_id);
        sy.initialize(&pm_admin, &underlying, &perm_id);

        let pt_id = env.register_contract(None, PTTokenContract);
        let pt = PTTokenContractClient::new(&env, &pt_id);
        pt.initialize(
            &pm_admin,
            &perm_id,
            &underlying,
            &maturity,
            &String::from_str(&env, "Principal Token USDY"),
            &String::from_str(&env, "PT-USDY"),
            &7,
        );

        let yt_id = env.register_contract(None, YTTokenContract);
        let yt = YTTokenContractClient::new(&env, &yt_id);
        yt.initialize(
            &pm_admin,
            &perm_id,
            &underlying,
            &oracle_id,
            &maturity,
            &String::from_str(&env, "Yield Token USDY"),
            &String::from_str(&env, "YT-USDY"),
            &7,
        );

        let pm_id = env.register_contract(None, PrincipalManagerContract);
        let client = PrincipalManagerContractClient::new(&env, &pm_id);
        client.initialize(
            &pm_admin, &sy_id, &pt_id, &yt_id, &oracle_id, &perm_id, &underlying, &maturity,
        );

        pt.set_minter(&pm_admin, &pm_id);
        yt.set_minter(&pm_admin, &pm_id);

        // PrincipalManager's own address becomes a genuine SY holder between mint and
        // redemption, and a transfer/withdraw recipient-or-sender on both sides -- it needs
        // the same two compliance layers as any other participant.
        perm.grant_account(&perm_admin, &pm_id);
        token::StellarAssetClient::new(&env, &underlying).set_authorized(&pm_id, &true);

        TestFixture {
            env,
            client,
            pm_id,
            pm_admin,
            underlying,
            oracle,
            oracle_admin,
            perm,
            perm_admin,
            sy,
            pt,
            yt,
        }
    }

    /// Grants a user both compliance layers on the shared Permissioning/SAC, plus the
    /// per-asset PT/YT grants PTToken/YTToken independently require, and mints them
    /// `underlying_amount` of the underlying asset. Does not deposit into SYWrapper --
    /// call `deposit_sy` for that, since not every test needs a real SY position.
    fn grant_user(f: &TestFixture, user: &Address) {
        f.perm.grant_account(&f.perm_admin, user);
        f.perm.grant_asset(&f.perm_admin, user, &f.pt.address);
        f.perm.grant_asset(&f.perm_admin, user, &f.yt.address);
        token::StellarAssetClient::new(&f.env, &f.underlying).set_authorized(user, &true);
    }

    /// Mints `amount` of the underlying to `user` and deposits it into SYWrapper, returning the
    /// SY shares received (1:1 at inception). `user` must already be granted.
    fn deposit_sy(f: &TestFixture, user: &Address, amount: i128) -> i128 {
        token::StellarAssetClient::new(&f.env, &f.underlying).mint(user, &amount);
        f.sy.deposit(user, &amount)
    }

    // --- tests ---

    #[test]
    fn mint_before_maturity() {
        let f = setup(u64::MAX);
        let user = Address::generate(&f.env);
        grant_user(&f, &user);
        let shares = deposit_sy(&f, &user, 100_i128 * SCALE);

        let result = f.client.mint(&user, &shares);
        // Oracle rate = SCALE → notional = 100 * SCALE * SCALE / SCALE = 100 * SCALE.
        assert_eq!(result.pt_minted, 100_i128 * SCALE);
        assert_eq!(result.yt_minted, 100_i128 * SCALE);
        assert_eq!(f.client.pt_balance(&user), 100_i128 * SCALE);
        assert_eq!(f.client.yt_balance(&user), 100_i128 * SCALE);
        // Real custody: the shares moved from the user to PrincipalManager itself.
        assert_eq!(f.sy.balance_of(&user), 0);
        assert_eq!(f.sy.balance_of(&f.pm_id), shares);
    }

    #[test]
    #[should_panic]
    fn mint_after_maturity_panics() {
        // maturity = T0 means the contract is already mature at ledger time T0.
        let f = setup(T0);
        let user = Address::generate(&f.env);
        grant_user(&f, &user);
        let shares = deposit_sy(&f, &user, 100_i128 * SCALE);
        f.client.mint(&user, &shares);
    }

    #[test]
    #[should_panic]
    fn redeem_before_maturity_panics() {
        let f = setup(u64::MAX);
        let user = Address::generate(&f.env);
        grant_user(&f, &user);
        let shares = deposit_sy(&f, &user, 10_i128 * SCALE);
        f.client.mint(&user, &shares);
        f.client.redeem(&user, &(10_i128 * SCALE), &0_i128);
    }

    #[test]
    fn total_supply_tracks_mints() {
        let f = setup(u64::MAX);
        let u1 = Address::generate(&f.env);
        let u2 = Address::generate(&f.env);
        grant_user(&f, &u1);
        grant_user(&f, &u2);
        let s1 = deposit_sy(&f, &u1, 30_i128 * SCALE);
        let s2 = deposit_sy(&f, &u2, 70_i128 * SCALE);

        f.client.mint(&u1, &s1);
        f.client.mint(&u2, &s2);
        assert_eq!(f.client.total_pt(), 100_i128 * SCALE);
        assert_eq!(f.client.total_yt(), 100_i128 * SCALE);
    }

    #[test]
    fn total_supply_decrements_after_redeem() {
        let maturity = T0 + 500;
        let f = setup(maturity);
        let user = Address::generate(&f.env);
        grant_user(&f, &user);
        let shares = deposit_sy(&f, &user, 100_i128 * SCALE);

        let result = f.client.mint(&user, &shares);
        let pt = result.pt_minted;
        let yt = result.yt_minted;
        assert_eq!(f.client.total_pt(), pt);
        assert_eq!(f.client.total_yt(), yt);

        // Advance past maturity; oracle stays fresh (T0+501 − T0 = 501 < 3600).
        f.env.ledger().with_mut(|li| li.timestamp = maturity + 1);

        f.client.redeem(&user, &pt, &0_i128);
        assert_eq!(f.client.total_pt(), 0);
        assert_eq!(f.client.total_yt(), yt); // YT supply unchanged

        // YT with no rate change → nothing accrued in YTToken's index → 0 returned, and the
        // YT balance itself is still burned down to 0.
        f.client.redeem(&user, &0_i128, &yt);
        assert_eq!(f.client.total_yt(), 0);
    }

    #[test]
    fn redeem_pt_correct_formula() {
        let maturity = T0 + 500;
        let f = setup(maturity);
        let user = Address::generate(&f.env);
        grant_user(&f, &user);
        let shares = deposit_sy(&f, &user, 100_i128 * SCALE);

        // Mint at rate = SCALE (1.0).
        let result = f.client.mint(&user, &shares);
        let pt = result.pt_minted; // = 100 * SCALE

        // Advance to maturity; update oracle to 1.03.
        f.env.ledger().with_mut(|li| li.timestamp = maturity + 1);
        let final_rate: i128 = 10_300_000;
        f.oracle
            .set_reference_value(&f.oracle_admin, &final_rate, &(maturity + 1));

        let r = f.client.redeem(&user, &pt, &0_i128);
        let expected = pt * SCALE / final_rate;
        assert_eq!(r.underlying_from_pt, expected);
        assert_eq!(r.underlying_from_yt, 0);
        // Real transfer: the user actually received the underlying asset.
        assert_eq!(token::Client::new(&f.env, &f.underlying).balance(&user), expected);
    }

    #[test]
    fn redeem_yt_correct_formula_with_yield() {
        let maturity = T0 + 500;
        let f = setup(maturity);
        let user = Address::generate(&f.env);
        grant_user(&f, &user);
        let shares = deposit_sy(&f, &user, 100_i128 * SCALE);

        // Mint at rate = SCALE (1.0).
        let result = f.client.mint(&user, &shares);
        let yt = result.yt_minted; // = 100 * SCALE

        // Advance to maturity; oracle → 1.03.
        f.env.ledger().with_mut(|li| li.timestamp = maturity + 1);
        let final_rate: i128 = 10_300_000;
        f.oracle
            .set_reference_value(&f.oracle_admin, &final_rate, &(maturity + 1));

        let r = f.client.redeem(&user, &0_i128, &yt);
        // Mirrors YTToken's own multiplicative index formula exactly (see
        // YTToken::update_yield_index / settle): new_factor = SCALE * SCALE / final_rate, then
        // pending = yt * (SCALE - new_factor) / SCALE for a user whose own snapshot is SCALE
        // (settled at mint, before the index ever moved). This is not the same arithmetic as
        // yt_amount * (final_rate - SCALE) / final_rate -- both are correct to within ordinary
        // floor-rounding, but they round differently, so the test must mirror the contract's
        // actual computation, not an algebraically-equivalent real-number rearrangement of it.
        let new_factor = SCALE * SCALE / final_rate;
        let expected = yt * (SCALE - new_factor) / SCALE;
        assert_eq!(r.underlying_from_yt, expected);
        assert_eq!(r.underlying_from_pt, 0);
    }

    #[test]
    fn redeem_yt_does_not_double_pay_yield_already_claimed_directly() {
        // YTToken.claim_yield is independently, publicly callable by any holder. If a user
        // claims yield directly and then redeems through PrincipalManager, redeem() must not
        // pay out that same accrued amount a second time -- both paths settle against the same
        // index, so whichever runs second sees nothing left pending.
        let maturity = T0 + 500;
        let f = setup(maturity);
        let user = Address::generate(&f.env);
        grant_user(&f, &user);
        let shares = deposit_sy(&f, &user, 100_i128 * SCALE);
        let result = f.client.mint(&user, &shares);
        let yt = result.yt_minted;

        f.env.ledger().with_mut(|li| li.timestamp = maturity + 1);
        f.oracle
            .set_reference_value(&f.oracle_admin, &10_300_000_i128, &(maturity + 1));

        // User claims directly, out-of-band from PrincipalManager, before redeeming.
        f.yt.update_yield_index();
        let direct_claim = f.yt.claim_yield(&user);
        assert!(direct_claim > 0);

        // Redemption must not pay the same yield again.
        let r = f.client.redeem(&user, &0_i128, &yt);
        assert_eq!(r.underlying_from_yt, 0);
    }

    #[test]
    fn redeem_yt_zero_when_no_yield() {
        let maturity = T0 + 500;
        let f = setup(maturity);
        let user = Address::generate(&f.env);
        grant_user(&f, &user);
        let shares = deposit_sy(&f, &user, 100_i128 * SCALE);

        let result = f.client.mint(&user, &shares);
        let yt = result.yt_minted;

        // Oracle set at T0; ledger at T0+501 → delta = 501 < 3600 → fresh.
        f.env.ledger().with_mut(|li| li.timestamp = maturity + 1);

        let r = f.client.redeem(&user, &0_i128, &yt);
        assert_eq!(r.underlying_from_yt, 0); // rate never moved → nothing accrued
    }

    #[test]
    #[should_panic]
    fn oracle_stale_blocks_redeem() {
        let maturity = T0 + 500;
        let f = setup(maturity);
        let user = Address::generate(&f.env);
        grant_user(&f, &user);
        let shares = deposit_sy(&f, &user, 10_i128 * SCALE);
        f.client.mint(&user, &shares);

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
        // stranger was never granted, so it also can't deposit into SYWrapper -- but even a
        // caller who somehow held shares would still be rejected at PrincipalManager's own gate.
        let stranger = Address::generate(&f.env);
        f.client.mint(&stranger, &(10_i128 * SCALE));
    }

    #[test]
    #[should_panic]
    fn revoked_user_cannot_redeem() {
        // Closes the audit gap: redeem() previously had no permissioning check at all.
        let maturity = T0 + 500;
        let f = setup(maturity);
        let user = Address::generate(&f.env);
        grant_user(&f, &user);
        let shares = deposit_sy(&f, &user, 10_i128 * SCALE);
        let result = f.client.mint(&user, &shares);

        f.perm.revoke_account(&f.perm_admin, &user);
        f.env.ledger().with_mut(|li| li.timestamp = maturity + 1);
        f.client.redeem(&user, &result.pt_minted, &0_i128);
    }

    #[test]
    fn admin_transfer() {
        let f = setup(u64::MAX);
        let new_admin = Address::generate(&f.env);
        f.client.transfer_admin(&f.pm_admin, &new_admin);
        assert_eq!(f.client.get_admin(), new_admin);
    }

    #[test]
    #[should_panic]
    fn initialize_rejects_admin_not_matching_sac_admin() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().with_mut(|li| li.timestamp = T0);

        let oracle_id = env.register_contract(None, OracleAdapterContract);
        let oracle = OracleAdapterContractClient::new(&env, &oracle_id);
        let oracle_admin = Address::generate(&env);
        oracle.initialize(&oracle_admin);
        oracle.set_reference_value(&oracle_admin, &SCALE, &T0);

        let perm_id = env.register_contract(None, PermissioningContract);
        let perm = PermissioningContractClient::new(&env, &perm_id);
        let perm_admin = Address::generate(&env);
        perm.initialize(&perm_admin);

        let real_sac_admin = Address::generate(&env);
        let underlying = env
            .register_stellar_asset_contract_v2(real_sac_admin.clone())
            .address();

        let impostor = Address::generate(&env);
        let pm_id = env.register_contract(None, PrincipalManagerContract);
        let client = PrincipalManagerContractClient::new(&env, &pm_id);
        let sy_wrapper = Address::generate(&env);
        let pt_token = Address::generate(&env);
        let yt_token = Address::generate(&env);
        // impostor is not the underlying SAC's admin -- market creation must be rejected.
        client.initialize(
            &impostor,
            &sy_wrapper,
            &pt_token,
            &yt_token,
            &oracle_id,
            &perm_id,
            &underlying,
            &u64::MAX,
        );
    }

    #[test]
    #[should_panic]
    fn deauthorized_on_sac_cannot_mint() {
        // Granted in Principal's own Permissioning, but never authorized (or since revoked) on
        // the underlying SAC -- the mandatory floor inherited from the issuer must still block.
        let f = setup(u64::MAX);
        let user = Address::generate(&f.env);
        f.perm.grant_account(&f.perm_admin, &user);
        token::StellarAssetClient::new(&f.env, &f.underlying).set_authorized(&user, &false);
        f.client.mint(&user, &(10_i128 * SCALE));
    }

    #[test]
    #[should_panic]
    fn deauthorized_on_sac_cannot_redeem() {
        let maturity = T0 + 500;
        let f = setup(maturity);
        let user = Address::generate(&f.env);
        grant_user(&f, &user);
        let shares = deposit_sy(&f, &user, 10_i128 * SCALE);
        let result = f.client.mint(&user, &shares);

        // Issuer deauthorizes the account directly on the underlying SAC (not via Principal's
        // own Permissioning) after mint but before redeem.
        token::StellarAssetClient::new(&f.env, &f.underlying).set_authorized(&user, &false);
        f.env.ledger().with_mut(|li| li.timestamp = maturity + 1);
        f.client.redeem(&user, &result.pt_minted, &0_i128);
    }
}
