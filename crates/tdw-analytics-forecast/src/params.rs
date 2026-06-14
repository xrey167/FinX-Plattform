//! Typed parameter and output structs for every forecasting catalog route.
//!
//! Like the econometric routes (and unlike the single-series technical/quant
//! routes, which take their value series via the dispatch envelope's
//! `data`/`source`), the forecasting routes carry their inputs inline in the
//! typed params — the historical series `y`, the forecast horizon, the seasonal
//! period, the smoothing knobs. Every struct derives `JsonSchema` so the catalog
//! can publish the exact request shape, and `serde` defaults so optional fields
//! may be omitted.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

const fn d1() -> usize {
    1
}
const fn d2() -> usize {
    2
}

/// Baseline forecast parameters (seasonal-naive and random-walk-with-drift): the
/// historical series and the forecast horizon, plus the seasonal period for the
/// seasonal-naive baseline.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct BaselineParams {
    /// Historical observations in time order (oldest first).
    #[serde(default)]
    pub y: Vec<f64>,
    /// Number of future steps to forecast (`>= 1`).
    #[serde(default = "d1")]
    pub horizon: usize,
    /// Seasonal period (`>= 2`); used only by the seasonal-naive baseline (the
    /// last full season is repeated). Ignored by random-walk-with-drift. Defaults
    /// to `1`, which the seasonal-naive route rejects as having no seasonal
    /// structure.
    #[serde(default = "d1")]
    pub period: usize,
}

impl Default for BaselineParams {
    fn default() -> Self {
        Self {
            y: Vec::new(),
            horizon: 1,
            period: 1,
        }
    }
}

const fn default_alpha() -> f64 {
    0.5
}
const fn default_beta() -> f64 {
    0.1
}
const fn default_gamma() -> f64 {
    0.1
}

/// Holt-Winters exponential-smoothing parameters: the series, the horizon, the
/// smoothing coefficients, and the optional additive seasonal period.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ExpoParams {
    /// Historical observations in time order (oldest first).
    #[serde(default)]
    pub y: Vec<f64>,
    /// Number of future steps to forecast (`>= 1`).
    #[serde(default = "d1")]
    pub horizon: usize,
    /// Level smoothing coefficient `alpha` in `[0, 1]`.
    #[serde(default = "default_alpha")]
    pub alpha: f64,
    /// Trend smoothing coefficient `beta` in `[0, 1]`. When `0` the forecast has
    /// no evolving trend (simple/level smoothing toward a fixed slope).
    #[serde(default = "default_beta")]
    pub beta: f64,
    /// Seasonal smoothing coefficient `gamma` in `[0, 1]`; used only when
    /// `period >= 2`.
    #[serde(default = "default_gamma")]
    pub gamma: f64,
    /// Additive seasonal period (`>= 2` to enable the seasonal component). When
    /// `0` or `1` the model is non-seasonal (Holt's linear trend). Defaults to
    /// `0` (non-seasonal).
    #[serde(default)]
    pub period: usize,
}

impl Default for ExpoParams {
    fn default() -> Self {
        Self {
            y: Vec::new(),
            horizon: 1,
            alpha: default_alpha(),
            beta: default_beta(),
            gamma: default_gamma(),
            period: 0,
        }
    }
}

/// ETS (error-trend-seasonal) model-selection parameters.
///
/// Carries the series, the horizon, and the candidate seasonal period. The
/// selector fits the additive variants (level-only, level+trend, and — when
/// `period >= 2` — level+trend+seasonal) and returns the one with the lowest
/// in-sample one-step SSE.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct EtsParams {
    /// Historical observations in time order (oldest first).
    #[serde(default)]
    pub y: Vec<f64>,
    /// Number of future steps to forecast (`>= 1`).
    #[serde(default = "d1")]
    pub horizon: usize,
    /// Candidate additive seasonal period (`>= 2` to include the seasonal
    /// variant). Defaults to `0` (only the non-seasonal variants are considered).
    #[serde(default)]
    pub period: usize,
}

impl Default for EtsParams {
    fn default() -> Self {
        Self {
            y: Vec::new(),
            horizon: 1,
            period: 0,
        }
    }
}

