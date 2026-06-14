//! Typed parameter and output structs for every option-pricing route.
//!
//! Each request struct carries the contract inputs (spot, strike, rate, dividend
//! yield, volatility, time-to-expiry) plus model-specific knobs (binomial step
//! count, exercise style, Monte-Carlo path count and seed). Every struct derives
//! `JsonSchema` so the daemon catalog can publish the parameter schema, and
//! `serde` defaults so a request may omit the optional fields (dividend yield,
//! exercise style) and still deserialize.
//!
//! # Sign and unit conventions
//!
//! - `spot`, `strike` are absolute prices in the same currency.
//! - `rate` (`r`) and `dividend_yield` (`q`) are *continuously compounded annual*
//!   rates expressed as decimals (`0.05` for 5%).
//! - `volatility` (`sigma`) is the annualized volatility as a decimal.
//! - `time_to_expiry` (`T`) is in years.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The option exercise right: a call (right to buy) or a put (right to sell).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum OptionType {
    /// A call option: the right to buy the underlying at the strike.
    #[default]
    Call,
    /// A put option: the right to sell the underlying at the strike.
    Put,
}

/// The exercise style: European or American.
///
/// European options are exercisable only at expiry; American options at any time
/// up to expiry. Only the binomial tree honours the early-exercise premium; the
/// closed-form Black-Scholes and the Monte-Carlo pricer are European.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ExerciseStyle {
    /// Exercisable only at expiry.
    #[default]
    European,
    /// Exercisable at any node up to and including expiry.
    American,
}

const fn f0() -> f64 {
    0.0
}

/// Contract inputs shared by the Black-Scholes price, the greeks, and the
/// Monte-Carlo pricer (all European). Dividend yield defaults to `0`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct BlackScholesParams {
    /// Current price of the underlying (`S`). Must be `> 0`.
    pub spot: f64,
    /// Strike price (`K`). Must be `> 0`.
    pub strike: f64,
    /// Continuously compounded annual risk-free rate (`r`), as a decimal.
    pub rate: f64,
    /// Continuously compounded annual dividend yield (`q`), as a decimal.
    #[serde(default = "f0")]
    pub dividend_yield: f64,
    /// Annualized volatility (`sigma`), as a decimal. Must be `> 0`.
    pub volatility: f64,
    /// Time to expiry in years (`T`). Must be `> 0`.
    pub time_to_expiry: f64,
    /// Call or put.
    #[serde(default)]
    pub option_type: OptionType,
}

/// Inputs for the implied-volatility solver: the same contract terms as
/// [`BlackScholesParams`] but with the observed `market_price` in place of the
/// (unknown) volatility being solved for.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ImpliedVolParams {
    /// Observed market price of the option to invert for volatility.
    pub market_price: f64,
    /// Current price of the underlying (`S`). Must be `> 0`.
    pub spot: f64,
    /// Strike price (`K`). Must be `> 0`.
    pub strike: f64,
    /// Continuously compounded annual risk-free rate (`r`), as a decimal.
    pub rate: f64,
    /// Continuously compounded annual dividend yield (`q`), as a decimal.
    #[serde(default = "f0")]
    pub dividend_yield: f64,
    /// Time to expiry in years (`T`). Must be `> 0`.
    pub time_to_expiry: f64,
    /// Call or put.
    #[serde(default)]
    pub option_type: OptionType,
}

const fn d100() -> usize {
    100
}

/// Inputs for the Cox-Ross-Rubinstein binomial tree, including the step count and
/// the exercise style (European or American). The continuous dividend yield is
/// honoured in the risk-neutral up-probability.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct BinomialParams {
    /// Current price of the underlying (`S`). Must be `> 0`.
    pub spot: f64,
    /// Strike price (`K`). Must be `> 0`.
    pub strike: f64,
    /// Continuously compounded annual risk-free rate (`r`), as a decimal.
    pub rate: f64,
    /// Continuously compounded annual dividend yield (`q`), as a decimal.
    #[serde(default = "f0")]
    pub dividend_yield: f64,
    /// Annualized volatility (`sigma`), as a decimal. Must be `> 0`.
    pub volatility: f64,
    /// Time to expiry in years (`T`). Must be `> 0`.
    pub time_to_expiry: f64,
    /// Call or put.
    #[serde(default)]
    pub option_type: OptionType,
    /// European or American exercise.
    #[serde(default)]
    pub exercise: ExerciseStyle,
    /// Number of tree time steps (`N`). Must be `>= 1`; more steps converge to
    /// the Black-Scholes value for the European case. Defaults to `100`.
    #[serde(default = "d100")]
    pub steps: usize,
}

const fn d10000() -> usize {
    10_000
}
const fn d42() -> u64 {
    42
}
const fn btrue() -> bool {
    true
}

