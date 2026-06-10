#![forbid(unsafe_code)]
#![deny(clippy::pedantic, clippy::nursery)]
//! Pure-Rust technical-analysis indicators (gap-matrix item **L4.1**).
//!
//! Hand-rolled, offline implementations of the common `OpenBB`-parity technical
//! indicators over an OHLCV series. The crate performs no I/O and no async work:
//! it is a deterministic numeric library that a daemon compute route or a UDF
//! can call directly.
//!
//! # Input shape
//!
//! Every indicator consumes a date-aligned slice of [`Bar`], which is a
//! re-export of [`tdw_domain::MarketDataBar`] — the canonical OHLCV record the
//! rest of the workspace already speaks. `MarketDataBar` (rather than
//! `EquityHistoricalData`) is chosen as the input because it is the venue- and
//! granularity-tagged shape used across crypto/index/equity, so one indicator
//! API serves every asset class. Only the `open`/`high`/`low`/`close`/`volume`
//! fields are read; `symbol`/`venue`/`ts`/`granularity`/`source` are ignored by
//! the math (the caller keeps them for date alignment).
//!
//! # Output shape and leading-`None` policy
//!
//! Indicators return a `Vec` whose length **equals the input length**, so the
//! result is positionally date-aligned with the input bars. Positions before an
//! indicator has enough history to be defined hold [`None`]; once the lookback
//! window is satisfied the row carries `Some(value)`. Multi-line indicators
//! (MACD, Stochastic, ADX, Bollinger, …) return a `Vec` of a typed row struct
//! whose component fields are individually `Option<f64>` (a component may become
//! defined later than another — e.g. MACD's signal line lags its MACD line).
//!
//! Wilder's smoothed indicators (RSI, ATR, ADX, +DI/-DI) seed the first defined
//! value with a simple average of the first `period` deltas, then apply Wilder's
//! recursive smoothing — exactly as defined in *J. Welles Wilder, New Concepts
//! in Technical Trading Systems (1978)*. EMA-based indicators seed with the SMA
//! of the first `period` samples, then apply the `2/(period+1)` smoothing.
//!
//! # Clean-room provenance
//!
//! Every formula here is textbook math cited to its standard definition in the
//! owning module's docs (Wilder 1978 for RSI/ATR/ADX/+DI/-DI; Lane for
//! Stochastic; Bollinger for the bands; Ehlers for the Fisher transform; etc.).
//! No reference implementation was consulted.

pub mod error;
pub mod indicators;
pub mod params;

pub use error::IndicatorError;

/// The canonical OHLCV bar input shape for every indicator: an alias for
/// [`tdw_domain::MarketDataBar`]. See the crate-level docs for why this shape
/// (rather than `EquityHistoricalData`) is the input.
pub type Bar = tdw_domain::MarketDataBar;

/// Result type for indicator computations.
pub type Result<T> = std::result::Result<T, IndicatorError>;

/// Extract the close-price column from a bar slice.
#[must_use]
pub fn closes(bars: &[Bar]) -> Vec<f64> {
    bars.iter().map(|b| b.close).collect()
}

/// Extract the high-price column from a bar slice.
#[must_use]
pub fn highs(bars: &[Bar]) -> Vec<f64> {
    bars.iter().map(|b| b.high).collect()
}

/// Extract the low-price column from a bar slice.
#[must_use]
pub fn lows(bars: &[Bar]) -> Vec<f64> {
    bars.iter().map(|b| b.low).collect()
}

/// Extract the volume column from a bar slice.
#[must_use]
pub fn volumes(bars: &[Bar]) -> Vec<f64> {
    bars.iter().map(|b| b.volume).collect()
}

/// Validate that `period` is at least one.
///
/// Indicator entry points call this before computing so a zero or nonsensical
/// window fails fast rather than producing an all-`None` column.
///
/// # Errors
///
/// Returns [`IndicatorError::InvalidPeriod`] when `period == 0`.
pub const fn require_period(period: usize) -> Result<()> {
    if period == 0 {
        return Err(IndicatorError::InvalidPeriod);
    }
    Ok(())
}

#[cfg(test)]
mod fixture {
    use super::Bar;
    use tdw_domain::TimeGranularity;

    /// A small, fixed daily OHLCV series used by every golden-vector test.
    ///
    /// The series is hand-authored so the expected indicator outputs can be
    /// derived by hand (or a one-off scratch calculation) from the textbook
    /// definitions and hard-coded in the tests. Bars satisfy the OHLC nesting
    /// invariant (`low <= open,close <= high`). Closes form a gentle
    /// up-then-down zig-zag so direction-sensitive indicators (RSI, ADX, Aroon,
    /// MACD) exercise both rising and falling regimes.
    pub fn series() -> Vec<Bar> {
        // (open, high, low, close, volume)
        const ROWS: &[(f64, f64, f64, f64, f64)] = &[
            (10.0, 10.5, 9.8, 10.0, 1000.0),
            (10.0, 11.2, 9.9, 11.0, 1100.0),
            (11.0, 12.4, 10.8, 12.0, 1200.0),
            (12.0, 13.1, 11.7, 12.5, 900.0),
            (12.5, 13.6, 12.2, 13.0, 1300.0),
            (13.0, 14.5, 12.8, 14.0, 1500.0),
            (14.0, 14.2, 12.9, 13.0, 1400.0),
            (13.0, 13.4, 11.9, 12.0, 1600.0),
            (12.0, 12.6, 10.9, 11.0, 1700.0),
            (11.0, 12.1, 10.4, 12.0, 1250.0),
            (12.0, 13.2, 11.8, 13.0, 1350.0),
            (13.0, 14.4, 12.8, 14.0, 1450.0),
            (14.0, 15.6, 13.9, 15.0, 1550.0),
            (15.0, 15.2, 13.6, 14.0, 1500.0),
            (14.0, 14.1, 12.7, 13.0, 1480.0),
        ];
        ROWS.iter()
            .enumerate()
            .map(|(i, &(open, high, low, close, volume))| Bar {
                symbol: "TEST".to_string(),
                venue: "XTST".to_string(),
                granularity: TimeGranularity::Day,
                ts: format!("2026-01-{:02}", i + 1),
                open,
                high,
                low,
                close,
                volume,
                source: "fixture".to_string(),
            })
            .collect()
    }

    /// Assert two `Option<f64>` columns agree within `eps` at every index.
    pub fn assert_close_opt(actual: &[Option<f64>], expected: &[Option<f64>], eps: f64) {
        assert_eq!(
            actual.len(),
            expected.len(),
            "length mismatch: {} vs {}",
            actual.len(),
            expected.len()
        );
        for (i, (a, e)) in actual.iter().zip(expected.iter()).enumerate() {
            match (a, e) {
                (None, None) => {}
                (Some(av), Some(ev)) => assert!(
                    (av - ev).abs() <= eps,
                    "index {i}: {av} vs {ev} (eps {eps})"
                ),
                _ => panic!("index {i}: None/Some mismatch: {a:?} vs {e:?}"),
            }
        }
    }
}