/// Theta-method parameters: the series and the horizon.
///
/// The classic Theta method (theta = 2) splits the series into a linear-trend
/// component and a simple-exponential-smoothing component and recombines them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ThetaParams {
    /// Historical observations in time order (oldest first).
    #[serde(default)]
    pub y: Vec<f64>,
    /// Number of future steps to forecast (`>= 1`).
    #[serde(default = "d1")]
    pub horizon: usize,
}

impl Default for ThetaParams {
    fn default() -> Self {
        Self {
            y: Vec::new(),
            horizon: 1,
        }
    }
}

/// MSTL / seasonal-trend-decomposition parameters: the series and the seasonal
/// period.
///
/// The decomposition is additive (`y = trend + seasonal + residual`) and uses a
/// centered moving average for the trend and the per-phase deseasonalized means
/// for the seasonal component (a simple, documented STL variant).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MstlParams {
    /// Historical observations in time order (oldest first).
    #[serde(default)]
    pub y: Vec<f64>,
    /// Seasonal period (`>= 2`).
    #[serde(default = "d2")]
    pub period: usize,
}

impl Default for MstlParams {
    fn default() -> Self {
        Self {
            y: Vec::new(),
            period: 2,
        }
    }
}

/// Linear-regression forecast parameters.
///
/// Carries the series, the horizon, the number of autoregressive lag features,
/// and optional exogenous covariates aligned to the series. The forecaster fits
/// `y_t = b0 + sum_i b_i y_{t-i} (+ covariate terms)` by OLS (reusing
/// `tdw-analytics-econometrics`) and rolls the fit forward.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LinRegrParams {
    /// Historical observations in time order (oldest first).
    #[serde(default)]
    pub y: Vec<f64>,
    /// Number of future steps to forecast (`>= 1`).
    #[serde(default = "d1")]
    pub horizon: usize,
    /// Number of autoregressive lag features `y_{t-1} .. y_{t-lags}` (`>= 1`).
    #[serde(default = "d1")]
    pub lags: usize,
    /// Optional exogenous covariate columns. Each column must have length
    /// `y.len() + horizon`: the first `y.len()` values align to the history and
    /// the trailing `horizon` values supply the future covariate path used to
    /// extrapolate. Omit (empty) for a pure autoregressive forecast.
    #[serde(default)]
    pub covariates: Vec<Vec<f64>>,
}

impl Default for LinRegrParams {
    fn default() -> Self {
        Self {
            y: Vec::new(),
            horizon: 1,
            lags: 1,
            covariates: Vec::new(),
        }
    }
}

const fn d3() -> usize {
    3
}

const fn default_criterion() -> String {
    String::new()
}

/// `ARIMA(p, d, q)` forecast parameters: the series, the horizon, and the fixed
/// model order.
///
/// The model is estimated by the `Hannan-Rissanen` two-stage least-squares
/// procedure (reusing the `tdw-analytics-econometrics` OLS core), forecast on the
/// `d`-th-difference scale, and integrated back to the level of the series.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ArimaParams {
    /// Historical observations in time order (oldest first).
    #[serde(default)]
    pub y: Vec<f64>,
    /// Number of future steps to forecast (`>= 1`).
    #[serde(default = "d1")]
    pub horizon: usize,
    /// Autoregressive order `p` (`>= 0`).
    #[serde(default = "d1")]
    pub p: usize,
    /// Differencing order `d` (`>= 0`).
    #[serde(default)]
    pub d: usize,
    /// Moving-average order `q` (`>= 0`).
    #[serde(default)]
    pub q: usize,
}

impl Default for ArimaParams {
    fn default() -> Self {
        Self {
            y: Vec::new(),
            horizon: 1,
            p: 1,
            d: 0,
            q: 0,
        }
    }
}

/// `AutoARIMA` forecast parameters: the series, the horizon, the order-search
/// bounds, and the selecting information criterion.
///
/// The order `(p, d, q)` is selected by a `Hyndman-Khandakar`-style stepwise
/// search bounded by `p_max` / `q_max` / `d_max`, picking the order with the
/// lowest information criterion. The chosen order is encoded in the forecast's
/// `model` label.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AutoArimaParams {
    /// Historical observations in time order (oldest first).
    #[serde(default)]
    pub y: Vec<f64>,
    /// Number of future steps to forecast (`>= 1`).
    #[serde(default = "d1")]
    pub horizon: usize,
    /// Maximum autoregressive order `p` to search. Defaults to `3`.
    #[serde(default = "d3")]
    pub p_max: usize,
    /// Maximum moving-average order `q` to search. Defaults to `3`.
    #[serde(default = "d3")]
    pub q_max: usize,
    /// Maximum differencing order `d` to consider. Defaults to `2`.
    #[serde(default = "d2")]
    pub d_max: usize,
    /// Information criterion driving the search: `"aic"`, `"aicc"`, or `"bic"`.
    /// Any other value (including the empty default) selects `AICc`.
    #[serde(default = "default_criterion")]
    pub criterion: String,
}

