//! RecoveryEscrow — compliance recovery for SY/PT/YT positions, per underlying asset.
//!
//! # Design
//! The native clawback function of a Stellar Asset only applies to balances of the underlying
//! asset itself. It cannot directly reach SY, PT, or YT positions, since those are separate
//! Soroban positions created by Principal Protocol. This contract lets the underlying SAC's
//! real admin (read live via `admin()`, never cached or configured separately) forcibly move a
//! restricted holder's SY/PT/YT balance here, then unwinds it back toward the underlying asset
//! so the issuer can execute their existing native `clawback`.
//!
//! This is the *only* place that authenticates the issuer's admin signature and verifies a
//! target is actually deauthorized (`!underlying_SAC.authorized(account)`) before seizing
//! anything. `SYWrapper`, `PTToken`, and `YTToken` each expose their own `seize`, but none of
//! them re-derive that authority themselves — they simply trust calls from the one
//! `RecoveryEscrow` address configured on each (via their own `set_recovery_escrow`). Keeping
//! that verification in a single, shared place means it isn't duplicated three times, and a bug
//! in this contract can't be worked around by attacking one of the token contracts directly.
//!
//! # No separate owner
//! This contract has no admin key of its own. Every seize function re-checks
//! `underlying_SAC.admin()` live, on every call. If the issuer rotates their SAC admin key, the
//! new key is authoritative here immediately, with nothing to update. The actual security
//! boundary is enforced on the other side: `SYWrapper.set_recovery_escrow` (and the equivalent
//! on `PTToken`/`YTToken`) is itself one-time and admin-gated, so pointing a market's contracts
//! at a rogue escrow requires that market's own real admin to have done so.
//!
//! # SY: seize and unwrap in one step
//! SY has no maturity, so `seize_sy` seizes the balance from `SYWrapper` and immediately calls
//! `SYWrapper.withdraw` (self-directed) to unwrap it into raw underlying, held by this contract
//! and ready for the issuer's native SAC `clawback` — no separate finalize step needed.
//!
//! # PT / YT: seize now, finalize once mature
//! `seize_pt` and `seize_yt` move the flagged account's balance to this contract, exactly like
//! `seize_sy`'s first step. Unlike SY, PT/YT can't be unwound immediately -- they only have
//! value at or after maturity, since that's what `PrincipalManager.redeem` requires. `finalize_pt`
//! and `finalize_yt` complete the unwind once that's possible: they call
//! `PrincipalManager.redeem(from=self, ...)` on this contract's own already-seized balance,
//! which burns it (`PTToken`/`YTToken`'s minter-only `burn`, authorized by `PrincipalManager`'s
//! own direct self-authorization) and pays the resulting underlying here via `SYWrapper.withdraw`
//! (same mechanism `seize_sy` uses directly). No separate deauthorization check is needed at
//! finalize time: the target was already verified deauthorized at `seize_pt`/`seize_yt` time, and
//! finalize only ever acts on this contract's own balance, not a third party's.

#![no_std]

use soroban_sdk::{
    contract, contractclient, contracterror, contractimpl, contracttype, panic_with_error,
    symbol_short, token, Address, Env,
};

#[contractclient(name = "SYWrapperClient")]
pub trait SYWrapperInterface {
    fn underlying_address(env: Env) -> Address;
    fn seize(env: Env, caller: Address, account: Address, shares: i128) -> i128;
    fn withdraw(env: Env, from: Address, shares: i128, to: Address) -> i128;
}

#[contractclient(name = "PTTokenClient")]
pub trait PTTokenInterface {
    fn underlying_address(env: Env) -> Address;
    fn seize(env: Env, caller: Address, account: Address, amount: i128) -> i128;
}

#[contractclient(name = "YTTokenClient")]
pub trait YTTokenInterface {
    fn underlying_address(env: Env) -> Address;
    fn seize(env: Env, caller: Address, account: Address, amount: i128) -> i128;
}

