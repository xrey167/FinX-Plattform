//! Cox-Ross-Rubinstein (CRR) binomial-tree option pricing.
//!
//! Builds an `N`-step recombining binomial tree over the time to expiry and
//! prices a European or American call/put by backward induction. With a
//! continuous dividend yield `q`, the per-step parameters are:
//!
//! ```text
//!   dt = T / N
//!   u  = e^{sigma * sqrt(dt)},   d = 1 / u
//!   p  = (e^{(r - q) * dt} - d) / (u - d)        (risk-neutral up probability)
//!   discount = e^{-r * dt}
//! ```
//!
//! Terminal payoffs are the option's intrinsic value; each earlier node is the
//! discounted risk-neutral expectation of its two children, and for an American
//! option that continuation value is floored at the immediate-exercise intrinsic
//! value. As `N` grows the European price converges to the Black-Scholes value.
//!
//! Source: Cox, Ross & Rubinstein (1979), "Option Pricing: A Simplified
//! Approach". Public textbook math; no reference implementation consulted.

use crate::Result;
use crate::error::OptionError;
use crate::params::{BinomialParams, ExerciseStyle, OptionType};

/// Intrinsic value (immediate-exercise payoff) of the option at underlying price
/// `spot`.
fn intrinsic(option_type: OptionType, spot: f64, strike: f64) -> f64 {
    match option_type {
        OptionType::Call => (spot - strike).max(0.0),
        OptionType::Put => (strike - spot).max(0.0),
    }
}

/// Price a European or American call/put with an `N`-step CRR binomial tree.
///
/// # Errors
///
/// Returns [`OptionError::NonPositive`] when `spot`, `strike`, `volatility`, or
/// `time_to_expiry` is not strictly positive, and [`OptionError::InvalidSteps`]
/// when `steps` is zero.
pub fn price(params: BinomialParams) -> Result<f64> {
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
    if params.volatility <= 0.0 {
        return Err(OptionError::NonPositive {
            field: "volatility",
            value: params.volatility,
        });
    }
    if params.time_to_expiry <= 0.0 {
        return Err(OptionError::NonPositive {
            field: "time_to_expiry",
            value: params.time_to_expiry,
        });
    }
    if params.steps == 0 {
        return Err(OptionError::InvalidSteps(params.steps));
    }

    let n = params.steps;
    #[allow(clippy::cast_precision_loss)]
    let n_f = n as f64;
    let dt = params.time_to_expiry / n_f;
    let up = (params.volatility * dt.sqrt()).exp();
    let down = 1.0 / up;
    // Per-up-move price ratio `u / d`: a node with `j` ups (and the rest downs)
    // at a layer of `m` total steps has price `S * d^m * ratio^j`, computed
    // multiplicatively to avoid integer-exponent casts.
    let ratio = up / down;
    let growth = ((params.rate - params.dividend_yield) * dt).exp();
    let p_up = (growth - down) / (up - down);
    let p_down = 1.0 - p_up;
    let discount = (-params.rate * dt).exp();

    // Spot price at node `j` of a `layers`-step layer: S * d^layers * ratio^j.
    let node_spot = |layers: usize, j: usize| {
        let mut spot = params.spot;
        for _ in 0..layers {
            spot *= down;
        }
        for _ in 0..j {
            spot *= ratio;
        }
        spot
    };

    // Terminal layer: node j (j = 0..=n) has had j up-moves and (n - j) downs.
    let mut values: Vec<f64> = (0..=n)
        .map(|j| intrinsic(params.option_type, node_spot(n, j), params.strike))
        .collect();

    // Backward induction from step n-1 down to 0.
    for step in (0..n).rev() {
        for j in 0..=step {
            let continuation = discount * p_up.mul_add(values[j + 1], p_down * values[j]);
            values[j] = match params.exercise {
                ExerciseStyle::European => continuation,
                ExerciseStyle::American => continuation.max(intrinsic(
                    params.option_type,
                    node_spot(step, j),
                    params.strike,
                )),
            };
        }
    }

    Ok(values[0])
}

#[cfg(test)]
mod tests {
    use super::price;
    use crate::black_scholes;
    use crate::params::{BinomialParams, BlackScholesParams, ExerciseStyle, OptionType};

    fn base(exercise: ExerciseStyle, option_type: OptionType, steps: usize) -> BinomialParams {
        BinomialParams {
            spot: 100.0,
            strike: 100.0,
            rate: 0.05,
            dividend_yield: 0.0,
            volatility: 0.20,
            time_to_expiry: 1.0,
            option_type,
            exercise,
            steps,
        }
    }

    #[test]
    fn european_call_converges_to_black_scholes() {
        let bs = black_scholes::price(BlackScholesParams {
            spot: 100.0,
            strike: 100.0,
            rate: 0.05,
            dividend_yield: 0.0,
            volatility: 0.20,
            time_to_expiry: 1.0,
            option_type: OptionType::Call,
        })
        .expect("bs");

        let coarse = price(base(ExerciseStyle::European, OptionType::Call, 10)).expect("n=10");
        let fine = price(base(ExerciseStyle::European, OptionType::Call, 500)).expect("n=500");
        // Convergence: the fine tree is much closer to BS than the coarse one.
        assert!((fine - bs).abs() < (coarse - bs).abs());
        assert!((fine - bs).abs() < 0.05, "fine {fine} vs bs {bs}");
    }

    #[test]
    fn european_put_converges_to_black_scholes() {
        let bs = black_scholes::price(BlackScholesParams {
            spot: 100.0,
            strike: 100.0,
            rate: 0.05,
            dividend_yield: 0.0,
            volatility: 0.20,
            time_to_expiry: 1.0,
            option_type: OptionType::Put,
        })
        .expect("bs");
        let fine = price(base(ExerciseStyle::European, OptionType::Put, 500)).expect("put");
        assert!((fine - bs).abs() < 0.05, "fine {fine} vs bs {bs}");
    }

    #[test]
    fn american_call_no_dividend_equals_european() {
        // With no dividends an American call is never early-exercised, so it
        // equals the European call.
        let euro = price(base(ExerciseStyle::European, OptionType::Call, 200)).expect("euro");
        let amer = price(base(ExerciseStyle::American, OptionType::Call, 200)).expect("amer");
        assert!((euro - amer).abs() < 1e-9, "euro {euro} amer {amer}");
    }

    #[test]
    fn american_put_premium_over_european() {
        // An American put carries an early-exercise premium, so it is worth at
        // least as much as (here strictly more than) the European put.
        let euro = price(base(ExerciseStyle::European, OptionType::Put, 200)).expect("euro");
        let amer = price(base(ExerciseStyle::American, OptionType::Put, 200)).expect("amer");
        assert!(amer > euro, "amer {amer} should exceed euro {euro}");
    }

    #[test]
    fn rejects_zero_steps_and_bad_inputs() {
        assert!(price(base(ExerciseStyle::European, OptionType::Call, 0)).is_err());
        let mut bad = base(ExerciseStyle::European, OptionType::Call, 10);
        bad.volatility = -0.1;
        assert!(price(bad).is_err());
    }
}
