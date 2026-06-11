//! Portfolio analytics metrics.
//!
//! Each function consumes a caller-supplied series (returns, an equity /
//! cumulative-return curve, position values, or an aligned weights+returns pair)
//! and returns either a derived series (`Vec<f64>`) or a typed summary row.
//! Formula citations are in each function's docs.
//!
//! Where a descriptive statistic is needed the crate reuses
//! `tdw-analytics-quant` rather than re-deriving it — `cumulative_returns`'s
//! inverse (a price series to returns) is exactly [`prices_to_returns`], so the
//! crate-level docs cross-reference the two.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// One maximum-drawdown row: the worst peak-to-trough decline over an equity
/// curve and the indices that bound it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MaxDrawdownRow {
    /// Maximum drawdown magnitude as a non-negative fraction of the running peak
    /// (e.g. `0.25` for a 25% peak-to-trough loss); `0.0` for a never-declining
    /// or empty series.
    pub max_drawdown: f64,
    /// Index of the running-peak level the worst drawdown is measured from.
    pub peak_index: usize,
    /// Index of the trough level (at or after `peak_index`) where the worst
    /// drawdown is reached.
    pub trough_index: usize,
}

/// Compounded running cumulative-return series from a periodic-returns series.
///
/// The cumulative return at step `t` is the compounded growth of one unit of
/// wealth through period `t`, expressed as a fraction:
/// `C_t = Π_{i≤t} (1 + r_i) − 1`. This is the standard geometric (compounded)
/// total-return convention: a `+10%` then `−5%` path yields
/// `(1.10)(0.95) − 1 = 0.045`, i.e. `+4.5%`, not the `+5%` an arithmetic sum
/// would give. The output has the same length as `returns`; an empty input
/// yields an empty series.
#[must_use]
pub fn cumulative_returns(returns: &[f64]) -> Vec<f64> {
    let mut wealth = 1.0;
    returns
        .iter()
        .map(|&r| {
            wealth *= 1.0 + r;
            wealth - 1.0
        })
        .collect()
}

/// Drawdown series of an equity / cumulative-return level curve.
///
/// At each step the drawdown is the level's shortfall from the highest level
/// seen so far (the running peak): `D_t = level_t / peak_t − 1`, where
/// `peak_t = max_{i≤t} level_i`. A level at a fresh high has drawdown `0`; a
/// level below the running peak has a negative drawdown. The output has the same
/// length as `equity`. A non-positive running peak (a zero or negative level)
/// yields `0.0` for that step rather than a non-finite value, keeping the series
/// `NaN`/`inf`-free; an empty input yields an empty series.
#[must_use]
pub fn drawdown(equity: &[f64]) -> Vec<f64> {
    let mut peak = f64::NEG_INFINITY;
    equity
        .iter()
        .map(|&level| {
            if level > peak {
                peak = level;
            }
            if peak > 0.0 { level / peak - 1.0 } else { 0.0 }
        })
        .collect()
}

/// Worst drawdown magnitude over an equity / cumulative-return curve, with the
/// peak and trough indices that bound it.
///
/// Scans the running peak (the highest level seen so far) and tracks the most
/// negative `level_t / peak_t − 1`; the reported `max_drawdown` is that value's
/// magnitude (a non-negative fraction). `peak_index` is where the running peak
/// the worst drawdown is measured from was last set, and `trough_index` is where
/// the worst drawdown is reached (`trough_index ≥ peak_index`). The first
/// occurrence wins on ties. A never-declining or empty series yields a zero
/// drawdown with both indices `0`.
#[must_use]
pub fn max_drawdown(equity: &[f64]) -> MaxDrawdownRow {
    let mut peak = f64::NEG_INFINITY;
    let mut peak_index = 0usize;
    let mut worst = MaxDrawdownRow::default();
    for (index, &level) in equity.iter().enumerate() {
        if level > peak {
            peak = level;
            peak_index = index;
        }
        if peak > 0.0 {
            let dd = level / peak - 1.0;
            if dd < -worst.max_drawdown {
                worst = MaxDrawdownRow {
                    max_drawdown: -dd,
                    peak_index,
                    trough_index: index,
                };
            }
        }
    }
    worst
}

/// Normalize a vector of position values to portfolio weights summing to `1`.
///
/// Each weight is `value_i / Σ value_j`, the share of the total portfolio value
/// held in position `i`. The output has the same length as `values` and (for a
/// non-zero total) sums to `1`. Negative values (short positions) are permitted
/// and produce signed weights that still sum to `1` against the net total.
///
/// # Errors
///
/// Returns [`PortfolioError::ZeroTotal`](crate::PortfolioError::ZeroTotal) when
/// the position values sum to zero (so they cannot be normalized).
pub fn allocation(values: &[f64]) -> crate::Result<Vec<f64>> {
    let total: f64 = values.iter().sum();
    if total == 0.0 {
        return Err(crate::PortfolioError::ZeroTotal);
    }
    Ok(values.iter().map(|&v| v / total).collect())
}