impl Default for AutoArimaParams {
    fn default() -> Self {
        Self {
            y: Vec::new(),
            horizon: 1,
            p_max: 3,
            q_max: 3,
            d_max: 2,
            criterion: String::new(),
        }
    }
}

/// The forecast model identifier for the backtest harness, selecting which
/// forecaster the expanding-window backtest re-fits at each fold.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ForecastModel {
    /// The naive baseline (repeat the last observation).
    #[default]
    Naive,
    /// Seasonal-naive (repeat the last full season).
    SeasonalNaive,
    /// Random walk with drift.
    Rwd,
    /// Holt-Winters exponential smoothing.
    Expo,
    /// The Theta method.
    Theta,
    /// `AutoARIMA` (automatic `ARIMA(p, d, q)` order selection).
    AutoArima,
}

/// Backtest (historical-forecasts) parameters: the series, the model to
/// re-fit, an expanding-window split, and the forecast step used at each fold.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct BacktestParams {
    /// Historical observations in time order (oldest first).
    #[serde(default)]
    pub y: Vec<f64>,
    /// The forecast model re-fit at each expanding-window origin.
    #[serde(default)]
    pub model: ForecastModel,
    /// Number of initial observations in the first training window (`>= 1`). The
    /// window then expands by `step` at each fold. Defaults to half the series
    /// when `0`.
    #[serde(default)]
    pub initial: usize,
    /// Forecast horizon evaluated at each origin (`>= 1`); the metric compares the
    /// `horizon`-step-ahead forecast against the realized values. Defaults to `1`.
    #[serde(default = "d1")]
    pub horizon: usize,
    /// Seasonal period for the seasonal-naive / Holt-Winters models (`>= 2` to
    /// enable seasonality). Defaults to `0`.
    #[serde(default)]
    pub period: usize,
}

impl Default for BacktestParams {
    fn default() -> Self {
        Self {
            y: Vec::new(),
            model: ForecastModel::Naive,
            initial: 0,
            horizon: 1,
            period: 0,
        }
    }
}

/// Precision-metrics parameters: a pair of equal-length series — the realized
/// `actual` values and the `forecast` values to score against them.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MetricsParams {
    /// The realized values.
    #[serde(default)]
    pub actual: Vec<f64>,
    /// The forecast values, aligned 1:1 with `actual`.
    #[serde(default)]
    pub forecast: Vec<f64>,
}

const fn default_lower_q() -> f64 {
    0.05
}
const fn default_upper_q() -> f64 {
    0.95
}

/// Quantile anomaly-detection parameters: the series plus the lower/upper
/// empirical-quantile bounds.
///
/// A point is flagged when it falls outside the `[lower_quantile,
/// upper_quantile]` empirical band of the series values.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AnomalyParams {
    /// Observations to scan for anomalies, in time order.
    #[serde(default)]
    pub y: Vec<f64>,
    /// Lower quantile in `[0, 1]`; values below this empirical quantile are
    /// flagged. Defaults to `0.05`.
    #[serde(default = "default_lower_q")]
    pub lower_quantile: f64,
    /// Upper quantile in `[0, 1]`; values above this empirical quantile are
    /// flagged. Defaults to `0.95`.
    #[serde(default = "default_upper_q")]
    pub upper_quantile: f64,
}

impl Default for AnomalyParams {
    fn default() -> Self {
        Self {
            y: Vec::new(),
            lower_quantile: default_lower_q(),
            upper_quantile: default_upper_q(),
        }
    }
}

// --- Output models -----------------------------------------------------------

/// A single forecast point: the step-ahead index (`1`-based) and the predicted
/// value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ForecastPoint {
    /// One-based step ahead of the last observation (`1` = next period).
    pub step: usize,
    /// The point forecast for this step.
    pub value: f64,
}

