//! Functional assertions for the PT/SY yield-curve AMM: the time-decay curve, PT convergence to
//! par at maturity, swap/fee/LP mechanics, and the flash-mint and flash-redeem YT paths.

use soroban_sdk::{testutils::Address as _, Address};

use principal_integration_tests::stack::*;
use principal_market_pool::Error as PoolError;

const WAD: f64 = 1e18;

/// A 180-day market, seeded with 1_000 PT against SY worth 950 (PT opens near a 5% discount).
fn seeded(cfg: Config) -> (Stack<'static>, Address) {
    let s = deploy(cfg);
    let (_, lp) = seed_pool(&s, 1_000 * SCALE, 950 * SCALE);
    (s, lp)
}

fn price(s: &Stack) -> f64 {
    s.pool.pt_price() as f64 / SCALE as f64
}

#[test]
fn first_deposit_sets_the_opening_price_and_locks_minimum_liquidity() {
    let s = deploy(Config::free(T0 + 180 * DAY));
    let (lp_minted, lp) = seed_pool(&s, 1_000 * SCALE, 950 * SCALE);

    let (pt_res, sy_res) = s.pool.reserves();
    assert_eq!(pt_res, 1_000 * SCALE);
    assert_eq!(sy_res, 950 * SCALE);
    // isqrt(1000e7 * 950e7) - MINIMUM_LIQUIDITY, with the minimum locked to the pool itself.
    let root = 9_746_794_344_i128; // floor(sqrt(1e10 * 9.5e9))
    assert_eq!(lp_minted, root - 1_000);
    assert_eq!(s.pool.lp_balance(&lp), root - 1_000);
    assert_eq!(s.pool.lp_balance(&s.pool.address), 1_000);
    assert_eq!(s.pool.lp_total_supply(), root);

    // Opening price: (950/1000)^(tau/stretch) = 0.95^(180/1460) ~ 0.9937 -- a discount that is
    // already smaller than the raw 5% ratio because the curve is time-stretched.
    let p = price(&s);
    let expected = 0.95_f64.powf(180.0 / (4.0 * 365.0));
    assert!((p - expected).abs() < 1e-6, "price {p} expected {expected}");
}

#[test]
fn first_deposit_must_not_price_pt_above_par() {
    let s = deploy(Config::free(T0 + 180 * DAY));
    let lp = s.new_user();
    let shares = deposit_sy(&s, &lp, 3_000 * SCALE);
    let minted = s.pm.mint(&lp, &(1_000 * SCALE));
    assert_eq!(shares, 3_000 * SCALE);
    // 1_001 SY of value against 1_000 PT would price PT above par.
    let r = s
        .pool
        .try_add_liquidity(&lp, &minted.pt_minted, &(1_001 * SCALE), &0);
    assert_eq!(r.err(), err_code(PoolError::InvalidInitialRatio as u32));
    // Exactly par is allowed.
    s.pool
        .add_liquidity(&lp, &minted.pt_minted, &(1_000 * SCALE), &0);
}

#[test]
fn time_decay_curve_pt_price_rises_monotonically_toward_par_with_no_trades() {
    let (s, _) = seeded(Config::free(T0 + 180 * DAY));
    let mut last = 0.0;
    let mut last_exp = 0_i128;
    // Sample the untraded pool every 15 days out to the day before maturity.
    for d in (0..180).step_by(15) {
        s.advance_flat(T0 + d as u64 * DAY + 1);
        let p = price(&s);
        let a = s.pool.time_exponent();
        assert!(
            p > last,
            "price must strictly rise: day {d} price {p} <= {last}"
        );
        assert!(a > last_exp, "exponent must rise toward 1 as time passes");
        assert!(
            p < 1.0,
            "PT stays at a discount before maturity: day {d} price {p}"
        );
        // The closed form: price = (x/y)^(tau/stretch) with x/y = 0.95.
        let tau_years = (180.0 - d as f64) / 365.0;
        let expected = 0.95_f64.powf(tau_years / 4.0);
        assert!((p - expected).abs() < 2e-6, "day {d}: {p} vs {expected}");
        last = p;
        last_exp = a;
    }
    // The reserves never moved: the curve slid under the LPs on its own.
    assert_eq!(s.pool.reserves(), (1_000 * SCALE, 950 * SCALE));
}

