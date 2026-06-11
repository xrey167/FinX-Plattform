//! Trend indicators: ADX (+DI/−DI), Aroon, Ichimoku, Vortex, and `SuperTrend`.
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
//! - **Ichimoku** (Hosoda): conversion (tenkan-sen) `= midpoint(HH,LL)` over
//!   `conversion`; base (kijun-sen) `= midpoint(HH,LL)` over `base`; leading
//!   span A `= midpoint(conversion, base)`; leading span B `= midpoint(HH,LL)`
//!   over `span_b`; lagging span (chikou) `= close` (plotted shifted back
//!   `lagging` bars). The leading spans are surfaced un-shifted at the current
//!   bar (the caller projects them forward `base` bars).
//! - **Vortex**: `VM+ = |high_t − low_{t−1}|`, `VM− = |low_t − high_{t−1}|`;
//!   `VI+ = Σ VM+ / Σ TR`, `VI− = Σ VM− / Σ TR`, each sum over a trailing
//!   `length`-bar window of the per-bar values.
//! - **`SuperTrend`**: basic bands `mid ± multiplier·ATR(length)` with
//!   `mid = midpoint(high, low)`; the final upper/lower bands are tightened
//!   (the upper band only moves down, the lower band only moves up while price
//!   stays on the same side); the line flips when price crosses it, and
//!   `direction` is `+1` below the line (up-trend) / `−1` above it.

use crate::params::{AdxParams, AroonParams, IchimokuParams, LengthParams, SupertrendParams};
use crate::{Bar, Result, highs, lows, require_period};

use super::indicator_helpers::{wilder_average, wilder_smooth};
use super::volume::{atr, true_range};
use super::{AdxRow, AroonRow, IchimokuRow, SupertrendRow, VortexRow};

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

/// Midpoint of the highest high and lowest low over the trailing `length`-bar
/// window ending at index `i`, or `None` before the first full window.
fn donchian_mid(highs: &[f64], lows: &[f64], length: usize, i: usize) -> Option<f64> {
    if i + 1 < length {
        return None;
    }
    let hh = highs[i + 1 - length..=i]
        .iter()
        .copied()
        .fold(f64::MIN, f64::max);
    let ll = lows[i + 1 - length..=i]
        .iter()
        .copied()
        .fold(f64::MAX, f64::min);
    Some(f64::midpoint(hh, ll))
}

/// Ichimoku Cloud over the bar slice.
///
/// # Errors
///
/// Returns [`crate::IndicatorError::InvalidPeriod`] when any window is `0`.
pub fn ichimoku(bars: &[Bar], params: IchimokuParams) -> Result<Vec<IchimokuRow>> {
    require_period(params.conversion)?;
    require_period(params.base)?;
    require_period(params.lagging)?;
    require_period(params.span_b)?;
    let highs = highs(bars);
    let lows = lows(bars);
    let n = bars.len();
    let mut out = vec![IchimokuRow::default(); n];
    for (i, row) in out.iter_mut().enumerate() {
        let conversion = donchian_mid(&highs, &lows, params.conversion, i);
        let base = donchian_mid(&highs, &lows, params.base, i);
        let senkou_a = match (conversion, base) {
            (Some(c), Some(b)) => Some(f64::midpoint(c, b)),
            _ => None,
        };
        let senkou_b = donchian_mid(&highs, &lows, params.span_b, i);
        *row = IchimokuRow {
            conversion,
            base,
            senkou_a,
            senkou_b,
            chikou: Some(bars[i].close),
        };
    }
    Ok(out)
}

