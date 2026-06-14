//! Standard-normal distribution primitives (hand-rolled, no numeric dependency).
//!
//! The Black-Scholes-Merton formula and its greeks need the standard-normal
//! cumulative distribution function `Phi(x)` and probability density function
//! `phi(x)`. Rather than pull in `statrs`/`libm`, both are hand-rolled here from
//! the standard definitions:
//!
//! - `phi(x) = exp(-x^2 / 2) / sqrt(2*pi)` — the textbook Gaussian density.
//! - `Phi(x) = (1 + erf(x / sqrt(2))) / 2` — the CDF via the error function.
//! - `erf(z)` uses the Abramowitz & Stegun rational approximation 7.1.26, whose
//!   maximum absolute error is `1.5e-7` — ample for option pricing, where market
//!   quotes carry far coarser precision.
//!
//! All functions are `NaN`/`inf`-free for any finite input and are odd/even per
//! the analytic symmetry (`erf(-z) = -erf(z)`, `phi(-x) = phi(x)`,
//! `Phi(-x) = 1 - Phi(x)`).

use std::f64::consts::FRAC_1_SQRT_2;

/// `1 / sqrt(2*pi)`, the standard-normal density's normalizing constant.
const INV_SQRT_2PI: f64 = 0.398_942_280_401_432_7;

/// The standard-normal probability density function
/// `phi(x) = exp(-x^2 / 2) / sqrt(2*pi)`.
#[must_use]
pub fn pdf(x: f64) -> f64 {
    INV_SQRT_2PI * (-0.5 * x * x).exp()
}

/// The Gauss error function `erf(z)` via Abramowitz & Stegun 7.1.26.
///
/// The approximation is stated for `z >= 0`; the odd symmetry `erf(-z) = -erf(z)`
/// extends it to the whole real line. Maximum absolute error `1.5e-7`.
#[must_use]
pub fn erf(z: f64) -> f64 {
    // A&S 7.1.26 coefficients.
    const A1: f64 = 0.254_829_592;
    const A2: f64 = -0.284_496_736;
    const A3: f64 = 1.421_413_741;
    const A4: f64 = -1.453_152_027;
    const A5: f64 = 1.061_405_429;
    const P: f64 = 0.327_591_1;

    let sign = if z < 0.0 { -1.0 } else { 1.0 };
    let x = z.abs();
    let t = 1.0 / P.mul_add(x, 1.0);
    // Horner evaluation of the degree-5 polynomial in t.
    let poly = A5
        .mul_add(t, A4)
        .mul_add(t, A3)
        .mul_add(t, A2)
        .mul_add(t, A1)
        * t;
    let y = 1.0 - poly * (-x * x).exp();
    sign * y
}

/// The standard-normal cumulative distribution function
/// `Phi(x) = (1 + erf(x / sqrt(2))) / 2`.
#[must_use]
pub fn cdf(x: f64) -> f64 {
    0.5 * (1.0 + erf(x * FRAC_1_SQRT_2))
}

#[cfg(test)]
mod tests {
    use super::{cdf, erf, pdf};

    #[test]
    fn cdf_at_zero_is_one_half() {
        assert!((cdf(0.0) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn cdf_symmetry_holds() {
        // Phi(-x) = 1 - Phi(x).
        for &x in &[0.1, 0.5, 1.0, 1.96, 2.5] {
            assert!((cdf(-x) - (1.0 - cdf(x))).abs() < 1e-7);
        }
    }

    #[test]
    fn cdf_matches_known_quantiles() {
        // Standard-normal table values.
        assert!((cdf(1.0) - 0.841_344_75).abs() < 1e-6);
        assert!((cdf(1.96) - 0.975).abs() < 1e-4);
        assert!((cdf(-1.96) - 0.025).abs() < 1e-4);
        assert!((cdf(2.326_347_9) - 0.99).abs() < 1e-5);
    }

    #[test]
    fn erf_is_odd_and_bounded() {
        // The A&S 7.1.26 approximation carries a max absolute error of 1.5e-7,
        // so erf(0) is near — not exactly — zero.
        assert!((erf(0.0)).abs() < 1.5e-7);
        assert!((erf(1.0) - 0.842_700_79).abs() < 1e-6);
        assert!((erf(-1.0) + 0.842_700_79).abs() < 1e-6);
        // erf saturates to +-1 in the tails.
        assert!((erf(5.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn pdf_matches_known_values() {
        // phi(0) = 1/sqrt(2*pi).
        assert!((pdf(0.0) - 0.398_942_280_4).abs() < 1e-9);
        // phi(1) = exp(-0.5)/sqrt(2*pi).
        assert!((pdf(1.0) - 0.241_970_72).abs() < 1e-7);
        assert!((pdf(-1.0) - pdf(1.0)).abs() < 1e-12);
    }

    #[test]
    fn normalizing_constant_equals_inv_sqrt_2pi() {
        use std::f64::consts::PI;
        // The normalizing constant equals 1/sqrt(2*pi); pin that identity.
        assert!((super::INV_SQRT_2PI - 1.0 / (2.0 * PI).sqrt()).abs() < 1e-15);
    }
}
