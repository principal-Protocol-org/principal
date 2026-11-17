//! Direct, isolated unit tests for `RecoveryEscrowContract`'s own logic -- fast coverage of this
//! contract's entry points in case `contracts/integration_tests/tests/recovery.rs` (the fuller
//! suite this crate relied on exclusively before) ever changes independently. Not a duplicate of
//! that suite: this focuses on the contract's own control flow (seize/finalize happy path,
//! double-finalize, finalize-before-seize, the batch-size bound, and the Fix 2 concurrent-YT
//! guard), deploying a minimal real stack (oracle, permissioning, underlying SAC, SY, PT, YT,
//! MarketConfig, PrincipalManager, MarketPool) rather than mocks, since `RecoveryEscrow` is a
//! thin orchestrator over those five real contracts and has essentially no logic of its own that
//! doesn't involve them.

use soroban_sdk::{
    testutils::{Address as _, IssuerFlags, Ledger as _, MockAuth, MockAuthInvoke},
    token, Address, BytesN, Env, IntoVal, String, Vec,
};

use principal_manager::{PrincipalManagerContract, PrincipalManagerContractClient};
use principal_market_config::{MarketConfigContract, MarketConfigContractClient};
use principal_market_pool::{MarketPoolContract, MarketPoolContractClient};
use principal_oracle_adapter::{OracleAdapterContract, OracleAdapterContractClient};
use principal_permissioning::{PermissioningContract, PermissioningContractClient};
use principal_pt_token::{PTTokenContract, PTTokenContractClient};
use principal_sy_wrapper::{SYWrapperContract, SYWrapperContractClient};
use principal_yt_token::{YTTokenContract, YTTokenContractClient};

use super::{Error, RecoveryEscrowContract, RecoveryEscrowContractClient, SeizeRequest, MAX_BATCH};

const SCALE: i128 = 10_000_000;
const T0: u64 = 1_000;
const DAY: u64 = 86_400;
const MATURITY: u64 = T0 + 300 * DAY;
const MAX_STALE_SECS: u64 = 365 * DAY;
const MAX_DEVIATION_BPS: u32 = 10_000;

/// The value `try_*` returns for a contract-defined error `e`.
fn expect_err(e: Error) -> Option<Result<soroban_sdk::Error, soroban_sdk::InvokeError>> {
    Some(Ok(soroban_sdk::Error::from_contract_error(e as u32)))
}

#[allow(dead_code)]
struct Fixture {
    env: Env,
    admin: Address,
    underlying: Address,
    oracle: OracleAdapterContractClient<'static>,
    oracle_source: BytesN<32>,
    perm: PermissioningContractClient<'static>,
    sy: SYWrapperContractClient<'static>,
    pt: PTTokenContractClient<'static>,
    yt: YTTokenContractClient<'static>,
    pm: PrincipalManagerContractClient<'static>,
    pool: MarketPoolContractClient<'static>,
    escrow: RecoveryEscrowContractClient<'static>,
}

