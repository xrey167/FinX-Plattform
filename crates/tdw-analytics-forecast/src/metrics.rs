//! Forecast-accuracy (precision) metrics: RMSE, MAE, MAPE, SMAPE.
//!
//! Definitions (Hyndman & Koehler 2006; Hyndman & Athanasopoulos, *Forecasting:
//! Principles and Practice*, §5.8), over an aligned actual/forecast pair of length
//! `n`:
//!
//! ```text
//! RMSE  = sqrt( (1/n) Σ (aᵢ − fᵢ)² )
//! MAE   = (1/n) Σ |aᵢ − fᵢ|
//! MAPE  = (100/n') Σ_{aᵢ≠0} |aᵢ − fᵢ| / |aᵢ|                  (percent)
//! SMAPE = (100/n'') Σ |aᵢ − fᵢ| / ((|aᵢ| + |fᵢ|)/2)           (percent)
//! ```
//!
//! `MAPE` skips terms with a zero actual (undefined division) and `SMAPE` skips
//! terms whose magnitude sum is zero, averaging over the count that remained.

use crate::error::ForecastError;
use crate::params::{MetricsParams, PrecisionMetrics};

/// Compute the four precision metrics for an aligned (`actual`, `forecast`) pair.
///
/// # Errors
///
/// [`ForecastError::EmptySeries`] when the inputs are empty;
/// [`ForecastError::LengthMismatch`] when the two series differ in length.
pub fn precision_metrics(
    actual: &[f64],
    forecast: &[f64],
) -> Result<PrecisionMetrics, ForecastError> {
    if actual.is_empty() {
        return Err(ForecastError::EmptySeries);
    }
    if actual.len() != forecast.len() {
        return Err(ForecastError::LengthMismatch {
            a: actual.len(),
            b: forecast.len(),
        });
    }

    let n = actual.len();
    let mut sse = 0.0;
    let mut sae = 0.0;
    let mut mape_sum = 0.0;
    let mut mape_n = 0usize;
    let mut smape_sum = 0.0;
    let mut smape_n = 0usize;

    for (&a, &f) in actual.iter().zip(forecast.iter()) {
        let err = a - f;
        let abs_err = err.abs();
        sse += err * err;
        sae += abs_err;

        if a != 0.0 {
            mape_sum += abs_err / a.abs();
            mape_n += 1;
        }
        let denom = f64::midpoint(a.abs(), f.abs());
        if denom != 0.0 {
            smape_sum += abs_err / denom;
            smape_n += 1;
        }
    }

    #[allow(clippy::cast_precision_loss)]
    let nf = n as f64;
    let root_mse = (sse / nf).sqrt();
    let mean_abs = sae / nf;
    #[allow(clippy::cast_precision_loss)]
    let pct_err = if mape_n > 0 {
        100.0 * mape_sum / mape_n as f64
    } else {
        0.0
    };
    #[allow(clippy::cast_precision_loss)]
    let sym_pct_err = if smape_n > 0 {
        100.0 * smape_sum / smape_n as f64
    } else {
        0.0
    };

    Ok(PrecisionMetrics {
        rmse: root_mse,
        mae: mean_abs,
        mape: pct_err,
        smape: sym_pct_err,
        n,
    })
}

/// Metrics route dispatch from typed params.
///
/// # Errors
///
/// Propagates [`precision_metrics`].
pub fn metrics_from_params(p: &MetricsParams) -> Result<PrecisionMetrics, ForecastError> {
    precision_metrics(&p.actual, &p.forecast)
}

#[cfg(test)]
mod tests {
    use super::precision_metrics;

    #[test]
    fn metrics_match_hand_calculation() {
        // actual = [100, 200, 300]; forecast = [110, 190, 330].
        // errors = [-10, 10, -30]; |err| = [10,10,30].
        // RMSE = sqrt((100+100+900)/3) = sqrt(1100/3) = sqrt(366.666..) = 19.1485..
        // MAE  = (10+10+30)/3 = 50/3 = 16.6666..
        // MAPE = 100/3 * (10/100 + 10/200 + 30/300)
        //      = 100/3 * (0.10 + 0.05 + 0.10) = 100/3 * 0.25 = 8.3333..
        // SMAPE term denoms = (210/2, 390/2, 630/2) = (105, 195, 315)
        //      = 100/3 * (10/105 + 10/195 + 30/315)
        //      = 100/3 * (0.0952381 + 0.0512821 + 0.0952381)
        //      = 100/3 * 0.2417582 = 8.058608..
        let actual = [100.0, 200.0, 300.0];
        let forecast = [110.0, 190.0, 330.0];
        let m = precision_metrics(&actual, &forecast).expect("metrics");

        assert!(
            (m.rmse - (1100.0_f64 / 3.0).sqrt()).abs() < 1e-9,
            "rmse {}",
            m.rmse
        );
        assert!((m.mae - 50.0 / 3.0).abs() < 1e-9, "mae {}", m.mae);
        assert!((m.mape - 25.0 / 3.0).abs() < 1e-9, "mape {}", m.mape);

        let smape_expected = 100.0 / 3.0 * (10.0 / 105.0 + 10.0 / 195.0 + 30.0 / 315.0);
        assert!((m.smape - smape_expected).abs() < 1e-9, "smape {}", m.smape);
        assert_eq!(m.n, 3);
    }

    #[test]
    fn perfect_forecast_has_zero_error() {
        let actual = [1.0, 2.0, 3.0, 4.0];
        let m = precision_metrics(&actual, &actual).expect("metrics");
        assert!(m.rmse.abs() < 1e-12);
        assert!(m.mae.abs() < 1e-12);
        assert!(m.mape.abs() < 1e-12);
        assert!(m.smape.abs() < 1e-12);
    }

    #[test]
    fn mape_skips_zero_actuals() {
        // The middle actual is zero: MAPE averages over the other two terms only.
        // actual = [10, 0, 20]; forecast = [12, 5, 18]
        // |err| = [2, 5, 2]; MAPE over a≠0: 100/2*(2/10 + 2/20) = 100/2*0.3 = 15.
        let actual = [10.0, 0.0, 20.0];
        let forecast = [12.0, 5.0, 18.0];
        let m = precision_metrics(&actual, &forecast).expect("metrics");
        assert!((m.mape - 15.0).abs() < 1e-9, "mape {}", m.mape);
    }

    #[test]
    fn length_mismatch_errors() {
        assert!(matches!(
            precision_metrics(&[1.0, 2.0], &[1.0]),
            Err(crate::error::ForecastError::LengthMismatch { .. })
        ));
    }
}
