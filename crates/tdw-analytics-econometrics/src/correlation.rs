//! Pearson correlation matrix and variance-inflation factors.
//!
//! # Definitions
//!
//! - **Pearson correlation**: for two columns `a`, `b`,
//!   `ρ = Σ(aᵢ−ā)(bᵢ−b̄) / √(Σ(aᵢ−ā)² · Σ(bᵢ−b̄)²)`. A column with zero variance
//!   correlates `0` with everything (and `1` with itself by convention).
//! - **Variance inflation factor**: for regressor `j`, `VIF_j = 1 / (1 − R²_j)`
//!   where `R²_j` is the coefficient of determination of the auxiliary
//!   regression of column `j` on all the *other* columns (with an intercept). A
//!   VIF near `1` means no collinearity; large values flag multicollinearity. A
//!   perfectly explained column (`R²_j = 1`) yields `f64::INFINITY`.

use crate::EconometricsError;
use crate::ols::{design_from_columns, ols};

#[allow(clippy::cast_precision_loss)]
const fn usize_to_f64(n: usize) -> f64 {
    n as f64
}

/// Pearson correlation coefficient between two equal-length series.
///
/// # Errors
///
/// Returns [`EconometricsError::LengthMismatch`] when the series differ in
/// length.
pub fn correlation(a: &[f64], b: &[f64]) -> Result<f64, EconometricsError> {
    if a.len() != b.len() {
        return Err(EconometricsError::LengthMismatch {
            first: a.len(),
            second: b.len(),
        });
    }
    Ok(pearson(a, b))
}

/// Pearson correlation without the length check (callers in this module pass
/// equal-length columns). Zero dispersion in either column ⇒ `0.0`.
fn pearson(a: &[f64], b: &[f64]) -> f64 {
    let n = a.len();
    if n == 0 {
        return 0.0;
    }
    let mean_a = a.iter().sum::<f64>() / usize_to_f64(n);
    let mean_b = b.iter().sum::<f64>() / usize_to_f64(n);
    let mut cov = 0.0;
    let mut var_a = 0.0;
    let mut var_b = 0.0;
    for (x, y) in a.iter().zip(b.iter()) {
        let da = x - mean_a;
        let db = y - mean_b;
        cov += da * db;
        var_a += da * da;
        var_b += db * db;
    }
    let denom = (var_a * var_b).sqrt();
    if denom == 0.0 { 0.0 } else { cov / denom }
}

/// The symmetric Pearson correlation matrix of the supplied equal-length
/// `columns`, returned as a row-major `m × m` nested vector with unit diagonal.
///
/// # Errors
///
/// Returns [`EconometricsError::EmptyDesign`] when `columns` is empty or its
/// columns have differing lengths.
pub fn correlation_matrix(columns: &[Vec<f64>]) -> Result<Vec<Vec<f64>>, EconometricsError> {
    if columns.is_empty() {
        return Err(EconometricsError::EmptyDesign);
    }
    let n = columns[0].len();
    if columns.iter().any(|c| c.len() != n) {
        return Err(EconometricsError::EmptyDesign);
    }
    let m = columns.len();
    let mut matrix = vec![vec![0.0; m]; m];
    for i in 0..m {
        matrix[i][i] = 1.0;
        for j in (i + 1)..m {
            let r = pearson(&columns[i], &columns[j]);
            matrix[i][j] = r;
            matrix[j][i] = r;
        }
    }
    Ok(matrix)
}

/// Variance-inflation factor for each column, via the auxiliary-regression `R²`.
///
/// Needs at least two columns (a VIF is collinearity *among* regressors) and
/// enough rows for each auxiliary regression to have residual degrees of
/// freedom. A column perfectly explained by the others yields `f64::INFINITY`.
///
/// # Errors
///
/// - [`EconometricsError::EmptyDesign`] when there are fewer than two columns or
///   the columns differ in length.
/// - Propagates [`EconometricsError::InsufficientRows`] when a column has too
///   few rows for the auxiliary regression.
pub fn vif(columns: &[Vec<f64>]) -> Result<Vec<f64>, EconometricsError> {
    let m = columns.len();
    if m < 2 {
        return Err(EconometricsError::EmptyDesign);
    }
    let n = columns[0].len();
    if columns.iter().any(|c| c.len() != n) {
        return Err(EconometricsError::EmptyDesign);
    }
    let mut out = Vec::with_capacity(m);
    for target in 0..m {
        let others: Vec<Vec<f64>> = columns
            .iter()
            .enumerate()
            .filter(|(idx, _)| *idx != target)
            .map(|(_, c)| c.clone())
            .collect();
        let design = design_from_columns(&others, true)?;
        let fit = ols(&columns[target], &design)?;
        let r2 = fit.r_squared;
        let v = if (1.0 - r2).abs() < f64::EPSILON {
            f64::INFINITY
        } else {
            1.0 / (1.0 - r2)
        };
        out.push(v);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::{correlation, correlation_matrix, vif};

    #[test]
    fn correlation_of_perfectly_linear_series_is_one() {
        let a = vec![1.0, 2.0, 3.0, 4.0];
        let b: Vec<f64> = a.iter().map(|x| 2.0 * x + 1.0).collect();
        assert!((correlation(&a, &b).expect("corr") - 1.0).abs() < 1e-12);
    }

    #[test]
    fn correlation_of_inverse_series_is_negative_one() {
        let a = vec![1.0, 2.0, 3.0, 4.0];
        let b: Vec<f64> = a.iter().map(|x| -x).collect();
        assert!((correlation(&a, &b).expect("corr") - -1.0).abs() < 1e-12);
    }

    #[test]
    fn correlation_length_mismatch_errors() {
        assert!(correlation(&[1.0, 2.0], &[1.0]).is_err());
    }

    #[test]
    fn correlation_matrix_is_symmetric_unit_diagonal() {
        let cols = vec![vec![1.0, 2.0, 3.0], vec![3.0, 2.0, 1.0]];
        let m = correlation_matrix(&cols).expect("matrix");
        assert!((m[0][0] - 1.0).abs() < 1e-12);
        assert!((m[1][1] - 1.0).abs() < 1e-12);
        // perfectly anti-correlated ⇒ -1.
        assert!((m[0][1] - -1.0).abs() < 1e-12);
        assert!((m[0][1] - m[1][0]).abs() < 1e-12);
    }

    #[test]
    fn vif_is_near_one_for_orthogonal_columns() {
        // Two (near-)uncorrelated columns ⇒ low auxiliary R² ⇒ VIF ≈ 1.
        let cols = vec![vec![1.0, 2.0, 3.0, 4.0, 5.0], vec![2.0, 1.0, 4.0, 3.0, 5.0]];
        let v = vif(&cols).expect("vif");
        assert_eq!(v.len(), 2);
        assert!(v.iter().all(|x| *x >= 1.0));
    }

    #[test]
    fn vif_requires_two_columns() {
        assert!(vif(&[vec![1.0, 2.0, 3.0]]).is_err());
    }
}