#[contracttype]
#[derive(Clone)]
pub struct RedeemResult {
    pub underlying_from_pt: i128,
    pub underlying_from_yt: i128,
}

#[contractclient(name = "PrincipalManagerClient")]
pub trait PrincipalManagerInterface {
    fn underlying_address(env: Env) -> Address;
    fn redeem(env: Env, from: Address, pt_amount: i128, yt_amount: i128) -> RedeemResult;
}

#[contracterror]
#[derive(Copy, Clone, Debug, PartialEq)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    Unauthorized = 3,
    TargetStillAuthorized = 4,
    ZeroAmount = 5,
    PositionUnderlyingMismatch = 6,
}

#[contracttype]
pub enum DataKey {
    Underlying,
    SYWrapper,
    PTToken,
    YTToken,
    PrincipalManager,
}

#[contract]
pub struct RecoveryEscrowContract;

#[contractimpl]
impl RecoveryEscrowContract {
    pub fn initialize(
        env: Env,
        underlying: Address,
        sy_wrapper: Address,
        pt_token: Address,
        yt_token: Address,
        principal_manager: Address,
    ) {
        if env.storage().instance().has(&DataKey::Underlying) {
            panic_with_error!(&env, Error::AlreadyInitialized);
        }
        Self::assert_position_underlying_matches(
            &env,
            &underlying,
            &sy_wrapper,
            &pt_token,
            &yt_token,
            &principal_manager,
        );
        env.storage()
            .instance()
            .set(&DataKey::Underlying, &underlying);
        env.storage()
            .instance()
            .set(&DataKey::SYWrapper, &sy_wrapper);
        env.storage().instance().set(&DataKey::PTToken, &pt_token);
        env.storage().instance().set(&DataKey::YTToken, &yt_token);
        env.storage()
            .instance()
            .set(&DataKey::PrincipalManager, &principal_manager);
    }

    /// Seize `shares` of `account`'s SY balance and immediately unwrap it into raw underlying,
    /// held by this contract and ready for the issuer's native SAC `clawback`. `caller` must be
    /// the underlying SAC's real admin (checked live); `account` must already be deauthorized
    /// on the SAC.
    pub fn seize_sy(env: Env, caller: Address, account: Address, shares: i128) -> i128 {
        Self::assert_issuer_admin(&env, &caller);
        Self::assert_target_deauthorized(&env, &account);
        if shares <= 0 {
            panic_with_error!(&env, Error::ZeroAmount);
        }

        let sy_wrapper = Self::get(&env, &DataKey::SYWrapper);
        let self_addr = env.current_contract_address();
        SYWrapperClient::new(&env, &sy_wrapper).seize(&self_addr, &account, &shares);
        let unwrapped =
            SYWrapperClient::new(&env, &sy_wrapper).withdraw(&self_addr, &shares, &self_addr);

        env.events().publish(
            (symbol_short!("seize_sy"),),
            (caller, account, shares, unwrapped),
        );
        unwrapped
    }

    /// Seize `amount` of `account`'s PT balance, moving it to this contract. Does not unwind
    /// it further — see module docs for why that step isn't implemented yet.
    pub fn seize_pt(env: Env, caller: Address, account: Address, amount: i128) -> i128 {
        Self::assert_issuer_admin(&env, &caller);
        Self::assert_target_deauthorized(&env, &account);
        if amount <= 0 {
            panic_with_error!(&env, Error::ZeroAmount);
        }

        let pt_token = Self::get(&env, &DataKey::PTToken);
        let self_addr = env.current_contract_address();
        let seized = PTTokenClient::new(&env, &pt_token).seize(&self_addr, &account, &amount);

        env.events()
            .publish((symbol_short!("seize_pt"),), (caller, account, seized));
        seized
    }