/// Inputs for the seeded Monte-Carlo GBM European pricer.
///
/// The `seed` makes the price reproducible (no wall-clock / global RNG);
/// `antithetic` reduces variance by pairing each normal draw with its negation.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MonteCarloParams {
    /// Current price of the underlying (`S`). Must be `> 0`.
    pub spot: f64,
    /// Strike price (`K`). Must be `> 0`.
    pub strike: f64,
    /// Continuously compounded annual risk-free rate (`r`), as a decimal.
    pub rate: f64,
    /// Continuously compounded annual dividend yield (`q`), as a decimal.
    #[serde(default = "f0")]
    pub dividend_yield: f64,
    /// Annualized volatility (`sigma`), as a decimal. Must be `> 0`.
    pub volatility: f64,
    /// Time to expiry in years (`T`). Must be `> 0`.
    pub time_to_expiry: f64,
    /// Call or put.
    #[serde(default)]
    pub option_type: OptionType,
    /// Number of simulated terminal prices (`paths`). Must be `>= 1`. Defaults to
    /// `10_000`.
    #[serde(default = "d10000")]
    pub paths: usize,
    /// Deterministic RNG seed: the same seed reproduces the same price exactly.
    /// Defaults to `42`.
    #[serde(default = "d42")]
    pub seed: u64,
    /// Whether to use antithetic variates (each standard-normal draw `z` is paired
    /// with `-z`), reducing variance. Defaults to `true`.
    #[serde(default = "btrue")]
    pub antithetic: bool,
}

/// The full set of analytic Black-Scholes greeks for one contract.
///
/// Sign conventions (for a long position): call delta in `[0, e^{-qT}]`, put
/// delta in `[-e^{-qT}, 0]`; gamma and vega are non-negative for both; theta is
/// reported as the *per-year* time decay (typically negative); rho is positive
/// for calls and negative for puts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Greeks {
    /// Sensitivity of price to a unit change in spot (`dPrice/dS`).
    pub delta: f64,
    /// Sensitivity of delta to a unit change in spot (`d^2 Price / dS^2`).
    pub gamma: f64,
    /// Sensitivity of price to the passage of time, per year (`dPrice/dt`,
    /// reported as the calendar-time decay, usually negative).
    pub theta: f64,
    /// Sensitivity of price to a unit (1.0 = 100 vol points) change in volatility
    /// (`dPrice/dsigma`).
    pub vega: f64,
    /// Sensitivity of price to a unit change in the risk-free rate
    /// (`dPrice/dr`).
    pub rho: f64,
}

/// A Monte-Carlo price estimate plus its sampling standard error (the standard
/// deviation of the discounted payoff divided by `sqrt(paths)`), so callers can
/// judge the estimate's precision.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MonteCarloPrice {
    /// The Monte-Carlo price: the discounted mean simulated payoff.
    pub price: f64,
    /// The standard error of the price estimate.
    pub std_error: f64,
    /// The number of simulated payoff samples that produced the estimate (twice
    /// the path count when antithetic variates are enabled).
    pub samples: usize,
}

#[cfg(test)]
mod tests {
    use super::{BinomialParams, BlackScholesParams, ExerciseStyle, MonteCarloParams, OptionType};

    #[test]
    fn black_scholes_defaults_dividend_and_type() {
        let p: BlackScholesParams = serde_json::from_str(
            r#"{ "spot": 100, "strike": 100, "rate": 0.05, "volatility": 0.2, "time_to_expiry": 1 }"#,
        )
        .expect("deserializes with defaults");
        assert!((p.dividend_yield - 0.0).abs() < f64::EPSILON);
        assert_eq!(p.option_type, OptionType::Call);
    }

    #[test]
    fn binomial_defaults_steps_and_exercise() {
        let p: BinomialParams = serde_json::from_str(
            r#"{ "spot": 100, "strike": 100, "rate": 0.05, "volatility": 0.2, "time_to_expiry": 1 }"#,
        )
        .expect("deserializes");
        assert_eq!(p.steps, 100);
        assert_eq!(p.exercise, ExerciseStyle::European);
    }

    #[test]
    fn monte_carlo_defaults_paths_seed_antithetic() {
        let p: MonteCarloParams = serde_json::from_str(
            r#"{ "spot": 100, "strike": 100, "rate": 0.05, "volatility": 0.2, "time_to_expiry": 1 }"#,
        )
        .expect("deserializes");
        assert_eq!(p.paths, 10_000);
        assert_eq!(p.seed, 42);
        assert!(p.antithetic);
    }

    #[test]
    fn option_type_serializes_lowercase() {
        assert_eq!(
            serde_json::to_string(&OptionType::Put).expect("serializes"),
            "\"put\""
        );
    }
}
