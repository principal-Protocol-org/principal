use soroban_sdk::{
    testutils::{Address as _, IssuerFlags, Ledger as _},
    token, Address, Env, String,
};

use principal_market_config::{MarketConfigContract, MarketConfigContractClient};
use principal_oracle_adapter::{OracleAdapterContract, OracleAdapterContractClient};
use principal_permissioning::{PermissioningContract, PermissioningContractClient};
use principal_pt_token::{PTTokenContract, PTTokenContractClient};
use principal_sy_wrapper::{SYWrapperContract, SYWrapperContractClient};
use principal_yt_token::{YTTokenContract, YTTokenContractClient};

use super::{
    Error, PrincipalManagerContract, PrincipalManagerContractClient, MAX_ORACLE_STALENESS_SECS,
    SCALE,
};

/// The value `try_*` returns for a contract-defined error `e`.
fn expect_err(e: Error) -> Option<Result<soroban_sdk::Error, soroban_sdk::InvokeError>> {
    Some(Ok(soroban_sdk::Error::from_contract_error(e as u32)))
}

/// Base ledger timestamp (> 0 so the oracle can accept its first update).
const T0: u64 = 1_000;

/// All contracts deployed into the same Env, returned together so tests can
/// create addresses, advance ledger time, and update the oracle/mint SY after setup.
#[allow(dead_code)]
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
    config: MarketConfigContractClient<'static>,
    protocol_admin: Address,
    treasury: Address,
}

/// Deploy the full contract set (oracle, permissioning, an underlying SAC, SYWrapper,
/// PTToken, YTToken, PrincipalManager) into a single Env, and wire PrincipalManager as the
/// registered minter on both token contracts -- mirroring the real two-phase deployment
/// order in DEPLOYMENT.md. Oracle rate is seeded at SCALE (1.0) at ledger timestamp T0.
/// `pm_admin` is also the underlying SAC's real admin, satisfying the market-creation gate
/// on every contract. No users are pre-granted -- tests call `grant_user` explicitly.
fn setup(maturity: u64) -> TestFixture {
    setup_with_fees(maturity, 0, 0)
}