    /// Seize `amount` of `account`'s YT balance, moving it to this contract. Does not unwind
    /// it further — see module docs for why that step isn't implemented yet.
    pub fn seize_yt(env: Env, caller: Address, account: Address, amount: i128) -> i128 {
        Self::assert_issuer_admin(&env, &caller);
        Self::assert_target_deauthorized(&env, &account);
        if amount <= 0 {
            panic_with_error!(&env, Error::ZeroAmount);
        }

        let yt_token = Self::get(&env, &DataKey::YTToken);
        let self_addr = env.current_contract_address();
        let seized = YTTokenClient::new(&env, &yt_token).seize(&self_addr, &account, &amount);

        env.events()
            .publish((symbol_short!("seize_yt"),), (caller, account, seized));
        seized
    }

    /// Finalize a previously seized PT position: redeem `pt_amount` of this contract's own PT
    /// balance through `PrincipalManager` (which burns it and pays the resulting underlying
    /// here via `SYWrapper.withdraw`), leaving raw underlying ready for the issuer's native SAC
    /// `clawback`. Only callable at or after maturity, since that's what `PrincipalManager.redeem`
    /// itself requires -- there is no separate maturity check here.
    pub fn finalize_pt(env: Env, caller: Address, pt_amount: i128) -> i128 {
        Self::assert_issuer_admin(&env, &caller);
        if pt_amount <= 0 {
            panic_with_error!(&env, Error::ZeroAmount);
        }

        let principal_manager = Self::get(&env, &DataKey::PrincipalManager);
        let self_addr = env.current_contract_address();
        let result =
            PrincipalManagerClient::new(&env, &principal_manager).redeem(&self_addr, &pt_amount, &0);

        env.events().publish(
            (symbol_short!("fin_pt"),),
            (caller, pt_amount, result.underlying_from_pt),
        );
        result.underlying_from_pt
    }

    /// Finalize a previously seized YT position: redeem `yt_amount` of this contract's own YT
    /// balance through `PrincipalManager`, which settles it against `YTToken`'s own accrual
    /// index (see `PrincipalManager`'s module docs) and pays the resulting underlying here via
    /// `SYWrapper.withdraw`.
    ///
    /// `PrincipalManager.redeem` calls `YTToken.claim_yield` two frames below this call
    /// (RecoveryEscrow -> PrincipalManager -> YTToken). `claim_yield` used to require `from`'s
    /// own authorization, which meant this contract had to explicitly pre-declare that
    /// sub-invocation via `authorize_as_current_contract` (a contract's ordinary
    /// self-authorization only covers calls it makes directly, one frame). Since `claim_yield`
    /// is now minter-gated instead (H-03) -- authorized on `PrincipalManager`'s own address, the
    /// contract that actually calls it directly -- that workaround is no longer needed here.
    pub fn finalize_yt(env: Env, caller: Address, yt_amount: i128) -> i128 {
        Self::assert_issuer_admin(&env, &caller);
        if yt_amount <= 0 {
            panic_with_error!(&env, Error::ZeroAmount);
        }

        let principal_manager = Self::get(&env, &DataKey::PrincipalManager);
        let self_addr = env.current_contract_address();

        let result =
            PrincipalManagerClient::new(&env, &principal_manager).redeem(&self_addr, &0, &yt_amount);

        env.events().publish(
            (symbol_short!("fin_yt"),),
            (caller, yt_amount, result.underlying_from_yt),
        );
        result.underlying_from_yt
    }

    // --- views ---

    pub fn underlying_address(env: Env) -> Address {
        Self::get(&env, &DataKey::Underlying)
    }

    // --- internal helpers ---

