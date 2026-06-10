//! Trend indicators: ADX (+DI/−DI) and Aroon.
//!
//! Definitions:
//! - **ADX / +DI / −DI** (Wilder 1978): directional movement
//!   `+DM = max(up, 0)` when `up > down` else 0, `−DM = max(down, 0)` when
//!   `down > up` else 0, where `up = high_t − high_{t−1}`,
//!   `down = low_{t−1} − low_t`. Wilder-smooth `TR`, `+DM`, `−DM` over `length`;
//!   `+DI = 100·smooth(+DM)/smooth(TR)`, likewise `−DI`;
//!   `DX = 100·|+DI − −DI|/(+DI + −DI)`; `ADX` is the Wilder-smoothed `DX`.
//! - **Aroon**: `AroonUp = 100·(length − barsSinceHigh)/length`,
//!   `AroonDown = 100·(length − barsSinceLow)/length` over a trailing
//!   `length`-bar window; oscillator `= up − down`.

use crate::params::{AdxParams, AroonParams};
use crate::{Bar, Result, highs, lows, require_period};

use super::indicator_helpers::{wilder_average, wilder_smooth};
use super::{AdxRow, AroonRow};

#[allow(clippy::cast_precision_loss)]
const fn len_f64(n: usize) -> f64 {
    n as f64
}

/// Wilder's ADX with the +DI/−DI directional lines over the bar slice.
///
/// # Errors
///
/// Returns [`crate::IndicatorError::InvalidPeriod`] when `length == 0`.
pub fn adx(bars: &[Bar], params: AdxParams) -> Result<Vec<AdxRow>> {
    let length = params.length;
    require_period(length)?;
    let n = bars.len();
    let mut out = vec![AdxRow::default(); n];
    if n <= length {
        return Ok(out);
    }

    // Per-bar TR, +DM, −DM (index 0 has no prior bar, so it is excluded).
    let mut tr = vec![0.0; n];
    let mut pos_dm = vec![0.0; n];
    let mut neg_dm = vec![0.0; n];
    for i in 1..n {
        let up = bars[i].high - bars[i - 1].high;
        let down = bars[i - 1].low - bars[i].low;
        pos_dm[i] = if up > down && up > 0.0 { up } else { 0.0 };
        neg_dm[i] = if down > up && down > 0.0 { down } else { 0.0 };
        let hl = bars[i].high - bars[i].low;
        let h_pc = (bars[i].high - bars[i - 1].close).abs();
        let l_pc = (bars[i].low - bars[i - 1].close).abs();
        tr[i] = hl.max(h_pc).max(l_pc);
    }

    // Wilder-smooth each series over `length`, seeded by the sum of the first
    // `length` values (deltas live at indices 1..=length).
    let tr_run = wilder_smooth(&tr[1..], length);
    let plus_run = wilder_smooth(&pos_dm[1..], length);
    let minus_run = wilder_smooth(&neg_dm[1..], length);

    // The smoothed series are offset by one (they start from index 1).
    let mut dx: Vec<Option<f64>> = vec![None; n];
    for k in 0..tr_run.len() {
        let i = k + 1;
        let (Some(trv), Some(pv), Some(mv)) = (tr_run[k], plus_run[k], minus_run[k]) else {
            continue;
        };
        if trv == 0.0 {
            continue;
        }
        let plus_di = 100.0 * pv / trv;
        let minus_di = 100.0 * mv / trv;
        let di_sum = plus_di + minus_di;
        let dx_val = if di_sum == 0.0 {
            0.0
        } else {
            100.0 * (plus_di - minus_di).abs() / di_sum
        };
        dx[i] = Some(dx_val);
        out[i].plus_di = Some(plus_di);
        out[i].minus_di = Some(minus_di);
    }

    // ADX = Wilder-smoothed DX. Smooth the contiguous defined DX tail.
    let first_dx = dx.iter().position(Option::is_some);
    if let Some(start) = first_dx {
        let tail: Vec<f64> = dx[start..].iter().map(|v| v.unwrap_or(0.0)).collect();
        let adx_tail = wilder_average(&tail, length);
        for (k, value) in adx_tail.into_iter().enumerate() {
            out[start + k].adx = value;
        }
    }
    Ok(out)
}

/// Aroon up/down/oscillator over the bar slice.
///
/// # Errors
///
/// Returns [`crate::IndicatorError::InvalidPeriod`] when `length == 0`.
pub fn aroon(bars: &[Bar], params: AroonParams) -> Result<Vec<AroonRow>> {
    let length = params.length;
    require_period(length)?;
    let highs = highs(bars);
    let lows = lows(bars);
    let n = bars.len();
    let mut out = vec![AroonRow::default(); n];
    // Aroon needs `length + 1` bars: a trailing window of `length` plus the
    // current bar, matching the common (length+1)-sample definition.
    for i in length..n {
        let window = &highs[i - length..=i];
        let lwindow = &lows[i - length..=i];
        let since_high = bars_since_extreme(window, true);
        let since_low = bars_since_extreme(lwindow, false);
        let up = 100.0 * (len_f64(length) - len_f64(since_high)) / len_f64(length);
        let down = 100.0 * (len_f64(length) - len_f64(since_low)) / len_f64(length);
        out[i] = AroonRow {
            up: Some(up),
            down: Some(down),
            oscillator: Some(up - down),
        };
    }
    Ok(out)
}

/// Bars since the most-recent extreme in `window`, counted back from the last
/// element (0 = the extreme is the current bar). `high` selects max vs min.
fn bars_since_extreme(window: &[f64], high: bool) -> usize {
    let last = window.len() - 1;
    let mut best_idx = last;
    let mut best = window[last];
    for (i, &v) in window.iter().enumerate() {
        let better = if high { v >= best } else { v <= best };
        if better {
            best = v;
            best_idx = i;
        }
    }
    last - best_idx
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::series;

    #[test]
    fn adx_components_in_range() {
        let out = adx(&series(), AdxParams { length: 5 }).expect("adx");
        assert_eq!(out.len(), series().len());
        for row in &out {
            if let Some(v) = row.plus_di {
                assert!((0.0..=100.0).contains(&v), "+DI out of range: {v}");
            }
            if let Some(v) = row.adx {
                assert!((0.0..=100.0).contains(&v), "ADX out of range: {v}");
            }
        }
    }

    #[test]
    fn aroon_up_is_hundred_at_new_high() {
        // Strictly rising highs ⇒ the current bar is always the window high ⇒
        // Aroon-up is 100 at every defined index.
        let bars = series();
        let out = aroon(&bars, AroonParams { length: 5 }).expect("aroon");
        // The fixture rises for the first six bars; index 5 is a fresh high.
        let row = out[5];
        assert_eq!(row.up, Some(100.0));
    }

    #[test]
    fn aroon_oscillator_is_up_minus_down() {
        let out = aroon(&series(), AroonParams { length: 5 }).expect("aroon");
        for row in out {
            if let (Some(u), Some(d), Some(o)) = (row.up, row.down, row.oscillator) {
                assert!((o - (u - d)).abs() < 1e-12);
            }
        }
    }
}
