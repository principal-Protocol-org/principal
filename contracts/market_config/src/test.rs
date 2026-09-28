use soroban_sdk::{
    testutils::{Address as _, IssuerFlags},
    token, Address, Env,
};

use super::*;

struct F {
    env: Env,
    client: MarketConfigContractClient<'static>,
    issuer: Address,
    underlying: Address,
    protocol_admin: Address,
    treasury: Address,
}

fn setup() -> F {
    let env = Env::default();
    env.mock_all_auths();
    let issuer = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(issuer.clone());
    sac.issuer().set_flag(IssuerFlags::RevocableFlag);
    let protocol_admin = Address::generate(&env);
    let treasury = Address::generate(&env);
    let client = MarketConfigContractClient::new(&env, &env.register(MarketConfigContract, ()));
    client.initialize(
        &issuer,
        &sac.address(),
        &1_000_000,
        &protocol_admin,
        &treasury,
        &5,
        &1_000,
        &10,
        &2_000,
    );
    F {
        env,
        client,
        issuer,
        underlying: sac.address(),
        protocol_admin,
        treasury,
    }
}

#[test]
fn initialize_stores_the_documented_example_parameters() {
    let f = setup();
    assert_eq!(f.client.underlying(), f.underlying);
    assert_eq!(f.client.maturity(), 1_000_000);
    assert_eq!(f.client.tokenization_fee_bps(), 5);
    assert_eq!(f.client.yt_fee_bps(), 1_000); // 10%
    assert_eq!(f.client.swap_fee_tier_bps(), 10); // 0.1%
    assert_eq!(f.client.protocol_share_bps(), 2_000); // 20%
    assert_eq!(f.client.protocol_admin(), f.protocol_admin);
    assert_eq!(f.client.treasury(), f.treasury);
    assert_eq!(f.client.creator(), f.issuer);
}

#[test]
fn initialize_requires_the_real_issuer_and_runs_once() {
    let f = setup();
    assert!(f
        .client
        .try_initialize(
            &f.issuer,
            &f.underlying,
            &1,
            &f.protocol_admin,
            &f.treasury,
            &0,
            &0,
            &0,
            &0
        )
        .is_err());

    // A stranger cannot stand up a market configuration on someone else's asset.
    let stranger = Address::generate(&f.env);
    let fresh = MarketConfigContractClient::new(&f.env, &f.env.register(MarketConfigContract, ()));
    assert!(fresh
        .try_initialize(
            &stranger,
            &f.underlying,
            &1,
            &f.protocol_admin,
            &f.treasury,
            &0,
            &0,
            &0,
            &0
        )
        .is_err());
}

#[test]
fn initialize_enforces_fee_caps_and_share_bound() {
    let f = setup();
    let mk = || MarketConfigContractClient::new(&f.env, &f.env.register(MarketConfigContract, ()));
    let a = |c: &MarketConfigContractClient, tok, yt, tier, share| {
        c.try_initialize(
            &f.issuer,
            &f.underlying,
            &1,
            &f.protocol_admin,
            &f.treasury,
            &tok,
            &yt,
            &tier,
            &share,
        )
    };
    assert!(a(&mk(), MAX_TOKENIZATION_FEE_BPS + 1, 0, 0, 0).is_err());
    assert!(a(&mk(), 0, MAX_YT_FEE_BPS + 1, 0, 0).is_err());
    assert!(a(&mk(), 0, 0, MAX_SWAP_FEE_TIER_BPS + 1, 0).is_err());
    assert!(a(&mk(), 0, 0, 0, 10_001).is_err());
    // Boundary values are accepted.
    assert!(a(
        &mk(),
        MAX_TOKENIZATION_FEE_BPS,
        MAX_YT_FEE_BPS,
        MAX_SWAP_FEE_TIER_BPS,
        10_000
    )
    .is_ok());
}

#[test]
fn issuer_reconfigures_fees_within_caps() {
    let f = setup();
    f.client.set_fees(&f.issuer, &10, &500, &20);
    assert_eq!(f.client.tokenization_fee_bps(), 10);
    assert_eq!(f.client.yt_fee_bps(), 500);
    assert_eq!(f.client.swap_fee_tier_bps(), 20);
    assert!(f
        .client
        .try_set_fees(&f.issuer, &(MAX_TOKENIZATION_FEE_BPS + 1), &0, &0)
        .is_err());
}

#[test]
fn only_the_live_sac_admin_may_set_fees() {
    let f = setup();
    let stranger = Address::generate(&f.env);
    assert!(f.client.try_set_fees(&stranger, &1, &1, &1).is_err());
    // Neither the protocol admin nor Principal can touch market fees.
    assert!(f
        .client
        .try_set_fees(&f.protocol_admin, &1, &1, &1)
        .is_err());

    // After the issuer rotates the SAC admin, authority moves immediately with nothing to
    // update here: the old key is dead, the new one is live.
    token::StellarAssetClient::new(&f.env, &f.underlying).set_admin(&stranger);
    assert_eq!(f.client.creator(), stranger);
    assert!(f.client.try_set_fees(&f.issuer, &1, &1, &1).is_err());
    f.client.set_fees(&stranger, &1, &1, &1);
}