    fn get(env: &Env, key: &DataKey) -> Address {
        match key {
            DataKey::Underlying => env
                .storage()
                .instance()
                .get(&DataKey::Underlying)
                .unwrap_or_else(|| panic_with_error!(env, Error::NotInitialized)),
            DataKey::SYWrapper => env
                .storage()
                .instance()
                .get(&DataKey::SYWrapper)
                .unwrap_or_else(|| panic_with_error!(env, Error::NotInitialized)),
            DataKey::PTToken => env
                .storage()
                .instance()
                .get(&DataKey::PTToken)
                .unwrap_or_else(|| panic_with_error!(env, Error::NotInitialized)),
            DataKey::YTToken => env
                .storage()
                .instance()
                .get(&DataKey::YTToken)
                .unwrap_or_else(|| panic_with_error!(env, Error::NotInitialized)),
            DataKey::PrincipalManager => env
                .storage()
                .instance()
                .get(&DataKey::PrincipalManager)
                .unwrap_or_else(|| panic_with_error!(env, Error::NotInitialized)),
        }
    }

    fn assert_issuer_admin(env: &Env, caller: &Address) {
        caller.require_auth();
        let underlying = Self::get(env, &DataKey::Underlying);
        let sac_admin = token::StellarAssetClient::new(env, &underlying).admin();
        if *caller != sac_admin {
            panic_with_error!(env, Error::Unauthorized);
        }
    }

    fn assert_target_deauthorized(env: &Env, account: &Address) {
        let underlying = Self::get(env, &DataKey::Underlying);
        if token::StellarAssetClient::new(env, &underlying).authorized(account) {
            panic_with_error!(env, Error::TargetStillAuthorized);
        }
    }

    fn assert_position_underlying_matches(
        env: &Env,
        underlying: &Address,
        sy_wrapper: &Address,
        pt_token: &Address,
        yt_token: &Address,
        principal_manager: &Address,
    ) {
        let sy_underlying = SYWrapperClient::new(env, sy_wrapper).underlying_address();
        let pt_underlying = PTTokenClient::new(env, pt_token).underlying_address();
        let yt_underlying = YTTokenClient::new(env, yt_token).underlying_address();
        let pm_underlying = PrincipalManagerClient::new(env, principal_manager).underlying_address();

        if sy_underlying != *underlying
            || pt_underlying != *underlying
            || yt_underlying != *underlying
            || pm_underlying != *underlying
        {
            panic_with_error!(env, Error::PositionUnderlyingMismatch);
        }
    }
}

#[cfg(test)]
mod test {
    use soroban_sdk::{
        testutils::{Address as _, IssuerFlags, Ledger as _},
        token, Address, Env, String,
    };

    use principal_manager::{PrincipalManagerContract, PrincipalManagerContractClient};
    use principal_oracle_adapter::{OracleAdapterContract, OracleAdapterContractClient};
    use principal_permissioning::{PermissioningContract, PermissioningContractClient};
    use principal_pt_token::{PTTokenContract, PTTokenContractClient};
    use principal_sy_wrapper::{SYWrapperContract, SYWrapperContractClient};
    use principal_yt_token::{YTTokenContract, YTTokenContractClient};

    use super::{RecoveryEscrowContract, RecoveryEscrowContractClient};

    const T0: u64 = 1_000;
    const SCALE: i128 = 10_000_000;

    struct Fixture {
        env: Env,
        client: RecoveryEscrowContractClient<'static>,
        sac_admin: Address,
        underlying: Address,
        perm: PermissioningContractClient<'static>,
        perm_admin: Address,
        oracle: OracleAdapterContractClient<'static>,
        sy: SYWrapperContractClient<'static>,
        pt: PTTokenContractClient<'static>,
        pt_id: Address,
        yt: YTTokenContractClient<'static>,
        yt_id: Address,
        pm: PrincipalManagerContractClient<'static>,
        pm_id: Address,
        escrow_id: Address,
    }

    fn setup() -> Fixture {
        setup_with_maturity(u64::MAX)
    }

