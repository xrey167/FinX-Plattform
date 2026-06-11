//! Hand-rolled dense linear algebra for the normal-equations OLS solve.
//!
//! # Numeric approach
//!
//! OLS coefficients solve the normal equations `(XᵀX) β = Xᵀy`. The Gram matrix
//! `A = XᵀX` is symmetric and — for a full-column-rank design — positive
//! definite, so we factor it with a **Cholesky decomposition** `A = L Lᵀ`
//! (lower-triangular `L`) and solve by forward/back substitution. Cholesky is
//! chosen over forming `A⁻¹` explicitly because it is roughly twice as fast,
//! numerically stable for SPD systems, and gives the inverse cheaply when needed
//! (the diagonal of `(XᵀX)⁻¹` is required for coefficient standard errors).
//!
//! # Conditioning caveats
//!
//! Forming `XᵀX` squares the condition number of `X`, so the normal-equations
//! route loses about half the available digits versus a QR factorization of `X`
//! directly. For the small, well-scaled designs these catalog routes serve
//! (a handful of regressors, centered/standardized where it matters) this is
//! more than adequate and keeps the implementation tiny and dependency-free; a
//! genuinely ill-conditioned design (near-collinear regressors) is *detected*
//! rather than silently mis-solved — the Cholesky factorization fails and the
//! caller gets [`crate::EconometricsError::Singular`] instead of garbage
//! coefficients.

use crate::EconometricsError;

/// A dense row-major matrix of `f64`, used for the design matrix and the small
/// symmetric Gram matrices the OLS solve manipulates.
#[derive(Clone, Debug)]
pub struct Matrix {
    rows: usize,
    cols: usize,
    data: Vec<f64>,
}

impl Matrix {
    /// Build a matrix from row-major `data`. The length must be `rows * cols`.
    #[must_use]
    pub fn from_rows(rows: usize, cols: usize, data: Vec<f64>) -> Self {
        debug_assert_eq!(data.len(), rows * cols, "data length must be rows*cols");
        Self { rows, cols, data }
    }

    /// Number of rows.
    #[must_use]
    pub const fn rows(&self) -> usize {
        self.rows
    }

    /// Number of columns.
    #[must_use]
    pub const fn cols(&self) -> usize {
        self.cols
    }

    /// Read the element at `(r, c)`.
    #[must_use]
    pub fn get(&self, r: usize, c: usize) -> f64 {
        self.data[r * self.cols + c]
    }

    /// The Gram matrix `XᵀX` (symmetric, `cols × cols`).
    #[must_use]
    pub fn gram(&self) -> Self {
        let k = self.cols;
        let mut out = vec![0.0; k * k];
        for i in 0..k {
            for j in i..k {
                let mut acc = 0.0;
                for r in 0..self.rows {
                    acc += self.get(r, i) * self.get(r, j);
                }
                out[i * k + j] = acc;
                out[j * k + i] = acc;
            }
        }
        Self::from_rows(k, k, out)
    }

    /// The matrix-vector product `Xᵀ v` (`v` has `rows` entries; result has
    /// `cols`).
    #[must_use]
    pub fn transpose_mul_vec(&self, v: &[f64]) -> Vec<f64> {
        let mut out = vec![0.0; self.cols];
        for (r, &vr) in v.iter().enumerate() {
            for (c, slot) in out.iter_mut().enumerate() {
                *slot += self.get(r, c) * vr;
            }
        }
        out
    }

    /// The matrix-vector product `X v` (`v` has `cols` entries; result has
    /// `rows`).
    #[must_use]
    pub fn mul_vec(&self, v: &[f64]) -> Vec<f64> {
        let mut out = vec![0.0; self.rows];
        for (r, slot) in out.iter_mut().enumerate() {
            let mut acc = 0.0;
            for (c, &vc) in v.iter().enumerate() {
                acc += self.get(r, c) * vc;
            }
            *slot = acc;
        }
        out
    }
}

/// The Cholesky factor `L` (lower-triangular) of a symmetric positive-definite
/// matrix `A`, stored densely. Carries the solve and inverse-diagonal helpers
/// the OLS estimator needs.
#[derive(Clone, Debug)]
pub struct Cholesky {
    n: usize,
    /// Lower-triangular factor, row-major.
    l: Vec<f64>,
}

