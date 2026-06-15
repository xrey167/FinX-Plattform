//! Implied-volatility solver: invert a market option price for its Black-Scholes
//! volatility.
//!
//! The Black-Scholes price is strictly increasing in volatility, so the inverse
//! is well-defined for any price strictly inside the no-arbitrage bounds. The
//! solver uses Newton-Raphson — whose derivative is vega (closed-form, always
//! positive), giving quadratic convergence near the root — with a robust
//! bisection fallback: it first brackets the root on `[lo, hi]` (expanding `hi`
//! until the model price exceeds the target), then accepts a Newton step only
//! when it stays inside the bracket and otherwise bisects. This converges or
//! returns a clear [`OptionError::ImpliedVolNoConvergence`]; it never diverges or
//! loops unboundedly.
//!
//! Source: standard root-finding (Newton-Raphson + bisection) applied to the
//! Black-Scholes price function; public textbook math.

use crate::Result;
use crate::black_scholes::{price, vega};
use crate::error::OptionError;
use crate::params::{BlackScholesParams, ImpliedVolParams};

/// The lowest volatility the solver will report (a floor that keeps the price
/// function well-conditioned).
const VOL_LO: f64 = 1e-6;
/// The upper volatility bound the bracket is allowed to expand to (`1000%`),
/// beyond which a price is treated as unattainable.
const VOL_HI_MAX: f64 = 10.0;
/// Absolute price tolerance for declaring convergence.
const PRICE_TOL: f64 = 1e-8;
/// Maximum combined Newton/bisection iterations.
const MAX_ITERS: usize = 100;

/// Build a Black-Scholes param set from the implied-vol request at a trial `vol`.
const fn bs_at(p: &ImpliedVolParams, vol: f64) -> BlackScholesParams {
    BlackScholesParams {
        spot: p.spot,
        strike: p.strike,
        rate: p.rate,
        dividend_yield: p.dividend_yield,
        volatility: vol,
        time_to_expiry: p.time_to_expiry,
        option_type: p.option_type,
    }
}