    fn setup_with_maturity(maturity: u64) -> Fixture {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().with_mut(|li| li.timestamp = T0);

        let sac_admin = Address::generate(&env);
        let underlying_sac = env.register_stellar_asset_contract_v2(sac_admin.clone());
        underlying_sac.issuer().set_flag(IssuerFlags::RevocableFlag);
        let underlying = underlying_sac.address();

        let perm_id = env.register_contract(None, PermissioningContract);
        let perm = PermissioningContractClient::new(&env, &perm_id);
        let perm_admin = Address::generate(&env);
        perm.initialize(&perm_admin);

        let oracle_id = env.register_contract(None, OracleAdapterContract);
        let oracle = OracleAdapterContractClient::new(&env, &oracle_id);
        oracle.initialize(&sac_admin);
        oracle.set_reference_value(&sac_admin, &SCALE, &T0);

        let sy_id = env.register_contract(None, SYWrapperContract);
        let sy = SYWrapperContractClient::new(&env, &sy_id);
        sy.initialize(&sac_admin, &underlying, &perm_id);

        let pt_id = env.register_contract(None, PTTokenContract);
        let pt = PTTokenContractClient::new(&env, &pt_id);
        pt.initialize(
            &sac_admin,
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
            &sac_admin,
            &perm_id,
            &underlying,
            &oracle_id,
            &maturity,
            &String::from_str(&env, "Yield Token USDY"),
            &String::from_str(&env, "YT-USDY"),
            &7,
        );

        let pm_id = env.register_contract(None, PrincipalManagerContract);
        let pm = PrincipalManagerContractClient::new(&env, &pm_id);
        pm.initialize(
            &sac_admin, &sy_id, &pt_id, &yt_id, &oracle_id, &perm_id, &underlying, &maturity,
        );
        pt.set_minter(&sac_admin, &pm_id);
        yt.set_minter(&sac_admin, &pm_id);
        // PrincipalManager's own address is a genuine SY holder between mint and redemption.
        perm.grant_account(&perm_admin, &pm_id);
        token::StellarAssetClient::new(&env, &underlying).set_authorized(&pm_id, &true);

        let escrow_id = env.register_contract(None, RecoveryEscrowContract);
        let client = RecoveryEscrowContractClient::new(&env, &escrow_id);
        client.initialize(&underlying, &sy_id, &pt_id, &yt_id, &pm_id);

        // Wire each contract's recovery escrow to this one, and pre-authorize the escrow
        // itself on both compliance layers, exactly as a real market setup would.
        sy.set_recovery_escrow(&sac_admin, &escrow_id);
        pt.set_recovery_escrow(&sac_admin, &escrow_id);
        yt.set_recovery_escrow(&sac_admin, &escrow_id);
        perm.grant_account(&perm_admin, &escrow_id);
        perm.grant_asset(&perm_admin, &escrow_id, &pt_id);
        perm.grant_asset(&perm_admin, &escrow_id, &yt_id);
        token::StellarAssetClient::new(&env, &underlying).set_authorized(&escrow_id, &true);

        Fixture {
            env,
            client,
            sac_admin,
            underlying,
            perm,
            perm_admin,
            oracle,
            sy,
            pt,
            pt_id,
            yt,
            yt_id,
            pm,
            pm_id,
            escrow_id,
        }
    }

    fn grant_sy(f: &Fixture, user: &Address) {
        f.perm.grant_account(&f.perm_admin, user);
        token::StellarAssetClient::new(&f.env, &f.underlying).set_authorized(user, &true);
    }

    fn mint_underlying(f: &Fixture, to: &Address, amount: i128) {
        token::StellarAssetClient::new(&f.env, &f.underlying).mint(to, &amount);
    }

    /// Grants a user for a real PrincipalManager.mint() flow (both PT and YT per-asset grants,
    /// SAC authorization, and enough underlying to deposit and mint with).
    fn grant_and_fund(f: &Fixture, user: &Address, underlying_amount: i128) {
        grant_sy(f, user);
        f.perm.grant_asset(&f.perm_admin, user, &f.pt_id);
        f.perm.grant_asset(&f.perm_admin, user, &f.yt_id);
        mint_underlying(f, user, underlying_amount);
    }

