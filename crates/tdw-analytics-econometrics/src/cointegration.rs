//! Engle-Granger cointegration step one, with a documented honest simplification.
//!
//! The Engle-Granger (1987) two-step procedure regresses one I(1) series on the
//! other and tests the regression residuals for stationarity. This crate
//! implements **step one in full** — the cointegrating OLS regression
//! `yₜ = α + β·xₜ + uₜ` — and, for the residual stationarity check, reports a
//! transparent **residual stationarity score** rather than a formal augmented
//! Dickey-Fuller test with `MacKinnon` critical-value tables.
//!
//! # Honest simplification (vs `OpenBB`)
//!
//! A formal Engle-Granger test surfaces an ADF statistic on the residuals
//! together with a p-value interpolated from `MacKinnon`'s response-surface
//! tables. Those tables (and the ADF lag-augmentation regression's
//! distribution) are reference data we do not embed in this dependency-free,
//! clean-room crate. Instead the score reported here is the **slope of the
//! Dickey-Fuller residual regression** `Δuₜ = ρ·u_{t−1} + e`: the OLS estimate
//! of `ρ`. Under stationarity `ρ` is significantly negative (the residual is
//! mean-reverting); near `0` indicates a unit root (no cointegration). The
//! companion t-statistic of `ρ` is also reported so a caller can compare it
//! against published ADF critical values *out of band* if they wish. This is the
//! minimal honest version: the cointegrating regression is exact, and the
//! stationarity evidence is a named, well-defined statistic — not a p-value we
//! cannot defensibly compute here. `OpenBB`'s richer p-value output is the
//! documented gap.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::EconometricsError;
use crate::ols::{design_from_columns, ols};

/// Engle-Granger step-one result: the cointegrating regression coefficients plus
/// the residual stationarity score (the Dickey-Fuller `ρ` slope and its t-stat).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CointegrationResult {
    /// Intercept `α` of the cointegrating regression `y = α + β·x + u`.
    pub alpha: f64,
    /// Slope `β` of the cointegrating regression (the cointegrating vector's
    /// loading on `x`).
    pub beta: f64,
    /// `R²` of the cointegrating regression (how tightly the two series move
    /// together in levels).
    pub r_squared: f64,
    /// Residual stationarity score: the Dickey-Fuller slope `ρ` from
    /// `Δuₜ = ρ·u_{t−1} + e`. Strongly negative ⇒ mean-reverting residual ⇒
    /// evidence of cointegration; near `0` ⇒ a residual unit root ⇒ no
    /// cointegration.
    pub residual_df_rho: f64,
    /// t-statistic of `ρ` (compare against published ADF critical values out of
    /// band; this crate ships no critical-value tables).
    pub residual_df_tstat: f64,
}

/// Run the Engle-Granger step-one cointegration regression of `y` on `x` and
/// score the residual stationarity.
///
/// # Errors
///
/// - [`EconometricsError::LengthMismatch`] when `y` and `x` differ in length.
/// - [`EconometricsError::InsufficientRows`] (propagated) when there are too few
///   observations for the cointegrating regression or the residual
///   Dickey-Fuller regression.
/// - [`EconometricsError::Singular`] (propagated) when a regression design is
///   rank-deficient.
pub fn engle_granger(y: &[f64], x: &[f64]) -> Result<CointegrationResult, EconometricsError> {
    if y.len() != x.len() {
        return Err(EconometricsError::LengthMismatch {
            first: y.len(),
            second: x.len(),
        });
    }

    // Step one: cointegrating regression y = α + β·x + u.
    let design = design_from_columns(&[x.to_vec()], true)?;
    let fit = ols(y, &design)?;
    let alpha = fit.coefficients[0].estimate;
    let beta = fit.coefficients[1].estimate;

    // Residuals uₜ = yₜ − (α + β·xₜ).
    let residuals: Vec<f64> = y
        .iter()
        .zip(x.iter())
        .map(|(yi, xi)| yi - (alpha + beta * xi))
        .collect();

    // Residual Dickey-Fuller regression Δuₜ = ρ·u_{t−1} + e (no intercept: the
    // residuals of an intercept-bearing regression are mean-zero by
    // construction). Aligns Δu (rows 1..n) with the lagged level u (rows 0..n-1).
    let n = residuals.len();
    let delta: Vec<f64> = (1..n).map(|t| residuals[t] - residuals[t - 1]).collect();
    let lagged: Vec<f64> = residuals[..n - 1].to_vec();

    let df_design = design_from_columns(&[lagged], false)?;
    let df_fit = ols(&delta, &df_design)?;
    let rho = df_fit.coefficients[0].estimate;
    let tstat = df_fit.coefficients[0].t_stat;

    Ok(CointegrationResult {
        alpha,
        beta,
        r_squared: fit.r_squared,
        residual_df_rho: rho,
        residual_df_tstat: tstat,
    })
}

#[cfg(test)]
mod tests {
    use super::engle_granger;

    #[test]
    fn cointegrating_regression_recovers_levels_relationship() {
        // y = 1 + 2x + small zig-zag noise; step-one OLS recovers α≈1, β≈2.
        let x = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];
        let noise = [0.1, -0.1, 0.1, -0.1, 0.1, -0.1, 0.1, -0.1];
        let y: Vec<f64> = x
            .iter()
            .zip(noise.iter())
            .map(|(xi, e)| 1.0 + 2.0 * xi + e)
            .collect();
        let result = engle_granger(&y, &x).expect("engle-granger");
        assert!((result.beta - 2.0).abs() < 0.05, "beta {}", result.beta);
        assert!((result.alpha - 1.0).abs() < 0.1, "alpha {}", result.alpha);
        // The residual is a tight stationary zig-zag ⇒ strongly negative ρ.
        assert!(
            result.residual_df_rho < 0.0,
            "rho should be negative (mean-reverting), got {}",
            result.residual_df_rho
        );
    }

    #[test]
    fn cointegration_length_mismatch_errors() {
        assert!(engle_granger(&[1.0, 2.0, 3.0], &[1.0, 2.0]).is_err());
    }
}