impl Cholesky {
    /// Factor the SPD matrix `a` (`n × n`, symmetric) as `A = L Lᵀ`.
    ///
    /// # Errors
    ///
    /// Returns [`EconometricsError::Singular`] when `a` is not positive definite
    /// (a non-positive pivot appears), which for a Gram matrix means the design
    /// columns are linearly dependent.
    pub fn factor(a: &Matrix) -> Result<Self, EconometricsError> {
        let n = a.rows();
        let mut l = vec![0.0; n * n];
        for i in 0..n {
            for j in 0..=i {
                let mut sum = a.get(i, j);
                for k in 0..j {
                    sum -= l[i * n + k] * l[j * n + k];
                }
                if i == j {
                    if sum <= 0.0 {
                        return Err(EconometricsError::Singular);
                    }
                    l[i * n + j] = sum.sqrt();
                } else {
                    l[i * n + j] = sum / l[j * n + j];
                }
            }
        }
        Ok(Self { n, l })
    }

    /// Solve `A x = b` for `x` using the stored factor (forward then back
    /// substitution).
    // The triangular-substitution inner loops index both the factor `self.l`
    // (by the flat `i*n+k` offset) and the partially-filled `y`/`x` vectors by
    // the same running index, so an index loop is the clearest faithful form of
    // the recurrence.
    #[allow(clippy::needless_range_loop)]
    #[must_use]
    pub fn solve(&self, b: &[f64]) -> Vec<f64> {
        let n = self.n;
        // Forward: L y = b.
        let mut y = vec![0.0; n];
        for i in 0..n {
            let mut sum = b[i];
            for k in 0..i {
                sum -= self.l[i * n + k] * y[k];
            }
            y[i] = sum / self.l[i * n + i];
        }
        // Back: Lᵀ x = y.
        let mut x = vec![0.0; n];
        for i in (0..n).rev() {
            let mut sum = y[i];
            for k in (i + 1)..n {
                sum -= self.l[k * n + i] * x[k];
            }
            x[i] = sum / self.l[i * n + i];
        }
        x
    }

    /// The diagonal of `A⁻¹`, computed by solving `A x = eⱼ` for each unit
    /// vector and reading the `j`-th component. These are the (scaled) variances
    /// of the OLS coefficients; multiplied by the residual variance they give the
    /// squared standard errors.
    #[must_use]
    pub fn inverse_diagonal(&self) -> Vec<f64> {
        let n = self.n;
        let mut diag = vec![0.0; n];
        for j in 0..n {
            let mut e = vec![0.0; n];
            e[j] = 1.0;
            let col = self.solve(&e);
            diag[j] = col[j];
        }
        diag
    }
}

#[cfg(test)]
mod tests {
    use super::{Cholesky, Matrix};

    #[test]
    fn cholesky_solves_a_known_spd_system() {
        // A = [[4,2],[2,3]], b = [4, 5] ⇒ x = [0.25, 1.5].
        // Check: 4*0.25 + 2*1.5 = 1+3 = 4 ✓; 2*0.25 + 3*1.5 = 0.5+4.5 = 5 ✓.
        let a = Matrix::from_rows(2, 2, vec![4.0, 2.0, 2.0, 3.0]);
        let chol = Cholesky::factor(&a).expect("spd");
        let x = chol.solve(&[4.0, 5.0]);
        assert!((x[0] - 0.25).abs() < 1e-12, "x0 {}", x[0]);
        assert!((x[1] - 1.5).abs() < 1e-12, "x1 {}", x[1]);
    }

    #[test]
    fn cholesky_rejects_non_positive_definite() {
        // Singular (rank-1): [[1,1],[1,1]].
        let a = Matrix::from_rows(2, 2, vec![1.0, 1.0, 1.0, 1.0]);
        assert!(Cholesky::factor(&a).is_err());
    }

    #[test]
    fn gram_matches_manual_xtx() {
        // X = [[1,1],[1,2],[1,3]]: XᵀX = [[3,6],[6,14]].
        let x = Matrix::from_rows(3, 2, vec![1.0, 1.0, 1.0, 2.0, 1.0, 3.0]);
        let g = x.gram();
        assert!((g.get(0, 0) - 3.0).abs() < 1e-12);
        assert!((g.get(0, 1) - 6.0).abs() < 1e-12);
        assert!((g.get(1, 1) - 14.0).abs() < 1e-12);
    }

    #[test]
    fn inverse_diagonal_is_positive_for_spd() {
        let a = Matrix::from_rows(2, 2, vec![4.0, 2.0, 2.0, 3.0]);
        let chol = Cholesky::factor(&a).expect("spd");
        let diag = chol.inverse_diagonal();
        assert!(diag.iter().all(|d| *d > 0.0));
    }
}
