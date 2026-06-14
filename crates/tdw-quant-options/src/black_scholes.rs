//! Black-Scholes-Merton European option pricing and analytic greeks.
//!
//! The generalized (Merton 1973) form with a continuous dividend yield `q`:
//!
//! ```text
//!   d1 = [ ln(S/K) + (r - q + sigma^2 / 2) * T ] / (sigma * sqrt(T))
//!   d2 = d1 - sigma * sqrt(T)
//!   call = S * e^{-qT} * Phi(d1) - K * e^{-rT} * Phi(d2)
//!   put  = K * e^{-rT} * Phi(-d2) - S * e^{-qT} * Phi(-d1)
//! ```
//!
//! where `Phi` is the standard-normal CDF (see [`crate::normal`]). The greeks are
//! the closed-form analytic derivatives of the same formula. Theta and rho are
//! reported per *unit* of their variable (theta per calendar year, rho per unit
//! rate, i.e. `+1.0` = `+100%`), and vega per unit volatility (`+1.0` = `+100`
//! vol points); callers scale to per-day / per-basis-point as needed.
//!
//! Source: Black & Scholes (1973), "The Pricing of Options and Corporate
//! Liabilities"; Merton (1973), "Theory of Rational Option Pricing" for the
//! continuous-dividend extension. Public textbook math; no reference
//! implementation consulted.

use crate::Result;
use crate::error::OptionError;
use crate::normal::{cdf, pdf};
use crate::params::{BlackScholesParams, Greeks, OptionType};

/// Validate that `value` is strictly positive, naming `field` on failure.
fn require_positive(field: &'static str, value: f64) -> Result<()> {
    if value > 0.0 {
        Ok(())
    } else {
        Err(OptionError::NonPositive { field, value })
    }
}

/// The `(d1, d2)` pair of the Black-Scholes formula for the given contract.
///
/// Separated out so the price and every greek share one definition. Assumes the
/// inputs have already been validated as positive where required.
fn d1_d2(p: &BlackScholesParams) -> (f64, f64) {
    let sqrt_t = p.time_to_expiry.sqrt();
    let vol_sqrt_t = p.volatility * sqrt_t;
    let half_var = 0.5 * p.volatility * p.volatility;
    let numerator =
        (p.rate - p.dividend_yield + half_var).mul_add(p.time_to_expiry, (p.spot / p.strike).ln());
    let d1 = numerator / vol_sqrt_t;
    let d2 = d1 - vol_sqrt_t;
    (d1, d2)
}

/// Black-Scholes-Merton price of a European call or put.
///
/// # Errors
///
/// Returns [`OptionError::NonPositive`] when `spot`, `strike`, `volatility`, or
/// `time_to_expiry` is not strictly positive.
pub fn price(params: BlackScholesParams) -> Result<f64> {
    require_positive("spot", params.spot)?;
    require_positive("strike", params.strike)?;
    require_positive("volatility", params.volatility)?;
    require_positive("time_to_expiry", params.time_to_expiry)?;

    let (d1, d2) = d1_d2(&params);
    let disc_r = (-params.rate * params.time_to_expiry).exp();
    let disc_q = (-params.dividend_yield * params.time_to_expiry).exp();
    let spot_pv = params.spot * disc_q;
    let strike_pv = params.strike * disc_r;

    let value = match params.option_type {
        OptionType::Call => spot_pv.mul_add(cdf(d1), -(strike_pv * cdf(d2))),
        OptionType::Put => strike_pv.mul_add(cdf(-d2), -(spot_pv * cdf(-d1))),
    };
    Ok(value)
}

