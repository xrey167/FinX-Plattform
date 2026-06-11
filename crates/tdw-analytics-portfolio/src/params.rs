//! Typed parameter structs for every portfolio metric.
//!
//! Unlike the single-series technical/quantitative routes (which take their value
//! series via the dispatch envelope's `data`/`source`), the portfolio metrics
//! carry richer, sometimes multi-vector inputs — a returns series for
//! `cumulative_returns`, an equity / cumulative-return series for `drawdown` and
//! `max_drawdown`, a vector of position values for `allocation`, and an aligned
//! `weights` + `returns` pair for `contribution`. These structs carry those
//! inputs inline (mirroring the `tdw-analytics-econometrics` convention) and
//! derive `JsonSchema` so the catalog can publish the exact request shape.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Parameters for the `cumulative_returns` route: a periodic-returns series.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ReturnsParams {
    /// Per-period simple returns, oldest first (e.g. `0.01` for a `+1%` period).
    #[serde(default)]
    pub returns: Vec<f64>,
}

/// Parameters for the `drawdown` and `max_drawdown` routes: a cumulative-return
/// or equity series.
///
/// The series is an equity / wealth curve (e.g. a `1.0`-based growth-of-a-dollar
/// index, or any positive level series). Each level is compared against the
/// running peak to its left to form the drawdown.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct EquityParams {
    /// Cumulative-return / equity levels, oldest first (e.g. `1.0, 1.1, 1.045`).
    #[serde(default)]
    pub equity: Vec<f64>,
}

/// Parameters for the `allocation` route: a vector of position values to
/// normalize to weights.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AllocationParams {
    /// Per-position market values (any currency unit); they are divided by their
    /// total to yield weights summing to `1`.
    #[serde(default)]
    pub values: Vec<f64>,
}

/// Parameters for the `contribution` route: an aligned `weights` + `returns`
/// pair.
///
/// `weights[i]` is asset `i`'s portfolio weight and `returns[i]` its per-period
/// return; the two vectors must be positionally aligned and the same length.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ContributionParams {
    /// Per-asset portfolio weights (typically summing to `1`, but not required).
    #[serde(default)]
    pub weights: Vec<f64>,
    /// Per-asset returns, aligned one-to-one with `weights`.
    #[serde(default)]
    pub returns: Vec<f64>,
}

#[cfg(test)]
mod tests {
    use super::{AllocationParams, ContributionParams, EquityParams, ReturnsParams};

    #[test]
    fn returns_params_default_is_empty() {
        let parsed: ReturnsParams = serde_json::from_str("{}").expect("deserializes");
        assert_eq!(parsed, ReturnsParams::default());
        assert!(parsed.returns.is_empty());
    }

    #[test]
    fn equity_params_parses_a_level_series() {
        let parsed: EquityParams =
            serde_json::from_str(r#"{ "equity": [1.0, 1.1, 1.045] }"#).expect("deserializes");
        assert_eq!(parsed.equity, vec![1.0, 1.1, 1.045]);
    }

    #[test]
    fn allocation_params_parses_values() {
        let parsed: AllocationParams =
            serde_json::from_str(r#"{ "values": [25.0, 75.0] }"#).expect("deserializes");
        assert_eq!(parsed.values, vec![25.0, 75.0]);
    }

    #[test]
    fn contribution_params_parses_aligned_pair() {
        let parsed: ContributionParams =
            serde_json::from_str(r#"{ "weights": [0.6, 0.4], "returns": [0.1, -0.05] }"#)
                .expect("deserializes");
        assert_eq!(parsed.weights, vec![0.6, 0.4]);
        assert_eq!(parsed.returns, vec![0.1, -0.05]);
    }
}