    #[test]
    fn seize_sy_unwraps_immediately_for_native_clawback() {
        let f = setup();
        let bad_actor = Address::generate(&f.env);
        grant_sy(&f, &bad_actor);
        mint_underlying(&f, &bad_actor, 1_000_000_000);
        f.sy.deposit(&bad_actor, &1_000_000_000_i128);

        // Issuer deauthorizes the account directly on the SAC before seizing.
        token::StellarAssetClient::new(&f.env, &f.underlying).set_authorized(&bad_actor, &false);

        let unwrapped = f
            .client
            .seize_sy(&f.sac_admin, &bad_actor, &1_000_000_000_i128);
        assert_eq!(unwrapped, 1_000_000_000);

        // Escrow now holds raw underlying, ready for the issuer's native clawback.
        let underlying_client = token::Client::new(&f.env, &f.underlying);
        assert_eq!(underlying_client.balance(&f.escrow_id), 1_000_000_000);
        assert_eq!(f.sy.balance_of(&bad_actor), 0);
    }

    #[test]
    #[should_panic]
    fn seize_sy_requires_real_issuer_admin() {
        let f = setup();
        let bad_actor = Address::generate(&f.env);
        let impostor = Address::generate(&f.env);
        grant_sy(&f, &bad_actor);
        mint_underlying(&f, &bad_actor, 500_000_000);
        f.sy.deposit(&bad_actor, &500_000_000_i128);
        token::StellarAssetClient::new(&f.env, &f.underlying).set_authorized(&bad_actor, &false);

        f.client.seize_sy(&impostor, &bad_actor, &500_000_000_i128);
    }

    #[test]
    #[should_panic]
    fn seize_sy_requires_target_already_deauthorized() {
        // The issuer's real admin key alone isn't enough -- the target must already be
        // deauthorized on the SAC, so this can't be used as a generic drain.
        let f = setup();
        let still_eligible = Address::generate(&f.env);
        grant_sy(&f, &still_eligible);
        mint_underlying(&f, &still_eligible, 500_000_000);
        f.sy.deposit(&still_eligible, &500_000_000_i128);

        f.client
            .seize_sy(&f.sac_admin, &still_eligible, &500_000_000_i128);
    }

    #[test]
    #[should_panic]
    fn initialize_rejects_position_with_different_underlying() {
        let f = setup();
        let other_admin = Address::generate(&f.env);
        let other_underlying = f
            .env
            .register_stellar_asset_contract_v2(other_admin.clone())
            .address();

        let mismatched_pt_id = f.env.register_contract(None, PTTokenContract);
        let mismatched_pt = PTTokenContractClient::new(&f.env, &mismatched_pt_id);
        mismatched_pt.initialize(
            &other_admin,
            &f.perm.address,
            &other_underlying,
            &u64::MAX,
            &String::from_str(&f.env, "Principal Token BENJI"),
            &String::from_str(&f.env, "PT-BENJI"),
            &7,
        );

        let escrow_id = f.env.register_contract(None, RecoveryEscrowContract);
        let client = RecoveryEscrowContractClient::new(&f.env, &escrow_id);
        client.initialize(
            &f.underlying,
            &f.sy.address,
            &mismatched_pt_id,
            &f.yt_id,
            &f.pm_id,
        );
    }

    #[test]
    #[should_panic]
    fn initialize_rejects_principal_manager_with_different_underlying() {
        let f = setup();
        let other_admin = Address::generate(&f.env);
        let other_underlying = f
            .env
            .register_stellar_asset_contract_v2(other_admin.clone())
            .address();
        let other_oracle = f.env.register_contract(None, OracleAdapterContract);
        OracleAdapterContractClient::new(&f.env, &other_oracle).initialize(&other_admin);

        let mismatched_pm_id = f.env.register_contract(None, PrincipalManagerContract);
        let mismatched_pm = PrincipalManagerContractClient::new(&f.env, &mismatched_pm_id);
        mismatched_pm.initialize(
            &other_admin,
            &f.sy.address,
            &f.pt_id,
            &f.yt_id,
            &other_oracle,
            &f.perm.address,
            &other_underlying,
            &u64::MAX,
        );

        let escrow_id = f.env.register_contract(None, RecoveryEscrowContract);
        let client = RecoveryEscrowContractClient::new(&f.env, &escrow_id);
        client.initialize(
            &f.underlying,
            &f.sy.address,
            &f.pt_id,
            &f.yt_id,
            &mismatched_pm_id,
        );
    }

