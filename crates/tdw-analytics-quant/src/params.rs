//! Typed parameter structs for every quantitative metric.
//!
//! Each struct carries the metric's tuning knobs (risk-free rate, annualization
//! factor, target threshold, confidence level) with `serde` defaults, and
//! derives `JsonSchema` so the daemon catalog can publish the parameter schema.
//! Defaults are supplied via `#[serde(default = "…")]` free functions so a
//! request may omit any field and still deserialize to the canonical default.
//!
//! # Annualization convention
//!
//! Ratios and volatility are reported per the supplied `periods` factor: pass
//! `252` for daily returns, `12` for monthly, `1` for already-annual returns.
//! The default of `1.0` leaves the per-period statistic un-scaled, so a caller
//! that omits `periods` gets the raw (un-annualized) figure.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

const fn f0() -> f64 {
    0.0
}
const fn f1() -> f64 {
    1.0
}
const fn f0_95() -> f64 {
    0.95
}

/// Parameters for the Sharpe ratio.
///
/// `OpenBB` / Sharpe (1966) defaults: risk-free rate `0`, no annualization
/// (`periods = 1`). The risk-free rate is a *per-period* rate matching the
/// return series' frequency.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SharpeParams {
    /// Per-period risk-free rate subtracted from each return (the excess-return
    /// baseline).
    #[serde(default = "f0")]
    pub risk_free_rate: f64,
    /// Annualization factor: the number of return periods per year (`252` daily,
    /// `12` monthly, `1` already annual). Must be `> 0`.
    #[serde(default = "f1")]
    pub periods: f64,
}

impl Default for SharpeParams {
    fn default() -> Self {
        Self {
            risk_free_rate: 0.0,
            periods: 1.0,
        }
    }
}

/// Parameters for the Sortino ratio.
///
/// Sortino (1994): like Sharpe but the denominator is downside deviation below
/// `target_return` rather than total standard deviation.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SortinoParams {
    /// Per-period risk-free rate subtracted from each return.
    #[serde(default = "f0")]
    pub risk_free_rate: f64,
    /// Minimum acceptable per-period return; only returns below this contribute
    /// to the downside-deviation denominator.
    #[serde(default = "f0")]
    pub target_return: f64,
    /// Annualization factor (periods per year). Must be `> 0`.
    #[serde(default = "f1")]
    pub periods: f64,
}

impl Default for SortinoParams {
    fn default() -> Self {
        Self {
            risk_free_rate: 0.0,
            target_return: 0.0,
            periods: 1.0,
        }
    }
}

/// Parameters for the Omega ratio.
///
/// Keating & Shadwick (2002): the probability-weighted ratio of gains above a
/// threshold to losses below it.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct OmegaParams {
    /// Threshold return separating gains (above) from losses (below).
    #[serde(default = "f0")]
    pub threshold: f64,
}

impl Default for OmegaParams {
    fn default() -> Self {
        Self { threshold: 0.0 }
    }
}

/// Parameters for annualized volatility (the standard deviation of returns
/// scaled by the square root of `periods`).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct VolatilityParams {
    /// Annualization factor (periods per year). Must be `> 0`.
    #[serde(default = "f1")]
    pub periods: f64,
}

impl Default for VolatilityParams {
    fn default() -> Self {
        Self { periods: 1.0 }
    }
}

/// Parameters for Value-at-Risk / Conditional `VaR`.
///
/// Historical-simulation `VaR` at the given confidence level (e.g. `0.95` for
/// the 5% left-tail loss quantile).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct VarParams {
    /// Confidence level in `(0, 1)`; the `VaR` quantile is the `1 − confidence`
    /// left-tail of the return distribution.
    #[serde(default = "f0_95")]
    pub confidence: f64,
}

impl Default for VarParams {
    fn default() -> Self {
        Self { confidence: 0.95 }
    }
}

/// Parameters for the CAPM regression (alpha / beta of the asset versus a
/// benchmark return series).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CapmParams {
    /// Per-period risk-free rate subtracted from both the asset and benchmark
    /// returns before the regression (so alpha/beta are excess-return based).
    #[serde(default = "f0")]
    pub risk_free_rate: f64,
}

impl Default for CapmParams {
    fn default() -> Self {
        Self {
            risk_free_rate: 0.0,
        }
    }
}

/// Empty parameter struct for the no-knob metrics (skewness, kurtosis, max
/// drawdown, expected shortfall when sharing the `VaR` confidence, Jarque-Bera).
///
/// A dedicated unit struct (rather than reusing one of the above) keeps the
/// catalog params schema honest: these routes take only the value series and no
/// tuning parameters, and the schema reflects exactly that.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NoParams {}

#[cfg(test)]
mod tests {
    use super::{SharpeParams, SortinoParams, VarParams, VolatilityParams};

    #[test]
    fn sharpe_defaults_are_zero_rf_unit_periods() {
        let parsed: SharpeParams = serde_json::from_str("{}").expect("empty object deserializes");
        assert_eq!(parsed, SharpeParams::default());
        assert!((parsed.risk_free_rate - 0.0).abs() < f64::EPSILON);
        assert!((parsed.periods - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn sortino_default_target_is_zero() {
        let parsed: SortinoParams = serde_json::from_str("{}").expect("deserializes");
        assert!((parsed.target_return - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn var_default_confidence_is_ninety_five_percent() {
        let parsed: VarParams = serde_json::from_str("{}").expect("deserializes");
        assert!((parsed.confidence - 0.95).abs() < f64::EPSILON);
    }

    #[test]
    fn volatility_periods_override_parses() {
        let parsed: VolatilityParams =
            serde_json::from_str(r#"{ "periods": 252 }"#).expect("deserializes");
        assert!((parsed.periods - 252.0).abs() < f64::EPSILON);
    }
}
