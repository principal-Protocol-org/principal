//! Fixed-point math for the yield-curve AMM. Pure integer arithmetic -- no floating point -- at
//! `WAD = 1e18` precision, so results are bit-for-bit reproducible on every validator.
//!
//! # The curve
//! `MarketPool` uses the *Yield Space* constant-power-sum invariant
//!
//! ```text
//!     x^a + y^a = k          a = 1 - tau / stretch          0 < a <= 1
//! ```
//!
//! where `x` is the SY reserve **valued in notional** (shares x oracle rate), `y` is the PT reserve,
//! `tau` is the time to maturity in years and `stretch` is the pool's time-stretch in years.
//!
//! * As `tau -> 0`, `a -> 1` and the invariant degenerates to the constant *sum* `x + y = k`: PT
//!   trades at exactly par against SY. Convergence is a property of the exponent, so no special
//!   case is needed and there is no time-decay impermanent loss -- LPs' holdings are fixed while the
//!   curve slides towards par underneath them.
//! * The marginal price of PT in SY value is `(x / y)^(1 - a) = (x / y)^(tau / stretch)`: a
//!   discount (`x < y`) that shrinks to zero as `tau` does.
//!
//! Swaps solve the invariant in closed form: hold `k` fixed, move one reserve, and recover the
//! other as `(k - p'^a)^(1/a)`. No iterative solver, so no convergence failure mode.

/// Fixed-point unit: 1.0 == 1e18.
pub const WAD: i128 = 1_000_000_000_000_000_000;
/// ln(2) at WAD precision.
const LN2: i128 = 693_147_180_559_945_309;

/// Why a curve computation was rejected.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum MathError {
    /// An argument was zero/negative where a positive value is required.
    Domain,
    /// A fixed-point intermediate overflowed `i128`.
    Overflow,
    /// The trade would drain a reserve (invariant has no solution).
    Insufficient,
}

#[inline]
fn mul(a: i128, b: i128) -> Result<i128, MathError> {
    a.checked_mul(b).map(|v| v / WAD).ok_or(MathError::Overflow)
}

#[inline]
fn div(a: i128, b: i128) -> Result<i128, MathError> {
    if b == 0 {
        return Err(MathError::Domain);
    }
    a.checked_mul(WAD).map(|v| v / b).ok_or(MathError::Overflow)
}

/// Natural logarithm of `x / WAD`, returned at WAD precision (negative for `x < WAD`).
pub fn ln(x: i128) -> Result<i128, MathError> {
    if x <= 0 {
        return Err(MathError::Domain);
    }
    // Reduce to m in [1, 2): x = m * 2^k.
    let mut m = x;
    let mut k: i128 = 0;
    while m >= 2 * WAD {
        m /= 2;
        k += 1;
    }
    while m < WAD {
        m *= 2;
        k -= 1;
    }
    // ln(m) = 2 * atanh(z), z = (m - 1) / (m + 1) in [0, 1/3).
    let z = div(m - WAD, m + WAD)?;
    let z2 = mul(z, z)?;
    let mut term = z;
    let mut sum = z;
    let mut n: i128 = 3;
    while term != 0 {
        term = mul(term, z2)?;
        sum += term / n;
        n += 2;
    }
    Ok(k * LN2 + 2 * sum)
}

/// `e^x` for `x` at WAD precision, returned at WAD precision. Errors on overflow (`x` above ~40).
pub fn exp(x: i128) -> Result<i128, MathError> {
    // Below e^-42 the result is < 1 WAD-unit: flush to zero.
    if x < -42 * WAD {
        return Ok(0);
    }
    if x > 40 * WAD {
        return Err(MathError::Overflow);
    }
    // x = k * ln2 + r with r in [-ln2/2, ln2/2].
    let half = LN2 / 2;
    let k = if x >= 0 {
        (x + half) / LN2
    } else {
        -((-x + half) / LN2)
    };
    let r = x - k * LN2;
    let mut term = WAD;
    let mut sum = WAD;
    let mut n: i128 = 1;
    while term != 0 {
        term = mul(term, r)? / n;
        sum += term;
        n += 1;
    }
    if k >= 0 {
        sum.checked_shl(k as u32)
            .filter(|v| *v >> k == sum)
            .ok_or(MathError::Overflow)
    } else {
        Ok(sum >> ((-k) as u32))
    }
}

