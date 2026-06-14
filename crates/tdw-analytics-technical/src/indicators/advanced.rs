//! Advanced / composite indicators: Center of Gravity, Clenow momentum,
//! `DeMark` TD-Sequential setup, Fibonacci retracement, volatility cones, and
//! the Relative Rotation Graph.
//!
//! These extend the core indicator families with the higher-level studies the
//! `OpenBB`-parity `technical/*` router exposes. They follow the same crate
//! conventions: per-bar studies return a date-aligned `Vec` of the input length
//! with a leading-`None` warm-up, while the genuinely summary studies (volatility
//! cones, Fibonacci levels) return a compact table.
//!
//! # Clean-room provenance
//!
//! - **Center of Gravity** (Ehlers 2002): `CG = −Σ_{i=0}^{N−1} (i+1)·p_{t−i} /
//!   Σ p_{t−i}` over the window, a low-lag oscillator.
//! - **Clenow momentum** (Clenow, *Stocks on the Move* 2015): the slope of the
//!   OLS regression of `ln(price)` on time over the window, annualized
//!   (`(exp(slope)^252 − 1)`) and multiplied by the regression `R²` to penalize
//!   noisy fits.
//! - **`DeMark` TD-Setup** (`DeMark` 1994): the count of consecutive closes
//!   above (sell setup) or below (buy setup) the close four bars earlier,
//!   `±1..=9`.
//! - **Fibonacci retracement**: the standard ratio ladder applied between the
//!   window's swing low and high.
//! - **Volatility cones** (Burghardt & Lane 1990): the distribution
//!   (min / lower-q / median / upper-q / max) of annualized realized volatility
//!   over a ladder of horizon windows.
//! - **Relative Rotation Graph** (Julius de Kempenaer): the relative-strength
//!   line `asset/benchmark` normalized to a `~100`-centered z-score (RS-Ratio)
//!   and its rate of change (RS-Momentum).
//!
//! No reference implementation was consulted.

use crate::indicators::{ConeRow, DemarkRow, FibRow, RrgRow};
use crate::params::{ClenowParams, ConesParams, FibParams, LengthParams, RrgParams};
use crate::{Bar, IndicatorError, Result, require_period};

#[allow(clippy::cast_precision_loss)]
const fn len_f64(n: usize) -> f64 {
    n as f64
}

/// Center of Gravity oscillator (Ehlers) over the close series.
///
/// For each bar with a full window, `CG = −Σ_{i=0}^{N−1} (i+1)·close_{t−i} /
/// Σ_{i=0}^{N−1} close_{t−i}`. The result is date-aligned with leading `None`s
/// until the window is full.
///
/// # Errors
///
/// Returns [`IndicatorError::InvalidPeriod`] when `length == 0`.
pub fn cg(bars: &[Bar], params: LengthParams) -> Result<Vec<Option<f64>>> {
    let length = params.length;
    require_period(length)?;
    let closes: Vec<f64> = bars.iter().map(|b| b.close).collect();
    let mut out = vec![None; closes.len()];
    if closes.len() < length {
        return Ok(out);
    }
    for t in (length - 1)..closes.len() {
        let window = &closes[t + 1 - length..=t];
        // window[length-1] is the current bar (i = 0); weight (i+1) increases
        // toward older bars (window[0]).
        let mut num = 0.0;
        let mut den = 0.0;
        for (i, &price) in window.iter().rev().enumerate() {
            num += len_f64(i + 1) * price;
            den += price;
        }
        out[t] = if den == 0.0 {
            Some(0.0)
        } else {
            Some(-num / den)
        };
    }
    Ok(out)
}

/// Clenow volatility-adjusted momentum over the close series.
///
/// For each bar with a full `period` window, fits `ln(close)` against the bar
/// index by OLS, annualizes the slope (`(exp(slope))^252 − 1`), and scales it by
/// the regression `R²`. Non-positive closes in a window yield `None` for that bar
/// (the log is undefined).
///
/// # Errors
///
/// Returns [`IndicatorError::InvalidPeriod`] when `period == 0`.
pub fn clenow(bars: &[Bar], params: ClenowParams) -> Result<Vec<Option<f64>>> {
    let period = params.period;
    require_period(period)?;
    let closes: Vec<f64> = bars.iter().map(|b| b.close).collect();
    let mut out = vec![None; closes.len()];
    if closes.len() < period {
        return Ok(out);
    }
    for t in (period - 1)..closes.len() {
        let window = &closes[t + 1 - period..=t];
        if window.iter().any(|&p| p <= 0.0) {
            continue;
        }
        let logs: Vec<f64> = window.iter().map(|&p| p.ln()).collect();
        out[t] = clenow_score(&logs);
    }
    Ok(out)
}