/// Deploys a minimal real stack with no fees, wires every position's `set_recovery_escrow` and
/// grants every protocol contract the compliance standing it needs -- mirroring (a trimmed-down
/// version of) `contracts/integration_tests/src/stack.rs`. `admin` is the underlying SAC's real
/// admin and every other contract's admin, for simplicity.
fn setup() -> Fixture {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = T0);

    let admin = Address::generate(&env);

    let oracle_id = env.register(OracleAdapterContract, ());
    let oracle = OracleAdapterContractClient::new(&env, &oracle_id);
    let oracle_source = BytesN::from_array(&env, &[1u8; 32]);
    oracle.initialize(&admin, &MAX_STALE_SECS, &MAX_DEVIATION_BPS);
    oracle.set_sources(&admin, &oracle_source, &None);
    oracle.set_reference_value(&admin, &oracle_source, &SCALE, &T0);

    let perm_id = env.register(PermissioningContract, ());
    let perm = PermissioningContractClient::new(&env, &perm_id);
    perm.initialize(&admin);

    let sac = env.register_stellar_asset_contract_v2(admin.clone());
    sac.issuer().set_flag(IssuerFlags::RevocableFlag);
    sac.issuer().set_flag(IssuerFlags::ClawbackEnabledFlag);
    let underlying = sac.address();

    let sy_id = env.register(SYWrapperContract, ());
    let sy = SYWrapperContractClient::new(&env, &sy_id);
    sy.initialize(&admin, &underlying, &perm_id);

    let pt_id = env.register(PTTokenContract, ());
    let pt = PTTokenContractClient::new(&env, &pt_id);
    pt.initialize(
        &admin,
        &perm_id,
        &underlying,
        &MATURITY,
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
        &MATURITY,
        &String::from_str(&env, "Yield Token USDY"),
        &String::from_str(&env, "YT-USDY"),
        &7,
    );

    let config_id = env.register(MarketConfigContract, ());
    let config = MarketConfigContractClient::new(&env, &config_id);
    config.initialize(
        &admin,
        &underlying,
        &MATURITY,
        &admin,
        &admin,
        &0_u32,
        &0_u32,
        &0_u32,
        &2_000_u32,
    );

    let pm_id = env.register(PrincipalManagerContract, ());
    let pm = PrincipalManagerContractClient::new(&env, &pm_id);
    pm.initialize(
        &admin,
        &sy_id,
        &pt_id,
        &yt_id,
        &oracle_id,
        &perm_id,
        &underlying,
        &MATURITY,
        &config_id,
    );
    pt.set_minter(&admin, &pm_id);
    yt.set_minter(&admin, &pm_id);

    let pool_id = env.register(MarketPoolContract, ());
    let pool = MarketPoolContractClient::new(&env, &pool_id);
    pool.initialize(&admin, &pm_id, &4_u32);

    let escrow_id = env.register(RecoveryEscrowContract, ());
    let escrow = RecoveryEscrowContractClient::new(&env, &escrow_id);
    escrow.initialize(
        &admin,
        &underlying,
        &sy_id,
        &pt_id,
        &yt_id,
        &pm_id,
        &pool_id,
    );
    sy.set_recovery_escrow(&admin, &escrow_id);
    pt.set_recovery_escrow(&admin, &escrow_id);
    yt.set_recovery_escrow(&admin, &escrow_id);
    pool.set_recovery_escrow(&admin, &escrow_id);

    // Every protocol contract that ever holds SY/PT/YT needs both compliance layers, same as any
    // other participant.
    for holder in [&sy_id, &pm_id, &pool_id, &escrow_id] {
        perm.grant_account(&admin, holder);
        perm.grant_asset(&admin, holder, &pt_id);
        perm.grant_asset(&admin, holder, &yt_id);
        token::StellarAssetClient::new(&env, &underlying).set_authorized(holder, &true);
    }

    Fixture {
        env,
        admin,
        underlying,
        oracle,
        oracle_source,
        perm,
        sy,
        pt,
        yt,
        pm,
        pool,
        escrow,
    }
}

fn grant_user(f: &Fixture, user: &Address) {
    f.perm.grant_account(&f.admin, user);
    f.perm.grant_asset(&f.admin, user, &f.pt.address);
    f.perm.grant_asset(&f.admin, user, &f.yt.address);
    token::StellarAssetClient::new(&f.env, &f.underlying).set_authorized(user, &true);
}

/// Mints `amount` underlying to `user` and deposits it into SYWrapper.
fn deposit_sy(f: &Fixture, user: &Address, amount: i128) -> i128 {
    token::StellarAssetClient::new(&f.env, &f.underlying).mint(user, &amount);
    f.sy.deposit(user, &amount, &0)
}

// ------------------------------------------------------------------ initialize authorization