#[test]
fn protocol_share_is_protocol_admin_only_and_bounded() {
    let f = setup();
    f.client.set_protocol_share(&f.protocol_admin, &3_500);
    assert_eq!(f.client.protocol_share_bps(), 3_500);
    assert!(f
        .client
        .try_set_protocol_share(&f.protocol_admin, &10_001)
        .is_err());
    // The market creator cannot zero out Principal's cut.
    assert!(f.client.try_set_protocol_share(&f.issuer, &0).is_err());
}

#[test]
fn treasury_and_protocol_admin_rotation() {
    let f = setup();
    let new_treasury = Address::generate(&f.env);
    let new_admin = Address::generate(&f.env);
    f.client.set_treasury(&f.protocol_admin, &new_treasury);
    assert_eq!(f.client.treasury(), new_treasury);
    assert!(f.client.try_set_treasury(&f.issuer, &new_treasury).is_err());
    f.client
        .transfer_protocol_admin(&f.protocol_admin, &new_admin);
    assert_eq!(f.client.protocol_admin(), new_admin);
    assert!(f
        .client
        .try_set_protocol_share(&f.protocol_admin, &1)
        .is_err());
    f.client.set_protocol_share(&new_admin, &1);
}

#[test]
fn fee_split_is_twenty_eighty_and_conserves_the_total() {
    let f = setup();
    assert_eq!(f.client.split(&1_000_000), (200_000, 800_000));
    assert_eq!(f.client.split(&0), (0, 0));
    // Rounding dust goes to the creator: 7 * 20% = 1.4 -> protocol 1, creator 6.
    assert_eq!(f.client.split(&7), (1, 6));
    f.client.set_protocol_share(&f.protocol_admin, &0);
    assert_eq!(f.client.split(&123), (0, 123));
    f.client.set_protocol_share(&f.protocol_admin, &10_000);
    assert_eq!(f.client.split(&123), (123, 0));
}

#[test]
fn swap_fee_decays_linearly_to_zero_at_maturity() {
    let f = setup(); // tier = 10 bps = 0.1%
    let day = 86_400u64;
    // 365 days out: exactly the Fee Tier, 0.1% = 1e9 at FEE_SCALE.
    assert_eq!(f.client.swap_fee_rate(&(365 * day)), 1_000_000_000);
    // 182.5 days out: half.
    assert_eq!(f.client.swap_fee_rate(&(365 * day / 2)), 500_000_000);
    // 90 days: 0.1% * 90/365 = 0.024657534% -> 246_575_342 at FEE_SCALE (floored).
    assert_eq!(f.client.swap_fee_rate(&(90 * day)), 246_575_342);
    // Maturity: zero.
    assert_eq!(f.client.swap_fee_rate(&0), 0);
    // Monotone in time to maturity.
    assert!(f.client.swap_fee_rate(&(60 * day)) < f.client.swap_fee_rate(&(61 * day)));
}

#[test]
fn swap_fee_is_capped() {
    let f = setup();
    f.client.set_fees(&f.issuer, &0, &0, &MAX_SWAP_FEE_TIER_BPS);
    // 5% tier over 20 years would be 100%: capped at 10%.
    assert_eq!(
        f.client.swap_fee_rate(&(20 * 365 * 86_400)),
        MAX_SWAP_FEE_RATE
    );
}

#[test]
fn uninitialized_views_revert() {
    let env = Env::default();
    let c = MarketConfigContractClient::new(&env, &env.register(MarketConfigContract, ()));
    assert!(c.try_underlying().is_err());
    assert!(c.try_split(&1).is_err());
}

// ---- SEP-57 (RWA token) underlying: no `admin()`, so the creator payee is explicit ----

mod rwa {
    use super::*;
    use principal_compliance::mock_rwa;

    #[test]
    fn rwa_market_config_uses_the_operator_as_authority_and_an_explicit_creator_payee() {
        let env = Env::default();
        env.mock_all_auths();
        let operator = Address::generate(&env);
        let (token, _verifier) = mock_rwa::deploy(&env, &operator);
        let protocol_admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        let client = MarketConfigContractClient::new(&env, &env.register(MarketConfigContract, ()));

        // A stranger who is not an operator cannot create the config.
        let stranger = Address::generate(&env);
        assert!(client
            .try_initialize(
                &stranger,
                &token.address,
                &1_000,
                &protocol_admin,
                &treasury,
                &5,
                &1_000,
                &10,
                &2_000
            )
            .is_err());

        client.initialize(
            &operator,
            &token.address,
            &1_000,
            &protocol_admin,
            &treasury,
            &5,
            &1_000,
            &10,
            &2_000,
        );
        // With no `admin()` to read live, the creator payee starts as the operator that created it...
        assert_eq!(client.creator(), operator);
        // ...and the operator can repoint it; a stranger cannot.
        let payee = Address::generate(&env);
        client.set_creator_payee(&operator, &payee);
        assert_eq!(client.creator(), payee);
        assert!(client.try_set_creator_payee(&stranger, &payee).is_err());
        // Fee retuning is gated on the same operator capability.
        client.set_fees(&operator, &1, &1, &1);
        assert!(client.try_set_fees(&stranger, &1, &1, &1).is_err());
    }

    #[test]
    fn a_sac_market_has_no_settable_creator_payee() {
        let f = setup();
        let payee = Address::generate(&f.env);
        assert_eq!(
            f.client.try_set_creator_payee(&f.issuer, &payee).err(),
            Some(Ok(soroban_sdk::Error::from_contract_error(
                Error::NotApplicable as u32
            )))
        );
    }
}