/// Annualized, R²-weighted slope of `ln(price)` vs the bar index over one window.
fn clenow_score(logs: &[f64]) -> Option<f64> {
    let n = logs.len();
    if n < 2 {
        return None;
    }
    let nf = len_f64(n);
    // x = 0..n-1; closed-form simple-regression slope and R².
    let sum_x = nf * (nf - 1.0) / 2.0;
    let mean_x = sum_x / nf;
    let mean_y = logs.iter().sum::<f64>() / nf;
    let mut sxy = 0.0;
    let mut sxx = 0.0;
    let mut syy = 0.0;
    for (i, &y) in logs.iter().enumerate() {
        let dx = len_f64(i) - mean_x;
        let dy = y - mean_y;
        sxy += dx * dy;
        sxx += dx * dx;
        syy += dy * dy;
    }
    if sxx == 0.0 {
        return Some(0.0);
    }
    let slope = sxy / sxx;
    let r_squared = if syy == 0.0 {
        0.0
    } else {
        let covariance_sq = sxy * sxy;
        let variance_product = sxx * syy;
        covariance_sq / variance_product
    };
    let annualized = slope.exp().powi(252) - 1.0;
    Some(annualized * r_squared)
}

/// `DeMark` TD-Setup count over the close series.
///
/// A *sell* setup counts consecutive closes strictly greater than the close four
/// bars earlier (`+1..=9`); a *buy* setup counts consecutive closes strictly less
/// (`−1..=−9`). The count resets when the comparison flips, and saturates at the
/// `±9` completion. Bars before a setup is in progress hold `None`.
///
/// # Errors
///
/// This indicator takes no window parameter and never errors; the `Result` is
/// kept for signature symmetry with the other indicators.
pub fn demark(bars: &[Bar]) -> Result<Vec<DemarkRow>> {
    let closes: Vec<f64> = bars.iter().map(|b| b.close).collect();
    let mut out = vec![DemarkRow { setup: None }; closes.len()];
    let mut count: i32 = 0;
    for t in 4..closes.len() {
        let diff = closes[t] - closes[t - 4];
        if diff > 0.0 {
            count = if count > 0 { count + 1 } else { 1 };
        } else if diff < 0.0 {
            count = if count < 0 { count - 1 } else { -1 };
        } else {
            count = 0;
        }
        count = count.clamp(-9, 9);
        out[t].setup = if count == 0 { None } else { Some(count) };
    }
    Ok(out)
}

/// Standard Fibonacci-retracement ratio ladder.
const FIB_RATIOS: [f64; 7] = [0.0, 0.236, 0.382, 0.5, 0.618, 0.786, 1.0];

/// Fibonacci retracement levels over the swing high/low of the last `period`
/// bars (or the whole series when `period == 0`).
///
/// Returns the seven standard ratios mapped to prices: ratio `0.0` is the swing
/// low and ratio `1.0` the swing high, with the retracement prices linearly
/// interpolated between. An empty series yields an empty table.
///
/// # Errors
///
/// This indicator does not error on its `period` (a `0` means "whole series");
/// the `Result` is kept for signature symmetry.
pub fn fib(bars: &[Bar], params: FibParams) -> Result<Vec<FibRow>> {
    if bars.is_empty() {
        return Ok(Vec::new());
    }
    let window: &[Bar] = if params.period == 0 || params.period >= bars.len() {
        bars
    } else {
        &bars[bars.len() - params.period..]
    };
    let low = window.iter().map(|b| b.low).fold(f64::INFINITY, f64::min);
    let high = window
        .iter()
        .map(|b| b.high)
        .fold(f64::NEG_INFINITY, f64::max);
    let span = high - low;
    let rows = FIB_RATIOS
        .iter()
        .map(|&ratio| FibRow {
            level: ratio,
            price: ratio.mul_add(span, low),
        })
        .collect();
    Ok(rows)
}

/// Horizon ladder for the volatility cones (in bars).
const CONE_WINDOWS: [usize; 5] = [10, 20, 30, 60, 90];

