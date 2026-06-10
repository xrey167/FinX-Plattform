//! Indicator error type.

use thiserror::Error;

/// Failure modes for an indicator computation.
///
/// Indicators never panic on short or empty input — they return a column of the
/// input length filled with `None` instead. The only error condition is a
/// caller-supplied parameter that cannot describe a valid window (e.g. a zero
/// period), which is a validation error rather than a data condition.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum IndicatorError {
    /// A period/length parameter was zero (every indicator window must span at
    /// least one bar).
    #[error("indicator period must be >= 1")]
    InvalidPeriod,
}