#[test]
fn pt_converges_to_par_at_maturity() {
    let maturity = T0 + 180 * DAY;
    let (s, lp) = seeded(Config::free(maturity));

    // One minute before maturity the price is par to within 1e-6...
    s.advance_flat(maturity - 60);
    let p = price(&s);
    assert!(
        (1.0 - p) < 1e-6,
        "PT price {p} should be ~1 one minute before maturity"
    );
    // ...the exponent is ~1 (constant-sum) and the implied rate has decayed to ~0.
    let a = s.pool.time_exponent() as f64 / WAD;
    assert!(a > 0.999_999, "exponent {a}");

    // A trade there is at par: buying 10 PT costs 10 SY to within a few units.
    let q = s.pool.quote_sy_for_pt(&(10 * SCALE));
    assert!(
        abs_diff(q.amount_out, 10 * SCALE) < 100,
        "10 SY should buy ~10 PT at par, got {}",
        q.amount_out
    );

    // At maturity itself the AMM is closed...
    s.advance_flat(maturity);
    assert_eq!(s.pool.time_exponent(), 1_000_000_000_000_000_000);
    let buyer = s.new_user();
    deposit_sy(&s, &buyer, 100 * SCALE);
    let r = s.pool.try_swap_sy_for_pt(&buyer, &buyer, &SCALE, &0);
    assert_eq!(r.err(), err_code(PoolError::Expired as u32));

    // ...LPs can still exit, and PT redeems at par: 1 PT -> 1 underlying (rate 1.0), independent
    // of any pool price.
    let lp_all = s.pool.lp_balance(&lp);
    let (pt_back, sy_back) = s.pool.remove_liquidity(&lp, &lp, &lp_all, &0, &0);
    assert!(pt_back > 999 * SCALE && sy_back > 949 * SCALE);
    let out = s.pm.redeem(&lp, &pt_back, &0);
    assert_eq!(
        out.underlying_from_pt, pt_back,
        "PT redeems at par (rate 1.0)"
    );
}

/// The curve invariant k = x^a + y^a from the pool's own reserves, in f64 (test-only reference).
fn invariant_k(s: &Stack) -> f64 {
    let (pt, sy) = s.pool.reserves();
    let rate = s.oracle.get_reference_value();
    let x = (sy * rate / SCALE) as f64;
    let a = s.pool.time_exponent() as f64 / WAD;
    x.powf(a) + (pt as f64).powf(a)
}

fn pt_bal(s: &Stack, who: &Address) -> i128 {
    s.pt.balance(who)
}

// ---------------------------------------------------------------- swaps

#[test]
fn buying_pt_moves_the_price_up_and_pays_the_quoted_amount() {
    let (s, _) = seeded(Config::free(T0 + 180 * DAY));
    let trader = s.new_user();
    deposit_sy(&s, &trader, 100 * SCALE);

    let p0 = price(&s);
    let quote = s.pool.quote_sy_for_pt(&(20 * SCALE));
    let out = s
        .pool
        .swap_sy_for_pt(&trader, &trader, &(20 * SCALE), &quote.amount_out);
    assert_eq!(out, quote.amount_out);
    assert_eq!(pt_bal(&s, &trader), out);
    assert_eq!(s.sy.balance_of(&trader), 80 * SCALE);
    // Buying PT with SY makes PT more expensive.
    assert!(price(&s) > p0);
    // 20 SY buys slightly more than 20 PT because PT starts at a discount (avg price ~0.996).
    assert!(
        out > 20 * SCALE && out < 20 * SCALE + SCALE / 2,
        "out {out}"
    );
}

