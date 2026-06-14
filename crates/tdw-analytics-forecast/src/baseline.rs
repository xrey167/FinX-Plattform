//! Baseline forecasters: the seasonal-naive method and random-walk-with-drift.
//!
//! These are the standard reference baselines any forecasting suite should beat
//! (Hyndman & Athanasopoulos, *Forecasting: Principles and Practice*, ch. 5).
//!
//! - **Naive**: every future value equals the last observation `y_n`.
//! - **Seasonal-naive**: the forecast for step `h` equals the observation one
//!   full season back from the corresponding future position,
//!   `ŷ_{n+h} = y_{n + h − m·⌈h/m⌉}` for season length `m` — i.e. the last full
//!   season is tiled forward.
//! - **Random walk with drift**: `ŷ_{n+h} = y_n + h · d`, where the drift `d` is
//!   the mean first difference `d = (y_n − y_1) / (n − 1)` (equivalently the
//!   average period-over-period change). This is the maximum-likelihood drift of
//!   a random walk and the slope of the line through the first and last points.

use crate::error::ForecastError;
use crate::params::{BaselineParams, ForecastPoint, ForecastResult};

/// Forecast by repeating the last observation (the plain naive method).
///
/// # Errors
///
/// [`ForecastError::EmptySeries`] when `y` is empty; [`ForecastError::ZeroHorizon`]
/// when `horizon == 0`.
pub fn naive(y: &[f64], horizon: usize) -> Result<ForecastResult, ForecastError> {
    if y.is_empty() {
        return Err(ForecastError::EmptySeries);
    }
    if horizon == 0 {
        return Err(ForecastError::ZeroHorizon);
    }
    let last = y[y.len() - 1];
    let points = (1..=horizon)
        .map(|step| ForecastPoint { step, value: last })
        .collect();
    Ok(ForecastResult {
        model: "naive".to_string(),
        points,
    })
}

/// Seasonal-naive forecast: tile the last full season of length `period` forward.
///
/// # Errors
///
/// [`ForecastError::EmptySeries`] when `y` is empty; [`ForecastError::ZeroHorizon`]
/// when `horizon == 0`; [`ForecastError::InvalidPeriod`] when `period < 2`;
/// [`ForecastError::TooShort`] when the series is shorter than one full season.
pub fn seasonal_naive(
    y: &[f64],
    horizon: usize,
    period: usize,
) -> Result<ForecastResult, ForecastError> {
    if y.is_empty() {
        return Err(ForecastError::EmptySeries);
    }
    if horizon == 0 {
        return Err(ForecastError::ZeroHorizon);
    }
    if period < 2 {
        return Err(ForecastError::InvalidPeriod(period));
    }
    let n = y.len();
    if n < period {
        return Err(ForecastError::TooShort {
            needed: period,
            got: n,
        });
    }
    // For step h (1-based), the matching in-sample index is the one a whole
    // number of seasons back: index = n − period + ((h − 1) mod period).
    let points = (1..=horizon)
        .map(|step| {
            let phase = (step - 1) % period;
            let idx = n - period + phase;
            ForecastPoint {
                step,
                value: y[idx],
            }
        })
        .collect();
    Ok(ForecastResult {
        model: "seasonalnaive".to_string(),
        points,
    })
}

/// Random-walk-with-drift forecast: `ŷ_{n+h} = y_n + h · d`, drift `d` the mean
/// first difference.
///
/// # Errors
///
/// [`ForecastError::ZeroHorizon`] when `horizon == 0`; [`ForecastError::TooShort`]
/// when fewer than two observations are supplied (the drift is undefined).
pub fn random_walk_drift(y: &[f64], horizon: usize) -> Result<ForecastResult, ForecastError> {
    if horizon == 0 {
        return Err(ForecastError::ZeroHorizon);
    }
    let n = y.len();
    if n < 2 {
        return Err(ForecastError::TooShort { needed: 2, got: n });
    }
    let last = y[n - 1];
    // Mean first difference telescopes to (y_n − y_1)/(n − 1).
    #[allow(clippy::cast_precision_loss)]
    let drift = (last - y[0]) / (n as f64 - 1.0);
    let points = (1..=horizon)
        .map(|step| {
            #[allow(clippy::cast_precision_loss)]
            let value = step as f64;
            ForecastPoint {
                step,
                value: value.mul_add(drift, last),
            }
        })
        .collect();
    Ok(ForecastResult {
        model: "rwd".to_string(),
        points,
    })
}

