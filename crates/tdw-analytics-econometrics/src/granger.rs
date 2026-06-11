//! Granger-causality test (Granger 1969).
//!
//! Tests whether the lagged values of `x` improve the one-step prediction of `y`
//! beyond `y`'s own lags. Two nested OLS models are fit over the aligned sample
//! that drops the first `lag` observations:
//!
//! - **Restricted**: `yₜ = c + Σ_{i=1}^{lag} α_i y_{t−i} + ε`.
//! - **Unrestricted**: the restricted model plus `Σ_{i=1}^{lag} β_i x_{t−i}`.
//!
//! The null "`x` does not Granger-cause `y`" is `β_1 = … = β_lag = 0`, tested
//! with the `F`-statistic comparing the two residual sums of squares:
//!
//! `F = ((RSS_r − RSS_u)/q) / (RSS_u/(n − k_u))`,
//!
//! where `q = lag` restrictions, `n` is the aligned sample size, and `k_u` is
//! the unrestricted parameter count. A large `F` rejects the null (evidence of
//! Granger causality). Only the F-statistic and its degrees of freedom are
//! reported — the F-distribution p-value needs an incomplete-beta evaluation
//! that this dependency-free crate intentionally omits (documented vs `OpenBB`,
//! which surfaces the p-value).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::EconometricsError;
use crate::ols::{OlsSummary, design_from_columns, ols};

#[allow(clippy::cast_precision_loss)]
const fn usize_to_f64(n: usize) -> f64 {
    n as f64
}

/// Granger-causality test result: the F-statistic for the joint exclusion of the
/// `x`-lags, its numerator/denominator degrees of freedom, and the two residual
/// sums of squares the test compares.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct GrangerResult {
    /// The F-statistic testing that all `x`-lag coefficients are zero.
    pub f_statistic: f64,
    /// Numerator degrees of freedom (`q = lag` restrictions).
    pub df_numerator: usize,
    /// Denominator degrees of freedom (`n − k_unrestricted`).
    pub df_denominator: usize,
    /// Restricted-model residual sum of squares (own lags only).
    pub rss_restricted: f64,
    /// Unrestricted-model residual sum of squares (own + `x` lags).
    pub rss_unrestricted: f64,
}

/// Build the lagged-regressor columns for a target series, dropping the first
/// `lag` rows so every column is aligned to the same `n − lag` sample.
///
/// Returns `lag` columns; column `i` (0-based) holds `series[t − (i + 1)]` for
/// `t = lag .. n`.
fn lag_columns(series: &[f64], lag: usize) -> Vec<Vec<f64>> {
    let n = series.len();
    (1..=lag)
        .map(|shift| series[(lag - shift)..(n - shift)].to_vec())
        .collect()
}

/// Run the Granger-causality test of `x → y` at the given `lag`.
///
/// # Errors
///
/// - [`EconometricsError::LengthMismatch`] when `x` and `y` differ in length.
/// - [`EconometricsError::EmptyDesign`] when `lag == 0`.
/// - [`EconometricsError::InsufficientRows`] (propagated from OLS) when there
///   are too few aligned observations for the unrestricted model.
pub fn granger_causality(
    y: &[f64],
    x: &[f64],
    lag: usize,
) -> Result<GrangerResult, EconometricsError> {
    if y.len() != x.len() {
        return Err(EconometricsError::LengthMismatch {
            first: y.len(),
            second: x.len(),
        });
    }
    if lag == 0 {
        return Err(EconometricsError::EmptyDesign);
    }
    let n = y.len();
    if n <= lag {
        return Err(EconometricsError::InsufficientRows { rows: n, cols: lag });
    }

    // The aligned response drops the first `lag` rows.
    let response: Vec<f64> = y[lag..].to_vec();
    let y_lags = lag_columns(y, lag);
    let x_lags = lag_columns(x, lag);

    // Restricted: own lags only.
    let restricted_design = design_from_columns(&y_lags, true)?;
    let restricted: OlsSummary = ols(&response, &restricted_design)?;
    let rss_r = residual_sum_of_squares(&response, &y_lags, &restricted);

    // Unrestricted: own lags + x lags.
    let mut unrestricted_cols = y_lags;
    unrestricted_cols.extend(x_lags);
    let unrestricted_design = design_from_columns(&unrestricted_cols, true)?;
    let unrestricted: OlsSummary = ols(&response, &unrestricted_design)?;
    let rss_u = residual_sum_of_squares(&response, &unrestricted_cols, &unrestricted);

    let df_num = lag;
    let df_den = unrestricted.residual_dof;
    let f_statistic = if rss_u > 0.0 && df_den > 0 {
        let num = (rss_r - rss_u) / usize_to_f64(df_num);
        let den = rss_u / usize_to_f64(df_den);
        if den > 0.0 { num / den } else { 0.0 }
    } else {
        0.0
    };

    Ok(GrangerResult {
        f_statistic,
        df_numerator: df_num,
        df_denominator: df_den,
        rss_restricted: rss_r,
        rss_unrestricted: rss_u,
    })
}

/// Recompute the residual sum of squares of a fitted model by reconstructing the
/// design from `columns` (intercept-prepended) and the fitted coefficients.
fn residual_sum_of_squares(response: &[f64], columns: &[Vec<f64>], fit: &OlsSummary) -> f64 {
    let betas: Vec<f64> = fit.coefficients.iter().map(|c| c.estimate).collect();
    let mut rss = 0.0;
    for (row, &actual) in response.iter().enumerate() {
        // Intercept first, then each column at this row.
        let mut predicted = betas[0];
        for (col_index, col) in columns.iter().enumerate() {
            predicted += betas[col_index + 1] * col[row];
        }
        let resid = actual - predicted;
        rss += resid * resid;
    }
    rss
}

#[cfg(test)]
mod tests {
    use super::{granger_causality, lag_columns};

    #[test]
    fn lag_columns_align_and_shrink() {
        // series [10,20,30,40,50], lag 2 ⇒ two columns of length 3.
        // col for shift 1: series[1..4] = [20,30,40];
        // col for shift 2: series[0..3] = [10,20,30].
        let cols = lag_columns(&[10.0, 20.0, 30.0, 40.0, 50.0], 2);
        assert_eq!(cols.len(), 2);
        assert_eq!(cols[0], vec![20.0, 30.0, 40.0]);
        assert_eq!(cols[1], vec![10.0, 20.0, 30.0]);
    }

    #[test]
    fn granger_runs_and_orders_rss() {
        // The unrestricted model nests the restricted one, so RSS_u ≤ RSS_r and
        // the F-statistic is finite and non-negative for any data.
        let y = vec![1.0, 2.0, 1.5, 3.0, 2.5, 4.0, 3.5, 5.0, 4.5, 6.0];
        let x = vec![0.5, 1.0, 0.8, 1.6, 1.2, 2.0, 1.7, 2.5, 2.1, 3.0];
        let result = granger_causality(&y, &x, 1).expect("granger");
        assert!(result.rss_unrestricted <= result.rss_restricted + 1e-9);
        assert!(result.f_statistic >= 0.0);
        assert_eq!(result.df_numerator, 1);
    }

    #[test]
    fn granger_length_mismatch_errors() {
        assert!(granger_causality(&[1.0, 2.0, 3.0], &[1.0, 2.0], 1).is_err());
    }

    #[test]
    fn granger_zero_lag_errors() {
        assert!(granger_causality(&[1.0, 2.0, 3.0], &[1.0, 2.0, 3.0], 0).is_err());
    }
}