#[test]
fn selling_pt_moves_the_price_down_and_pays_the_quoted_amount() {
    let (s, lp) = seeded(Config::free(T0 + 180 * DAY));
    let p0 = price(&s);
    // The LP provider holds PT? No: they deposited it all. Mint fresh PT for a seller.
    let seller = s.new_user();
    let shares = deposit_sy(&s, &seller, 100 * SCALE);
    s.pm.mint(&seller, &shares);
    let _ = lp;

    let quote = s.pool.quote_pt_for_sy(&(40 * SCALE));
    let out = s
        .pool
        .swap_pt_for_sy(&seller, &seller, &(40 * SCALE), &quote.amount_out);
    assert_eq!(out, quote.amount_out);
    assert_eq!(pt_bal(&s, &seller), 60 * SCALE);
    assert_eq!(s.sy.balance_of(&seller), out);
    assert!(price(&s) < p0);
    // Selling 40 PT at a discount returns less than 40 SY.
    assert!(out < 40 * SCALE && out > 35 * SCALE, "out {out}");
}

#[test]
fn round_trip_never_profits_and_costs_only_price_impact_when_fee_free() {
    let (s, _) = seeded(Config::free(T0 + 180 * DAY));
    let trader = s.new_user();
    deposit_sy(&s, &trader, 100 * SCALE);
    let pt = s.pool.swap_sy_for_pt(&trader, &trader, &(20 * SCALE), &0);
    let sy_back = s.pool.swap_pt_for_sy(&trader, &trader, &pt, &0);
    // Pool-favoring rounding: never more than we put in, and within 1e-6 of it.
    assert!(sy_back <= 20 * SCALE, "round trip profited: {sy_back}");
    assert!(
        20 * SCALE - sy_back < 200,
        "round trip lost too much: {}",
        20 * SCALE - sy_back
    );
}

#[test]
fn invariant_never_decreases_across_a_pseudo_random_swap_sequence() {
    let (s, _) = seeded(Config::free(T0 + 180 * DAY));
    let trader = s.new_user();
    deposit_sy(&s, &trader, 400 * SCALE);
    let shares = deposit_sy(&s, &trader, 400 * SCALE);
    s.pm.mint(&trader, &shares);

    let mut k_prev = invariant_k(&s);
    // A fixed linear-congruential sequence keeps the test deterministic.
    let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
    for i in 0..40 {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let amt = ((seed >> 40) as i128 % 20 + 1) * SCALE / 2; // 0.5 .. 10 tokens
        if i % 2 == 0 {
            s.pool.swap_sy_for_pt(&trader, &trader, &amt, &0);
        } else {
            s.pool.swap_pt_for_sy(&trader, &trader, &amt, &0);
        }
        let k = invariant_k(&s);
        // Rounding is always in the pool's favor: k may only grow (allow 1e-12 relative noise).
        assert!(
            k >= k_prev * (1.0 - 1e-12),
            "step {i}: k fell {k_prev} -> {k}"
        );
        k_prev = k;
    }
}

#[test]
fn pool_token_balances_always_match_internal_reserves_plus_fees() {
    let (s, _) = seeded(Config::with_example_fees(T0 + 180 * DAY));
    let trader = s.new_user();
    let shares = deposit_sy(&s, &trader, 300 * SCALE);
    s.pm.mint(&trader, &(100 * SCALE));
    let _ = shares;
    s.pool.swap_sy_for_pt(&trader, &trader, &(30 * SCALE), &0);
    s.pool.swap_pt_for_sy(&trader, &trader, &(25 * SCALE), &0);

    let (pt_res, sy_res) = s.pool.reserves();
    let (fp, fc) = s.pool.accrued_fees();
    assert_eq!(s.pt.balance(&s.pool.address), pt_res);
    assert_eq!(s.sy.balance_of(&s.pool.address), sy_res + fp + fc);
}

