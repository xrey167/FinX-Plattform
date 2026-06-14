//! Error type for the classical-forecasting computations.

use thiserror::Error;

/// An error from a forecasting, backtesting, or anomaly-detection computation.
#[derive(Clone, Debug, PartialEq, Error)]
pub enum ForecastError {
    /// The input series was empty when at least one observation is required.
    #[error("forecast: the input series is empty")]
    EmptySeries,

    /// The input series was too short for the requested operation (e.g. fewer
    /// observations than the seasonal period, or not enough history to form the
    /// requested lag features or backtest folds). `needed` is the minimum length
    /// the operation requires; `got` is the actual length.
    #[error("forecast: series too short: need at least {needed} observations, got {got}")]
    TooShort {
        /// Minimum number of observations the operation requires.
        needed: usize,
        /// Actual number of observations supplied.
        got: usize,
    },

    /// A requested horizon (`steps`) was zero where a positive horizon is
    /// required.
    #[error("forecast: the forecast horizon must be at least 1, got 0")]
    ZeroHorizon,

    /// A seasonal period parameter was invalid (zero, or one — a period of one
    /// has no seasonal structure).
    #[error("forecast: the seasonal period must be at least 2, got {0}")]
    InvalidPeriod(usize),

    /// Two series that must be the same length were not (e.g. the actuals and the
    /// forecasts handed to a precision metric).
    #[error("forecast: length mismatch: {a} vs {b}")]
    LengthMismatch {
        /// Length of the first series.
        a: usize,
        /// Length of the second series.
        b: usize,
    },

    /// A smoothing or model parameter fell outside its valid range (the
    /// exponential-smoothing coefficients must lie in `[0, 1]`; a quantile must
    /// lie in `[0, 1]`).
    #[error("forecast: parameter `{name}` out of range: {value}")]
    ParamOutOfRange {
        /// Name of the offending parameter.
        name: &'static str,
        /// The out-of-range value supplied.
        value: f64,
    },

    /// The underlying OLS solve failed (a rank-deficient lag-feature design, or
    /// too few rows). Carries the econometrics error's message.
    #[error("forecast: regression solve failed: {0}")]
    Regression(String),
}
