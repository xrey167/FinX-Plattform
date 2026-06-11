#![forbid(unsafe_code)]
#![deny(clippy::pedantic, clippy::nursery)]
//! Pure-Rust returns-based quantitative metrics (gap-matrix item **L4.2**).
//!
//! Hand-rolled, offline implementations of the common `OpenBB`-parity
//! quantitative-analysis metrics over a single value series. The crate performs
//! no I/O and no async work: it is a deterministic numeric library that a daemon
//! compute route or a UDF can call directly.
//!
//! # Input convention: returns, not prices
//!
//! Every metric in [`metrics`] consumes a slice of **per-period returns**
//! (`&[f64]`), not prices. A return is the fractional change between two
//! consecutive observations, `r_t = (p_t − p_{t−1}) / p_{t−1}`. This is the
//! convention `OpenBB`'s `quantitative` router uses (its performance ratios and
//! moment statistics are all defined over a returns column), and it keeps the
//! metrics composable: any price series can be reduced to returns once via
//! [`prices_to_returns`] and then fed to every metric.
//!
//! When a caller has prices rather than returns, convert first:
//!
//! ```
//! use tdw_analytics_quant::{prices_to_returns, metrics, params::SharpeParams};
//! let prices = [100.0, 101.0, 99.0, 102.0];
//! let returns = prices_to_returns(&prices);
//! let sharpe = metrics::sharpe_ratio(&returns, SharpeParams::default());
//! assert!(sharpe.is_ok());
//! ```
//!
//! # Output shape
//!
//! Each metric returns a single typed value (a scalar `f64`, or a small typed
//! row struct for the multi-field metrics — CAPM's alpha+beta, the drawdown
//! triple, the Jarque-Bera statistic+p-value). Unlike the technical indicators
//! (which return one row per bar), a quantitative metric summarizes the whole
//! series into one figure, so there is no leading-`None` policy: a metric that
//! is undefined for the supplied series (too few points, zero dispersion)
//! reports a documented sentinel (`0.0` dispersion ⇒ ratio `0.0`) rather than
//! `NaN`.
//!
//! # Clean-room provenance
//!
//! Every formula here is textbook math cited to its standard definition in the
//! owning module's docs (Sharpe 1966; Sortino 1994; Keating & Shadwick 2002 for
//! Omega; Young 1991 for Calmar; the standard sample skewness/excess-kurtosis
//! moment estimators; Jarque & Bera 1980 for the normality statistic; the
//! single-factor market-model regression for CAPM). The Jarque-Bera p-value uses
//! a hand-rolled chi-squared (2 d.o.f.) survival function, `p = exp(−JB/2)`,
//! which is *exact* for the χ²₂ distribution — see [`metrics::jarque_bera`]. No
//! reference implementation was consulted.

pub mod error;
pub mod metrics;
pub mod params;
pub mod stats;

pub use error::QuantError;

/// Result type for quantitative-metric computations.
pub type Result<T> = std::result::Result<T, QuantError>;

/// Convert a price series to a period-over-period simple-returns series.
///
/// Returns a vector of length `prices.len().saturating_sub(1)`: each element is
/// `(p_t − p_{t−1}) / p_{t−1}`. A zero prior price yields `0.0` for that step
/// (rather than an infinity), keeping the output `NaN`/`inf`-free for downstream
/// metrics. An input of fewer than two prices yields an empty vector.
#[must_use]
pub fn prices_to_returns(prices: &[f64]) -> Vec<f64> {
    prices
        .windows(2)
        .map(|w| {
            let prev = w[0];
            if prev == 0.0 {
                0.0
            } else {
                (w[1] - prev) / prev
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::prices_to_returns;

    #[test]
    fn prices_to_returns_is_one_shorter() {
        let returns = prices_to_returns(&[100.0, 110.0, 99.0]);
        assert_eq!(returns.len(), 2);
        assert!((returns[0] - 0.1).abs() < 1e-12);
        // (99 - 110) / 110 = -0.1
        assert!((returns[1] - -0.1).abs() < 1e-12);
    }

    #[test]
    fn prices_to_returns_handles_zero_prior_price() {
        let returns = prices_to_returns(&[0.0, 5.0]);
        assert_eq!(returns, vec![0.0]);
    }

    #[test]
    fn prices_to_returns_empty_for_short_input() {
        assert!(prices_to_returns(&[42.0]).is_empty());
        assert!(prices_to_returns(&[]).is_empty());
    }
}
