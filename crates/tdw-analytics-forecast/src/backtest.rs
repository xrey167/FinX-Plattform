//! Backtesting harness: expanding-window historical forecasts + precision
//! metrics.
//!
//! Mirrors `darts`' `historical_forecasts` / `OpenBB`'s backtest precision
//! report. An
//! expanding-window backtest fixes a first training window of `initial`
//! observations, fits the chosen model, forecasts `horizon` steps, records the
//! `horizon`-step-ahead forecast against the realized value, then expands the
//! window by one observation and repeats until the window reaches the end of the
//! series. The collected (actual, forecast) pairs are scored with the standard
//! precision metrics ([`crate::metrics`]).
//!
//! This is deterministic: the only models offered to the backtester are the
//! deterministic classical forecasters (naive, seasonal-naive, random-walk-drift,
//! Holt-Winters, Theta).

use crate::autoarima::{self, Criterion};
use crate::error::ForecastError;
use crate::metrics::precision_metrics;
use crate::params::{BacktestParams, ForecastModel, PrecisionMetrics};
use crate::{baseline, expo, theta};

/// Forecast the `horizon`-step-ahead value of `train` with `model`, returning the
/// single horizon-step point (the value compared against the realized future).
fn forecast_point(
    model: ForecastModel,
    train: &[f64],
    horizon: usize,
    period: usize,
) -> Result<f64, ForecastError> {
    let result = match model {
        ForecastModel::Naive => baseline::naive(train, horizon)?,
        ForecastModel::SeasonalNaive => baseline::seasonal_naive(train, horizon, period.max(2))?,
        ForecastModel::Rwd => baseline::random_walk_drift(train, horizon)?,
        ForecastModel::Expo => expo::holt_winters(train, horizon, 0.5, 0.1, 0.1, period)?,
        ForecastModel::Theta => theta::theta(train, horizon)?,
        ForecastModel::AutoArima => {
            autoarima::auto_arima(train, horizon, 3, 3, 2, Criterion::Aicc)?.forecast
        }
    };
    // The horizon-step-ahead point is the last forecast point.
    result
        .points
        .last()
        .map(|p| p.value)
        .ok_or(ForecastError::ZeroHorizon)
}

/// Run an expanding-window backtest and score the realized-vs-forecast pairs.
///
/// # Errors
///
/// [`ForecastError::ZeroHorizon`] when `horizon == 0`; [`ForecastError::TooShort`]
/// when the series cannot supply a single fold (`initial + horizon` exceeds the
/// series length); propagates the model's own errors.
pub fn backtest(
    y: &[f64],
    model: ForecastModel,
    initial: usize,
    horizon: usize,
    period: usize,
) -> Result<PrecisionMetrics, ForecastError> {
    if horizon == 0 {
        return Err(ForecastError::ZeroHorizon);
    }
    let n = y.len();
    // Default the first training window to half the series when unset.
    let initial = if initial == 0 { n / 2 } else { initial };
    let initial = initial.max(2); // every offered model needs >= 2 points.

    if initial + horizon > n {
        return Err(ForecastError::TooShort {
            needed: initial + horizon,
            got: n,
        });
    }

    let mut actuals = Vec::new();
    let mut forecasts = Vec::new();
    // Origin `o` trains on y[..o] and predicts y[o + horizon − 1].
    let mut origin = initial;
    while origin + horizon <= n {
        let train = &y[..origin];
        let predicted = forecast_point(model, train, horizon, period)?;
        let realized = y[origin + horizon - 1];
        forecasts.push(predicted);
        actuals.push(realized);
        origin += 1;
    }

    if actuals.is_empty() {
        return Err(ForecastError::TooShort {
            needed: initial + horizon,
            got: n,
        });
    }

    precision_metrics(&actuals, &forecasts)
}

/// Backtest route dispatch from typed params.
///
/// # Errors
///
/// Propagates [`backtest`].
pub fn backtest_from_params(p: &BacktestParams) -> Result<PrecisionMetrics, ForecastError> {
    backtest(&p.y, p.model, p.initial, p.horizon, p.period)
}

#[cfg(test)]
mod tests {
    use super::backtest;
    use crate::params::ForecastModel;

    #[test]
    fn backtest_of_rwd_on_a_line_is_near_perfect() {
        // A clean line: random-walk-with-drift learns the exact slope, so the
        // one-step backtest error should be ~0.
        let y: Vec<f64> = (1..=12).map(|v| 3.0 * f64::from(v)).collect();
        let m = backtest(&y, ForecastModel::Rwd, 4, 1, 0).expect("backtest");
        assert!(m.rmse < 1e-9, "rmse {}", m.rmse);
        assert!(m.mae < 1e-9, "mae {}", m.mae);
        assert!(m.n >= 1);
    }

    #[test]
    fn backtest_naive_has_positive_error_on_a_trend() {
        // Naive (repeat last) lags a trend, so its backtest error is positive and
        // equals the step size (slope) at one-step horizon. Slope 5 ⇒ each
        // one-step error is exactly 5 ⇒ RMSE = MAE = 5.
        let y: Vec<f64> = (0..10).map(|v| 5.0 * f64::from(v)).collect();
        let m = backtest(&y, ForecastModel::Naive, 4, 1, 0).expect("backtest");
        assert!((m.rmse - 5.0).abs() < 1e-9, "rmse {}", m.rmse);
        assert!((m.mae - 5.0).abs() < 1e-9, "mae {}", m.mae);
    }

    #[test]
    fn backtest_rejects_too_short() {
        assert!(matches!(
            backtest(&[1.0, 2.0, 3.0], ForecastModel::Naive, 5, 1, 0),
            Err(crate::error::ForecastError::TooShort { .. })
        ));
    }

    #[test]
    fn backtest_autoarima_on_a_trend_is_well_defined() {
        // AutoARIMA backtests like the other models: on a clean linear trend its
        // differencing decision makes the one-step error small and finite.
        let y: Vec<f64> = (0..24).map(|v| 2.0 * f64::from(v)).collect();
        let m = backtest(&y, ForecastModel::AutoArima, 14, 1, 0).expect("backtest");
        assert!(m.n >= 1);
        assert!(m.rmse.is_finite());
        assert!(m.mae.is_finite());
        // The slope-2 trend is exactly differenced away, so the one-step error is
        // small relative to the series scale.
        assert!(m.rmse < 5.0, "autoarima trend rmse {} too large", m.rmse);
    }

    #[test]
    fn backtest_seasonal_naive_recovers_period() {
        // A perfectly periodic series: seasonal-naive backtest error is ~0.
        let y = vec![1.0, 2.0, 3.0, 1.0, 2.0, 3.0, 1.0, 2.0, 3.0];
        let m = backtest(&y, ForecastModel::SeasonalNaive, 6, 1, 3).expect("backtest");
        assert!(m.rmse < 1e-9, "rmse {}", m.rmse);
    }
}
