//! Portfolio-metric error type.

use thiserror::Error;

/// Failure modes for a portfolio-metric computation.
///
/// Metrics do not panic on short or empty input — a metric that is undefined for
/// the supplied series (e.g. a drawdown over an empty equity curve) returns an
/// empty / sentinel typed output rather than erroring. The error conditions here
/// are caller-supplied inputs that cannot describe a valid computation: a
/// paired-series metric (`contribution`) whose weights and returns differ in
/// length, or an `allocation` whose position values sum to zero (so they cannot
/// be normalized to weights).
#[derive(Debug, Error, PartialEq, Eq)]
pub enum PortfolioError {
    /// A paired-series metric (`contribution`) received `weights` and `returns`
    /// whose lengths differ; the two must be positionally aligned (one weight
    /// per asset return).
    #[error("series length mismatch: weights has {weights} entries, returns has {returns}")]
    LengthMismatch {
        /// Number of weights supplied.
        weights: usize,
        /// Number of per-asset returns supplied.
        returns: usize,
    },

    /// `allocation` received position values whose sum is zero, so they cannot be
    /// normalized to weights (the normalization divides by the total).
    #[error("position values sum to zero; cannot normalize to weights")]
    ZeroTotal,
}