    #[test]
    fn seize_pt_moves_balance_to_escrow() {
        let f = setup();
        let bad_actor = Address::generate(&f.env);
        grant_and_fund(&f, &bad_actor, 500_000_000);
        let shares = f.sy.deposit(&bad_actor, &500_000_000);
        let result = f.pm.mint(&bad_actor, &shares);

        token::StellarAssetClient::new(&f.env, &f.underlying).set_authorized(&bad_actor, &false);
        let seized = f.client.seize_pt(&f.sac_admin, &bad_actor, &result.pt_minted);
        assert_eq!(seized, result.pt_minted);
        assert_eq!(f.pt.balance(&bad_actor), 0);
        assert_eq!(f.pt.balance(&f.escrow_id), result.pt_minted);
    }

    #[test]
    fn seize_yt_moves_balance_to_escrow() {
        let f = setup();
        let bad_actor = Address::generate(&f.env);
        grant_and_fund(&f, &bad_actor, 500_000_000);
        let shares = f.sy.deposit(&bad_actor, &500_000_000);
        let result = f.pm.mint(&bad_actor, &shares);

        token::StellarAssetClient::new(&f.env, &f.underlying).set_authorized(&bad_actor, &false);
        let seized = f.client.seize_yt(&f.sac_admin, &bad_actor, &result.yt_minted);
        assert_eq!(seized, result.yt_minted);
        assert_eq!(f.yt.balance(&bad_actor), 0);
        assert_eq!(f.yt.balance(&f.escrow_id), result.yt_minted);
    }

    // --- finalize (post-maturity unwind of a seized PT/YT position) ---

    #[test]
    fn finalize_pt_redeems_seized_position_after_maturity() {
        let maturity = T0 + 500;
        let f = setup_with_maturity(maturity);
        let bad_actor = Address::generate(&f.env);
        grant_and_fund(&f, &bad_actor, 1_000_000_000);
        let shares = f.sy.deposit(&bad_actor, &1_000_000_000);
        let result = f.pm.mint(&bad_actor, &shares);

        token::StellarAssetClient::new(&f.env, &f.underlying).set_authorized(&bad_actor, &false);
        let seized = f.client.seize_pt(&f.sac_admin, &bad_actor, &result.pt_minted);

        f.env.ledger().with_mut(|li| li.timestamp = maturity + 1);
        let released = f.client.finalize_pt(&f.sac_admin, &seized);

        // Rate never moved (still SCALE), so PT redeems 1:1 for underlying.
        assert_eq!(released, seized);
        assert_eq!(f.pt.balance(&f.escrow_id), 0);
        let underlying_client = token::Client::new(&f.env, &f.underlying);
        assert_eq!(underlying_client.balance(&f.escrow_id), released);
    }

    #[test]
    fn finalize_yt_redeems_seized_position_after_maturity() {
        let maturity = T0 + 500;
        let f = setup_with_maturity(maturity);
        let bad_actor = Address::generate(&f.env);
        grant_and_fund(&f, &bad_actor, 1_000_000_000);
        let shares = f.sy.deposit(&bad_actor, &1_000_000_000);
        let result = f.pm.mint(&bad_actor, &shares);

        f.env.ledger().with_mut(|li| li.timestamp = maturity + 1);
        f.oracle
            .set_reference_value(&f.sac_admin, &10_300_000_i128, &(maturity + 1));

        token::StellarAssetClient::new(&f.env, &f.underlying).set_authorized(&bad_actor, &false);
        let seized = f.client.seize_yt(&f.sac_admin, &bad_actor, &result.yt_minted);

        let released = f.client.finalize_yt(&f.sac_admin, &seized);
        assert!(released > 0);
        assert_eq!(f.yt.balance(&f.escrow_id), 0);
        let underlying_client = token::Client::new(&f.env, &f.underlying);
        assert_eq!(underlying_client.balance(&f.escrow_id), released);
    }

