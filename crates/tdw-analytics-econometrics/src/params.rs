//! Typed parameter structs for the econometric catalog routes.
//!
//! Unlike the technical/quant routes (which take a single value series via the
//! dispatch envelope's `data`/`source`), the econometric routes need richer,
//! multi-series inputs — a response `y` and a design `X` for OLS, a list of
//! columns for the correlation matrix and VIF, a pair of series plus a lag for
//! Granger causality. These structs carry those inputs inline and derive
//! `JsonSchema` so the catalog can publish the exact request shape.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

const fn d1() -> usize {
    1
}

/// Parameters for the OLS route: a response vector `y` and a design `x` given as
/// a list of regressor columns. An intercept column of ones is added
/// automatically unless `intercept` is `false`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct OlsParams {
    /// Response vector (one value per observation).
    #[serde(default)]
    pub y: Vec<f64>,
    /// Design columns: `x[j]` is the `j`-th regressor, each the same length as
    /// `y`. The intercept is added separately (see `intercept`), so do not
    /// include a column of ones.
    #[serde(default)]
    pub x: Vec<Vec<f64>>,
    /// Whether to prepend an intercept column of ones (default `true`).
    #[serde(default = "default_true")]
    pub intercept: bool,
}

const fn default_true() -> bool {
    true
}

impl Default for OlsParams {
    fn default() -> Self {
        Self {
            y: Vec::new(),
            x: Vec::new(),
            intercept: true,
        }
    }
}

/// Parameters for the correlation-matrix and VIF routes: a list of equal-length
/// columns.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ColumnsParams {
    /// The columns to correlate / score, each the same length.
    #[serde(default)]
    pub columns: Vec<Vec<f64>>,
}

/// Parameters for the Granger-causality route: a pair of equal-length series and
/// the lag order.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct GrangerParams {
    /// The dependent series `y` whose prediction is tested.
    #[serde(default)]
    pub y: Vec<f64>,
    /// The candidate causal series `x` whose lags may improve `y`'s prediction.
    #[serde(default)]
    pub x: Vec<f64>,
    /// Number of lags of each series to include (default `1`).
    #[serde(default = "d1")]
    pub lag: usize,
}

impl Default for GrangerParams {
    fn default() -> Self {
        Self {
            y: Vec::new(),
            x: Vec::new(),
            lag: 1,
        }
    }
}

/// Parameters for the cointegration route: a pair of equal-length level series.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CointegrationParams {
    /// The first (dependent) level series `y`.
    #[serde(default)]
    pub y: Vec<f64>,
    /// The second (regressor) level series `x`.
    #[serde(default)]
    pub x: Vec<f64>,
}

#[cfg(test)]
mod tests {
    use super::{ColumnsParams, GrangerParams, OlsParams};

    #[test]
    fn ols_intercept_defaults_to_true() {
        let parsed: OlsParams = serde_json::from_str(r#"{ "y": [1,2,3] }"#).expect("deserializes");
        assert!(parsed.intercept);
        assert_eq!(parsed.y, vec![1.0, 2.0, 3.0]);
    }

    #[test]
    fn granger_lag_defaults_to_one() {
        let parsed: GrangerParams = serde_json::from_str("{}").expect("deserializes");
        assert_eq!(parsed.lag, 1);
    }

    #[test]
    fn columns_params_parses_nested_arrays() {
        let parsed: ColumnsParams =
            serde_json::from_str(r#"{ "columns": [[1,2],[3,4]] }"#).expect("deserializes");
        assert_eq!(parsed.columns.len(), 2);
    }
}
