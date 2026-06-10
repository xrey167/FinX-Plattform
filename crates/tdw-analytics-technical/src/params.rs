//! Typed parameter structs for every indicator.
//!
//! Each struct carries the indicator's window lengths (and similar tuning knobs)
//! with `serde` defaults matching `OpenBB`'s documented defaults, and derives
//! `JsonSchema` so the daemon catalog can publish the parameter schema. Defaults
//! are supplied via `#[serde(default = "…")]` free functions so a request may
//! omit any field and still deserialize to the canonical default.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

const fn d3() -> usize {
    3
}
const fn d9() -> usize {
    9
}
const fn d10() -> usize {
    10
}
const fn d12() -> usize {
    12
}
const fn d14() -> usize {
    14
}
const fn d20() -> usize {
    20
}
const fn d25() -> usize {
    25
}
const fn d26() -> usize {
    26
}
const fn f2() -> f64 {
    2.0
}
const fn f0_015() -> f64 {
    0.015
}

/// A single window-length parameter (`length`), used by SMA/EMA/WMA/HMA, RSI,
/// CCI, ATR, ADX, Aroon, Donchian, ROC, and momentum.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LengthParams {
    /// Lookback window in bars.
    #[serde(default = "d14")]
    pub length: usize,
}

impl Default for LengthParams {
    fn default() -> Self {
        Self { length: 14 }
    }
}

impl LengthParams {
    /// Construct with an explicit length.
    #[must_use]
    pub const fn new(length: usize) -> Self {
        Self { length }
    }
}

/// MACD parameters (fast/slow EMA spans and the signal EMA span).
///
/// `OpenBB` defaults: fast 12, slow 26, signal 9.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MacdParams {
    /// Fast EMA span.
    #[serde(default = "d12")]
    pub fast: usize,
    /// Slow EMA span.
    #[serde(default = "d26")]
    pub slow: usize,
    /// Signal EMA span (applied to the MACD line).
    #[serde(default = "d9")]
    pub signal: usize,
}

impl Default for MacdParams {
    fn default() -> Self {
        Self {
            fast: 12,
            slow: 26,
            signal: 9,
        }
    }
}

/// Stochastic-oscillator parameters.
///
/// `OpenBB` defaults: `%K` lookback 14, `%K` smoothing 3 (slow `%K`), `%D` 3.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StochasticParams {
    /// `%K` lookback window (high/low range).
    #[serde(default = "d14")]
    pub k_period: usize,
    /// `%K` smoothing window (the slow `%K` SMA).
    #[serde(default = "d3")]
    pub k_smooth: usize,
    /// `%D` window (SMA of the smoothed `%K`).
    #[serde(default = "d3")]
    pub d_period: usize,
}

impl Default for StochasticParams {
    fn default() -> Self {
        Self {
            k_period: 14,
            k_smooth: 3,
            d_period: 3,
        }
    }
}

/// Bollinger-band parameters: SMA window and the standard-deviation multiple.
///
/// `OpenBB` defaults: length 20, std multiple 2.0.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct BollingerParams {
    /// SMA / standard-deviation window.
    #[serde(default = "d20")]
    pub length: usize,
    /// Number of standard deviations for the upper/lower band.
    #[serde(default = "f2")]
    pub std: f64,
}

impl Default for BollingerParams {
    fn default() -> Self {
        Self {
            length: 20,
            std: 2.0,
        }
    }
}

/// Keltner-channel parameters: EMA window, ATR window, and the ATR multiple.
///
/// `OpenBB` defaults: length 20, ATR length 10, multiplier 2.0.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct KeltnerParams {
    /// EMA window for the channel midline.
    #[serde(default = "d20")]
    pub length: usize,
    /// ATR window for the channel width.
    #[serde(default = "d10")]
    pub atr_length: usize,
    /// ATR multiplier for the upper/lower band.
    #[serde(default = "f2")]
    pub multiplier: f64,
}

impl Default for KeltnerParams {
    fn default() -> Self {
        Self {
            length: 20,
            atr_length: 10,
            multiplier: 2.0,
        }
    }
}

/// CCI parameters: window and the Lambert constant.
///
/// `OpenBB` / Lambert defaults: length 20, constant 0.015.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CciParams {
    /// Typical-price SMA window.
    #[serde(default = "d20")]
    pub length: usize,
    /// Lambert scaling constant (0.015 by convention).
    #[serde(default = "f0_015")]
    pub constant: f64,
}

impl Default for CciParams {
    fn default() -> Self {
        Self {
            length: 20,
            constant: 0.015,
        }
    }
}

/// ADX parameters: the directional-movement / ATR window.
///
/// `OpenBB` / Wilder default: length 14.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AdxParams {
    /// Wilder smoothing window for DM and TR.
    #[serde(default = "d14")]
    pub length: usize,
}

impl Default for AdxParams {
    fn default() -> Self {
        Self { length: 14 }
    }
}

/// Aroon parameters: the high/low extreme lookback.
///
/// `OpenBB` default: length 25.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AroonParams {
    /// Lookback for the most-recent high/low extreme.
    #[serde(default = "d25")]
    pub length: usize,
}

impl Default for AroonParams {
    fn default() -> Self {
        Self { length: 25 }
    }
}

/// Fisher-transform parameters: the high/low normalisation window.
///
/// `OpenBB` / Ehlers default: length 9.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FisherParams {
    /// High/low normalisation window.
    #[serde(default = "d9")]
    pub length: usize,
}

impl Default for FisherParams {
    fn default() -> Self {
        Self { length: 9 }
    }
}

/// Donchian-channel parameters: the high/low extreme window.
///
/// `OpenBB` default: length 20.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DonchianParams {
    /// Lookback window for the rolling high and low.
    #[serde(default = "d20")]
    pub length: usize,
}

impl Default for DonchianParams {
    fn default() -> Self {
        Self { length: 20 }
    }
}

/// Rate-of-change / momentum parameters: the lag window.
///
/// `OpenBB` default: length 10.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RocParams {
    /// Number of bars to look back for the prior price.
    #[serde(default = "d10")]
    pub length: usize,
}

impl Default for RocParams {
    fn default() -> Self {
        Self { length: 10 }
    }
}

/// Hull moving-average parameters.
///
/// `OpenBB` / Hull common default: length 20. (HMA internally uses
/// `floor(sqrt(length))` for its final smoothing pass.)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HmaParams {
    /// WMA window for the Hull moving average.
    #[serde(default = "d20")]
    pub length: usize,
}

impl Default for HmaParams {
    fn default() -> Self {
        Self { length: 20 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn length_defaults_to_fourteen_when_omitted() {
        let parsed: LengthParams = serde_json::from_str("{}").expect("empty object deserializes");
        assert_eq!(parsed, LengthParams::default());
        assert_eq!(parsed.length, 14);
    }

    #[test]
    fn macd_defaults_match_openbb() {
        let parsed: MacdParams = serde_json::from_str("{}").expect("empty object deserializes");
        assert_eq!(parsed.fast, 12);
        assert_eq!(parsed.slow, 26);
        assert_eq!(parsed.signal, 9);
    }

    #[test]
    fn bollinger_defaults_match_openbb() {
        let parsed: BollingerParams = serde_json::from_str("{}").expect("deserializes");
        assert_eq!(parsed.length, 20);
        assert!((parsed.std - 2.0).abs() < f64::EPSILON);
    }
}
