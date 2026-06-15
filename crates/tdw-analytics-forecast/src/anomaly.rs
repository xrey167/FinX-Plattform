//! Quantile-band anomaly detection.
//!
//! Mirrors darts' `QuantileDetector`: flag any observation that falls outside an
//! empirical-quantile band of the series. Given a lower quantile `q_lo` and an
//! upper quantile `q_hi`, the band is `[Q(q_lo), Q(q_hi)]` where `Q` is the
//! empirical quantile of the series values (linear interpolation between the two
//! nearest order statistics, the standard "type 7"/`numpy.percentile` rule). A
//! value below the lower bound or above the upper bound is reported as an anomaly,
//! tagged with which edge it breached.
//!
//! This detects level outliers directly on the values; callers wanting residual
//! anomalies pass a residual series (e.g. the residual component from
//! [`crate::mstl::decompose`]).

use crate::error::ForecastError;
use crate::params::{Anomaly, AnomalyParams, AnomalyResult};

/// Empirical quantile of `sorted` (ascending) at probability `q` in `[0, 1]`,
/// using linear interpolation between order statistics (type-7 / numpy rule).
#[allow(clippy::cast_precision_loss)]
fn quantile_sorted(sorted: &[f64], q: f64) -> f64 {
    let n = sorted.len();
    if n == 1 {
        return sorted[0];
    }
    let pos = q * (n as f64 - 1.0);
    let lo = pos.floor();
    let hi = pos.ceil();
    // `pos` lies in [0, n−1], so both floor and ceil are non-negative and in
    // range: the casts neither truncate nor lose a sign.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let lo_idx = lo as usize;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let hi_idx = hi as usize;
    if lo_idx == hi_idx {
        return sorted[lo_idx];
    }
    let frac = pos - lo;
    sorted[hi_idx].mul_add(frac, sorted[lo_idx] * (1.0 - frac))
}

/// Flag the observations outside the empirical `[lower_quantile, upper_quantile]`
/// band of the series.
///
/// # Errors
///
/// [`ForecastError::EmptySeries`] when `y` is empty;
/// [`ForecastError::ParamOutOfRange`] when a quantile is outside `[0, 1]` or the
/// lower quantile exceeds the upper.
pub fn detect(
    y: &[f64],
    lower_quantile: f64,
    upper_quantile: f64,
) -> Result<AnomalyResult, ForecastError> {
    if y.is_empty() {
        return Err(ForecastError::EmptySeries);
    }
    if !(0.0..=1.0).contains(&lower_quantile) {
        return Err(ForecastError::ParamOutOfRange {
            name: "lower_quantile",
            value: lower_quantile,
        });
    }
    if !(0.0..=1.0).contains(&upper_quantile) {
        return Err(ForecastError::ParamOutOfRange {
            name: "upper_quantile",
            value: upper_quantile,
        });
    }
    if lower_quantile > upper_quantile {
        return Err(ForecastError::ParamOutOfRange {
            name: "lower_quantile",
            value: lower_quantile,
        });
    }

    let mut sorted = y.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let lower_bound = quantile_sorted(&sorted, lower_quantile);
    let upper_bound = quantile_sorted(&sorted, upper_quantile);

    let anomalies = y
        .iter()
        .enumerate()
        .filter_map(|(index, &value)| {
            if value > upper_bound {
                Some(Anomaly {
                    index,
                    value,
                    above: true,
                })
            } else if value < lower_bound {
                Some(Anomaly {
                    index,
                    value,
                    above: false,
                })
            } else {
                None
            }
        })
        .collect();

    Ok(AnomalyResult {
        lower_bound,
        upper_bound,
        anomalies,
    })
}

/// Anomaly route dispatch from typed params.
///
/// # Errors
///
/// Propagates [`detect`].
pub fn anomaly_from_params(p: &AnomalyParams) -> Result<AnomalyResult, ForecastError> {
    detect(&p.y, p.lower_quantile, p.upper_quantile)
}

#[cfg(test)]
mod tests {
    use super::detect;

    #[test]
    fn flags_an_injected_spike() {
        // A flat series around 10 with one injected spike at 100. With a tight
        // band the spike is flagged as an above-band anomaly.
        let mut y = vec![10.0, 11.0, 9.0, 10.0, 11.0, 9.0, 10.0, 11.0, 9.0, 10.0];
        y.push(100.0); // injected spike at index 10
        let result = detect(&y, 0.05, 0.95).expect("detect");
        assert!(
            result.anomalies.iter().any(|a| a.index == 10 && a.above),
            "spike at index 10 must be flagged: {:?}",
            result.anomalies
        );
    }

    #[test]
    fn flags_a_low_outlier() {
        let mut y = vec![50.0, 51.0, 49.0, 50.0, 51.0, 49.0, 50.0, 51.0];
        y.push(-20.0); // injected dip
        let result = detect(&y, 0.1, 0.9).expect("detect");
        let dip = result.anomalies.iter().find(|a| a.index == 8);
        assert!(dip.is_some(), "dip must be flagged");
        assert!(!dip.expect("dip").above, "dip is a below-band anomaly");
    }

    #[test]
    fn quantile_band_matches_hand_calculation() {
        // y = [1,2,3,4,5]; type-7 quantiles: Q(0.0)=1, Q(1.0)=5,
        // Q(0.25)= 1 + 0.25*4 = 2, Q(0.75)= 1 + 0.75*4 = 4.
        let y = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let r = detect(&y, 0.25, 0.75).expect("detect");
        assert!((r.lower_bound - 2.0).abs() < 1e-12, "lo {}", r.lower_bound);
        assert!((r.upper_bound - 4.0).abs() < 1e-12, "hi {}", r.upper_bound);
        // Values 1 (below 2) and 5 (above 4) are flagged.
        assert_eq!(r.anomalies.len(), 2);
    }

    #[test]
    fn clean_series_has_no_anomalies_at_full_band() {
        // With band [0,1] (min..max), nothing is outside.
        let y = vec![3.0, 1.0, 4.0, 1.0, 5.0, 9.0, 2.0, 6.0];
        let r = detect(&y, 0.0, 1.0).expect("detect");
        assert!(r.anomalies.is_empty());
    }

    #[test]
    fn rejects_out_of_range_quantile() {
        assert!(matches!(
            detect(&[1.0, 2.0], 1.5, 0.9),
            Err(crate::error::ForecastError::ParamOutOfRange { .. })
        ));
    }
}
