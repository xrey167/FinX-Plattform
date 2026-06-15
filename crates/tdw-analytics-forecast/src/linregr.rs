//! Linear-regression forecast with autoregressive lag features and optional
//! exogenous covariates.
//!
//! Fits `y_t = b0 + Σ_i b_i·y_{t−i} (+ Σ_j c_j·x_{j,t})` by ordinary least
//! squares — **reusing the OLS core from `tdw-analytics-econometrics`** rather
//! than re-implementing it — then rolls the fitted model forward one step at a
//! time. Each forecast step appends its own prediction to the lag history so the
//! next step's lag features are well-defined (recursive multi-step forecasting).
//! Exogenous covariates, when supplied, must carry their future path so the
//! covariate term is available at forecast time.

use tdw_analytics_econometrics::ols::{design_from_columns, ols};

use crate::error::ForecastError;
use crate::params::{ForecastPoint, ForecastResult, LinRegrParams};

/// Forecast by an OLS regression on `lags` autoregressive features plus optional
/// exogenous covariate columns.
///
/// `covariates`, when non-empty, each have length `y.len() + horizon`: the
/// leading `y.len()` entries align to history, the trailing `horizon` entries
/// supply the future covariate path.
///
/// # Errors
///
/// [`ForecastError::ZeroHorizon`] when `horizon == 0`; [`ForecastError::TooShort`]
/// when the series has fewer than `lags + 2` observations (too few training
/// rows); [`ForecastError::LengthMismatch`] when a covariate column has the wrong
/// length; [`ForecastError::Regression`] when the OLS solve fails (e.g. a
/// rank-deficient lag design from a constant series).
pub fn linregr(
    y: &[f64],
    horizon: usize,
    lags: usize,
    covariates: &[Vec<f64>],
) -> Result<ForecastResult, ForecastError> {
    if horizon == 0 {
        return Err(ForecastError::ZeroHorizon);
    }
    let lags = lags.max(1);
    let n = y.len();
    // Need at least one training row beyond the parameter count: rows = n − lags,
    // params = 1 (intercept) + lags + covariate count. Require rows > params.
    let min_needed = lags + 2;
    if n < min_needed {
        return Err(ForecastError::TooShort {
            needed: min_needed,
            got: n,
        });
    }

    let cov_count = covariates.len();
    let expected_cov_len = n + horizon;
    for col in covariates {
        if col.len() != expected_cov_len {
            return Err(ForecastError::LengthMismatch {
                a: col.len(),
                b: expected_cov_len,
            });
        }
    }

    // Build the training design: one row per t in [lags, n). Columns are the
    // `lags` autoregressive features (y_{t−1}..y_{t−lags}) then the covariate
    // values at time t.
    let rows = n - lags;
    let mut response = Vec::with_capacity(rows);
    let mut lag_cols: Vec<Vec<f64>> = vec![Vec::with_capacity(rows); lags];
    let mut cov_cols: Vec<Vec<f64>> = vec![Vec::with_capacity(rows); cov_count];

    for t in lags..n {
        response.push(y[t]);
        for (i, col) in lag_cols.iter_mut().enumerate() {
            col.push(y[t - 1 - i]);
        }
        for (j, col) in cov_cols.iter_mut().enumerate() {
            col.push(covariates[j][t]);
        }
    }

    let mut columns = lag_cols;
    columns.extend(cov_cols);

    let design = design_from_columns(&columns, true)
        .map_err(|e| ForecastError::Regression(e.to_string()))?;
    let fit = ols(&response, &design).map_err(|e| ForecastError::Regression(e.to_string()))?;
    let beta: Vec<f64> = fit.coefficients.iter().map(|c| c.estimate).collect();
    // beta[0] = intercept, beta[1..=lags] = AR coefficients, beta[lags+1..] = cov.

    // Roll forward recursively, extending the history with each prediction.
    let mut history: Vec<f64> = y.to_vec();
    let mut points = Vec::with_capacity(horizon);
    for h in 1..=horizon {
        let cur = history.len();
        let mut value = beta[0];
        for i in 0..lags {
            value += beta[1 + i] * history[cur - 1 - i];
        }
        for (j, col) in covariates.iter().enumerate() {
            // The future covariate value for this step sits at index n − 1 + h.
            value += beta[1 + lags + j] * col[n - 1 + h];
        }
        points.push(ForecastPoint { step: h, value });
        history.push(value);
    }

    Ok(ForecastResult {
        model: "linregr".to_string(),
        points,
    })
}