#[test]
fn sweeping_the_oracle_rate_keeps_the_curve_in_value_units() {
    // A comparable protocol once mixed raw SY shares with asset units in its curve, which leaked
    // value once the SY rate moved off 1.0. Sweep the rate and require identical economics.
    for &(num, den) in &[(100_i128, 100_i128), (101, 100), (105, 100), (110, 100)] {
        let maturity = T0 + 180 * DAY;
        let s = deploy(Config::free(maturity));
        let rate = SCALE * num / den;
        s.advance(T0 + 1, rate);

        // Seed so that the pool's SY *value* is 95% of its PT, whatever the rate.
        let lp = s.new_user();
        let sy_for_pool = 950 * SCALE * SCALE / rate; // shares worth 950 notional
        let shares_for_pt = 1_000 * SCALE * SCALE / rate; // shares that tokenize into 1000 PT
        deposit_sy(&s, &lp, shares_for_pt + sy_for_pool);
        let minted = s.pm.mint(&lp, &shares_for_pt);
        assert!(abs_diff(minted.pt_minted, 1_000 * SCALE) < 2);
        s.pool
            .add_liquidity(&lp, &minted.pt_minted, &sy_for_pool, &0);

        // Opening price is identical at every rate: it depends on value, not on share count.
        let p = price(&s);
        let expected = 0.95_f64.powf(180.0 / (4.0 * 365.0));
        assert!(
            (p - expected).abs() < 1e-4,
            "rate {num}/{den}: price {p} vs {expected}"
        );

        // Round trip stays fee-free-neutral: never profitable.
        let trader = s.new_user();
        let sy = deposit_sy(&s, &trader, 60 * SCALE);
        let pt = s.pool.swap_sy_for_pt(&trader, &trader, &sy, &0);
        let back = s.pool.swap_pt_for_sy(&trader, &trader, &pt, &0);
        assert!(
            back <= sy,
            "rate {num}/{den}: round trip profited {back} > {sy}"
        );
        assert!(
            sy - back < sy / 100_000 + 10,
            "rate {num}/{den}: lost {}",
            sy - back
        );

        // Convergence at non-unit rate: a minute before maturity PT is at par *in value*, i.e.
        // 1 PT is worth 1 notional = SCALE/rate shares.
        s.advance(maturity - 60, rate);
        assert!((1.0 - price(&s)) < 1e-6, "rate {num}/{den}: not converged");
        let q = s.pool.quote_pt_for_sy(&(10 * SCALE));
        let value = q.amount_out * rate / SCALE;
        assert!(
            abs_diff(value, 10 * SCALE) < 1_000,
            "rate {num}/{den}: 10 PT sold for {value} value at par"
        );
    }
}

#[test]
fn slippage_guards_revert_and_exact_minimum_passes() {
    let (s, _) = seeded(Config::free(T0 + 180 * DAY));
    let trader = s.new_user();
    deposit_sy(&s, &trader, 100 * SCALE);
    let q = s.pool.quote_sy_for_pt(&(10 * SCALE));

    let r = s
        .pool
        .try_swap_sy_for_pt(&trader, &trader, &(10 * SCALE), &(q.amount_out + 1));
    assert_eq!(r.err(), err_code(PoolError::SlippageExceeded as u32));
    // The exact quoted minimum is accepted (boundary).
    assert_eq!(
        s.pool
            .swap_sy_for_pt(&trader, &trader, &(10 * SCALE), &q.amount_out),
        q.amount_out
    );
}

#[test]
fn swap_input_validation_and_liquidity_bounds() {
    let (s, _) = seeded(Config::free(T0 + 180 * DAY));
    let trader = s.new_user();
    deposit_sy(&s, &trader, 100 * SCALE);
    assert_eq!(
        s.pool.try_swap_sy_for_pt(&trader, &trader, &0, &0).err(),
        err_code(PoolError::ZeroAmount as u32)
    );
    assert_eq!(
        s.pool.try_swap_pt_for_sy(&trader, &trader, &0, &0).err(),
        err_code(PoolError::ZeroAmount as u32)
    );
    // Buying more PT than the pool can supply cannot be quoted.
    assert_eq!(
        s.pool.try_quote_buy_exact_pt(&(1_000 * SCALE)).err(),
        err_code(PoolError::InsufficientLiquidity as u32)
    );
    // Selling an absurd amount of PT drains more SY than exists.
    assert!(s
        .pool
        .try_quote_pt_for_sy(&(1_000_000_000 * SCALE))
        .is_err());
}