/// Same as `setup` but with a configured tokenization fee and YT fee (bps). Protocol share
/// is the documented 20%.
fn setup_with_fees(maturity: u64, tokenization_fee_bps: u32, yt_fee_bps: u32) -> TestFixture {
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

    let protocol_admin = Address::generate(&env);
    let treasury = Address::generate(&env);
    let config_id = env.register(MarketConfigContract, ());
    let config = MarketConfigContractClient::new(&env, &config_id);
    config.initialize(
        &pm_admin,
        &underlying,
        &maturity,
        &protocol_admin,
        &treasury,
        &tokenization_fee_bps,
        &yt_fee_bps,
        &10,
        &2_000,
    );

    let pm_id = env.register_contract(None, PrincipalManagerContract);
    let client = PrincipalManagerContractClient::new(&env, &pm_id);
    client.initialize(
        &pm_admin,
        &sy_id,
        &pt_id,
        &yt_id,
        &oracle_id,
        &perm_id,
        &underlying,
        &maturity,
        &config_id,
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
        config,
        protocol_admin,
        treasury,
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
    f.sy.deposit(user, &amount, &0)
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
    assert_eq!(
        token::Client::new(&f.env, &f.underlying).balance(&user),
        expected
    );
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
    // The YT index is the closed form 1/rate, so a single move from 1.0 to `final_rate` pays
    // exactly yt * (final_rate - SCALE) / final_rate underlying, floored (the index rounds up
    // by at most 1e-12, worth well under one unit here).
    let expected = yt * (final_rate - SCALE) / final_rate;
    assert_eq!(r.underlying_from_yt, expected);
    assert_eq!(r.underlying_from_pt, 0);
}

#[test]
fn redeem_yt_does_not_double_pay_yield_already_claimed_via_claim_yield() {
    // PrincipalManager.claim_yield lets a holder collect accrued yield ahead of redemption,
    // paying real underlying (H-03). If a user claims this way and then redeems, redeem()
    // must not pay out that same accrued amount a second time -- both paths settle against
    // the same YTToken index, so whichever runs second sees nothing left pending.
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

    // User claims ahead of redemption; this must actually pay out real underlying.
    let direct_claim = f.client.claim_yield(&user);
    assert!(direct_claim > 0);
    assert_eq!(
        token::Client::new(&f.env, &f.underlying).balance(&user),
        direct_claim
    );

    // Redemption must not pay the same yield again.
    let r = f.client.redeem(&user, &0_i128, &yt);
    assert_eq!(r.underlying_from_yt, 0);
}

#[test]
#[should_panic]
fn claim_yield_cannot_be_called_directly_on_yt_token() {
    // H-03 regression: the old direct-claim footgun the audit flagged must be closed --
    // YTToken.claim_yield is now minter-gated, so a user (not PrincipalManager) calling it
    // directly must fail, not silently zero their pending claim with no payment.
    let maturity = T0 + 500;
    let f = setup(maturity);
    let user = Address::generate(&f.env);
    grant_user(&f, &user);
    let shares = deposit_sy(&f, &user, 100_i128 * SCALE);
    f.client.mint(&user, &shares);

    f.env.ledger().with_mut(|li| li.timestamp = maturity + 1);
    f.oracle
        .set_reference_value(&f.oracle_admin, &10_300_000_i128, &(maturity + 1));

    f.yt.update_yield_index();
    f.yt.claim_yield(&user, &user);
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
fn oracle_stale_blocks_mint() {
    // H-02 regression: mint() must enforce the same freshness gate redeem() already has.
    let f = setup(u64::MAX);
    let user = Address::generate(&f.env);
    grant_user(&f, &user);
    let shares = deposit_sy(&f, &user, 10_i128 * SCALE);

    f.env
        .ledger()
        .with_mut(|li| li.timestamp = T0 + MAX_ORACLE_STALENESS_SECS + 1);
    f.client.mint(&user, &shares);
}

#[test]
fn late_minter_does_not_receive_prior_yield() {
    // H-01 regression (PrincipalManager side): mint() must bring the YT yield index
    // current BEFORE crediting a new mint's balance. Otherwise a rate movement that
    // happened before a user's mint would sit unrealized in the global factor and get
    // credited to that user as soon as anyone eventually calls update_yield_index(),
    // double-counting the same oracle move already priced into their notional at mint.
    let f = setup(u64::MAX);

    let early_user = Address::generate(&f.env);
    grant_user(&f, &early_user);
    let early_shares = deposit_sy(&f, &early_user, 10_i128 * SCALE);
    f.client.mint(&early_user, &early_shares);

    // Rate moves up before the second user ever mints.
    f.env.ledger().with_mut(|li| li.timestamp = T0 + 100);
    f.oracle
        .set_reference_value(&f.oracle_admin, &(SCALE * 11 / 10), &(T0 + 100));

    let late_user = Address::generate(&f.env);
    grant_user(&f, &late_user);
    let late_shares = deposit_sy(&f, &late_user, 10_i128 * SCALE);
    f.client.mint(&late_user, &late_shares);

    // The late minter must not be able to claim any of the yield generated by the rate
    // movement that happened before they ever held YT.
    assert_eq!(f.client.claim_yield(&late_user), 0);
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
    let config_id = Address::generate(&env);
    let result = client.try_initialize(
        &impostor,
        &sy_wrapper,
        &pt_token,
        &yt_token,
        &oracle_id,
        &perm_id,
        &underlying,
        &u64::MAX,
        &config_id,
    );
    assert_eq!(result.err(), expect_err(Error::IssuerMismatch));
}

/// Deploys oracle + permissioning + an underlying SAC (admined by a freshly generated
/// address) shared by every test in the M-01 topology-mismatch group below, so each test
/// only needs to construct the one deliberately-mismatched piece itself.
struct TopologyBase {
    env: Env,
    admin: Address,
    underlying: Address,
    oracle_id: Address,
    perm_id: Address,
}

/// A real MarketConfig for `maturity`, so the PrincipalManager under test only fails on the
/// one deliberately mismatched piece.
fn topology_config(b: &TopologyBase, maturity: &u64) -> Address {
    let config_id = b.env.register(MarketConfigContract, ());
    let treasury = Address::generate(&b.env);
    MarketConfigContractClient::new(&b.env, &config_id).initialize(
        &b.admin,
        &b.underlying,
        maturity,
        &b.admin,
        &treasury,
        &0,
        &0,
        &0,
        &2_000,
    );
    config_id
}

fn topology_base() -> TopologyBase {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = T0);

    let oracle_id = env.register_contract(None, OracleAdapterContract);
    let oracle = OracleAdapterContractClient::new(&env, &oracle_id);
    let admin = Address::generate(&env);
    oracle.initialize(&admin);
    oracle.set_reference_value(&admin, &SCALE, &T0);

    let perm_id = env.register_contract(None, PermissioningContract);
    PermissioningContractClient::new(&env, &perm_id).initialize(&admin);

    let underlying = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();

    TopologyBase {
        env,
        admin,
        underlying,
        oracle_id,
        perm_id,
    }
}

#[test]
fn initialize_rejects_mismatched_underlying() {
    // M-01 regression: PTToken deployed against a different underlying SAC than the one
    // this market's PrincipalManager is configured with.
    let b = topology_base();
    let other_underlying = b
        .env
        .register_stellar_asset_contract_v2(b.admin.clone())
        .address();

    let sy_id = b.env.register_contract(None, SYWrapperContract);
    SYWrapperContractClient::new(&b.env, &sy_id).initialize(&b.admin, &b.underlying, &b.perm_id);

    let pt_id = b.env.register_contract(None, PTTokenContract);
    PTTokenContractClient::new(&b.env, &pt_id).initialize(
        &b.admin,
        &b.perm_id,
        &other_underlying, // mismatched
        &u64::MAX,
        &String::from_str(&b.env, "Principal Token USDY"),
        &String::from_str(&b.env, "PT-USDY"),
        &7,
    );

    let yt_id = b.env.register_contract(None, YTTokenContract);
    YTTokenContractClient::new(&b.env, &yt_id).initialize(
        &b.admin,
        &b.perm_id,
        &b.underlying,
        &b.oracle_id,
        &u64::MAX,
        &String::from_str(&b.env, "Yield Token USDY"),
        &String::from_str(&b.env, "YT-USDY"),
        &7,
    );

    let pm_id = b.env.register_contract(None, PrincipalManagerContract);
    let config_id = topology_config(&b, &u64::MAX);
    let result = PrincipalManagerContractClient::new(&b.env, &pm_id).try_initialize(
        &b.admin,
        &sy_id,
        &pt_id,
        &yt_id,
        &b.oracle_id,
        &b.perm_id,
        &b.underlying,
        &u64::MAX,
        &config_id,
    );
    assert_eq!(result.err(), expect_err(Error::TopologyMismatch));
}

#[test]
fn initialize_rejects_mismatched_permissioning() {
    // M-01 regression: YTToken deployed against a different permissioning contract than
    // the one this market's PrincipalManager is configured with.
    let b = topology_base();
    let other_perm_id = b.env.register_contract(None, PermissioningContract);
    PermissioningContractClient::new(&b.env, &other_perm_id).initialize(&b.admin);

    let sy_id = b.env.register_contract(None, SYWrapperContract);
    SYWrapperContractClient::new(&b.env, &sy_id).initialize(&b.admin, &b.underlying, &b.perm_id);

    let pt_id = b.env.register_contract(None, PTTokenContract);
    PTTokenContractClient::new(&b.env, &pt_id).initialize(
        &b.admin,
        &b.perm_id,
        &b.underlying,
        &u64::MAX,
        &String::from_str(&b.env, "Principal Token USDY"),
        &String::from_str(&b.env, "PT-USDY"),
        &7,
    );

    let yt_id = b.env.register_contract(None, YTTokenContract);
    YTTokenContractClient::new(&b.env, &yt_id).initialize(
        &b.admin,
        &other_perm_id, // mismatched
        &b.underlying,
        &b.oracle_id,
        &u64::MAX,
        &String::from_str(&b.env, "Yield Token USDY"),
        &String::from_str(&b.env, "YT-USDY"),
        &7,
    );

    let pm_id = b.env.register_contract(None, PrincipalManagerContract);
    let config_id = topology_config(&b, &u64::MAX);
    let result = PrincipalManagerContractClient::new(&b.env, &pm_id).try_initialize(
        &b.admin,
        &sy_id,
        &pt_id,
        &yt_id,
        &b.oracle_id,
        &b.perm_id,
        &b.underlying,
        &u64::MAX,
        &config_id,
    );
    assert_eq!(result.err(), expect_err(Error::TopologyMismatch));
}

#[test]
fn initialize_rejects_mismatched_maturity() {
    // M-01 regression: YTToken deployed with a different maturity than the one passed to
    // PrincipalManager.initialize (PTToken's maturity matches; YTToken's doesn't).
    let b = topology_base();
    let maturity = T0 + 500;

    let sy_id = b.env.register_contract(None, SYWrapperContract);
    SYWrapperContractClient::new(&b.env, &sy_id).initialize(&b.admin, &b.underlying, &b.perm_id);

    let pt_id = b.env.register_contract(None, PTTokenContract);
    PTTokenContractClient::new(&b.env, &pt_id).initialize(
        &b.admin,
        &b.perm_id,
        &b.underlying,
        &maturity,
        &String::from_str(&b.env, "Principal Token USDY"),
        &String::from_str(&b.env, "PT-USDY"),
        &7,
    );

    let yt_id = b.env.register_contract(None, YTTokenContract);
    YTTokenContractClient::new(&b.env, &yt_id).initialize(
        &b.admin,
        &b.perm_id,
        &b.underlying,
        &b.oracle_id,
        &(maturity + 1), // mismatched
        &String::from_str(&b.env, "Yield Token USDY"),
        &String::from_str(&b.env, "YT-USDY"),
        &7,
    );

    let pm_id = b.env.register_contract(None, PrincipalManagerContract);
    let config_id = topology_config(&b, &maturity);
    let result = PrincipalManagerContractClient::new(&b.env, &pm_id).try_initialize(
        &b.admin,
        &sy_id,
        &pt_id,
        &yt_id,
        &b.oracle_id,
        &b.perm_id,
        &b.underlying,
        &maturity,
        &config_id,
    );
    assert_eq!(result.err(), expect_err(Error::TopologyMismatch));
}

#[test]
fn initialize_rejects_mismatched_oracle() {
    // M-01 regression: YTToken deployed against a different oracle than the one passed to
    // PrincipalManager.initialize.
    let b = topology_base();
    let other_oracle_id = b.env.register_contract(None, OracleAdapterContract);
    let other_oracle = OracleAdapterContractClient::new(&b.env, &other_oracle_id);
    other_oracle.initialize(&b.admin);
    other_oracle.set_reference_value(&b.admin, &SCALE, &T0);

    let sy_id = b.env.register_contract(None, SYWrapperContract);
    SYWrapperContractClient::new(&b.env, &sy_id).initialize(&b.admin, &b.underlying, &b.perm_id);

    let pt_id = b.env.register_contract(None, PTTokenContract);
    PTTokenContractClient::new(&b.env, &pt_id).initialize(
        &b.admin,
        &b.perm_id,
        &b.underlying,
        &u64::MAX,
        &String::from_str(&b.env, "Principal Token USDY"),
        &String::from_str(&b.env, "PT-USDY"),
        &7,
    );

    let yt_id = b.env.register_contract(None, YTTokenContract);
    YTTokenContractClient::new(&b.env, &yt_id).initialize(
        &b.admin,
        &b.perm_id,
        &b.underlying,
        &other_oracle_id, // mismatched
        &u64::MAX,
        &String::from_str(&b.env, "Yield Token USDY"),
        &String::from_str(&b.env, "YT-USDY"),
        &7,
    );

    let pm_id = b.env.register_contract(None, PrincipalManagerContract);
    let config_id = topology_config(&b, &u64::MAX);
    let result = PrincipalManagerContractClient::new(&b.env, &pm_id).try_initialize(
        &b.admin,
        &sy_id,
        &pt_id,
        &yt_id,
        &b.oracle_id,
        &b.perm_id,
        &b.underlying,
        &u64::MAX,
        &config_id,
    );
    assert_eq!(result.err(), expect_err(Error::TopologyMismatch));
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

#[test]
#[should_panic]
fn double_initialize_panics() {
    let f = setup(u64::MAX);
    f.client.initialize(
        &f.pm_admin,
        &f.sy.address,
        &f.pt.address,
        &f.yt.address,
        &f.oracle.address,
        &f.perm.address,
        &f.underlying,
        &u64::MAX,
        &f.config.address,
    );
}

#[test]
#[should_panic]
fn mint_zero_shares_panics() {
    let f = setup(u64::MAX);
    let user = Address::generate(&f.env);
    grant_user(&f, &user);
    f.client.mint(&user, &0_i128);
}

#[test]
#[should_panic]
fn redeem_zero_amounts_panics() {
    let maturity = T0 + 500;
    let f = setup(maturity);
    let user = Address::generate(&f.env);
    grant_user(&f, &user);
    let shares = deposit_sy(&f, &user, 10_i128 * SCALE);
    f.client.mint(&user, &shares);

    f.env.ledger().with_mut(|li| li.timestamp = maturity + 1);
    f.client.redeem(&user, &0_i128, &0_i128);
}

#[test]
#[should_panic]
fn non_admin_cannot_transfer_admin() {
    let f = setup(u64::MAX);
    let impostor = Address::generate(&f.env);
    let new_admin = Address::generate(&f.env);
    f.client.transfer_admin(&impostor, &new_admin);
}
