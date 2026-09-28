//! The worked examples in `docs/YIELD_MATH_AND_FEES.md`, executed against the real contracts. If
//! this file passes, every number quoted in that guide is true. When a contract change moves a
//! number, this test fails and the guide must be updated with it.

use principal_integration_tests::stack::*;

/// `actual` within `tol` raw units of `expected`.
fn near(actual: i128, expected: i128, tol: i128, what: &str) {
    assert!(
        (actual - expected).abs() <= tol,
        "{what}: got {actual}, guide says {expected} (tolerance {tol})"
    );
}

#[test]
fn example_1_tokenizing_100_sy_at_rate_1_05() {
    // Fee-free: 100 SY at 1.05 -> 105 PT + 105 YT (notional = shares x rate).
    let s = deploy_stack(T0 + 180 * DAY);
    s.advance(T0 + 1, SCALE * 105 / 100);
    let u = s.new_user();
    let shares = deposit_sy(&s, &u, 100 * SCALE);
    let m = s.pm.mint(&u, &shares);
    assert_eq!(m.pt_minted, 1_050_000_000);
    assert_eq!(m.yt_minted, 1_050_000_000);

    // With the 5 bps tokenization fee: 0.05 SY withheld, 99.95 SY tokenized -> 104.9475.
    let f = deploy(Config::with_example_fees(T0 + 180 * DAY));
    f.advance(T0 + 1, SCALE * 105 / 100);
    let u = f.new_user();
    let shares = deposit_sy(&f, &u, 100 * SCALE);
    let m = f.pm.mint(&u, &shares);
    assert_eq!(m.fee_shares, 500_000); // 0.05 SY
    assert_eq!(m.pt_minted, 1_049_475_000); // 104.9475
    assert_eq!(f.pm.accrued_fees(), (100_000, 400_000)); // 0.01 Principal / 0.04 creator
}

#[test]
fn example_2_pt_plus_yt_always_add_back_to_the_deposit() {
    // Rate 1.05 -> 1.10 over the life of the market, fee-free.
    let s = deploy_stack(T0 + 180 * DAY);
    s.advance(T0 + 1, SCALE * 105 / 100);
    let u = s.new_user();
    let shares = deposit_sy(&s, &u, 100 * SCALE);
    let m = s.pm.mint(&u, &shares);
    s.advance(s.maturity, SCALE * 110 / 100);
    let out = s.pm.redeem(&u, &m.pt_minted, &m.yt_minted);
    // PT: 105 / 1.10 = 95.4545...; YT: 105 x (1/1.05 - 1/1.10) = 4.5454...; total = 100.
    near(out.underlying_from_pt, 954_545_454, 1, "PT leg");
    near(out.underlying_from_yt, 45_454_545, 1, "YT leg");
    near(
        out.underlying_from_pt + out.underlying_from_yt,
        1_000_000_000,
        2,
        "PT + YT",
    );
}

#[test]
fn example_3_claiming_yield_mid_life_with_the_yt_fee() {
    let s = deploy(Config::with_example_fees(T0 + 180 * DAY));
    s.advance(T0 + 1, SCALE * 105 / 100);
    let u = s.new_user();
    let shares = deposit_sy(&s, &u, 100 * SCALE);
    let m = s.pm.mint(&u, &shares);
    assert_eq!(m.yt_minted, 1_049_475_000);
    s.advance(T0 + 90 * DAY, SCALE * 110 / 100);
    // gross = 104.9475 x (1/1.05 - 1/1.10) = 4.5430...; the 10% YT fee leaves 4.0888...
    let net = s.pm.claim_yield(&u);
    near(net, 40_888_636, 20, "net YT claim");
    // The fee (tokenization 0.05 + YT ~0.4543) accrued 20% / 80%.
    let (p, c) = s.pm.accrued_fees();
    near(p, 1_008_636, 10, "protocol fee bucket");
    near(c, 4_034_546, 40, "creator fee bucket");
}

