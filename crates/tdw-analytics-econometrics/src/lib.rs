#![forbid(unsafe_code)]
#![deny(clippy::pedantic, clippy::nursery)]
//! Pure-Rust regression and econometric tests (gap-matrix item **L4.3**).
//!
//! Hand-rolled, offline implementations of the common `OpenBB`-parity
//! econometric estimators over caller-supplied series and design matrices. The
//! crate performs no I/O and no async work: it is a deterministic numeric
//! library that a daemon compute route or a UDF can call directly.
//!
//! # Estimators
//!
//! - [`ols::ols`] — ordinary least squares with coefficient standard errors,
//!   t-statistics, `R²` / adjusted `R²`, the overall `F`-statistic, and the
//!   Durbin-Watson residual diagnostic.
//! - [`correlation::correlation_matrix`] — the Pearson correlation matrix of a
//!   set of columns.
//! - [`correlation::vif`] — variance-inflation factors via auxiliary-regression
//!   `R²`.
//! - [`granger::granger_causality`] — the Granger (1969) `F`-test comparing a
//!   restricted (own-lags) and unrestricted (own + `x`-lags) OLS model.
//! - [`cointegration::engle_granger`] — Engle-Granger (1987) step one (the
//!   cointegrating regression) plus a documented residual stationarity score.
//!
//! # Numeric approach
//!
//! All regression machinery routes through one solver: the normal equations
//! `(XᵀX) β = Xᵀy` factored by a hand-rolled Cholesky decomposition (see
//! [`linalg`]). There is **no** third-party linear-algebra dependency (no
//! `nalgebra` / `faer` / `ndarray`); the workspace pulls none and this crate
//! adds none. The normal-equations route squares the design's condition number,
//! so [`linalg`] documents the conditioning trade-off and *detects* a
//! rank-deficient design (the Cholesky factorization fails ⇒
//! [`EconometricsError::Singular`]) rather than returning silently wrong
//! coefficients.
//!
//! # Honest simplifications (vs `OpenBB`)
//!
//! - **Granger causality** reports the `F`-statistic and its degrees of freedom
//!   but not the F-distribution p-value (which needs an incomplete-beta
//!   evaluation this crate omits).
//! - **Cointegration** implements Engle-Granger step one exactly and scores
//!   residual stationarity with a named Dickey-Fuller `ρ` slope + t-statistic
//!   rather than a `MacKinnon`-table p-value. See [`cointegration`] for the full
//!   rationale.
//! - **Formal unit-root tests** (a standalone ADF / KPSS route) are deliberately
//!   out of scope: ADF needs the same lag-augmentation regression and
//!   critical-value tables the cointegration module already documents away, and
//!   KPSS needs a long-run-variance estimator. The residual stationarity score
//!   inside [`cointegration`] is the one unit-root-flavored statistic shipped.
//!
//! # Clean-room provenance
//!
//! Every formula here is textbook math cited to its standard definition in the
//! owning module's docs (the Gauss-Markov / normal-equations OLS; Durbin &
//! Watson 1950; the variance-inflation-factor definition; Granger 1969;
//! Engle & Granger 1987; Dickey-Fuller for the residual regression). Golden-test
//! expectations are hand-derived from these formulas over tiny worked examples
//! (cited in the test comments). No reference implementation was consulted.

pub mod cointegration;
pub mod correlation;
pub mod error;
pub mod granger;
pub mod linalg;
pub mod ols;
pub mod params;

pub use error::EconometricsError;

/// Result type for econometric computations.
pub type Result<T> = std::result::Result<T, EconometricsError>;