    #[test]
    #[should_panic]
    fn finalize_pt_requires_real_issuer_admin() {
        let maturity = T0 + 500;
        let f = setup_with_maturity(maturity);
        let bad_actor = Address::generate(&f.env);
        let impostor = Address::generate(&f.env);
        grant_and_fund(&f, &bad_actor, 500_000_000);
        let shares = f.sy.deposit(&bad_actor, &500_000_000);
        let result = f.pm.mint(&bad_actor, &shares);

        token::StellarAssetClient::new(&f.env, &f.underlying).set_authorized(&bad_actor, &false);
        let seized = f.client.seize_pt(&f.sac_admin, &bad_actor, &result.pt_minted);

        f.env.ledger().with_mut(|li| li.timestamp = maturity + 1);
        f.client.finalize_pt(&impostor, &seized);
    }

    #[test]
    #[should_panic]
    fn double_initialize_panics() {
        let f = setup();
        f.client.initialize(
            &f.underlying,
            &f.sy.address,
            &f.pt_id,
            &f.yt_id,
            &f.pm_id,
        );
    }

    #[test]
    #[should_panic]
    fn seize_sy_rejects_zero_shares() {
        let f = setup();
        let bad_actor = Address::generate(&f.env);
        grant_and_fund(&f, &bad_actor, 500_000_000);
        f.sy.deposit(&bad_actor, &500_000_000);
        token::StellarAssetClient::new(&f.env, &f.underlying).set_authorized(&bad_actor, &false);
        f.client.seize_sy(&f.sac_admin, &bad_actor, &0_i128);
    }

    #[test]
    #[should_panic]
    fn seize_pt_rejects_zero_amount() {
        let f = setup();
        let bad_actor = Address::generate(&f.env);
        grant_and_fund(&f, &bad_actor, 500_000_000);
        let shares = f.sy.deposit(&bad_actor, &500_000_000);
        f.pm.mint(&bad_actor, &shares);
        token::StellarAssetClient::new(&f.env, &f.underlying).set_authorized(&bad_actor, &false);
        f.client.seize_pt(&f.sac_admin, &bad_actor, &0_i128);
    }

    #[test]
    #[should_panic]
    fn seize_yt_rejects_zero_amount() {
        let f = setup();
        let bad_actor = Address::generate(&f.env);
        grant_and_fund(&f, &bad_actor, 500_000_000);
        let shares = f.sy.deposit(&bad_actor, &500_000_000);
        f.pm.mint(&bad_actor, &shares);
        token::StellarAssetClient::new(&f.env, &f.underlying).set_authorized(&bad_actor, &false);
        f.client.seize_yt(&f.sac_admin, &bad_actor, &0_i128);
    }

    #[test]
    #[should_panic]
    fn finalize_pt_rejects_zero_amount() {
        let maturity = T0 + 500;
        let f = setup_with_maturity(maturity);
        f.env.ledger().with_mut(|li| li.timestamp = maturity + 1);
        f.client.finalize_pt(&f.sac_admin, &0_i128);
    }

    #[test]
    #[should_panic]
    fn finalize_yt_rejects_zero_amount() {
        let maturity = T0 + 500;
        let f = setup_with_maturity(maturity);
        f.env.ledger().with_mut(|li| li.timestamp = maturity + 1);
        f.client.finalize_yt(&f.sac_admin, &0_i128);
    }
}
