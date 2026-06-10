//! Indicator implementations, grouped by family.
//!
//! Each public function takes a `&[Bar]` (or a price column derived from one)
//! plus a typed param struct and returns a date-aligned output column of the
//! input length. See the crate-level docs for the leading-`None` policy and the
//! clean-room provenance statement.

pub mod bands;
pub mod moving_average;
pub mod oscillators;
pub mod trend;
pub mod volume;

mod indicator_helpers;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// One MACD row: the MACD line, its signal EMA, and the histogram.
///
/// The histogram is line − signal. Components are individually optional because
/// the signal line (and thus the histogram) becomes defined later than the MACD
/// line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MacdRow {
    /// Fast EMA − slow EMA.
    pub macd: Option<f64>,
    /// EMA of the MACD line over the signal span.
    pub signal: Option<f64>,
    /// MACD line − signal line.
    pub histogram: Option<f64>,
}

/// One stochastic-oscillator row: the smoothed `%K` and the `%D` SMA of it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StochasticRow {
    /// Smoothed (slow) `%K`, in 0..=100.
    pub k: Option<f64>,
    /// `%D`, the SMA of `%K`, in 0..=100.
    pub d: Option<f64>,
}

/// One ADX row: the trend-strength ADX plus the +DI / −DI directional lines.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AdxRow {
    /// Average Directional Index (trend strength, 0..=100).
    pub adx: Option<f64>,
    /// Positive Directional Indicator (+DI).
    pub plus_di: Option<f64>,
    /// Negative Directional Indicator (−DI).
    pub minus_di: Option<f64>,
}

/// One Aroon row: Aroon-up, Aroon-down, and the oscillator (up − down).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AroonRow {
    /// Aroon-up (0..=100): recency of the period high.
    pub up: Option<f64>,
    /// Aroon-down (0..=100): recency of the period low.
    pub down: Option<f64>,
    /// Aroon oscillator: up − down (−100..=100).
    pub oscillator: Option<f64>,
}

/// One channel row (Bollinger / Keltner / Donchian): the upper band, the
/// middle line, and the lower band.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ChannelRow {
    /// Upper band.
    pub upper: Option<f64>,
    /// Middle line (the band's central tendency).
    pub middle: Option<f64>,
    /// Lower band.
    pub lower: Option<f64>,
}

/// One Fisher-transform row: the Fisher value and its one-bar-lagged trigger.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FisherRow {
    /// Fisher-transform value.
    pub fisher: Option<f64>,
    /// Trigger line: the prior bar's Fisher value.
    pub trigger: Option<f64>,
}