/// Regression for a follow-up finding (closes the same root-cause class as C-1/M-12):
/// `initialize` must bind to the live issuer authority's own signature, not just check topology.
/// Builds the real supporting stack under `mock_all_auths()` (every sibling initializer already
/// requires its own admin's signature, so this is the simplest way to stand them up), then
/// disables mocking with `env.set_auths(&[])` -- which, per `set_auths`'s own documentation,
/// turns off auth recording/mocking so a subsequent call only succeeds against an explicit,
/// matching authorization entry -- before testing `escrow.initialize` itself in isolation.
#[test]
fn initialize_requires_the_issuers_own_signature() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = T0);

    let admin = Address::generate(&env);

    let oracle_id = env.register(OracleAdapterContract, ());
    let oracle = OracleAdapterContractClient::new(&env, &oracle_id);
    let oracle_source = BytesN::from_array(&env, &[1u8; 32]);
    oracle.initialize(&admin, &MAX_STALE_SECS, &MAX_DEVIATION_BPS);
    oracle.set_sources(&admin, &oracle_source, &None);
    oracle.set_reference_value(&admin, &oracle_source, &SCALE, &T0);

    let perm_id = env.register(PermissioningContract, ());
    let perm = PermissioningContractClient::new(&env, &perm_id);
    perm.initialize(&admin);

    let sac = env.register_stellar_asset_contract_v2(admin.clone());
    sac.issuer().set_flag(IssuerFlags::RevocableFlag);
    sac.issuer().set_flag(IssuerFlags::ClawbackEnabledFlag);
    let underlying = sac.address();

    let sy_id = env.register(SYWrapperContract, ());
    let sy = SYWrapperContractClient::new(&env, &sy_id);
    sy.initialize(&admin, &underlying, &perm_id);

    let pt_id = env.register(PTTokenContract, ());
    let pt = PTTokenContractClient::new(&env, &pt_id);
    pt.initialize(
        &admin,
        &perm_id,
        &underlying,
        &MATURITY,
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
        &MATURITY,
        &String::from_str(&env, "Yield Token USDY"),
        &String::from_str(&env, "YT-USDY"),
        &7,
    );

    let config_id = env.register(MarketConfigContract, ());
    let config = MarketConfigContractClient::new(&env, &config_id);
    config.initialize(
        &admin,
        &underlying,
        &MATURITY,
        &admin,
        &admin,
        &0_u32,
        &0_u32,
        &0_u32,
        &2_000_u32,
    );

    let pm_id = env.register(PrincipalManagerContract, ());
    let pm = PrincipalManagerContractClient::new(&env, &pm_id);
    pm.initialize(
        &admin,
        &sy_id,
        &pt_id,
        &yt_id,
        &oracle_id,
        &perm_id,
        &underlying,
        &MATURITY,
        &config_id,
    );

    let pool_id = env.register(MarketPoolContract, ());
    let pool = MarketPoolContractClient::new(&env, &pool_id);
    pool.initialize(&admin, &pm_id, &4_u32);

    let escrow_id = env.register(RecoveryEscrowContract, ());
    let escrow = RecoveryEscrowContractClient::new(&env, &escrow_id);

    // Disable mocking: from here on, a call only succeeds against an explicit, matching
    // authorization entry -- exactly what a missing `require_auth()` call would never produce.
    env.set_auths(&[]);
    assert!(escrow
        .try_initialize(
            &admin,
            &underlying,
            &sy_id,
            &pt_id,
            &yt_id,
            &pm_id,
            &pool_id
        )
        .is_err());
    assert!(escrow.try_underlying_address().is_err()); // did not take effect

    // The issuer's own signature, mocked for exactly this one invocation, is enough.
    escrow
        .mock_auths(&[MockAuth {
            address: &admin,
            invoke: &MockAuthInvoke {
                contract: &escrow_id,
                fn_name: "initialize",
                args: (
                    &admin,
                    &underlying,
                    &sy_id,
                    &pt_id,
                    &yt_id,
                    &pm_id,
                    &pool_id,
                )
                    .into_val(&env),
                sub_invokes: &[],
            },
        }])
        .initialize(
            &admin,
            &underlying,
            &sy_id,
            &pt_id,
            &yt_id,
            &pm_id,
            &pool_id,
        );
    assert_eq!(escrow.underlying_address(), underlying);
}

// ------------------------------------------------------------------ seize/finalize happy path

#[test]
fn seize_sy_unwraps_at_once_and_writes_a_record() {
    let f = setup();
    let flagged = Address::generate(&f.env);
    grant_user(&f, &flagged);
    deposit_sy(&f, &flagged, 50 * SCALE);

    token::StellarAssetClient::new(&f.env, &f.underlying).set_authorized(&flagged, &false);

    let unwrapped = f.escrow.seize_sy(&f.admin, &flagged, &(50 * SCALE));
    assert_eq!(unwrapped, 50 * SCALE);
    assert_eq!(f.sy.balance_of(&flagged), 0);

    let ids = f.escrow.account_records(&flagged);
    assert_eq!(ids.len(), 1);
    let rec = f.escrow.get_record(&ids.get(0).unwrap());
    assert_eq!(rec.sy_shares, 50 * SCALE);
    assert_eq!(rec.underlying_from_sy, 50 * SCALE);
    assert!(!rec.finalized);
}