#[test]
fn an_unseeded_pool_refuses_to_trade_and_quote() {
    let s = deploy(Config::free(T0 + 180 * DAY));
    let trader = s.new_user();
    deposit_sy(&s, &trader, 10 * SCALE);
    assert_eq!(
        s.pool
            .try_swap_sy_for_pt(&trader, &trader, &SCALE, &0)
            .err(),
        err_code(PoolError::InsufficientLiquidity as u32)
    );
    assert!(s.pool.try_pt_price().is_err());
}

// ---------------------------------------------------------------- fees

#[test]
fn swap_fee_follows_fee_tier_times_days_to_maturity_over_365() {
    // Tier 10 bps (0.1%). Sell the same PT at 300 and at 30 days out and compare fees.
    let maturity = T0 + 300 * DAY;
    let (s, _) = seeded(Config::with_example_fees(maturity));
    let q300 = s.pool.quote_sy_for_pt(&(100 * SCALE));
    // fee = 100e7 * 0.001 * 300/365 = 821_917.8 (rounded up to whole units).
    let exact = 100.0 * 1e7 * 0.001 * 300.0 / 365.0;
    assert!(
        (q300.fee_shares as f64 - exact).abs() <= 2.0,
        "fee {} vs formula {exact}",
        q300.fee_shares
    );

    s.advance_flat(maturity - 30 * DAY);
    let q30 = s.pool.quote_sy_for_pt(&(100 * SCALE));
    let exact30 = 100.0 * 1e7 * 0.001 * 30.0 / 365.0;
    assert!(
        (q30.fee_shares as f64 - exact30).abs() <= 2.0,
        "{} vs {exact30}",
        q30.fee_shares
    );
    assert!(
        q30.fee_shares * 9 < q300.fee_shares,
        "fee must fall with time to maturity"
    );

    // Zero fee at (just before) maturity.
    s.advance_flat(maturity - 1);
    assert!(s.pool.quote_sy_for_pt(&(100 * SCALE)).fee_shares <= 1);
}

#[test]
fn swap_fees_split_twenty_eighty_and_are_claimable_by_treasury_and_creator() {
    let (s, _) = seeded(Config::with_example_fees(T0 + 300 * DAY));
    let trader = s.new_user();
    deposit_sy(&s, &trader, 400 * SCALE);
    for _ in 0..4 {
        s.pool.swap_sy_for_pt(&trader, &trader, &(50 * SCALE), &0);
    }
    let (protocol, creator) = s.pool.accrued_fees();
    let total = protocol + creator;
    assert!(total > 0);
    // 20% / 80%. Each of the 4 swaps splits its own fee with floor rounding (dust to the
    // creator), so the protocol bucket is within 4 units of exactly 20% of the total.
    let exact_protocol = total * 2_000 / 10_000;
    assert!(exact_protocol - protocol >= 0 && exact_protocol - protocol <= 4);
    assert!(creator > protocol * 3, "creator holds ~80%");

    let treasury_before = s.sy.balance_of(&s.treasury);
    let creator_before = s.sy.balance_of(&s.admin);
    assert_eq!(s.pool.claim_protocol_fees(), protocol);
    assert_eq!(s.pool.claim_creator_fees(), creator);
    assert_eq!(s.sy.balance_of(&s.treasury), treasury_before + protocol);
    assert_eq!(s.sy.balance_of(&s.admin), creator_before + creator);
    assert_eq!(s.pool.accrued_fees(), (0, 0));

    // Nothing left to claim.
    assert_eq!(
        s.pool.try_claim_protocol_fees().err(),
        err_code(PoolError::NothingToClaim as u32)
    );
}

#[test]
fn creator_fee_share_follows_a_sac_admin_rotation() {
    let (s, _) = seeded(Config::with_example_fees(T0 + 300 * DAY));
    let trader = s.new_user();
    deposit_sy(&s, &trader, 100 * SCALE);
    s.pool.swap_sy_for_pt(&trader, &trader, &(50 * SCALE), &0);

    // The issuer hands the SAC to a new admin; the creator share follows it live.
    let new_admin = Address::generate(&s.env);
    grant_user(&s, &new_admin);
    s.sac().set_admin(&new_admin);
    let (_, creator) = s.pool.accrued_fees();
    s.pool.claim_creator_fees();
    assert_eq!(s.sy.balance_of(&new_admin), creator);
}