#[test]
fn example_4_the_amm_curve_and_its_time_decay() {
    let s = deploy(Config::with_example_fees(T0 + 180 * DAY));
    let (lp_minted, _lp) = seed_pool(&s, 1_000 * SCALE, 950 * SCALE);
    near(lp_minted, 9_744_356_341, 2, "LP minted"); // sqrt(1000 x 950) - 1000 raw
                                                    // a = 1 - (180/365)/4 = 0.876712...
    near(
        s.pool.time_exponent() / 1_000_000_000_000,
        876_712,
        1,
        "exponent (1e6 scale)",
    );
    // price = 0.95^(a') where a' = tau/stretch = 0.123287  -> 0.993757
    near(s.pool.pt_price(), 9_937_573, 3, "opening PT price");
    near(s.pool.implied_rate(), 127_382, 5, "opening implied rate"); // 1.27% a year
                                                                     // Buying PT with 20 SY: 20.0648 PT, fee 0.0098631 SY (0.1% x 180/365 of 20).
    let q = s.pool.quote_sy_for_pt(&(20 * SCALE));
    near(q.amount_out, 200_648_034, 5, "PT out for 20 SY");
    near(q.fee_shares, 98_631, 2, "swap fee on 20 SY");

    // No trades at all, only time passing: the price climbs to par and the implied rate holds.
    for (day, price) in [
        (0_u64, 9_937_573_i128),
        (60, 9_958_340),
        (90, 9_968_740),
        (120, 9_979_150),
        (150, 9_989_570),
        (179, 9_999_650),
    ] {
        s.advance_flat(T0 + day * DAY + 1);
        near(s.pool.pt_price(), price, 15, &format!("price at day {day}"));
        near(
            s.pool.implied_rate(),
            127_382,
            400,
            &format!("implied rate at day {day}"),
        );
    }
    s.advance_flat(s.maturity - 60);
    assert!(
        s.pool.pt_price() >= 9_999_998,
        "par one minute before maturity"
    );
}

#[test]
fn example_5_the_swap_fee_schedule() {
    let s = deploy(Config::with_example_fees(T0 + 365 * DAY));
    // Fee Tier 0.1% x days-to-maturity / 365, at 1e12 (FEE_SCALE).
    for (days, rate) in [
        (365_u64, 1_000_000_000_i128),
        (180, 493_150_684),
        (90, 246_575_342),
        (30, 82_191_780),
        (7, 19_178_082),
        (1, 2_739_726),
    ] {
        near(
            s.config.swap_fee_rate(&(days * DAY)),
            rate,
            1,
            &format!("fee at {days} days"),
        );
    }
    assert_eq!(s.config.swap_fee_rate(&0), 0);
}

#[test]
fn example_6_flash_mint_and_flash_redeem_yt() {
    let s = deploy(Config::free(T0 + 180 * DAY));
    seed_pool(&s, 1_000 * SCALE, 950 * SCALE);
    let b = s.new_user();
    deposit_sy(&s, &b, 100 * SCALE);
    let (yt, sy_back) = s.router.swap_sy_for_yt(
        &b,
        &s.pool.address,
        &(100 * SCALE),
        &0,
        &(100 * SCALE),
        &u64::MAX,
    );
    assert_eq!(yt, 1_000_000_000); // 100 YT
                                   // The 100 PT sell for 98.13 SY (this trade is 10% of the pool, so it moves the price):
                                   // the YT costs 1.87 SY, i.e. 1.87 cents per YT.
    near(sy_back, 981_284_452, 10, "SY back from the PT");
    near(100 * SCALE - sy_back, 18_715_548, 10, "net cost of 100 YT");
    // Selling the YT straight back through flash-redeem returns that cost (minus rounding).
    let payout = s
        .router
        .swap_yt_for_sy(&b, &s.pool.address, &yt, &0, &u64::MAX);
    near(payout, 18_715_546, 10, "flash-redeem payout");
    assert!(
        s.sy.balance_of(&b) <= 100 * SCALE,
        "never more than the starting 100 SY"
    );
    near(
        s.sy.balance_of(&b),
        1_000_000_000,
        10,
        "round trip returns the capital",
    );
}