#[test]
fn seize_pt_holds_until_finalize_and_finalize_pays_out_par_at_maturity() {
    let f = setup();
    let flagged = Address::generate(&f.env);
    grant_user(&f, &flagged);
    let shares = deposit_sy(&f, &flagged, 100 * SCALE);
    let minted = f.pm.mint(&flagged, &shares);

    token::StellarAssetClient::new(&f.env, &f.underlying).set_authorized(&flagged, &false);
    f.escrow.seize_pt(&f.admin, &flagged, &minted.pt_minted);

    let ids = f.escrow.account_records(&flagged);
    assert_eq!(ids.len(), 1);
    let id = ids.get(0).unwrap();
    let rec = f.escrow.get_record(&id);
    assert_eq!(rec.pt_amount, minted.pt_minted);
    assert!(!rec.finalized);

    f.env.ledger().with_mut(|li| li.timestamp = MATURITY + 1);
    f.oracle
        .set_reference_value(&f.admin, &f.oracle_source, &SCALE, &(MATURITY + 1));

    let (from_pt, from_yt) = f.escrow.finalize_record(&f.admin, &id);
    assert_eq!(from_pt, minted.pt_minted); // rate unchanged -> par redemption
    assert_eq!(from_yt, 0);

    let rec = f.escrow.get_record(&id);
    assert!(rec.finalized);
    assert_eq!(rec.underlying_from_pt, minted.pt_minted);
}

// ------------------------------------------------------------------------- negative paths

#[test]
fn double_finalize_is_rejected() {
    let f = setup();
    let flagged = Address::generate(&f.env);
    grant_user(&f, &flagged);
    let shares = deposit_sy(&f, &flagged, 100 * SCALE);
    let minted = f.pm.mint(&flagged, &shares);
    token::StellarAssetClient::new(&f.env, &f.underlying).set_authorized(&flagged, &false);
    f.escrow.seize_pt(&f.admin, &flagged, &minted.pt_minted);
    let id = f.escrow.account_records(&flagged).get(0).unwrap();

    f.env.ledger().with_mut(|li| li.timestamp = MATURITY + 1);
    f.oracle
        .set_reference_value(&f.admin, &f.oracle_source, &SCALE, &(MATURITY + 1));
    f.escrow.finalize_record(&f.admin, &id);

    assert_eq!(
        f.escrow.try_finalize_record(&f.admin, &id).err(),
        expect_err(Error::AlreadyFinalized)
    );
}

#[test]
fn finalizing_before_seizing_reverts_record_not_found() {
    let f = setup();
    assert_eq!(
        f.escrow.try_finalize_record(&f.admin, &0_u64).err(),
        expect_err(Error::RecordNotFound)
    );
}

#[test]
fn finalizing_a_pure_sy_record_reverts_nothing_to_finalize() {
    let f = setup();
    let flagged = Address::generate(&f.env);
    grant_user(&f, &flagged);
    deposit_sy(&f, &flagged, 10 * SCALE);
    token::StellarAssetClient::new(&f.env, &f.underlying).set_authorized(&flagged, &false);
    f.escrow.seize_sy(&f.admin, &flagged, &(10 * SCALE));
    let id = f.escrow.account_records(&flagged).get(0).unwrap();

    assert_eq!(
        f.escrow.try_finalize_record(&f.admin, &id).err(),
        expect_err(Error::NothingToFinalize)
    );
}

#[test]
fn seize_batch_rejects_more_than_max_batch_accounts() {
    let f = setup();
    let mut requests: Vec<SeizeRequest> = Vec::new(&f.env);
    for _ in 0..=MAX_BATCH {
        let acct = Address::generate(&f.env);
        grant_user(&f, &acct);
        deposit_sy(&f, &acct, SCALE);
        token::StellarAssetClient::new(&f.env, &f.underlying).set_authorized(&acct, &false);
        requests.push_back(SeizeRequest {
            account: acct,
            sy_shares: SCALE,
            pt_amount: 0,
            yt_amount: 0,
            lp_amount: 0,
        });
    }
    assert_eq!(requests.len(), MAX_BATCH + 1);

    assert_eq!(
        f.escrow.try_seize_batch(&f.admin, &requests).err(),
        expect_err(Error::BatchTooLarge)
    );
}