/// `base^exponent` where both are WAD-scaled and `base > 0`.
pub fn pow(base: i128, exponent: i128) -> Result<i128, MathError> {
    if base <= 0 {
        return Err(MathError::Domain);
    }
    if exponent == 0 {
        return Ok(WAD);
    }
    exp(mul(exponent, ln(base)?)?)
}

/// Smallest exponent `a` the curve accepts (`stretch` must keep `tau / stretch <= 0.75`).
pub const MIN_EXPONENT: i128 = WAD / 4;

/// Below this the residual `k - p'^a` is indistinguishable from rounding noise; treat the reserve
/// as drained.
const MIN_RESIDUAL: i128 = WAD / 1_000_000_000_000;

/// Divisor of the rounding pad: `solve` pads its result by `1 + u / PAD_DIVISOR` units.
///
/// The fixed-point pipeline (three `pow`s, one more for `1/a`) carries a relative error of a few
/// 1e-17 of the largest reserve `u`, i.e. up to ~`u / 1e16` raw units -- a unit in the last place is
/// *not* enough once `u` is large (a fee-free round trip on a 1e17-unit pool profited by 6 units
/// before this pad existed). Padding by `u / 1e14` is ~100x the worst observed error and costs a
/// trader at most 1e-14 of the pool's size per swap.
pub const PAD_DIVISOR: i128 = 100_000_000_000_000;

/// Solve the invariant for the other reserve: given reserves `(p, q)` on `p^a + q^a = k`, move `p`
/// to `p_new` and return `q_new`, **rounded up** by the pad above so that it always errs in the
/// pool's favor (every caller pays out `q - q_new` or charges `q_new - q`). All reserves are raw
/// integers of one common unit; the computation is scale-free (it works on ratios to the largest
/// reserve), so pool size never overflows the fixed-point intermediates.
pub fn solve(p: i128, q: i128, p_new: i128, a: i128) -> Result<i128, MathError> {
    if p <= 0 || q <= 0 || p_new <= 0 || !(MIN_EXPONENT..=WAD).contains(&a) {
        return Err(MathError::Domain);
    }
    let u = p.max(q).max(p_new);
    let pa = pow(div(p, u)?, a)?;
    let qa = pow(div(q, u)?, a)?;
    let pna = pow(div(p_new, u)?, a)?;
    let resid = pa + qa - pna;
    if resid < MIN_RESIDUAL {
        return Err(MathError::Insufficient);
    }
    // w = resid^(1/a), 1/a in [1, 4].
    let w = pow(resid, div(WAD, a)?)?;
    u.checked_mul(w)
        .map(|v| v / WAD + 1 + u / PAD_DIVISOR)
        .ok_or(MathError::Overflow)
}

/// The exponent `a = 1 - seconds_to_maturity / stretch_secs`, at WAD, floored at zero elapsed
/// error: `stretch_secs` must be positive.
pub fn exponent(seconds_to_maturity: u64, stretch_secs: u64) -> Result<i128, MathError> {
    if stretch_secs == 0 {
        return Err(MathError::Domain);
    }
    let frac = (seconds_to_maturity as i128) * WAD / (stretch_secs as i128);
    if frac >= WAD {
        return Err(MathError::Domain);
    }
    Ok(WAD - frac)
}

/// Integer square root (floor).
pub fn isqrt(n: i128) -> i128 {
    if n <= 0 {
        return 0;
    }
    let mut x = n;
    let mut y = (x + 1) / 2;
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}

#[cfg(test)]
#[path = "math_test.rs"]
mod test;