/// Vortex Indicator (VI+ / VI−) over the bar slice.
///
/// # Errors
///
/// Returns [`crate::IndicatorError::InvalidPeriod`] when `length == 0`.
pub fn vortex(bars: &[Bar], params: LengthParams) -> Result<Vec<VortexRow>> {
    let length = params.length;
    require_period(length)?;
    let n = bars.len();
    let mut out = vec![VortexRow::default(); n];
    if n <= length {
        return Ok(out);
    }
    let tr = true_range(bars);
    // Per-bar vortex movements; index 0 has no prior bar.
    let mut vm_plus = vec![0.0; n];
    let mut vm_minus = vec![0.0; n];
    for i in 1..n {
        vm_plus[i] = (bars[i].high - bars[i - 1].low).abs();
        vm_minus[i] = (bars[i].low - bars[i - 1].high).abs();
    }
    // Trailing `length`-bar sums of the per-bar values (defined from index 1).
    for i in length..n {
        let window = i + 1 - length;
        let sum_tr: f64 = tr[window..=i].iter().sum();
        if sum_tr == 0.0 {
            continue;
        }
        let sum_plus: f64 = vm_plus[window..=i].iter().sum();
        let sum_minus: f64 = vm_minus[window..=i].iter().sum();
        out[i] = VortexRow {
            plus_vi: Some(sum_plus / sum_tr),
            minus_vi: Some(sum_minus / sum_tr),
        };
    }
    Ok(out)
}

