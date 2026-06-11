//! Quantitative-metric error type.

use thiserror::Error;

/// Failure modes for a quantitative-metric computation.
///
/// Metrics do not panic on short or empty input — a metric that is undefined for
/// the supplied series (e.g. a standard deviation of fewer than two points)
/// returns a `None`/`NaN`-free sentinel through its typed output rather than
/// erroring. The error conditions here are caller-supplied parameters that
/// cannot describe a valid computation (a non-positive annualization factor, a
/// benchmark whose length does not match the asset series).
#[derive(Debug, Error, PartialEq, Eq)]
pub enum QuantError {
    /// The annualization period factor was not strictly positive (it scales a
    /// per-period statistic to an annual one and must be `> 0`).
    #[error("annualization periods must be > 0")]
    InvalidPeriods,

    /// A paired-series metric (CAPM, beta) received a benchmark whose length
    /// differs from the asset series; the two must be positionally aligned.
    #[error("series length mismatch: asset has {asset} points, benchmark has {benchmark}")]
    LengthMismatch {
        /// Number of points in the asset series.
        asset: usize,
        /// Number of points in the benchmark series.
        benchmark: usize,
    },
}