/// Volatility cones: for a ladder of horizon windows, the distribution of
/// annualized realized volatility (rolling close-to-close standard deviation,
/// scaled by `√252`) observed across the series.
///
/// Returns one [`ConeRow`] per horizon that the series is long enough to support,
/// each carrying the min / lower-quantile / median / upper-quantile / max of the
/// rolling realized volatilities plus the most recent (current) reading. Windows
/// longer than the series are skipped.
///
/// # Errors
///
/// Returns [`IndicatorError::InvalidPeriod`] when `lower_q`/`upper_q` are outside
/// `[0, 1]` or `lower_q > upper_q`.
pub fn cones(bars: &[Bar], params: ConesParams) -> Result<Vec<ConeRow>> {
    if !(0.0..=1.0).contains(&params.lower_q)
        || !(0.0..=1.0).contains(&params.upper_q)
        || params.lower_q > params.upper_q
    {
        return Err(IndicatorError::InvalidPeriod);
    }
    let closes: Vec<f64> = bars.iter().map(|b| b.close).collect();
    // Close-to-close log returns.
    let returns: Vec<f64> = closes
        .windows(2)
        .map(|w| if w[0] > 0.0 { (w[1] / w[0]).ln() } else { 0.0 })
        .collect();
    let annualization = 252.0_f64.sqrt();

    let mut rows = Vec::new();
    for &window in &CONE_WINDOWS {
        if returns.len() < window {
            continue;
        }
        // Rolling realized volatility (sample std of `window` returns), annualized.
        let mut vols: Vec<f64> = returns
            .windows(window)
            .map(|w| sample_std(w) * annualization)
            .collect();
        let Some(&realized) = vols.last() else {
            continue;
        };
        vols.sort_by(|first, second| {
            first
                .partial_cmp(second)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        rows.push(ConeRow {
            window,
            min: vols[0],
            lower: quantile_sorted(&vols, params.lower_q),
            median: quantile_sorted(&vols, 0.5),
            upper: quantile_sorted(&vols, params.upper_q),
            max: vols[vols.len() - 1],
            realized,
        });
    }
    Ok(rows)
}

/// Relative Rotation Graph (RRG) RS-Ratio and RS-Momentum of the asset closes
/// against a `benchmark` close series of the same length.
///
/// The relative-strength line is `rs_t = asset_close_t / benchmark_close_t`. The
/// RS-Ratio centers `rs` on a rolling mean/standard-deviation z-score
/// (`100 + (rs − μ)/σ`), and the RS-Momentum is the `~100`-centered rate of
/// change of the RS-Ratio over the same window. Date-aligned with leading
/// `None`s during warm-up.
///
/// # Errors
///
/// - [`IndicatorError::InvalidPeriod`] when `length == 0`.
/// - [`IndicatorError::InvalidPeriod`] when the benchmark length does not match
///   the bar count (reused as the single structured error for this crate).
pub fn relative_rotation(bars: &[Bar], params: RrgParams) -> Result<Vec<RrgRow>> {
    let RrgParams { benchmark, length } = params;
    require_period(length)?;
    if benchmark.len() != bars.len() {
        return Err(IndicatorError::InvalidPeriod);
    }
    let n = bars.len();
    let mut out = vec![RrgRow::default(); n];
    if n == 0 {
        return Ok(out);
    }
    let rs: Vec<f64> = bars
        .iter()
        .zip(benchmark.iter())
        .map(|(bar, &bench)| {
            if bench == 0.0 {
                0.0
            } else {
                bar.close / bench
            }
        })
        .collect();

    // RS-Ratio: rolling z-score of rs, centered at 100.
    let mut rs_ratio: Vec<Option<f64>> = vec![None; n];
    for t in (length - 1)..n {
        let window = &rs[t + 1 - length..=t];
        let mu = window.iter().sum::<f64>() / len_f64(length);
        let sd = sample_std(window);
        rs_ratio[t] = if sd == 0.0 {
            Some(100.0)
        } else {
            Some(100.0 + (rs[t] - mu) / sd)
        };
        out[t].rs_ratio = rs_ratio[t];
    }

    // RS-Momentum: ~100-centered rate of change of the RS-Ratio over `length`.
    for t in length..n {
        if let (Some(now), Some(prev)) = (rs_ratio[t], rs_ratio[t - length]) {
            out[t].rs_momentum = Some(100.0 + (now - prev));
        }
    }
    Ok(out)
}

/// Unbiased sample standard deviation of a slice (`0.0` for fewer than two
/// points).
fn sample_std(values: &[f64]) -> f64 {
    let n = values.len();
    if n < 2 {
        return 0.0;
    }
    let mean = values.iter().sum::<f64>() / len_f64(n);
    let ss: f64 = values.iter().map(|v| (v - mean).powi(2)).sum();
    (ss / len_f64(n - 1)).sqrt()
}

/// Type-7 linear-interpolation quantile of an already-sorted slice.
fn quantile_sorted(sorted: &[f64], q: f64) -> f64 {
    let n = sorted.len();
    if n == 0 {
        return 0.0;
    }
    if n == 1 {
        return sorted[0];
    }
    let rank = q.clamp(0.0, 1.0) * len_f64(n - 1);
    let lower = rank.floor();
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let lower_index = lower as usize;
    if lower_index + 1 >= n {
        return sorted[n - 1];
    }
    let frac = rank - lower;
    frac.mul_add(
        sorted[lower_index + 1] - sorted[lower_index],
        sorted[lower_index],
    )
}

#[cfg(test)]
mod tests {
    use super::{cg, clenow, cones, demark, fib, relative_rotation};
    use crate::fixture::series;
    use crate::params::{ClenowParams, ConesParams, FibParams, LengthParams, RrgParams};

    #[test]
    fn cg_is_date_aligned_with_warmup() {
        let bars = series();
        let out = cg(&bars, LengthParams::new(3)).expect("cg");
        assert_eq!(out.len(), bars.len());
        assert!(out[0].is_none() && out[1].is_none());
        assert!(out[2].is_some());
    }

    #[test]
    fn cg_known_window_value() {
        // closes 10,11,12 over window 3. weights (i+1) on reversed window where
        // i=0 is the current bar (12): num = 1*12 + 2*11 + 3*10 = 12+22+30 = 64;
        // den = 33; CG = -64/33 = -1.9393939...
        let bars = series();
        let out = cg(&bars, LengthParams::new(3)).expect("cg");
        let v = out[2].expect("defined");
        assert!((v - (-64.0 / 33.0)).abs() < 1e-12, "cg {v}");
    }

    #[test]
    fn cg_zero_period_errors() {
        assert!(cg(&series(), LengthParams::new(0)).is_err());
    }

    #[test]
    fn clenow_uptrend_is_positive() {
        // A clean monotonic up-trend ⇒ positive log-price slope ⇒ positive,
        // R²-weighted Clenow score at the final bar.
        let bars = series();
        let out = clenow(&bars, ClenowParams { period: 5 }).expect("clenow");
        assert_eq!(out.len(), bars.len());
        // The first 5-bar window (closes 10,11,12,12.5,13) is strictly rising.
        let first = out[4].expect("defined");
        assert!(
            first > 0.0,
            "clenow {first} should be positive on an uptrend"
        );
    }

    #[test]
    fn demark_counts_consecutive_directional_closes() {
        let bars = series();
        let out = demark(&bars).expect("demark");
        assert_eq!(out.len(), bars.len());
        // closes[4]=13 vs closes[0]=10 ⇒ up ⇒ sell setup starts at +1.
        assert_eq!(out[4].setup, Some(1));
        // closes[5]=14 vs closes[1]=11 ⇒ up ⇒ +2.
        assert_eq!(out[5].setup, Some(2));
    }

    #[test]
    fn fib_levels_span_low_to_high() {
        let bars = series();
        let rows = fib(&bars, FibParams { period: 0 }).expect("fib");
        assert_eq!(rows.len(), 7);
        // ratio 0 ⇒ swing low; ratio 1 ⇒ swing high.
        let low = bars.iter().map(|b| b.low).fold(f64::INFINITY, f64::min);
        let high = bars
            .iter()
            .map(|b| b.high)
            .fold(f64::NEG_INFINITY, f64::max);
        assert!((rows[0].price - low).abs() < 1e-12, "low {}", rows[0].price);
        assert!(
            (rows[6].price - high).abs() < 1e-12,
            "high {}",
            rows[6].price
        );
        // ratio 0.5 ⇒ midpoint.
        assert!((rows[3].price - f64::midpoint(low, high)).abs() < 1e-12);
    }

    #[test]
    fn cones_reports_a_row_per_supported_horizon() {
        let bars = series(); // 15 bars ⇒ only the 10-bar horizon is supported.
        let rows = cones(&bars, ConesParams::default()).expect("cones");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].window, 10);
        // The quantiles are ordered min <= lower <= median <= upper <= max.
        let r = rows[0];
        assert!(r.min <= r.lower + 1e-12);
        assert!(r.lower <= r.median + 1e-12);
        assert!(r.median <= r.upper + 1e-12);
        assert!(r.upper <= r.max + 1e-12);
    }

    #[test]
    fn cones_rejects_inverted_quantiles() {
        let err = cones(
            &series(),
            ConesParams {
                lower_q: 0.8,
                upper_q: 0.2,
            },
        );
        assert!(err.is_err());
    }

    #[test]
    fn relative_rotation_flat_when_asset_tracks_benchmark() {
        // asset == benchmark ⇒ rs is constant ⇒ zero dispersion ⇒ RS-Ratio pinned
        // at 100.
        let bars = series();
        let benchmark: Vec<f64> = bars.iter().map(|b| b.close).collect();
        let out = relative_rotation(
            &bars,
            RrgParams {
                benchmark,
                length: 3,
            },
        )
        .expect("rrg");
        assert_eq!(out.len(), bars.len());
        let v = out[2].rs_ratio.expect("defined");
        assert!((v - 100.0).abs() < 1e-9, "rs_ratio {v}");
    }

    #[test]
    fn relative_rotation_benchmark_length_mismatch_errors() {
        let bars = series();
        let out = relative_rotation(
            &bars,
            RrgParams {
                benchmark: vec![1.0, 2.0],
                length: 3,
            },
        );
        assert!(out.is_err());
    }
}
