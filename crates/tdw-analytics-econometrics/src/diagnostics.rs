//! Regression residual diagnostics: the augmented Dickey-Fuller unit-root test
//! and the Breusch-Godfrey serial-correlation test.
//!
//! Both reuse the crate's single OLS solver (see [`crate::ols`]). Like the rest
//! of the crate they report the test statistic and its degrees of freedom but
//! not a distribution p-value (the ADF and `LM` reference distributions need
//! critical-value tables / an incomplete-gamma evaluation this dependency-free
//! crate intentionally omits — the same documented simplification the
//! [`crate::cointegration`] module makes).
//!
//! # Definitions
//!
//! - **Augmented Dickey-Fuller** (Dickey & Fuller 1979; Said & Dickey 1984):
//!   regress `Δyₜ = α + γ·y_{t−1} + Σ_{i=1}^{p} δ_i·Δy_{t−i} + e` and report the
//!   t-statistic of `γ`. A strongly negative t-stat rejects the unit-root null
//!   (the series is stationary); near `0` indicates a unit root. The `γ` estimate
//!   and the augmentation lag order are reported alongside.
//! - **Breusch-Godfrey** (Breusch 1978; Godfrey 1978): fit the primary OLS, then
//!   regress its residuals `ε̂ₜ` on the original regressors plus `p` lagged
//!   residuals `ε̂_{t−i}`. The `LM = n·R²` statistic of that auxiliary regression
//!   is asymptotically χ²(p) under the no-serial-correlation null. The auxiliary
//!   `R²`, `n`, and the lag order `p` are reported so the statistic is fully
//!   reconstructible.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::EconometricsError;
use crate::linalg::Matrix;
use crate::ols::{Coefficient, design_from_columns, ols};

#[allow(clippy::cast_precision_loss)]
const fn usize_to_f64(n: usize) -> f64 {
    n as f64
}

/// Augmented Dickey-Fuller unit-root test result.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct UnitRootResult {
    /// Estimated `γ` (the coefficient on the lagged level `y_{t−1}`). Negative
    /// under stationarity; near `0` under a unit root.
    pub gamma: f64,
    /// t-statistic of `γ`; compare against published ADF critical values out of
    /// band (this crate ships no critical-value tables). Strongly negative ⇒
    /// reject the unit-root null.
    pub t_statistic: f64,
    /// Augmentation lag order `p` (number of lagged-difference terms included).
    pub lags: usize,
    /// Number of aligned observations the test regression was fit over.
    pub n_obs: usize,
}

/// Breusch-Godfrey serial-correlation test result.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ResidualAutocorrelationResult {
    /// The Lagrange-multiplier statistic `LM = n·R²` of the auxiliary regression;
    /// asymptotically χ²(`lags`) under the no-autocorrelation null. Large values
    /// reject (residual serial correlation present).
    pub lm_statistic: f64,
    /// `R²` of the auxiliary residual regression.
    pub r_squared: f64,
    /// Degrees of freedom of the LM statistic (equal to `lags`).
    pub df: usize,
    /// Durbin-Watson statistic of the primary regression residuals (carried for
    /// convenience; `≈ 2` ⇒ no first-order autocorrelation).
    pub durbin_watson: f64,
}

/// Run the augmented Dickey-Fuller unit-root test on `series` with `lags`
/// lagged-difference augmentation terms.
///
/// Fits `Δyₜ = α + γ·y_{t−1} + Σ δ_i·Δy_{t−i} + e` by OLS and returns the `γ`
/// estimate and its t-statistic.
///
/// # Errors
///
/// - [`EconometricsError::InsufficientRows`] when the series is too short for the
///   requested lag order to leave residual degrees of freedom.
/// - [`EconometricsError::Singular`] (propagated) when the test design is
///   rank-deficient.
pub fn augmented_dickey_fuller(
    series: &[f64],
    lags: usize,
) -> Result<UnitRootResult, EconometricsError> {
    let n = series.len();
    // The test regresses Δyₜ (t = 1..n) on y_{t−1} and `lags` lagged differences.
    // The first usable observation is at index `lags + 1` (it needs y_{t−1} and
    // Δy_{t−1..t−lags}). The aligned sample size is therefore n − lags − 1.
    if n < lags + 3 {
        return Err(EconometricsError::InsufficientRows {
            rows: n,
            cols: lags + 2,
        });
    }

    let diffs: Vec<f64> = (1..n).map(|t| series[t] - series[t - 1]).collect();
    // diffs[i] corresponds to Δy_{i+1}. Build the aligned sample for t = lags+1..n.
    let start = lags; // index into diffs for the first aligned Δyₜ
    let response: Vec<f64> = diffs[start..].to_vec();

    let mut columns: Vec<Vec<f64>> = Vec::with_capacity(lags + 1);
    // Lagged level y_{t−1}: for aligned Δyₜ (t = start+1 in series terms), the
    // level term is series[t−1]. diffs[start] is Δy_{start+1}, so y_{t−1} =
    // series[start].
    let level: Vec<f64> = (start..(n - 1)).map(|t| series[t]).collect();
    columns.push(level);
    // Lagged differences Δy_{t−i} for i = 1..=lags.
    for i in 1..=lags {
        let lag_col: Vec<f64> = (start..diffs.len()).map(|t| diffs[t - i]).collect();
        columns.push(lag_col);
    }

    let design = design_from_columns(&columns, true)?;
    let fit = ols(&response, &design)?;
    // Coefficient order: intercept, γ (level), then the δ lagged-diff terms.
    let gamma = fit.coefficients[1].estimate;
    let t_statistic = fit.coefficients[1].t_stat;

    Ok(UnitRootResult {
        gamma,
        t_statistic,
        lags,
        n_obs: response.len(),
    })
}

