//! The Theta method (Assimakopoulos & Nikolopoulos 2000).
//!
//! The classic Theta method with `θ = 2` decomposes the series into two "theta
//! lines" and recombines them. For `θ = 2` the standard simplification (Hyndman &
//! Billah 2003) is exact and dependency-free:
//!
//! 1. Fit the ordinary-least-squares trend line `a + b·t` to the series over
//!    `t = 0..n−1` (the `θ = 0` line is this linear regression).
//! 2. Form the `θ = 2` line `Z_t = 2·y_t − (a + b·t)`, which doubles the local
//!    curvature around the trend.
//! 3. Extrapolate the `θ = 2` line by simple exponential smoothing (SES) to get
//!    its `h`-step forecast level `ℓ`.
//! 4. Recombine: the forecast is the average of the two theta lines extrapolated,
//!    `ŷ_{n+h} = ½·ℓ + ½·(a + b·(n − 1 + h))`. With `θ = 2` the SES level
//!    captures the short-term signal and the regression supplies the long-term
//!    drift, so the recombination is `ℓ/… ` plus half the linear trend's slope per
//!    step.
//!
//! Concretely we extrapolate the SES level flat and add half the regression slope
//! per step ahead, the standard `θ = 2` closed form. This beats the naive method
//! on trended series because it carries the regression drift forward.

use crate::error::ForecastError;
use crate::params::{ForecastPoint, ForecastResult, ThetaParams};

/// Ordinary-least-squares fit of `y_t = a + b·t` over `t = 0..n−1`. Returns
/// `(a, b)`.
#[allow(clippy::cast_precision_loss)]
fn linear_trend(y: &[f64]) -> (f64, f64) {
    let n = y.len() as f64;
    let mean_t = (n - 1.0) / 2.0;
    let mean_y = y.iter().sum::<f64>() / n;
    let mut sxy = 0.0;
    let mut sxx = 0.0;
    for (t, &yt) in y.iter().enumerate() {
        let dt = t as f64 - mean_t;
        sxy += dt * (yt - mean_y);
        sxx += dt * dt;
    }
    let b = if sxx > 0.0 { sxy / sxx } else { 0.0 };
    let a = mean_y - b * mean_t;
    (a, b)
}

/// Simple exponential smoothing: return the final level after smoothing `y` with
/// coefficient `alpha`. The level is initialized to the first observation.
fn ses_level(y: &[f64], alpha: f64) -> f64 {
    let mut level = y[0];
    for &yt in &y[1..] {
        level = alpha.mul_add(yt, (1.0 - alpha) * level);
    }
    level
}

/// Forecast with the Theta method (`θ = 2`).
///
/// # Errors
///
/// [`ForecastError::ZeroHorizon`] when `horizon == 0`;
/// [`ForecastError::TooShort`] when fewer than two observations are supplied (a
/// trend line is undefined).
pub fn theta(y: &[f64], horizon: usize) -> Result<ForecastResult, ForecastError> {
    if horizon == 0 {
        return Err(ForecastError::ZeroHorizon);
    }
    let n = y.len();
    if n < 2 {
        return Err(ForecastError::TooShort { needed: 2, got: n });
    }

    let (intercept, slope) = linear_trend(y);

    // The θ=2 line Z_t = 2 y_t − (a + b t). Its SES level is the short-term
    // forecast component; the regression supplies the drift.
    let theta_line: Vec<f64> = y
        .iter()
        .enumerate()
        .map(|(t, &yt)| {
            #[allow(clippy::cast_precision_loss)]
            let trend = slope.mul_add(t as f64, intercept);
            2.0_f64.mul_add(yt, -trend)
        })
        .collect();

    // A fixed, deterministic SES coefficient (the Theta-method default α ≈ 0.5
    // performs well and keeps the method parameter-free for the caller).
    let alpha = 0.5;
    let level = ses_level(&theta_line, alpha);

    // Recombine the two theta lines. The θ=0 (linear-regression) line contributes
    // half the slope per step; the θ=2 SES level contributes the other half,
    // held flat. So ŷ_{n+h} = ½·level + ½·(a + b·(n − 1 + h)).
    let points = (1..=horizon)
        .map(|h| {
            #[allow(clippy::cast_precision_loss)]
            let future_t = (n - 1 + h) as f64;
            let regression = slope.mul_add(future_t, intercept);
            ForecastPoint {
                step: h,
                value: 0.5_f64.mul_add(level, 0.5 * regression),
            }
        })
        .collect();

    Ok(ForecastResult {
        model: "theta".to_string(),
        points,
    })
}

/// Theta route dispatch from typed params.
///
/// # Errors
///
/// Propagates [`theta`].
pub fn theta_from_params(p: &ThetaParams) -> Result<ForecastResult, ForecastError> {
    theta(&p.y, p.horizon)
}

#[cfg(test)]
mod tests {
    use super::theta;
    use crate::baseline::naive;
    use crate::params::PrecisionMetrics;

    /// One-step RMSE of a forecaster over an expanding window (helper for the
    /// "theta beats naive" claim).
    fn one_step_rmse(values: &[f64], forecasts: &[f64]) -> f64 {
        let sq: f64 = values
            .iter()
            .zip(forecasts.iter())
            .map(|(a, f)| (a - f) * (a - f))
            .sum();
        #[allow(clippy::cast_precision_loss)]
        (sq / values.len() as f64).sqrt()
    }

    #[test]
    fn theta_carries_the_trend_forward() {
        // A clean upward line: theta should extrapolate upward (carries the
        // regression drift), unlike naive which would repeat the last value.
        let y: Vec<f64> = (1..=10).map(f64::from).collect();
        let fc = theta(&y, 3).expect("forecast");
        let values: Vec<f64> = fc.points.iter().map(|p| p.value).collect();
        assert!(values[0] > 10.0, "theta h1 {} should exceed 10", values[0]);
        assert!(values[1] > values[0]);
        assert!(values[2] > values[1]);
        assert_eq!(fc.model, "theta");
    }

    #[test]
    fn theta_beats_naive_on_a_trended_series() {
        // Hold out the last 3 points of a trended series and compare the 1-step
        // forecast accuracy of theta vs naive. Theta, which carries the drift,
        // must have the lower error.
        let full: Vec<f64> = (1..=13).map(|v| 2.0 * f64::from(v)).collect(); // slope 2
        let train = &full[..10];
        let holdout = &full[10..]; // [22, 24, 26]

        // Naive forecasts the last train value (20) flat.
        let naive_fc = naive(train, 3).expect("naive");
        let naive_vals: Vec<f64> = naive_fc.points.iter().map(|p| p.value).collect();

        let theta_fc = theta(train, 3).expect("theta");
        let theta_vals: Vec<f64> = theta_fc.points.iter().map(|p| p.value).collect();

        let naive_rmse = one_step_rmse(holdout, &naive_vals);
        let theta_rmse = one_step_rmse(holdout, &theta_vals);
        assert!(
            theta_rmse < naive_rmse,
            "theta RMSE {theta_rmse} should beat naive RMSE {naive_rmse}"
        );
        // Sanity on the metric helper type usage.
        let _ = PrecisionMetrics::default();
    }

    #[test]
    fn theta_on_flat_series_is_flat() {
        // A constant series has zero slope: theta forecasts the constant.
        let y = vec![7.0; 8];
        let fc = theta(&y, 2).expect("forecast");
        for p in &fc.points {
            assert!((p.value - 7.0).abs() < 1e-9, "flat {}", p.value);
        }
    }
}
