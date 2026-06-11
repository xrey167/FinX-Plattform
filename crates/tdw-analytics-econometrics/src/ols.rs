//! Ordinary least squares with summary diagnostics.
//!
//! Fits `y = X β + ε` by the normal equations (see [`crate::linalg`] for the
//! Cholesky numeric approach) and reports the coefficient estimates with their
//! standard errors and t-statistics, the coefficient of determination `R²` and
//! its degrees-of-freedom-adjusted form, the overall `F`-statistic, and the
//! Durbin-Watson residual-autocorrelation diagnostic.
//!
//! # Definitions
//!
//! - **Coefficients**: `β̂ = (XᵀX)⁻¹ Xᵀy`.
//! - **Residual variance**: `s² = RSS / (n − k)` with `RSS = Σ ε̂²`, `n`
//!   observations, `k` parameters (the unbiased estimator).
//! - **Standard errors**: `se(β̂ⱼ) = √( s² · [(XᵀX)⁻¹]ⱼⱼ )`; `t = β̂ⱼ / se`.
//! - **R²**: `1 − RSS/TSS`, `TSS = Σ (yᵢ − ȳ)²`. **Adjusted R²**:
//!   `1 − (1 − R²)·(n − 1)/(n − k)`.
//! - **F-statistic**: `((TSS − RSS)/(k − 1)) / (RSS/(n − k))` — the joint test
//!   that all non-intercept slopes are zero (requires an intercept column, i.e.
//!   `k ≥ 2`).
//! - **Durbin-Watson** (Durbin & Watson 1950):
//!   `DW = Σ_{t=2}^{n} (ε̂ₜ − ε̂_{t−1})² / Σ ε̂ₜ²`, in `[0, 4]`; `≈ 2` indicates
//!   no first-order residual autocorrelation.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::EconometricsError;
use crate::linalg::{Cholesky, Matrix};

#[allow(clippy::cast_precision_loss)]
const fn usize_to_f64(n: usize) -> f64 {
    n as f64
}

/// One estimated coefficient: the point estimate, its standard error, and the
/// t-statistic (`estimate / std_error`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Coefficient {
    /// Point estimate `β̂ⱼ`.
    pub estimate: f64,
    /// Standard error `se(β̂ⱼ)`.
    pub std_error: f64,
    /// t-statistic `β̂ⱼ / se(β̂ⱼ)`; `0.0` when the standard error is zero.
    pub t_stat: f64,
}

/// The OLS fit summary: per-coefficient estimates plus the model-level
/// diagnostics.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct OlsSummary {
    /// Estimated coefficients in design-column order (intercept first when the
    /// caller prepends a constant column).
    pub coefficients: Vec<Coefficient>,
    /// Coefficient of determination `R²`.
    pub r_squared: f64,
    /// Degrees-of-freedom-adjusted `R²`.
    pub adj_r_squared: f64,
    /// Overall `F`-statistic (all slopes jointly zero); `0.0` when undefined
    /// (no intercept / single parameter).
    pub f_statistic: f64,
    /// Durbin-Watson residual-autocorrelation statistic, in `[0, 4]`.
    pub durbin_watson: f64,
    /// Residual degrees of freedom `n − k`.
    pub residual_dof: usize,
}

