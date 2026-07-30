//! Shared full-stack deployment helper for cross-contract integration tests. Deploys all eight
//! contracts into a single `Env`, wires minters and recovery-escrow registrations, and grants the
//! `PrincipalManager` contract's own address both compliance layers -- mirroring the real
//! two-phase deployment order documented in DEPLOYMENT.md.

use soroban_sdk::{
    testutils::{Address as _, IssuerFlags, Ledger as _},
    token, Address, Env, String,
};

use principal_manager::{PrincipalManagerContract, PrincipalManagerContractClient, SCALE};
use principal_oracle_adapter::{OracleAdapterContract, OracleAdapterContractClient};
use principal_permissioning::{PermissioningContract, PermissioningContractClient};
use principal_pt_token::{PTTokenContract, PTTokenContractClient};
use principal_recovery_escrow::{RecoveryEscrowContract, RecoveryEscrowContractClient};
use principal_risk_control::{RiskControlContract, RiskControlContractClient};
use principal_sy_wrapper::{SYWrapperContract, SYWrapperContractClient};
use principal_yt_token::{YTTokenContract, YTTokenContractClient};

/// Base ledger timestamp (> 0 so the oracle can accept its first update).
pub const T0: u64 = 1_000;

/// Every deployed contract in one `Env`, wired together the way a real deployment would be.
/// `admin` is the underlying SAC's real, live admin -- also used as the oracle/permissioning/
/// risk-control admin for simplicity, since nothing here requires those to be different keys.
/// Not every field is read by every test file that includes this module -- each `tests/*.rs`
/// file is its own compiled crate, so a field only exercised by a sibling file still needs to
/// exist here for the ones that do use it.
#[allow(dead_code)]
pub struct Stack<'a> {
    pub env: Env,
    pub admin: Address,
    pub underlying: Address,
    pub oracle: OracleAdapterContractClient<'a>,
    pub perm: PermissioningContractClient<'a>,
    pub risk: RiskControlContractClient<'a>,
    pub sy: SYWrapperContractClient<'a>,
    pub pt: PTTokenContractClient<'a>,
    pub yt: YTTokenContractClient<'a>,
    pub pm: PrincipalManagerContractClient<'a>,
    pub escrow: RecoveryEscrowContractClient<'a>,
}

/// Deploys the full stack with `maturity` as the PT/YT maturity timestamp. The oracle is seeded
/// at `SCALE` (1.0) at ledger time `T0`; the circuit breaker starts disabled (`cb_limit = 0`) --
/// tests that need it active call `risk.set_cb_limit` themselves.
pub fn deploy_stack(maturity: u64) -> Stack<'static> {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = T0);

    let admin = Address::generate(&env);

    let oracle_id = env.register(OracleAdapterContract, ());
    let oracle = OracleAdapterContractClient::new(&env, &oracle_id);
    oracle.initialize(&admin);
    oracle.set_reference_value(&admin, &SCALE, &T0);

    let perm_id = env.register(PermissioningContract, ());
    let perm = PermissioningContractClient::new(&env, &perm_id);
    perm.initialize(&admin);

    let risk_id = env.register(RiskControlContract, ());
    let risk = RiskControlContractClient::new(&env, &risk_id);
    risk.initialize(&admin, &0_i128);

    let underlying_sac = env.register_stellar_asset_contract_v2(admin.clone());
    underlying_sac.issuer().set_flag(IssuerFlags::RevocableFlag);
    let underlying = underlying_sac.address();

    let sy_id = env.register(SYWrapperContract, ());
    let sy = SYWrapperContractClient::new(&env, &sy_id);
    sy.initialize(&admin, &underlying, &perm_id);

    let pt_id = env.register(PTTokenContract, ());
    let pt = PTTokenContractClient::new(&env, &pt_id);
    pt.initialize(
        &admin,
        &perm_id,
        &underlying,
        &maturity,
        &String::from_str(&env, "Principal Token USDY"),
        &String::from_str(&env, "PT-USDY"),
        &7,
    );

    let yt_id = env.register(YTTokenContract, ());
    let yt = YTTokenContractClient::new(&env, &yt_id);
    yt.initialize(
        &admin,
        &perm_id,
        &underlying,
        &oracle_id,
        &maturity,
        &String::from_str(&env, "Yield Token USDY"),
        &String::from_str(&env, "YT-USDY"),
        &7,
    );

    let pm_id = env.register(PrincipalManagerContract, ());
    let pm = PrincipalManagerContractClient::new(&env, &pm_id);
    pm.initialize(
        &admin, &sy_id, &pt_id, &yt_id, &oracle_id, &perm_id, &underlying, &maturity,
    );

    pt.set_minter(&admin, &pm_id);
    yt.set_minter(&admin, &pm_id);

    // PrincipalManager's own address is a genuine SY holder between mint and redemption, and
    // both sender/recipient on its own SYWrapper.transfer/withdraw calls.
    perm.grant_account(&admin, &pm_id);
    token::StellarAssetClient::new(&env, &underlying).set_authorized(&pm_id, &true);

    let escrow_id = env.register(RecoveryEscrowContract, ());
    let escrow = RecoveryEscrowContractClient::new(&env, &escrow_id);
    escrow.initialize(&underlying, &sy_id, &pt_id, &yt_id, &pm_id);

    sy.set_recovery_escrow(&admin, &escrow_id);
    pt.set_recovery_escrow(&admin, &escrow_id);
    yt.set_recovery_escrow(&admin, &escrow_id);

    // RecoveryEscrow ends up holding SY/PT/YT itself (seize moves balances to it, and
    // seize_sy/finalize_pt/finalize_yt unwind via SYWrapper.withdraw / PrincipalManager.redeem,
    // both of which check the caller's own compliance) -- it needs the same two layers any
    // other participant does.
    perm.grant_account(&admin, &escrow_id);
    token::StellarAssetClient::new(&env, &underlying).set_authorized(&escrow_id, &true);

    Stack {
        env,
        admin,
        underlying,
        oracle,
        perm,
        risk,
        sy,
        pt,
        yt,
        pm,
        escrow,
    }
}

/// Grants `user` both compliance layers (account + per-asset Permissioning, and SAC
/// authorization) needed to hold/move SY, PT, and YT.
#[allow(dead_code)]
pub fn grant_user(s: &Stack, user: &Address) {
    s.perm.grant_account(&s.admin, user);
    s.perm.grant_asset(&s.admin, user, &s.pt.address);
    s.perm.grant_asset(&s.admin, user, &s.yt.address);
    token::StellarAssetClient::new(&s.env, &s.underlying).set_authorized(user, &true);
}

/// Mints `amount` of the underlying to `user` and deposits it into SYWrapper, returning the SY
/// shares received (1:1 at inception). `user` must already be granted.
pub fn deposit_sy(s: &Stack, user: &Address, amount: i128) -> i128 {
    token::StellarAssetClient::new(&s.env, &s.underlying).mint(user, &amount);
    s.sy.deposit(user, &amount)
}