/// Run the Breusch-Godfrey serial-correlation test for the OLS of `y` on the
/// design built from `columns` (intercept-prepended when `intercept`), testing
/// `lags` orders of residual autocorrelation.
///
/// # Errors
///
/// - [`EconometricsError::EmptyDesign`] when `lags == 0` or the design is empty.
/// - [`EconometricsError::InsufficientRows`] / [`EconometricsError::Singular`]
///   (propagated) when a regression cannot be fit.
pub fn breusch_godfrey(
    y: &[f64],
    columns: &[Vec<f64>],
    intercept: bool,
    lags: usize,
) -> Result<ResidualAutocorrelationResult, EconometricsError> {
    if lags == 0 {
        return Err(EconometricsError::EmptyDesign);
    }
    let design = design_from_columns(columns, intercept)?;
    let primary = ols(y, &design)?;
    let residuals = residuals_of(y, &design, &primary.coefficients);

    let n = residuals.len();
    if n <= lags {
        return Err(EconometricsError::InsufficientRows {
            rows: n,
            cols: lags,
        });
    }

    // Auxiliary regression: ε̂ₜ on the original regressors + `lags` lagged
    // residuals, over the aligned sample dropping the first `lags` rows. Missing
    // pre-sample lagged residuals are set to zero (the standard BG convention),
    // so the auxiliary sample keeps all `n` rows.
    let aux_response = residuals.clone();
    let mut aux_columns: Vec<Vec<f64>> = columns.to_vec();
    for i in 1..=lags {
        let lag_col: Vec<f64> = (0..n)
            .map(|t| if t >= i { residuals[t - i] } else { 0.0 })
            .collect();
        aux_columns.push(lag_col);
    }
    let aux_design = design_from_columns(&aux_columns, intercept)?;
    let aux = ols(&aux_response, &aux_design)?;

    let lm_statistic = usize_to_f64(n) * aux.r_squared;

    Ok(ResidualAutocorrelationResult {
        lm_statistic,
        r_squared: aux.r_squared,
        df: lags,
        durbin_watson: primary.durbin_watson,
    })
}

/// Reconstruct the OLS residuals `y − X β̂` from a fitted coefficient vector.
fn residuals_of(y: &[f64], design: &Matrix, coefficients: &[Coefficient]) -> Vec<f64> {
    let beta: Vec<f64> = coefficients.iter().map(|c| c.estimate).collect();
    let fitted = design.mul_vec(&beta);
    y.iter()
        .zip(fitted.iter())
        .map(|(yi, fi)| yi - fi)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{augmented_dickey_fuller, breusch_godfrey};

    #[test]
    fn adf_rejects_for_a_mean_reverting_ar1_series() {
        // A noisy mean-reverting AR(1): yₜ = 0.2·y_{t−1} + eₜ (|ρ| < 1) generated
        // deterministically here. Mean reversion ⇒ the DF γ on y_{t−1} is
        // negative with a negative t-statistic, and the small zig-zag innovation
        // keeps the regression residual variance non-zero (so the t-stat is a real
        // finite value, not the perfect-fit sentinel).
        let innovations = [
            0.5, -0.3, 0.4, -0.2, 0.35, -0.25, 0.3, -0.15, 0.45, -0.35, 0.2, -0.1, 0.25, -0.2,
        ];
        let mut series = vec![0.0_f64];
        for &innovation in &innovations {
            let prev = *series.last().expect("nonempty");
            series.push(0.2_f64.mul_add(prev, innovation));
        }
        let result = augmented_dickey_fuller(&series, 1).expect("adf");
        assert!(
            result.gamma < 0.0,
            "gamma should be negative for a mean-reverting series, got {}",
            result.gamma
        );
        assert!(
            result.t_statistic < 0.0,
            "t-stat should be negative, got {}",
            result.t_statistic
        );
        assert_eq!(result.lags, 1);
    }

    #[test]
    fn adf_short_series_errors() {
        assert!(augmented_dickey_fuller(&[1.0, 2.0, 3.0], 2).is_err());
    }

    #[test]
    fn breusch_godfrey_runs_and_reports_nonnegative_lm() {
        // y = 1 + 2x + alternating residual ⇒ the auxiliary regression has a
        // well-defined, non-negative LM statistic in [0, n].
        let x = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0];
        let noise = [0.2, -0.2, 0.2, -0.2, 0.2, -0.2, 0.2, -0.2, 0.2, -0.2];
        let y: Vec<f64> = x
            .iter()
            .zip(noise.iter())
            .map(|(xi, e)| 1.0 + 2.0 * xi + e)
            .collect();
        let n = y.len();
        let result = breusch_godfrey(&y, &[x], true, 1).expect("bg");
        assert!(result.lm_statistic >= 0.0);
        // LM = n·R², bounded by n since R² ∈ [0, 1].
        assert!(result.lm_statistic <= u32::try_from(n).map(f64::from).expect("small n") + 1e-9);
        assert_eq!(result.df, 1);
        assert!(result.durbin_watson >= 0.0 && result.durbin_watson <= 4.0);
    }

    #[test]
    fn breusch_godfrey_zero_lags_errors() {
        assert!(breusch_godfrey(&[1.0, 2.0, 3.0], &[vec![1.0, 2.0, 3.0]], true, 0).is_err());
    }
}