/// The Black-Scholes-Merton vega (price sensitivity to volatility) of a European
/// call or put.
///
/// Vega is exercise-symmetric (identical for calls and puts) and depends only on
/// the standard-normal *pdf* of `d1`: `vega = S * e^{-qT} * phi(d1) * sqrt(T)`.
/// This is the same value as [`greeks`]'s `vega` field but skips the four other
/// greeks (and their extra cdf evaluations) — useful for the implied-vol Newton
/// step, which needs only the derivative.
///
/// `vega` is per unit volatility (`+1.0` = a `+100` vol-point move).
///
/// # Errors
///
/// Returns [`OptionError::NonPositive`] when `spot`, `strike`, `volatility`, or
/// `time_to_expiry` is not strictly positive.
pub fn vega(params: BlackScholesParams) -> Result<f64> {
    require_positive("spot", params.spot)?;
    require_positive("strike", params.strike)?;
    require_positive("volatility", params.volatility)?;
    require_positive("time_to_expiry", params.time_to_expiry)?;

    let (d1, _d2) = d1_d2(&params);
    let sqrt_t = params.time_to_expiry.sqrt();
    let disc_q = (-params.dividend_yield * params.time_to_expiry).exp();
    Ok(params.spot * disc_q * pdf(d1) * sqrt_t)
}

/// The full analytic greeks (delta, gamma, theta, vega, rho) of a European call
/// or put under Black-Scholes-Merton.
///
/// `vega` is per unit volatility (`+1.0` = a `+100` vol-point move); `theta` is
/// per calendar year (usually negative); `rho` is per unit rate.
///
/// # Errors
///
/// Returns [`OptionError::NonPositive`] when `spot`, `strike`, `volatility`, or
/// `time_to_expiry` is not strictly positive.
pub fn greeks(params: BlackScholesParams) -> Result<Greeks> {
    require_positive("spot", params.spot)?;
    require_positive("strike", params.strike)?;
    require_positive("volatility", params.volatility)?;
    require_positive("time_to_expiry", params.time_to_expiry)?;

    let (d1, d2) = d1_d2(&params);
    let sqrt_t = params.time_to_expiry.sqrt();
    let disc_r = (-params.rate * params.time_to_expiry).exp();
    let disc_q = (-params.dividend_yield * params.time_to_expiry).exp();
    let pdf_d1 = pdf(d1);

    // Gamma and vega are exercise-symmetric (identical for calls and puts).
    let gamma = disc_q * pdf_d1 / (params.spot * params.volatility * sqrt_t);
    let vega = params.spot * disc_q * pdf_d1 * sqrt_t;

    // The dividend and rate terms shared by the theta expression.
    let theta_pdf_term = -params.spot * disc_q * pdf_d1 * params.volatility / (2.0 * sqrt_t);

    let (delta, theta, rho) = match params.option_type {
        OptionType::Call => {
            let delta = disc_q * cdf(d1);
            let theta = (params.dividend_yield * params.spot * disc_q).mul_add(
                cdf(d1),
                (-params.rate * params.strike * disc_r).mul_add(cdf(d2), theta_pdf_term),
            );
            let rho = params.strike * params.time_to_expiry * disc_r * cdf(d2);
            (delta, theta, rho)
        }
        OptionType::Put => {
            let delta = -disc_q * cdf(-d1);
            let theta = (params.rate * params.strike * disc_r).mul_add(
                cdf(-d2),
                (-params.dividend_yield * params.spot * disc_q).mul_add(cdf(-d1), theta_pdf_term),
            );
            let rho = -params.strike * params.time_to_expiry * disc_r * cdf(-d2);
            (delta, theta, rho)
        }
    };

    Ok(Greeks {
        delta,
        gamma,
        theta,
        vega,
        rho,
    })
}

#[cfg(test)]
mod tests {
    use super::{greeks, price};
    use crate::params::{BlackScholesParams, OptionType};

    /// The canonical textbook fixture: S=100, K=100, r=5%, sigma=20%, T=1, q=0.
    fn base(option_type: OptionType) -> BlackScholesParams {
        BlackScholesParams {
            spot: 100.0,
            strike: 100.0,
            rate: 0.05,
            dividend_yield: 0.0,
            volatility: 0.20,
            time_to_expiry: 1.0,
            option_type,
        }
    }

    #[test]
    fn call_matches_textbook_value() {
        // The standard reference value for this fixture is approx 10.4506.
        let c = price(base(OptionType::Call)).expect("call");
        assert!((c - 10.450_583_572).abs() < 1e-4, "call was {c}");
    }

    #[test]
    fn put_matches_textbook_value() {
        // Put = Call - S + K*e^{-rT} = 10.4506 - 100 + 95.1229 = 5.5735.
        let p = price(base(OptionType::Put)).expect("put");
        assert!((p - 5.573_526_022).abs() < 1e-4, "put was {p}");
    }