// ------------------------------------------------------- flash-mint YT

#[test]
fn flash_mint_buys_yt_by_minting_and_selling_the_pt_in_one_transaction() {
    let (s, _) = seeded(Config::free(T0 + 180 * DAY));
    let buyer = s.new_user();
    deposit_sy(&s, &buyer, 100 * SCALE);

    let pt_supply_before = s.pm.total_pt();
    let quote = s.pool.quote_pt_for_sy(&(100 * SCALE)); // what the minted PT will fetch
    let (yt_out, sy_back) = s.router.swap_sy_for_yt(
        &buyer,
        &s.pool.address,
        &(100 * SCALE),
        &(100 * SCALE),
        &(100 * SCALE),
        &u64::MAX,
    );

    // 100 SY tokenized -> 100 YT. The 100 PT were sold into the pool for SY.
    assert_eq!(yt_out, 100 * SCALE);
    assert_eq!(s.yt.balance(&buyer), 100 * SCALE);
    assert_eq!(pt_bal(&s, &buyer), 0, "the buyer keeps no PT");
    assert_eq!(sy_back, quote.amount_out);
    assert_eq!(s.sy.balance_of(&buyer), sy_back);

    // Net cost of 100 YT = 100 - sy_back SY; YT is priced at ~ 1 - PT price.
    let net_cost = 100 * SCALE - sy_back;
    let yt_price = net_cost as f64 / yt_out as f64;
    assert!(yt_price > 0.0 && yt_price < 0.1, "YT price {yt_price}");
    // PT and YT supplies grew together: tokenization stays balanced.
    assert_eq!(s.pm.total_pt() - pt_supply_before, 100 * SCALE);
    assert_eq!(s.pm.total_yt() - (pt_supply_before), 100 * SCALE);
}

#[test]
fn flash_mint_enforces_min_yt_out_and_the_deadline() {
    let (s, _) = seeded(Config::free(T0 + 180 * DAY));
    let buyer = s.new_user();
    deposit_sy(&s, &buyer, 100 * SCALE);
    let pool = s.pool.address.clone();
    assert!(s
        .router
        .try_swap_sy_for_yt(
            &buyer,
            &pool,
            &(10 * SCALE),
            &(10 * SCALE + 1),
            &(10 * SCALE),
            &u64::MAX
        )
        .is_err());
    // Deadline in the past.
    assert!(s
        .router
        .try_swap_sy_for_yt(&buyer, &pool, &(10 * SCALE), &0, &(10 * SCALE), &(T0 - 1))
        .is_err());
    // Exactly at the deadline is still valid.
    s.router.swap_sy_for_yt(
        &buyer,
        &pool,
        &(10 * SCALE),
        &(10 * SCALE),
        &(10 * SCALE),
        &T0,
    );
}

// ----------------------------------------------------- flash-redeem YT

#[test]
fn flash_redeem_sells_yt_for_sy_by_recombining_with_pool_pt() {
    let (s, _) = seeded(Config::free(T0 + 180 * DAY));
    let holder = s.new_user();
    deposit_sy(&s, &holder, 100 * SCALE);
    let pool = s.pool.address.clone();
    let (yt, sy_after_buy) = s.router.swap_sy_for_yt(
        &holder,
        &pool,
        &(100 * SCALE),
        &0,
        &(100 * SCALE),
        &u64::MAX,
    );
    assert_eq!(yt, 100 * SCALE);

    let (pt_res0, sy_res0) = s.pool.reserves();
    let quote = s.pool.quote_buy_exact_pt(&yt);
    let pt_supply = s.pm.total_pt();

    let payout = s.router.swap_yt_for_sy(&holder, &pool, &yt, &0, &u64::MAX);

    // The holder's YT is gone and they were paid the recombined SY minus the PT's price.
    assert_eq!(s.yt.balance(&holder), 0);
    assert_eq!(s.sy.balance_of(&holder), sy_after_buy + payout);
    // Recombining 100 YT + 100 PT returns 100 SY (rate 1.0): payout = 100 - curve cost of 100 PT.
    assert_eq!(payout, 100 * SCALE - quote.amount_in);
    // The pool gave up exactly `yt` PT and took in the cost as SY.
    let (pt_res1, sy_res1) = s.pool.reserves();
    assert_eq!(pt_res0 - pt_res1, yt);
    assert_eq!(sy_res1 - sy_res0, quote.amount_in - quote.fee_shares);
    // 100 PT and 100 YT were burned together.
    assert_eq!(pt_supply - s.pm.total_pt(), yt);
    assert_eq!(s.pm.total_pt(), s.pm.total_yt());
}