/// Fit `y = X β + ε` by ordinary least squares and return the summary.
///
/// `design` is the row-major design matrix `X` (the caller includes an intercept
/// column of ones when an intercept is wanted; the F-statistic and adjusted R²
/// assume an intercept is present). `y` is the response vector.
///
/// # Errors
///
/// - [`EconometricsError::EmptyDesign`] when `X` has zero columns.
/// - [`EconometricsError::RowMismatch`] when `y.len()` ≠ `X` row count.
/// - [`EconometricsError::InsufficientRows`] when `n ≤ k` (no residual dof).
/// - [`EconometricsError::Singular`] when `XᵀX` is not positive definite
///   (collinear regressors).
pub fn ols(y: &[f64], design: &Matrix) -> Result<OlsSummary, EconometricsError> {
    let n = design.rows();
    let k = design.cols();
    if k == 0 {
        return Err(EconometricsError::EmptyDesign);
    }
    if y.len() != n {
        return Err(EconometricsError::RowMismatch { y: y.len(), x: n });
    }
    if n <= k {
        return Err(EconometricsError::InsufficientRows { rows: n, cols: k });
    }

    let gram = design.gram();
    let chol = Cholesky::factor(&gram)?;
    let xty = design.transpose_mul_vec(y);
    let beta = chol.solve(&xty);

    // Residuals and residual sum of squares.
    let fitted = design.mul_vec(&beta);
    let residuals: Vec<f64> = y
        .iter()
        .zip(fitted.iter())
        .map(|(yi, fi)| yi - fi)
        .collect();
    let rss: f64 = residuals.iter().map(|e| e * e).sum();

    let dof = n - k;
    let sigma2 = rss / usize_to_f64(dof);
    let inv_diag = chol.inverse_diagonal();

    let coefficients = beta
        .iter()
        .zip(inv_diag.iter())
        .map(|(&b, &d)| {
            let var = sigma2 * d;
            let se = if var > 0.0 { var.sqrt() } else { 0.0 };
            let t = if se > 0.0 { b / se } else { 0.0 };
            Coefficient {
                estimate: b,
                std_error: se,
                t_stat: t,
            }
        })
        .collect();

    let y_mean = y.iter().sum::<f64>() / usize_to_f64(n);
    let tss: f64 = y.iter().map(|yi| (yi - y_mean).powi(2)).sum();
    let r_squared = if tss > 0.0 { 1.0 - rss / tss } else { 0.0 };
    let adj_r_squared = if tss > 0.0 && dof > 0 {
        1.0 - (1.0 - r_squared) * usize_to_f64(n - 1) / usize_to_f64(dof)
    } else {
        0.0
    };

    // F-statistic for the joint significance of the k-1 slopes (needs intercept).
    let f_statistic = if k >= 2 && rss > 0.0 {
        let model_ss = tss - rss;
        let num = model_ss / usize_to_f64(k - 1);
        let den = rss / usize_to_f64(dof);
        if den > 0.0 { num / den } else { 0.0 }
    } else {
        0.0
    };

    let durbin_watson = durbin_watson_stat(&residuals, rss);

    Ok(OlsSummary {
        coefficients,
        r_squared,
        adj_r_squared,
        f_statistic,
        durbin_watson,
        residual_dof: dof,
    })
}

/// Durbin-Watson statistic from a residual series and its sum of squares.
/// Returns `0.0` when `rss` is zero (a perfect fit has no defined DW).
fn durbin_watson_stat(residuals: &[f64], rss: f64) -> f64 {
    if rss == 0.0 || residuals.len() < 2 {
        return 0.0;
    }
    let diff_sq: f64 = residuals.windows(2).map(|w| (w[1] - w[0]).powi(2)).sum();
    diff_sq / rss
}

/// Convenience: build a design matrix from a column-major list of regressor
/// columns, optionally prepending an intercept column of ones.
///
/// `columns` is `[c0, c1, …]` where each `cⱼ` is a length-`n` regressor. With
/// `intercept = true` a leading all-ones column is added, so the resulting
/// design is `n × (columns.len() + 1)`.
///
/// # Errors
///
/// Returns [`EconometricsError::EmptyDesign`] when `columns` is empty or its
/// columns have differing lengths.
pub fn design_from_columns(
    columns: &[Vec<f64>],
    intercept: bool,
) -> Result<Matrix, EconometricsError> {
    if columns.is_empty() {
        return Err(EconometricsError::EmptyDesign);
    }
    let n = columns[0].len();
    if n == 0 || columns.iter().any(|c| c.len() != n) {
        return Err(EconometricsError::EmptyDesign);
    }
    let k = columns.len() + usize::from(intercept);
    let mut data = Vec::with_capacity(n * k);
    for r in 0..n {
        if intercept {
            data.push(1.0);
        }
        for col in columns {
            data.push(col[r]);
        }
    }
    Ok(Matrix::from_rows(n, k, data))
}

#[cfg(test)]
mod tests {
    use super::{design_from_columns, ols};