    #[test]
    fn put_call_parity_holds() {
        // C - P = S*e^{-qT} - K*e^{-rT}.
        let c = price(base(OptionType::Call)).expect("call");
        let p = price(base(OptionType::Put)).expect("put");
        let params = base(OptionType::Call);
        let spot_pv = params.spot * (-params.dividend_yield * params.time_to_expiry).exp();
        let strike_pv = params.strike * (-params.rate * params.time_to_expiry).exp();
        let rhs = spot_pv - strike_pv;
        assert!((c - p - rhs).abs() < 1e-9, "parity gap {}", c - p - rhs);
    }

    #[test]
    fn parity_holds_with_dividend_yield() {
        let mut call = base(OptionType::Call);
        call.dividend_yield = 0.03;
        let mut put = call;
        put.option_type = OptionType::Put;
        let c = price(call).expect("call");
        let p = price(put).expect("put");
        let spot_pv = call.spot * (-call.dividend_yield * call.time_to_expiry).exp();
        let strike_pv = call.strike * (-call.rate * call.time_to_expiry).exp();
        let rhs = spot_pv - strike_pv;
        assert!((c - p - rhs).abs() < 1e-9);
    }

    #[test]
    fn call_delta_in_unit_interval_put_delta_negative() {
        let cg = greeks(base(OptionType::Call)).expect("call greeks");
        let pg = greeks(base(OptionType::Put)).expect("put greeks");
        assert!(cg.delta > 0.0 && cg.delta < 1.0, "call delta {}", cg.delta);
        assert!(pg.delta < 0.0 && pg.delta > -1.0, "put delta {}", pg.delta);
        // delta_call - delta_put = e^{-qT} = 1 for q=0.
        assert!((cg.delta - pg.delta - 1.0).abs() < 1e-9);
    }

    #[test]
    fn gamma_and_vega_match_textbook_and_are_shared() {
        let cg = greeks(base(OptionType::Call)).expect("call");
        let pg = greeks(base(OptionType::Put)).expect("put");
        // Gamma and vega are identical for call and put.
        assert!((cg.gamma - pg.gamma).abs() < 1e-12);
        assert!((cg.vega - pg.vega).abs() < 1e-12);
        // Reference values for the base fixture.
        assert!(
            (cg.gamma - 0.018_762_017).abs() < 1e-6,
            "gamma {}",
            cg.gamma
        );
        assert!((cg.vega - 37.524_035).abs() < 1e-3, "vega {}", cg.vega);
        assert!(cg.gamma > 0.0 && cg.vega > 0.0);
    }

    #[test]
    fn vega_helper_matches_full_greeks_vega() {
        // The cheap pdf-only `vega` helper must equal the `vega` field of the
        // full greek set, for both calls and puts (vega is exercise-symmetric).
        for option_type in [OptionType::Call, OptionType::Put] {
            let v = super::vega(base(option_type)).expect("vega");
            let g = greeks(base(option_type)).expect("greeks");
            assert!(
                (v - g.vega).abs() < 1e-12,
                "vega {v} vs greeks.vega {}",
                g.vega
            );
        }
    }

    #[test]
    fn vega_rejects_non_positive_inputs() {
        let mut bad = base(OptionType::Call);
        bad.volatility = 0.0;
        assert!(super::vega(bad).is_err());
    }

    #[test]
    fn theta_negative_rho_signs_match_type() {
        let cg = greeks(base(OptionType::Call)).expect("call");
        let pg = greeks(base(OptionType::Put)).expect("put");
        // ATM theta is negative for both; call rho > 0, put rho < 0.
        assert!(cg.theta < 0.0, "call theta {}", cg.theta);
        assert!(pg.theta < 0.0, "put theta {}", pg.theta);
        assert!(cg.rho > 0.0, "call rho {}", cg.rho);
        assert!(pg.rho < 0.0, "put rho {}", pg.rho);
    }

    #[test]
    fn rejects_non_positive_inputs() {
        let mut bad = base(OptionType::Call);
        bad.volatility = 0.0;
        assert!(price(bad).is_err());
        let mut bad2 = base(OptionType::Call);
        bad2.spot = -1.0;
        assert!(greeks(bad2).is_err());
    }
}