/// Solve for the Black-Scholes implied volatility that reproduces
/// `params.market_price`.
///
/// Returns the volatility (a decimal, e.g. `0.20` for 20%).
///
/// # Errors
///
/// Returns [`OptionError::NonPositive`] when `spot`, `strike`, or
/// `time_to_expiry` is not strictly positive, and
/// [`OptionError::ImpliedVolNoConvergence`] when the market price lies outside the
/// volatility-attainable range or the bracketed search exhausts its iteration
/// budget.
pub fn implied_volatility(params: ImpliedVolParams) -> Result<f64> {
    if params.spot <= 0.0 {
        return Err(OptionError::NonPositive {
            field: "spot",
            value: params.spot,
        });
    }
    if params.strike <= 0.0 {
        return Err(OptionError::NonPositive {
            field: "strike",
            value: params.strike,
        });
    }
    if params.time_to_expiry <= 0.0 {
        return Err(OptionError::NonPositive {
            field: "time_to_expiry",
            value: params.time_to_expiry,
        });
    }
    if params.market_price <= 0.0 {
        return Err(OptionError::ImpliedVolNoConvergence(
            "market price must be > 0",
        ));
    }

    // Bracket: the price is increasing in vol. `lo` at the floor is below the
    // target (after the price-below check), so expand `hi` until the model price
    // brackets the target.
    let mut lo = VOL_LO;
    let mut hi = 1.0;
    let price_at = |vol: f64| price(bs_at(&params, vol));

    // If even the floor vol already overprices the target, no positive vol
    // reproduces it (price below the intrinsic/discount bound).
    if price_at(lo)? > params.market_price + PRICE_TOL {
        return Err(OptionError::ImpliedVolNoConvergence(
            "market price below the zero-volatility lower bound",
        ));
    }
    // Double `hi` until the model price brackets the target from above. The
    // bound `VOL_HI_MAX` caps the search to a finite number of doublings.
    let mut bracketed = false;
    loop {
        if price_at(hi)? >= params.market_price {
            bracketed = true;
            break;
        }
        hi *= 2.0;
        if hi > VOL_HI_MAX {
            break;
        }
    }
    if !bracketed {
        return Err(OptionError::ImpliedVolNoConvergence(
            "market price exceeds the attainable range (vol > 1000%)",
        ));
    }

    // Start Newton from the bracket midpoint.
    let mut vol = 0.5 * (lo + hi);
    for _ in 0..MAX_ITERS {
        let model = price_at(vol)?;
        let diff = model - params.market_price;
        if diff.abs() < PRICE_TOL {
            return Ok(vol);
        }
        // Tighten the bracket using the sign of the residual (price is monotone).
        if diff > 0.0 {
            hi = vol;
        } else {
            lo = vol;
        }
        // Newton step: vega is the derivative of price w.r.t. vol. Use the
        // pdf-only `vega` helper rather than computing the full greek set —
        // the other four greeks (and their extra cdf evaluations) are unused
        // here.
        let dprice_dvol = vega(bs_at(&params, vol))?;
        let next = if dprice_dvol.abs() > 1e-12 {
            vol - diff / dprice_dvol
        } else {
            f64::INFINITY
        };
        // Accept the Newton step only if it stays strictly inside the bracket;
        // otherwise bisect. This keeps the iteration bounded and convergent.
        vol = if next > lo && next < hi {
            next
        } else {
            0.5 * (lo + hi)
        };
    }

    // A last residual check after the iteration budget.
    let final_diff = (price_at(vol)? - params.market_price).abs();
    if final_diff < 1e-6 {
        Ok(vol)
    } else {
        Err(OptionError::ImpliedVolNoConvergence(
            "iteration budget exhausted before reaching price tolerance",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::implied_volatility;
    use crate::black_scholes::price;
    use crate::params::{BlackScholesParams, ImpliedVolParams, OptionType};

    fn iv_request(market_price: f64, option_type: OptionType) -> ImpliedVolParams {
        ImpliedVolParams {
            market_price,
            spot: 100.0,
            strike: 100.0,
            rate: 0.05,
            dividend_yield: 0.0,
            time_to_expiry: 1.0,
            option_type,
        }
    }

    #[test]
    fn round_trips_price_to_vol_to_price_call() {
        // Price a call at sigma=0.2, recover the vol, re-price: should match.
        let known_vol = 0.20;
        let priced = price(BlackScholesParams {
            spot: 100.0,
            strike: 100.0,
            rate: 0.05,
            dividend_yield: 0.0,
            volatility: known_vol,
            time_to_expiry: 1.0,
            option_type: OptionType::Call,
        })
        .expect("price");
        let recovered = implied_volatility(iv_request(priced, OptionType::Call)).expect("iv");
        assert!((recovered - known_vol).abs() < 1e-6, "iv {recovered}");
    }

    #[test]
    fn round_trips_for_put_and_otm_strike() {
        let known_vol = 0.35;
        let req = BlackScholesParams {
            spot: 100.0,
            strike: 120.0,
            rate: 0.03,
            dividend_yield: 0.01,
            volatility: known_vol,
            time_to_expiry: 0.5,
            option_type: OptionType::Put,
        };
        let priced = price(req).expect("price");
        let mut iv = ImpliedVolParams {
            market_price: priced,
            spot: req.spot,
            strike: req.strike,
            rate: req.rate,
            dividend_yield: req.dividend_yield,
            time_to_expiry: req.time_to_expiry,
            option_type: OptionType::Put,
        };
        let recovered = implied_volatility(iv).expect("iv");
        assert!((recovered - known_vol).abs() < 1e-6, "iv {recovered}");
        // And a clearly-impossible price (above spot) fails cleanly.
        iv.market_price = 1e6;
        assert!(implied_volatility(iv).is_err());
    }

    #[test]
    fn rejects_non_positive_price() {
        assert!(implied_volatility(iv_request(0.0, OptionType::Call)).is_err());
    }
}