/// Dispatch the seasonal-naive route from its typed params.
///
/// # Errors
///
/// Propagates [`seasonal_naive`].
pub fn seasonal_naive_from_params(p: &BaselineParams) -> Result<ForecastResult, ForecastError> {
    seasonal_naive(&p.y, p.horizon, p.period)
}

/// Dispatch the random-walk-with-drift route from its typed params.
///
/// # Errors
///
/// Propagates [`random_walk_drift`].
pub fn rwd_from_params(p: &BaselineParams) -> Result<ForecastResult, ForecastError> {
    random_walk_drift(&p.y, p.horizon)
}

#[cfg(test)]
mod tests {
    use super::{naive, random_walk_drift, seasonal_naive};

    #[test]
    fn seasonal_naive_reproduces_the_period_exactly() {
        // A clean period-4 series; the last full season is [10,20,30,40].
        // Forecasting 8 steps must tile that season exactly twice.
        let y = vec![1.0, 2.0, 3.0, 4.0, 10.0, 20.0, 30.0, 40.0];
        let fc = seasonal_naive(&y, 8, 4).expect("forecast");
        let values: Vec<f64> = fc.points.iter().map(|p| p.value).collect();
        assert_eq!(values, vec![10.0, 20.0, 30.0, 40.0, 10.0, 20.0, 30.0, 40.0]);
        assert_eq!(fc.model, "seasonalnaive");
        // Steps are 1-based and contiguous.
        assert_eq!(fc.points[0].step, 1);
        assert_eq!(fc.points[7].step, 8);
    }

    #[test]
    fn seasonal_naive_on_a_perfectly_periodic_series_is_exact() {
        // y repeats [5,7,9] forever; the seasonal-naive forecast equals the
        // continuation of the pattern with zero error.
        let y = vec![5.0, 7.0, 9.0, 5.0, 7.0, 9.0];
        let fc = seasonal_naive(&y, 3, 3).expect("forecast");
        let values: Vec<f64> = fc.points.iter().map(|p| p.value).collect();
        assert_eq!(values, vec![5.0, 7.0, 9.0]);
    }

    #[test]
    fn rwd_drift_equals_mean_first_difference() {
        // y = [2,4,6,8,10]; first differences all 2 ⇒ drift = 2.
        // mean first diff = (10 − 2)/4 = 2. Forecast continues 12,14,16.
        let y = vec![2.0, 4.0, 6.0, 8.0, 10.0];
        let fc = random_walk_drift(&y, 3).expect("forecast");
        let values: Vec<f64> = fc.points.iter().map(|p| p.value).collect();
        assert_eq!(values, vec![12.0, 14.0, 16.0]);
        assert_eq!(fc.model, "rwd");
    }

    #[test]
    fn rwd_drift_on_uneven_series_uses_telescoped_mean() {
        // y = [1, 5, 6]; mean first diff = (6 − 1)/2 = 2.5.
        // Forecast: 6 + 1*2.5 = 8.5; 6 + 2*2.5 = 11.0.
        let y = vec![1.0, 5.0, 6.0];
        let fc = random_walk_drift(&y, 2).expect("forecast");
        let values: Vec<f64> = fc.points.iter().map(|p| p.value).collect();
        assert!((values[0] - 8.5).abs() < 1e-12);
        assert!((values[1] - 11.0).abs() < 1e-12);
    }

    #[test]
    fn naive_repeats_last_value() {
        let y = vec![3.0, 1.0, 4.0, 1.0, 5.0];
        let fc = naive(&y, 3).expect("forecast");
        assert!(fc.points.iter().all(|p| (p.value - 5.0).abs() < 1e-12));
    }

    #[test]
    fn seasonal_naive_rejects_period_one_and_short_series() {
        assert!(matches!(
            seasonal_naive(&[1.0, 2.0], 1, 1),
            Err(crate::error::ForecastError::InvalidPeriod(1))
        ));
        assert!(matches!(
            seasonal_naive(&[1.0, 2.0], 1, 4),
            Err(crate::error::ForecastError::TooShort { .. })
        ));
    }
}
