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
//! - [`diagnostics::augmented_dickey_fuller`] — the ADF unit-root regression
//!   (statistic and `γ`, no critical-value-table p-value; same documented
//!   simplification as cointegration).
//! - [`diagnostics::breusch_godfrey`] — the Breusch-Godfrey serial-correlation
//!   `LM = n·R²` test of the regression residuals.
//! - [`panel`] — the long-format panel family (pooled OLS, within / fixed
//!   effects, between, first difference, Swamy-Arora random effects, and the
//!   Fama-MacBeth two-pass estimator), each routing its final solve through the
//!   same OLS core.
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
//! - **ADF** ([`diagnostics::augmented_dickey_fuller`]) ships the
//!   lag-augmentation regression and reports the `γ` t-statistic, but not the
//!   `MacKinnon`-table p-value (the same critical-value-table omission the
//!   cointegration module documents). A formal **KPSS** test remains out of scope
//!   (it needs a long-run-variance estimator).
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
pub mod diagnostics;
pub mod error;
pub mod granger;
pub mod linalg;
pub mod ols;
pub mod panel;
pub mod params;

pub use error::EconometricsError;

/// Result type for econometric computations.
pub type Result<T> = std::result::Result<T, EconometricsError>;