#[test]
fn seize_batch_at_exactly_max_batch_succeeds() {
    let f = setup();
    let mut requests: Vec<SeizeRequest> = Vec::new(&f.env);
    for _ in 0..MAX_BATCH {
        let acct = Address::generate(&f.env);
        grant_user(&f, &acct);
        deposit_sy(&f, &acct, SCALE);
        token::StellarAssetClient::new(&f.env, &f.underlying).set_authorized(&acct, &false);
        requests.push_back(SeizeRequest {
            account: acct,
            sy_shares: SCALE,
            pt_amount: 0,
            yt_amount: 0,
            lp_amount: 0,
        });
    }
    let ids = f.escrow.seize_batch(&f.admin, &requests);
    assert_eq!(ids.len(), MAX_BATCH);
}

// -------------------------------------------------------- Fix 2: concurrent-YT-record guard

#[test]
fn seize_yt_from_a_second_account_is_rejected_while_the_first_is_unfinalized() {
    let f = setup();
    let a = Address::generate(&f.env);
    let b = Address::generate(&f.env);
    grant_user(&f, &a);
    grant_user(&f, &b);
    let shares_a = deposit_sy(&f, &a, 50 * SCALE);
    let shares_b = deposit_sy(&f, &b, 50 * SCALE);
    let minted_a = f.pm.mint(&a, &shares_a);
    let minted_b = f.pm.mint(&b, &shares_b);

    token::StellarAssetClient::new(&f.env, &f.underlying).set_authorized(&a, &false);
    token::StellarAssetClient::new(&f.env, &f.underlying).set_authorized(&b, &false);

    // A's YT is seized first and left unfinalized.
    f.escrow.seize_yt(&f.admin, &a, &minted_a.yt_minted);

    // B's YT seizure must now be refused -- pooling both would break per-record yield
    // attribution (Fix 2).
    assert_eq!(
        f.escrow
            .try_seize_yt(&f.admin, &b, &minted_b.yt_minted)
            .err(),
        expect_err(Error::UnfinalizedYtRecordExists)
    );

    // Finalize A; B's YT seizure now succeeds.
    let id_a = f.escrow.account_records(&a).get(0).unwrap();
    f.env.ledger().with_mut(|li| li.timestamp = MATURITY + 1);
    f.oracle
        .set_reference_value(&f.admin, &f.oracle_source, &SCALE, &(MATURITY + 1));
    f.escrow.finalize_record(&f.admin, &id_a);

    let seized_b = f.escrow.seize_yt(&f.admin, &b, &minted_b.yt_minted);
    assert_eq!(seized_b, minted_b.yt_minted);
}

#[test]
fn pt_only_seizures_are_unaffected_by_the_yt_guard() {
    // PT has no pooled yield accounting, so the guard must only ever look at `yt_amount`.
    let f = setup();
    let a = Address::generate(&f.env);
    let b = Address::generate(&f.env);
    grant_user(&f, &a);
    grant_user(&f, &b);
    let shares_a = deposit_sy(&f, &a, 50 * SCALE);
    let shares_b = deposit_sy(&f, &b, 50 * SCALE);
    let minted_a = f.pm.mint(&a, &shares_a);
    let minted_b = f.pm.mint(&b, &shares_b);

    token::StellarAssetClient::new(&f.env, &f.underlying).set_authorized(&a, &false);
    token::StellarAssetClient::new(&f.env, &f.underlying).set_authorized(&b, &false);

    // A's YT is outstanding (unfinalized)...
    f.escrow.seize_yt(&f.admin, &a, &minted_a.yt_minted);
    // ...but a *PT*-only seizure from a different account is still allowed.
    let seized_pt = f.escrow.seize_pt(&f.admin, &b, &minted_b.pt_minted);
    assert_eq!(seized_pt, minted_b.pt_minted);
}