/// Per-asset return contribution: `contribution_i = weight_i · return_i`.
///
/// Each asset's contribution to the portfolio's period return is its weight times
/// its return; summing the contributions reconstructs the portfolio return
/// `Σ w_i r_i`. The two input vectors must be positionally aligned. The output
/// has the same length as the inputs.
///
/// # Errors
///
/// Returns
/// [`PortfolioError::LengthMismatch`](crate::PortfolioError::LengthMismatch) when
/// `weights` and `returns` differ in length.
pub fn contribution(weights: &[f64], returns: &[f64]) -> crate::Result<Vec<f64>> {
    if weights.len() != returns.len() {
        return Err(crate::PortfolioError::LengthMismatch {
            weights: weights.len(),
            returns: returns.len(),
        });
    }
    Ok(weights
        .iter()
        .zip(returns.iter())
        .map(|(&w, &r)| w * r)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::{allocation, contribution, cumulative_returns, drawdown, max_drawdown};
    use crate::PortfolioError;

    // Golden returns series: [+10%, -5%, +10%]. Hand-compounded wealth path:
    //   after r0: 1.10            -> cumulative 0.10
    //   after r1: 1.10*0.95=1.045 -> cumulative 0.045
    //   after r2: 1.045*1.10=1.1495 -> cumulative 0.1495
    fn golden_returns() -> Vec<f64> {
        vec![0.10, -0.05, 0.10]
    }

    // Golden equity curve = 1.0 prepended to the compounded wealth path of
    // [+10%, -5%, +10%, -5%]:
    //   1.0, 1.10, 1.045, 1.1495, 1.092025
    // Peaks: 1.0, 1.10, 1.10, 1.1495, 1.1495.
    // Drawdowns: 0, 0, 1.045/1.10-1=-0.05, 0, 1.092025/1.1495-1=-0.05.
    fn golden_equity() -> Vec<f64> {
        vec![1.0, 1.10, 1.045, 1.1495, 1.092_025]
    }

    #[test]
    fn cumulative_returns_compounds_geometrically() {
        let c = cumulative_returns(&golden_returns());
        assert_eq!(c.len(), 3);
        assert!((c[0] - 0.10).abs() < 1e-12, "got {}", c[0]);
        assert!((c[1] - 0.045).abs() < 1e-12, "got {}", c[1]);
        // (1.10 * 0.95 * 1.10) - 1 = 1.1495 - 1 = 0.1495.
        assert!((c[2] - 0.1495).abs() < 1e-12, "got {}", c[2]);
    }

    #[test]
    fn cumulative_returns_empty_is_empty() {
        assert!(cumulative_returns(&[]).is_empty());
    }

    #[test]
    fn drawdown_tracks_shortfall_from_running_peak() {
        let d = drawdown(&golden_equity());
        assert_eq!(d.len(), 5);
        assert!(d[0].abs() < 1e-12, "got {}", d[0]);
        assert!(d[1].abs() < 1e-12, "got {}", d[1]);
        assert!((d[2] - -0.05).abs() < 1e-12, "got {}", d[2]);
        assert!(d[3].abs() < 1e-12, "got {}", d[3]);
        assert!((d[4] - -0.05).abs() < 1e-12, "got {}", d[4]);
    }

    #[test]
    fn drawdown_empty_is_empty() {
        assert!(drawdown(&[]).is_empty());
    }

    #[test]
    fn max_drawdown_reports_worst_with_indices() {
        // Worst drawdown is -0.05; the first occurrence (peak idx1=1.10, trough
        // idx2=1.045) wins on the tie with the later -0.05 at idx4.
        let row = max_drawdown(&golden_equity());
        assert!(
            (row.max_drawdown - 0.05).abs() < 1e-12,
            "got {}",
            row.max_drawdown
        );
        assert_eq!(row.peak_index, 1);
        assert_eq!(row.trough_index, 2);
    }

    #[test]
    fn max_drawdown_monotone_up_has_no_drawdown() {
        let row = max_drawdown(&[1.0, 1.1, 1.2]);
        assert!(row.max_drawdown.abs() < 1e-12);
        assert_eq!(row.peak_index, 0);
        assert_eq!(row.trough_index, 0);
    }

    #[test]
    fn max_drawdown_empty_is_zero() {
        assert_eq!(max_drawdown(&[]), super::MaxDrawdownRow::default());
    }

    #[test]
    fn allocation_normalizes_to_weights_summing_to_one() {
        // [25, 75] / 100 = [0.25, 0.75].
        let w = allocation(&[25.0, 75.0]).expect("allocation");
        assert!((w[0] - 0.25).abs() < 1e-12, "got {}", w[0]);
        assert!((w[1] - 0.75).abs() < 1e-12, "got {}", w[1]);
        let sum: f64 = w.iter().sum();
        assert!((sum - 1.0).abs() < 1e-12, "weights sum {sum}");
    }

    #[test]
    fn allocation_zero_total_errors() {
        assert_eq!(
            allocation(&[10.0, -10.0]).expect_err("zero total"),
            PortfolioError::ZeroTotal
        );
    }

    #[test]
    fn contribution_is_weight_times_return() {
        // weights [0.6, 0.4], returns [0.10, -0.05] -> [0.06, -0.02].
        let c = contribution(&[0.6, 0.4], &[0.10, -0.05]).expect("contribution");
        assert!((c[0] - 0.06).abs() < 1e-12, "got {}", c[0]);
        assert!((c[1] - -0.02).abs() < 1e-12, "got {}", c[1]);
        // Summed contributions reconstruct the portfolio return 0.04.
        let portfolio: f64 = c.iter().sum();
        assert!((portfolio - 0.04).abs() < 1e-12, "portfolio {portfolio}");
    }

    #[test]
    fn contribution_length_mismatch_errors() {
        assert_eq!(
            contribution(&[0.5, 0.5], &[0.1]).expect_err("mismatch"),
            PortfolioError::LengthMismatch {
                weights: 2,
                returns: 1,
            }
        );
    }
}