/// A forecast result: the per-step point forecasts plus the model label that
/// produced them.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ForecastResult {
    /// The model that produced the forecast (e.g. `"seasonalnaive"`, `"expo"`).
    pub model: String,
    /// The point forecasts, one per future step in horizon order.
    pub points: Vec<ForecastPoint>,
}

/// The four standard scale-dependent / percentage forecast-accuracy measures.
///
/// All are computed over the aligned (`actual`, `forecast`) pair: `RMSE` and
/// `MAE` in the data's units, `MAPE` and `SMAPE` as percentages. `MAPE` and
/// `SMAPE` skip terms whose denominator is zero (a zero actual for MAPE, a zero
/// magnitude sum for SMAPE) and report the count actually averaged.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PrecisionMetrics {
    /// Root mean squared error `sqrt(mean((actual - forecast)^2))`.
    pub rmse: f64,
    /// Mean absolute error `mean(|actual - forecast|)`.
    pub mae: f64,
    /// Mean absolute percentage error `100 * mean(|actual - forecast| /
    /// |actual|)`, in percent, skipping zero-actual terms.
    pub mape: f64,
    /// Symmetric MAPE `100 * mean(|actual - forecast| / ((|actual| +
    /// |forecast|) / 2))`, in percent, skipping zero-magnitude terms.
    pub smape: f64,
    /// Number of aligned points scored.
    pub n: usize,
}

/// An additive seasonal-trend decomposition.
///
/// Carries the per-observation trend, seasonal, and residual components such that
/// `y[t] = trend[t] + seasonal[t] + residual[t]` (the trend carries `NaN`-free
/// finite values at the interior where the centered moving average is defined and
/// is linearly extrapolated at the edges).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Decomposition {
    /// The estimated trend-cycle component (centered moving average).
    pub trend: Vec<f64>,
    /// The estimated seasonal component (per-phase deseasonalized means, mean
    /// zero across one full period).
    pub seasonal: Vec<f64>,
    /// The residual `y - trend - seasonal`.
    pub residual: Vec<f64>,
    /// The seasonal period used for the decomposition.
    pub period: usize,
}

/// One flagged anomaly: the index in the series, the value, and which band edge
/// it breached.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Anomaly {
    /// Zero-based index of the flagged observation in the series.
    pub index: usize,
    /// The flagged value.
    pub value: f64,
    /// `true` when the value is above the upper band, `false` when below the
    /// lower band.
    pub above: bool,
}

/// The anomaly-detection result: the empirical band bounds and the flagged
/// points.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AnomalyResult {
    /// The lower band value (the empirical `lower_quantile` of the series).
    pub lower_bound: f64,
    /// The upper band value (the empirical `upper_quantile` of the series).
    pub upper_bound: f64,
    /// The flagged anomalies, in series order.
    pub anomalies: Vec<Anomaly>,
}

#[cfg(test)]
mod tests {
    use super::{AnomalyParams, BacktestParams, BaselineParams, ExpoParams, ForecastModel};

    #[test]
    fn baseline_horizon_defaults_to_one() {
        let p: BaselineParams = serde_json::from_str(r#"{ "y": [1,2,3] }"#).expect("parses");
        assert_eq!(p.horizon, 1);
        assert_eq!(p.period, 1);
        assert_eq!(p.y, vec![1.0, 2.0, 3.0]);
    }

    #[test]
    fn expo_defaults_coefficients() {
        let p: ExpoParams = serde_json::from_str("{}").expect("parses");
        assert!((p.alpha - 0.5).abs() < f64::EPSILON);
        assert!((p.beta - 0.1).abs() < f64::EPSILON);
        assert_eq!(p.period, 0);
    }

    #[test]
    fn backtest_model_defaults_to_naive() {
        let p: BacktestParams = serde_json::from_str("{}").expect("parses");
        assert_eq!(p.model, ForecastModel::Naive);
    }

    #[test]
    fn forecast_model_parses_snake_case() {
        let p: ForecastModel = serde_json::from_str(r#""seasonal_naive""#).expect("parses");
        assert_eq!(p, ForecastModel::SeasonalNaive);
    }

    #[test]
    fn anomaly_defaults_quantiles() {
        let p: AnomalyParams = serde_json::from_str(r#"{ "y": [1,2,3] }"#).expect("parses");
        assert!((p.lower_quantile - 0.05).abs() < f64::EPSILON);
        assert!((p.upper_quantile - 0.95).abs() < f64::EPSILON);
    }
}
