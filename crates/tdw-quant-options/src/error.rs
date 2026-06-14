//! Option-pricing error type.
//!
//! Pricing functions reject caller-supplied contract inputs that cannot describe
//! a valid computation (a non-positive spot/strike/volatility/time, a zero or
//! negative step count, a Monte-Carlo path count of zero) and report a
//! convergence failure when the implied-volatility solver cannot bracket a root.
//! They do not panic on bad input.

use thiserror::Error;

/// Failure modes for an option-pricing computation.
#[derive(Debug, Error, PartialEq)]
pub enum OptionError {
    /// A required positive contract input was not strictly positive. The field
    /// names the offending input (e.g. `"spot"`, `"strike"`, `"volatility"`,
    /// `"time_to_expiry"`).
    #[error("`{field}` must be > 0 (got {value})")]
    NonPositive {
        /// The offending input field name.
        field: &'static str,
        /// The rejected value.
        value: f64,
    },

    /// The binomial tree received a step count of zero (it must take at least one
    /// step to span the time to expiry).
    #[error("binomial `steps` must be >= 1 (got {0})")]
    InvalidSteps(usize),

    /// The Monte-Carlo pricer received a path count of zero (it must simulate at
    /// least one path).
    #[error("monte-carlo `paths` must be >= 1 (got {0})")]
    InvalidPaths(usize),

    /// The implied-volatility solver could not converge: the supplied market
    /// price is outside the no-arbitrage bounds for the contract (so no positive
    /// volatility reproduces it), or the bracketed search exhausted its iteration
    /// budget without reaching the price tolerance.
    #[error("implied-volatility solver did not converge: {0}")]
    ImpliedVolNoConvergence(&'static str),
}
