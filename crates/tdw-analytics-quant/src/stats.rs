//! Shared descriptive-statistics primitives over a value slice.
//!
//! These are the textbook sample estimators the metrics build on. Every helper
//! is `NaN`/`inf`-free for any finite input: a statistic that is undefined for
//! the supplied series (a variance of fewer than two points, a percentile of an
//! empty slice) returns `0.0` / `None` rather than a non-finite value, so the
//! downstream metric output stays clean.
//!
//! # Definitions and bias
//!
//! - **Mean**: arithmetic mean.
//! - **Sample variance / standard deviation**: the unbiased `n − 1` (Bessel)
//!   estimators.
//! - **Skewness**: the bias-corrected sample skewness `g1 · √(n(n−1))/(n−2)`,
//!   where `g1 = m3 / m2^{3/2}` uses the population central moments. This is the
//!   adjusted Fisher-Pearson coefficient (the form spreadsheet `SKEW` and most
//!   statistics texts report).
//! - **Excess kurtosis**: the bias-corrected sample excess kurtosis,
//!   `((n+1)·g2 + 6)·(n−1)/((n−2)(n−3))` with `g2 = m4/m2² − 3`. "Excess" means
//!   the normal distribution's kurtosis (3) is subtracted, so a Gaussian sample
//!   has excess kurtosis ≈ 0.

#[allow(clippy::cast_precision_loss)]
const fn len_f64(n: usize) -> f64 {
    n as f64
}

/// Arithmetic mean of `values`, or `0.0` for an empty slice.
#[must_use]
pub fn mean(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.iter().sum::<f64>() / len_f64(values.len())
}

/// Unbiased (`n − 1`) sample variance, or `0.0` for fewer than two points.
#[must_use]
pub fn variance(values: &[f64]) -> f64 {
    let n = values.len();
    if n < 2 {
        return 0.0;
    }
    let mu = mean(values);
    let ss: f64 = values.iter().map(|v| (v - mu).powi(2)).sum();
    ss / len_f64(n - 1)
}

/// Unbiased sample standard deviation (`√variance`).
#[must_use]
pub fn std_dev(values: &[f64]) -> f64 {
    variance(values).sqrt()
}

/// The `k`-th population central moment `m_k = (1/n) Σ (x − x̄)^k`.
#[must_use]
fn central_moment(values: &[f64], k: i32) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mu = mean(values);
    values.iter().map(|v| (v - mu).powi(k)).sum::<f64>() / len_f64(values.len())
}

/// Adjusted Fisher-Pearson sample skewness, or `0.0` when undefined (fewer than
/// three points or zero dispersion).
#[must_use]
pub fn skewness(values: &[f64]) -> f64 {
    let n = values.len();
    if n < 3 {
        return 0.0;
    }
    let m2 = central_moment(values, 2);
    if m2 == 0.0 {
        return 0.0;
    }
    let m3 = central_moment(values, 3);
    let g1 = m3 / m2.powf(1.5);
    let nf = len_f64(n);
    g1 * (nf * (nf - 1.0)).sqrt() / (nf - 2.0)
}

/// Bias-corrected sample excess kurtosis, or `0.0` when undefined (fewer than
/// four points or zero dispersion).
#[must_use]
pub fn excess_kurtosis(values: &[f64]) -> f64 {
    let n = values.len();
    if n < 4 {
        return 0.0;
    }
    let m2 = central_moment(values, 2);
    if m2 == 0.0 {
        return 0.0;
    }
    let m4 = central_moment(values, 4);
    let g2 = m4 / m2.powi(2) - 3.0;
    let nf = len_f64(n);
    (nf + 1.0).mul_add(g2, 6.0) * (nf - 1.0) / ((nf - 2.0) * (nf - 3.0))
}

/// The lower-tail percentile of `values` at fraction `q ∈ [0, 1]`.
///
/// Uses linear interpolation between the two nearest order statistics (the
/// "linear" / type-7 quantile, matching `numpy`/spreadsheet `PERCENTILE`).
/// Returns `None` for an empty slice.
#[must_use]
pub fn percentile(values: &[f64], q: f64) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted: Vec<f64> = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = sorted.len();
    if n == 1 {
        return Some(sorted[0]);
    }
    let clamped = q.clamp(0.0, 1.0);
    let rank = clamped * len_f64(n - 1);
    let lower = rank.floor();
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let lower_index = lower as usize;
    if lower_index + 1 >= n {
        return Some(sorted[n - 1]);
    }
    let frac = rank - lower;
    let low = sorted[lower_index];
    let high = sorted[lower_index + 1];
    Some(frac.mul_add(high - low, low))
}

#[cfg(test)]
mod tests {
    use super::{excess_kurtosis, mean, percentile, skewness, std_dev, variance};

    #[test]
    fn mean_and_variance_match_hand_values() {
        // [2, 4, 6]: mean 4; sample variance = ((4)+(0)+(4))/2 = 4; std = 2.
        let v = [2.0, 4.0, 6.0];
        assert!((mean(&v) - 4.0).abs() < 1e-12);
        assert!((variance(&v) - 4.0).abs() < 1e-12);
        assert!((std_dev(&v) - 2.0).abs() < 1e-12);
    }

    #[test]
    fn variance_of_flat_series_is_zero() {
        assert!((variance(&[5.0, 5.0, 5.0])).abs() < 1e-12);
    }

    #[test]
    fn skewness_of_symmetric_series_is_zero() {
        // Symmetric about 0 ⇒ zero skew.
        let v = [-2.0, -1.0, 0.0, 1.0, 2.0];
        assert!(skewness(&v).abs() < 1e-12);
    }

    #[test]
    fn excess_kurtosis_defined_for_four_plus_points() {
        // A flat-ish symmetric set; exact value not asserted, only finiteness and
        // the short-input guard.
        assert!((excess_kurtosis(&[1.0, 2.0, 3.0])).abs() < 1e-12); // n<4 ⇒ 0
        assert!(excess_kurtosis(&[1.0, 2.0, 3.0, 4.0]).is_finite());
    }

    #[test]
    fn percentile_interpolates_linearly() {
        // [1,2,3,4]: q=0.5 ⇒ rank 1.5 ⇒ between idx1(2) and idx2(3) ⇒ 2.5.
        let v = [1.0, 2.0, 3.0, 4.0];
        assert!((percentile(&v, 0.5).expect("median") - 2.5).abs() < 1e-12);
        assert!((percentile(&v, 0.0).expect("min") - 1.0).abs() < 1e-12);
        assert!((percentile(&v, 1.0).expect("max") - 4.0).abs() < 1e-12);
    }

    #[test]
    fn percentile_empty_is_none() {
        assert!(percentile(&[], 0.5).is_none());
    }
}