/// Linregr route dispatch from typed params.
///
/// # Errors
///
/// Propagates [`linregr`].
pub fn linregr_from_params(p: &LinRegrParams) -> Result<ForecastResult, ForecastError> {
    linregr(&p.y, p.horizon, p.lags, &p.covariates)
}

#[cfg(test)]
mod tests {
    use super::linregr;

    #[test]
    fn linregr_extrapolates_a_linear_trend() {
        // y = 1..12 (slope 1). An AR(1) regression on a line learns
        // y_t = 1 + 1·y_{t−1}, so it continues 13, 14, 15 (within tolerance).
        let y: Vec<f64> = (1..=12).map(f64::from).collect();
        let fc = linregr(&y, 3, 1, &[]).expect("forecast");
        let values: Vec<f64> = fc.points.iter().map(|p| p.value).collect();
        assert!((values[0] - 13.0).abs() < 1e-6, "h1 {}", values[0]);
        assert!((values[1] - 14.0).abs() < 1e-6, "h2 {}", values[1]);
        assert!((values[2] - 15.0).abs() < 1e-6, "h3 {}", values[2]);
        assert_eq!(fc.model, "linregr");
    }

    #[test]
    fn linregr_with_covariate_uses_the_future_path() {
        // y_t = 3·x_t + 2·y_{t−1} with x an oscillating exogenous driver, so the
        // lag feature and the covariate are NOT collinear (the design is full
        // rank). The regression recovers both coefficients and applies the known
        // future covariate path to extrapolate.
        let x = vec![
            1.0, -1.0, 1.0, -1.0, 1.0, -1.0, 1.0, -1.0, 1.0, -1.0, 1.0, -1.0,
        ];
        let n = 9; // history length; covariate length must be n + horizon = 12.
        let mut y = vec![0.0_f64; n];
        y[0] = 3.0 * x[0];
        for t in 1..n {
            y[t] = 2.0_f64.mul_add(y[t - 1], 3.0 * x[t]);
        }
        let fc = linregr(&y, 3, 1, std::slice::from_ref(&x)).expect("forecast");
        let values: Vec<f64> = fc.points.iter().map(|p| p.value).collect();
        // The true continuation: y_{t} = 3·x_t + 2·y_{t−1}.
        let mut expected = Vec::new();
        let mut prev = y[n - 1];
        for h in 0..3 {
            let next = 2.0_f64.mul_add(prev, 3.0 * x[n + h]);
            expected.push(next);
            prev = next;
        }
        for (got, want) in values.iter().zip(expected.iter()) {
            assert!((got - want).abs() < 1e-6, "got {got} want {want}");
        }
    }

    #[test]
    fn linregr_rejects_short_series() {
        assert!(matches!(
            linregr(&[1.0, 2.0], 1, 3, &[]),
            Err(crate::error::ForecastError::TooShort { .. })
        ));
    }

    #[test]
    fn linregr_rejects_bad_covariate_length() {
        let y: Vec<f64> = (1..=8).map(f64::from).collect();
        // covariate should be length 8 + 2 = 10; give 5.
        let bad = vec![1.0; 5];
        assert!(matches!(
            linregr(&y, 2, 1, &[bad]),
            Err(crate::error::ForecastError::LengthMismatch { .. })
        ));
    }
}
