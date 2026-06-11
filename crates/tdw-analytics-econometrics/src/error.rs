//! Econometric-estimator error type.

use thiserror::Error;

/// Failure modes for an econometric estimator.
///
/// The estimators validate their inputs up front (matching column counts,
/// enough rows for the requested degrees of freedom) and return a structured
/// error rather than panicking or producing a `NaN`-laced result. A design that
/// is rank-deficient (a constant regressor duplicated, perfect collinearity)
/// surfaces as [`EconometricsError::Singular`] from the Cholesky solve.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum EconometricsError {
    /// The response vector `y` and the design matrix `X` disagree on the number
    /// of observations (rows).
    #[error("row mismatch: y has {y} observations, X has {x}")]
    RowMismatch {
        /// Number of observations in `y`.
        y: usize,
        /// Number of observation rows in `X`.
        x: usize,
    },

    /// The design matrix had no columns, or the rows had inconsistent widths.
    #[error("design matrix is empty or ragged")]
    EmptyDesign,

    /// There were not enough observations to estimate the model with positive
    /// residual degrees of freedom (`n > k` is required).
    #[error("insufficient observations: {rows} rows for {cols} parameters (need rows > cols)")]
    InsufficientRows {
        /// Number of observation rows.
        rows: usize,
        /// Number of parameters (design columns).
        cols: usize,
    },

    /// The normal-equations Gram matrix `XᵀX` was not positive-definite, so the
    /// Cholesky factorization failed — the regressors are (numerically)
    /// collinear / rank-deficient.
    #[error("normal-equations matrix is singular (collinear regressors)")]
    Singular,

    /// A paired-series estimator (correlation, Granger, cointegration) received
    /// series of differing lengths.
    #[error("series length mismatch: {first} vs {second}")]
    LengthMismatch {
        /// Length of the first series.
        first: usize,
        /// Length of the second series.
        second: usize,
    },
}
