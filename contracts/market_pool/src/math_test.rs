extern crate std;
use super::*;

fn wad(f: f64) -> i128 {
    (f * 1e18) as i128
}

fn approx(actual: i128, expected: f64, rel: f64, abs: f64) {
    let a = actual as f64 / 1e18;
    let tol = expected.abs() * rel + abs;
    assert!(
        (a - expected).abs() <= tol,
        "got {a} expected {expected} (tol {tol})"
    );
}

#[test]
fn ln_matches_std_across_the_range() {
    for &v in &[
        1e-12, 1e-6, 0.001, 0.25, 0.5, 0.9, 1.0, 1.0001, 1.5, 2.0, 7.0, 1234.5, 1e9,
    ] {
        approx(ln(wad(v)).unwrap(), v.ln(), 1e-13, 1e-15);
    }
}

#[test]
fn ln_exact_at_one_and_domain_errors() {
    assert_eq!(ln(WAD).unwrap(), 0);
    assert_eq!(ln(0), Err(MathError::Domain));
    assert_eq!(ln(-5), Err(MathError::Domain));
}

#[test]
fn exp_matches_std_across_the_range() {
    for &v in &[
        -30.0, -10.0, -1.0, -0.3, 0.0, 1e-9, 0.3, 1.0, 5.0, 20.0, 39.0,
    ] {
        approx(exp(wad(v)).unwrap(), v.exp(), 1e-13, 1e-15);
    }
}

#[test]
fn exp_bounds() {
    assert_eq!(exp(-43 * WAD).unwrap(), 0);
    assert_eq!(exp(41 * WAD), Err(MathError::Overflow));
    assert_eq!(exp(0).unwrap(), WAD);
}

#[test]
fn exp_and_ln_are_inverse() {
    for &v in &[0.01, 0.4, 0.999, 1.0, 3.7, 900.0] {
        let back = exp(ln(wad(v)).unwrap()).unwrap();
        approx(back, v, 1e-13, 1e-15);
    }
}

#[test]
fn pow_matches_std() {
    for &(b, e) in &[
        (0.5, 0.75),
        (0.9, 0.9),
        (0.01, 0.3),
        (2.0, 0.5),
        (0.99, 0.25),
    ] {
        approx(pow(wad(b), wad(e)).unwrap(), f64::powf(b, e), 1e-12, 1e-15);
    }
    assert_eq!(pow(WAD, 0).unwrap(), WAD);
    assert_eq!(pow(0, WAD), Err(MathError::Domain));
}

#[test]
fn solve_preserves_the_invariant() {
    // p^a + q^a = k must hold (to rounding) before and after the move, for many shapes.
    let cases: &[(i128, i128, i128, f64)] = &[
        (1_000_000_000, 1_000_000_000, 1_100_000_000, 0.9),
        (950_000_000_000, 1_000_000_000_000, 960_000_000_000, 0.75),
        (
            5_000_000_000_000_000,
            5_200_000_000_000_000,
            5_250_000_000_000_000,
            0.98,
        ),
        (10_000_000, 12_000_000, 11_000_000, 0.3),
        (1_000_000_000, 900_000_000, 500_000_000, 0.999),
    ];
    for &(p, q, pn, a) in cases {
        let qn = solve(p, q, pn, wad(a)).unwrap();
        // The reference answer in f64: q' = (k - p'^a)^(1/a). The result is padded *up*, so
        // it sits at or above the exact value, by at most the pad plus f64 noise.
        let k0 = (p as f64).powf(a) + (q as f64).powf(a);
        let exact = (k0 - (pn as f64).powf(a)).powf(1.0 / a);
        let err = exact - qn as f64;
        assert!(
            err > -1.0 - 1e-11 * exact && err < 1.0 + 1e-11 * exact,
            "q' off by {err} (exact {exact}) for {p},{q},{pn},{a}"
        );
    }
}

#[test]
fn solve_at_a_equal_one_is_constant_sum() {
    // a = 1: x + y = k, so moving p by d moves q by exactly -d (up to the rounding pad).
    let qn = solve(400_000_000, 600_000_000, 450_000_000, WAD).unwrap();
    assert!(qn >= 550_000_000 && qn - 550_000_000 <= 3, "{qn}");
}

#[test]
fn solve_rejects_bad_inputs_and_drained_reserves() {
    assert_eq!(solve(0, 1, 1, WAD), Err(MathError::Domain));
    assert_eq!(solve(1, 0, 1, WAD), Err(MathError::Domain));
    assert_eq!(solve(1, 1, 0, WAD), Err(MathError::Domain));
    assert_eq!(solve(10, 10, 10, MIN_EXPONENT - 1), Err(MathError::Domain));
    assert_eq!(solve(10, 10, 10, WAD + 1), Err(MathError::Domain));
    // Pushing p far past k^(1/a) leaves no solution for q.
    assert_eq!(
        solve(1_000_000, 1_000_000, 1_000_000_000_000, WAD),
        Err(MathError::Insufficient)
    );
}

#[test]
fn exponent_shrinks_to_one_at_maturity() {
    let year = 31_536_000u64;
    let stretch = 4 * year;
    assert_eq!(exponent(0, stretch).unwrap(), WAD);
    assert_eq!(exponent(year, stretch).unwrap(), WAD * 3 / 4);
    assert_eq!(exponent(3 * year, stretch).unwrap(), WAD / 4);
    assert_eq!(exponent(4 * year, stretch), Err(MathError::Domain));
    assert_eq!(exponent(1, 0), Err(MathError::Domain));
}

#[test]
fn isqrt_is_floor_sqrt() {
    assert_eq!(isqrt(0), 0);
    assert_eq!(isqrt(-4), 0);
    assert_eq!(isqrt(1), 1);
    assert_eq!(isqrt(15), 3);
    assert_eq!(isqrt(16), 4);
    assert_eq!(isqrt(1_000_000_000_000_000_000_000_000), 1_000_000_000_000);
}