#[test]
fn flash_mint_then_flash_redeem_round_trip_returns_the_capital_minus_impact() {
    let (s, _) = seeded(Config::free(T0 + 180 * DAY));
    let user = s.new_user();
    deposit_sy(&s, &user, 100 * SCALE);
    let pool = s.pool.address.clone();
    s.router
        .swap_sy_for_yt(&user, &pool, &(100 * SCALE), &0, &(100 * SCALE), &u64::MAX);
    let yt = s.yt.balance(&user);
    s.router.swap_yt_for_sy(&user, &pool, &yt, &0, &u64::MAX);
    let end = s.sy.balance_of(&user);
    // Never more than the starting 100 SY; loses only price impact + rounding on ~100 notional.
    assert!(end <= 100 * SCALE, "round trip minted value: {end}");
    assert!(100 * SCALE - end < SCALE / 10, "lost {}", 100 * SCALE - end);
}

#[test]
fn flash_redeem_reverts_when_the_yt_cannot_cover_the_pt_it_consumes() {
    // A day before maturity PT trades at par, so YT is worth ~nothing; add a 5% fee tier and the
    // YT cannot pay for the PT it needs to recombine. The call must revert rather than take value
    // from the pool.
    let maturity = T0 + 300 * DAY;
    let mut cfg = Config::with_example_fees(maturity);
    cfg.swap_fee_tier_bps = 500;
    let (s, _) = seeded(cfg);
    let holder = s.new_user();
    let shares = deposit_sy(&s, &holder, 100 * SCALE);
    s.pm.mint(&holder, &shares);
    s.advance_flat(maturity - DAY);
    let pool = s.pool.address.clone();

    // (The 5 bps tokenization fee means the holder has slightly under 100 YT.)
    let yt = s.yt.balance(&holder);
    let q = s.pool.quote_buy_exact_pt(&yt);
    assert!(
        q.amount_in > yt,
        "the PT costs more than the recombined SY is worth"
    );
    let before = (s.pool.reserves(), s.yt.balance(&holder), s.pm.total_pt());
    assert_eq!(
        s.pool.try_swap_yt_for_sy(&holder, &holder, &yt, &0).err(),
        err_code(PoolError::InsufficientLiquidity as u32)
    );
    // The revert is atomic: nothing moved.
    assert_eq!(
        before,
        (s.pool.reserves(), s.yt.balance(&holder), s.pm.total_pt())
    );
    let _ = pool;
}

#[test]
fn flash_redeem_min_out_is_a_hard_floor_at_the_exact_payout() {
    let (s, _) = seeded(Config::free(T0 + 180 * DAY));
    let holder = s.new_user();
    deposit_sy(&s, &holder, 100 * SCALE);
    let pool = s.pool.address.clone();
    s.router.swap_sy_for_yt(
        &holder,
        &pool,
        &(100 * SCALE),
        &0,
        &(100 * SCALE),
        &u64::MAX,
    );
    let yt = s.yt.balance(&holder);
    let q = s.pool.quote_buy_exact_pt(&yt);
    let payout = 100 * SCALE - q.amount_in;
    assert!(payout > 0);
    assert!(s
        .router
        .try_swap_yt_for_sy(&holder, &pool, &yt, &(payout + 1), &u64::MAX)
        .is_err());
    assert_eq!(
        s.router
            .swap_yt_for_sy(&holder, &pool, &yt, &payout, &u64::MAX),
        payout
    );
}