/// `SuperTrend` (ATR trailing stop and direction) over the bar slice.
///
/// # Errors
///
/// Returns [`crate::IndicatorError::InvalidPeriod`] when `length == 0`.
pub fn supertrend(bars: &[Bar], params: SupertrendParams) -> Result<Vec<SupertrendRow>> {
    let length = params.length;
    require_period(length)?;
    let n = bars.len();
    let mut out = vec![SupertrendRow::default(); n];
    let atr = atr(bars, LengthParams::new(length))?;

    // Final bands and the active line carry forward across bars.
    let mut final_upper: Option<f64> = None;
    let mut final_lower: Option<f64> = None;
    let mut prev_line: Option<f64> = None;
    let mut prev_dir: i8 = 1;

    for i in 0..n {
        let Some(atr_i) = atr[i] else {
            continue;
        };
        let mid = f64::midpoint(bars[i].high, bars[i].low);
        let basic_upper = params.multiplier.mul_add(atr_i, mid);
        let basic_lower = (-params.multiplier).mul_add(atr_i, mid);
        let prev_close = if i == 0 {
            bars[i].close
        } else {
            bars[i - 1].close
        };

        // Tighten the bands: the upper band ratchets down (and the lower band up)
        // unless the prior close already broke through, which resets it.
        let upper = match final_upper {
            Some(prev) if basic_upper < prev || prev_close > prev => basic_upper,
            Some(prev) => prev,
            None => basic_upper,
        };
        let lower = match final_lower {
            Some(prev) if basic_lower > prev || prev_close < prev => basic_lower,
            Some(prev) => prev,
            None => basic_lower,
        };
        final_upper = Some(upper);
        final_lower = Some(lower);

        let close = bars[i].close;
        // Determine the active line/direction from the prior direction and this
        // close. In an up-trend the line is the lower band and flips down when
        // the close breaks below it; in a down-trend the line is the upper band
        // and flips up when the close breaks above it.
        let (line, dir) = if prev_line.is_none() {
            // Seed from this bar's close relative to the bands.
            if close <= upper {
                (upper, -1)
            } else {
                (lower, 1)
            }
        } else if prev_dir == 1 {
            if close < lower {
                (upper, -1)
            } else {
                (lower, 1)
            }
        } else if close > upper {
            (lower, 1)
        } else {
            (upper, -1)
        };
        prev_line = Some(line);
        prev_dir = dir;
        out[i] = SupertrendRow {
            supertrend: Some(line),
            direction: Some(dir),
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

    #[test]
    fn ichimoku_known_vector() {
        // conversion=2, base=3 over the fixture, evaluated at index 2.
        //  conversion(2) over bars[1..=2]: HH = max(11.2,12.4) = 12.4,
        //   LL = min(9.9,10.8) = 9.9 ⇒ mid = 11.15.
        //  base(3) over bars[0..=2]: HH = 12.4, LL = 9.8 ⇒ mid = 11.1.
        //  senkou_a = midpoint(11.15, 11.1) = 11.125.
        //  chikou = close[2] = 12.0.
        let out = ichimoku(
            &series(),
            IchimokuParams {
                conversion: 2,
                base: 3,
                lagging: 26,
                span_b: 4,
            },
        )
        .expect("ichimoku");
        let row = out[2];
        assert!((row.conversion.expect("conv") - 11.15).abs() < 1e-12);
        assert!((row.base.expect("base") - 11.1).abs() < 1e-12);
        assert!((row.senkou_a.expect("a") - 11.125).abs() < 1e-12);
        assert!((row.chikou.expect("chikou") - 12.0).abs() < 1e-12);
        // span_b(4) needs four bars ⇒ undefined at index 2, defined at 3.
        assert!(row.senkou_b.is_none());
        assert!(out[3].senkou_b.is_some());
    }

    #[test]
    fn vortex_known_vector() {
        // Hand-built 3-bar series:
        //  b0: H10 L8 C9; b1: H12 L9 C11; b2: H11 L7 C8.
        //  TR  = [2, 3, 4]; VM+ = [_, 4, 2]; VM- = [_, 1, 5].
        //  length=2, window = idx 1..=2: ΣTR=7, ΣVM+=6, ΣVM-=6 ⇒ both = 6/7.
        let mut bars = series()[..3].to_vec();
        let rows = [(10.0, 8.0, 9.0), (12.0, 9.0, 11.0), (11.0, 7.0, 8.0)];
        for (bar, &(high, low, close)) in bars.iter_mut().zip(rows.iter()) {
            bar.high = high;
            bar.low = low;
            bar.open = close;
            bar.close = close;
        }
        let out = vortex(&bars, LengthParams::new(2)).expect("vortex");
        assert!(out[1].plus_vi.is_none());
        assert!((out[2].plus_vi.expect("vi+") - 6.0 / 7.0).abs() < 1e-12);
        assert!((out[2].minus_vi.expect("vi-") - 6.0 / 7.0).abs() < 1e-12);
    }

    #[test]
    fn vortex_zero_length_is_invalid() {
        assert!(vortex(&series(), LengthParams::new(0)).is_err());
    }

    #[test]
    fn supertrend_direction_is_signed_and_brackets_close() {
        let bars = series();
        let out = supertrend(&bars, SupertrendParams::default()).expect("supertrend");
        assert_eq!(out.len(), bars.len());
        for (i, row) in out.iter().enumerate() {
            if let (Some(line), Some(dir)) = (row.supertrend, row.direction) {
                assert!(dir == 1 || dir == -1, "direction not ±1: {dir}");
                // In an up-trend the line is a lower stop (≤ close); in a
                // down-trend it is an upper stop (≥ close).
                if dir == 1 {
                    assert!(line <= bars[i].close + 1e-9, "up-trend line above close");
                } else {
                    assert!(line >= bars[i].close - 1e-9, "down-trend line below close");
                }
            }
        }
    }

    #[test]
    fn supertrend_flips_to_uptrend_on_breakout() {
        // A short down-then-sharp-up series: once the close pierces the upper
        // band the direction must flip to +1 (up-trend) at some later bar.
        let mut bars = series();
        // Force a strong rally in the back half so a flip is guaranteed.
        for (i, bar) in bars.iter_mut().enumerate().skip(9) {
            let lift = f64::from(u8::try_from(i - 8).expect("small")) * 2.0;
            bar.high += lift + 1.0;
            bar.low += lift;
            bar.close += lift + 0.5;
            bar.open += lift;
        }
        let out = supertrend(
            &bars,
            SupertrendParams {
                length: 3,
                multiplier: 2.0,
            },
        )
        .expect("supertrend");
        assert!(
            out.iter().any(|r| r.direction == Some(1)),
            "expected at least one up-trend bar after the rally"
        );
    }
}