    #[test]
    fn ols_recovers_an_exact_line() {
        // y = 2 + 3x exactly over x=[1,2,3,4]; OLS must recover intercept 2,
        // slope 3, R² = 1, residuals 0 ⇒ DW 0 (perfect fit sentinel).
        let x = vec![1.0, 2.0, 3.0, 4.0];
        let y: Vec<f64> = x.iter().map(|xi| 2.0 + 3.0 * xi).collect();
        let design = design_from_columns(&[x], true).expect("design");
        let fit = ols(&y, &design).expect("ols");
        assert!((fit.coefficients[0].estimate - 2.0).abs() < 1e-9);
        assert!((fit.coefficients[1].estimate - 3.0).abs() < 1e-9);
        assert!((fit.r_squared - 1.0).abs() < 1e-9);
        assert_eq!(fit.residual_dof, 2);
    }

    #[test]
    fn ols_matches_textbook_two_point_worked_example() {
        // Worked example (Wooldridge, simple-regression illustration form):
        // n=5, x=[1,2,3,4,5], y=[1,3,2,5,4].
        //   x̄=3, ȳ=3; Sxy = Σ(x-x̄)(y-ȳ) = (-2)(-2)+(-1)(0)+0+( 1)(2)+(2)(1)
        //              = 4 + 0 + 0 + 2 + 2 = 8.
        //   Sxx = Σ(x-x̄)² = 4+1+0+1+4 = 10.
        //   slope = 8/10 = 0.8; intercept = ȳ - slope*x̄ = 3 - 2.4 = 0.6.
        let x = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let y = vec![1.0, 3.0, 2.0, 5.0, 4.0];
        let design = design_from_columns(&[x], true).expect("design");
        let fit = ols(&y, &design).expect("ols");
        assert!(
            (fit.coefficients[0].estimate - 0.6).abs() < 1e-9,
            "intercept {}",
            fit.coefficients[0].estimate
        );
        assert!(
            (fit.coefficients[1].estimate - 0.8).abs() < 1e-9,
            "slope {}",
            fit.coefficients[1].estimate
        );
        // RSS: fitted = 0.6+0.8x = [1.4,2.2,3.0,3.8,4.6];
        //   resid = [-0.4,0.8,-1.0,1.2,-0.6]; RSS = 0.16+0.64+1.0+1.44+0.36 = 3.6.
        //   TSS = Σ(y-3)² = 4+0+1+4+1 = 10. R² = 1 - 3.6/10 = 0.64.
        assert!((fit.r_squared - 0.64).abs() < 1e-9, "r2 {}", fit.r_squared);
        // adj R² = 1 - (1-0.64)*(5-1)/(5-2) = 1 - 0.36*4/3 = 1 - 0.48 = 0.52.
        assert!(
            (fit.adj_r_squared - 0.52).abs() < 1e-9,
            "adjr2 {}",
            fit.adj_r_squared
        );
        // F = ((TSS-RSS)/(k-1)) / (RSS/(n-k)) = (6.4/1)/(3.6/3) = 6.4/1.2
        //   = 5.3333...
        assert!(
            (fit.f_statistic - 16.0 / 3.0).abs() < 1e-9,
            "F {}",
            fit.f_statistic
        );
        // se(slope) = √(s² / Sxx), s² = RSS/(n-k) = 1.2; Sxx = 10 ⇒
        //   se = √0.12 = 0.34641016; t = 0.8/0.34641016 = 2.309401.
        assert!(
            (fit.coefficients[1].std_error - 0.346_410_161_513_8).abs() < 1e-9,
            "se {}",
            fit.coefficients[1].std_error
        );
        assert!(
            (fit.coefficients[1].t_stat - 2.309_401_076_758_5).abs() < 1e-9,
            "t {}",
            fit.coefficients[1].t_stat
        );
    }

    #[test]
    fn ols_rejects_collinear_design() {
        // Two identical regressors with an intercept ⇒ rank-deficient ⇒ Singular.
        let c = vec![1.0, 2.0, 3.0, 4.0];
        let design = design_from_columns(&[c.clone(), c], true).expect("design");
        let y = vec![1.0, 2.0, 3.0, 4.0];
        assert_eq!(ols(&y, &design), Err(crate::EconometricsError::Singular));
    }

    #[test]
    fn ols_rejects_too_few_rows() {
        let design = design_from_columns(&[vec![1.0, 2.0]], true).expect("design");
        let y = vec![1.0, 2.0];
        // n=2, k=2 ⇒ no residual dof.
        assert!(matches!(
            ols(&y, &design),
            Err(crate::EconometricsError::InsufficientRows { .. })
        ));
    }
}
