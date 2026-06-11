#![forbid(unsafe_code)]
#![deny(clippy::pedantic, clippy::nursery)]
//! Pure-Rust portfolio analytics (gap-matrix item **L4.4**).
//!
//! Hand-rolled, offline implementations of the common portfolio-analysis metrics
//! over caller-supplied returns, equity curves, position values, and weights.
//! The crate performs no I/O and no async work: it is a deterministic numeric
//! library that a daemon compute route or a UDF can call directly. It builds on
//! the returns-based primitives in `tdw-analytics-quant` (re-exported below)
//! rather than re-deriving them.
//!
//! # Metrics
//!
//! - [`metrics::cumulative_returns`] — the running compounded total-return series
//!   `C_t = Π_{i≤t}(1 + r_i) − 1` from a periodic-returns series.
//! - [`metrics::drawdown`] — the per-step peak-to-trough drawdown series of an
//!   equity / cumulative-return curve.
//! - [`metrics::max_drawdown`] — the worst drawdown magnitude over an equity
//!   curve plus its peak and trough indices.
//! - [`metrics::allocation`] — normalize a vector of position values to weights
//!   summing to `1`.
//! - [`metrics::contribution`] — the per-asset return contribution
//!   `weight_i · return_i`.
//!
//! # Input convention
//!
//! `cumulative_returns` consumes per-period **returns** (the same convention
//! `tdw-analytics-quant` documents); `drawdown` and `max_drawdown` consume an
//! **equity / cumulative-return level** curve (a `1.0`-based growth series, or
//! any positive level series). A caller holding returns can build the level
//! curve by prepending `1.0` to `1 + cumulative_returns(r)`; a caller holding
//! prices can reduce them to returns once via
//! [`tdw_analytics_quant::prices_to_returns`] and feed `cumulative_returns`.
//!
//! # Output shape
//!
//! The series metrics return a `Vec<f64>` aligned with their input; `allocation`
//! and `contribution` return one weight / contribution per input element;
//! `max_drawdown` returns a typed [`metrics::MaxDrawdownRow`]. Metrics undefined
//! for the supplied series (an empty curve, a zero allocation total) report an
//! empty / sentinel output or a typed [`PortfolioError`] rather than a `NaN`.
//!
//! # Clean-room provenance
//!
//! Every formula here is standard public portfolio math: the geometric
//! (compounded) cumulative return, the running peak-to-trough drawdown, the
//! normalize-to-weights allocation, and the `weight_i · return_i` contribution
//! identity whose sum is the portfolio return. No reference implementation was
//! consulted; the unit tests assert each metric against hand-compounded
//! reference values.

pub mod error;
pub mod metrics;
pub mod params;

pub use error::PortfolioError;

/// Result type for portfolio-metric computations.
pub type Result<T> = std::result::Result<T, PortfolioError>;

#[cfg(test)]
mod tests {
    use crate::metrics::cumulative_returns;

    /// `cumulative_returns` is the compounding inverse of
    /// `tdw_analytics_quant::prices_to_returns`: reducing a price path to returns
    /// and recompounding recovers the path's total growth.
    #[test]
    fn cumulative_returns_inverts_prices_to_returns() {
        let prices = [100.0, 110.0, 99.0, 108.9];
        let returns = tdw_analytics_quant::prices_to_returns(&prices);
        let cumulative = cumulative_returns(&returns);
        // Final cumulative return = last_price / first_price - 1 = 108.9/100 - 1.
        let final_cumulative = cumulative.last().copied().expect("non-empty");
        assert!(
            (final_cumulative - 0.089).abs() < 1e-9,
            "got {final_cumulative}"
        );
    }
}
