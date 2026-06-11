//! Volatility and volume indicators: ATR, OBV, A/D, and VWAP.
//!
//! Definitions:
//! - **True Range** (Wilder 1978): `TR = max(high−low, |high−prevClose|,
//!   |low−prevClose|)`.
//! - **ATR** (Wilder): seed with the simple mean of the first `length` TRs,
//!   then `ATR_t = (ATR_{t−1}·(length−1) + TR_t)/length`.
//! - **OBV** (Granville): running total that adds the bar volume on an up-close
//!   and subtracts it on a down-close.
//! - **A/D** (Chaikin Accumulation/Distribution): running total of the money-
//!   flow volume `((close−low)−(high−close))/(high−low)·volume`.
//! - **VWAP**: cumulative `Σ(typicalPrice·volume)/Σ(volume)`, typical price
//!   `(high+low+close)/3` (running, session-anchored at the first bar).

use crate::params::{AdoscParams, LengthParams};
use crate::{Bar, Result, require_period};

use super::moving_average::ema;

#[allow(clippy::cast_precision_loss)]
const fn len_f64(n: usize) -> f64 {
    n as f64
}

/// True-range column for the bar slice; the first bar has no prior close so its
/// TR is `high − low`.
#[must_use]
pub fn true_range(bars: &[Bar]) -> Vec<f64> {
    let mut out = Vec::with_capacity(bars.len());
    for (i, bar) in bars.iter().enumerate() {
        let hl = bar.high - bar.low;
        if i == 0 {
            out.push(hl);
        } else {
            let prev_close = bars[i - 1].close;
            let h_pc = (bar.high - prev_close).abs();
            let l_pc = (bar.low - prev_close).abs();
            out.push(hl.max(h_pc).max(l_pc));
        }
    }
    out
}

/// Wilder's Average True Range over the bar slice.
///
/// The first defined ATR sits at index `length` (it consumes `length` true
/// ranges, the first of which is bar 0's `high−low`).
///
/// # Errors
///
/// Returns [`crate::IndicatorError::InvalidPeriod`] when `length == 0`.
pub fn atr(bars: &[Bar], params: LengthParams) -> Result<Vec<Option<f64>>> {
    let length = params.length;
    require_period(length)?;
    let tr = true_range(bars);
    let mut out = vec![None; bars.len()];
    if bars.len() < length {
        return Ok(out);
    }
    let seed = tr[..length].iter().sum::<f64>() / len_f64(length);
    let mut prev = seed;
    out[length - 1] = Some(seed);
    for (i, &range) in tr.iter().enumerate().skip(length) {
        prev = (prev.mul_add(len_f64(length - 1), range)) / len_f64(length);
        out[i] = Some(prev);
    }
    Ok(out)
}

/// On-Balance Volume running total over the bar slice. Defined from bar 0 (seed
/// 0), accumulating from bar 1 onward.
#[must_use]
pub fn obv(bars: &[Bar]) -> Vec<Option<f64>> {
    let mut out = vec![None; bars.len()];
    if bars.is_empty() {
        return out;
    }
    let mut total = 0.0_f64;
    out[0] = Some(0.0);
    for i in 1..bars.len() {
        let delta = bars[i].close - bars[i - 1].close;
        if delta > 0.0 {
            total += bars[i].volume;
        } else if delta < 0.0 {
            total -= bars[i].volume;
        }
        out[i] = Some(total);
    }
    out
}

/// Chaikin Accumulation/Distribution running line over the bar slice.
#[must_use]
pub fn ad(bars: &[Bar]) -> Vec<Option<f64>> {
    let mut out = vec![None; bars.len()];
    let mut total = 0.0_f64;
    for (i, bar) in bars.iter().enumerate() {
        let span = bar.high - bar.low;
        let mfv = if span == 0.0 {
            0.0
        } else {
            (((bar.close - bar.low) - (bar.high - bar.close)) / span) * bar.volume
        };
        total += mfv;
        out[i] = Some(total);
    }
    out
}

/// Session-anchored VWAP running over the bar slice (cumulative from bar 0).
#[must_use]
pub fn vwap(bars: &[Bar]) -> Vec<Option<f64>> {
    let mut out = vec![None; bars.len()];
    let mut price_volume = 0.0_f64;
    let mut total_volume = 0.0_f64;
    for (i, bar) in bars.iter().enumerate() {
        let typical = (bar.high + bar.low + bar.close) / 3.0;
        price_volume += typical * bar.volume;
        total_volume += bar.volume;
        if total_volume != 0.0 {
            out[i] = Some(price_volume / total_volume);
        }
    }
    out
}

/// Chaikin Accumulation/Distribution Oscillator over the bar slice.
///
/// `ADOSC = EMA(AD, fast) − EMA(AD, slow)` where `AD` is the Chaikin
/// accumulation/distribution line (reusing [`ad`]). Both EMAs are SMA-seeded
/// over the AD line, so the oscillator first becomes defined once the slow EMA
/// is defined (at index `slow − 1`).
///
/// # Errors
///
/// Returns [`crate::IndicatorError::InvalidPeriod`] when either span is `0`.
pub fn adosc(bars: &[Bar], params: AdoscParams) -> Result<Vec<Option<f64>>> {
    require_period(params.fast)?;
    require_period(params.slow)?;
    // The A/D line is defined at every bar, so unwrap to a dense f64 column.
    let ad_line: Vec<f64> = ad(bars).into_iter().map(|v| v.unwrap_or(0.0)).collect();
    let fast = ema(&ad_line, LengthParams::new(params.fast))?;
    let slow = ema(&ad_line, LengthParams::new(params.slow))?;
    Ok(fast
        .iter()
        .zip(slow.iter())
        .map(|(f, s)| match (f, s) {
            (Some(f), Some(s)) => Some(f - s),
            _ => None,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::series;
    use crate::params::AdoscParams;

    #[test]
    fn true_range_first_bar_is_high_minus_low() {
        let bars = series();
        let tr = true_range(&bars);
        assert!((tr[0] - (bars[0].high - bars[0].low)).abs() < 1e-12);
    }

    #[test]
    fn atr_first_defined_at_length_minus_one() {
        let out = atr(&series(), LengthParams::new(14)).expect("atr");
        assert!(out[12].is_none());
        assert!(out[13].is_some());
    }

    #[test]
    fn obv_accumulates_on_direction() {
        // closes up,up,down ⇒ obv: 0, +v1, +v1+v2, +v1+v2-v3
        let bars = series();
        let out = obv(&bars);
        assert_eq!(out[0], Some(0.0));
        // bar1 close (11) > bar0 close (10): add bar1 volume.
        assert!((out[1].unwrap_or(0.0) - bars[1].volume).abs() < 1e-12);
    }

    #[test]
    fn vwap_first_bar_is_typical_price() {
        let bars = series();
        let out = vwap(&bars);
        let typical = (bars[0].high + bars[0].low + bars[0].close) / 3.0;
        assert!((out[0].unwrap_or(0.0) - typical).abs() < 1e-12);
    }

    #[test]
    fn ad_defined_everywhere() {
        let out = ad(&series());
        assert!(out.iter().all(Option::is_some));
    }

    fn adosc_fixture() -> Vec<Bar> {
        // Three bars engineered for an easy hand calculation. Each spans
        // high=2, low=0 so the money-flow multiplier is (2*close - 2)/2.
        //  A: close=2 ⇒ mfv = +1*10 = +10  ⇒ AD = 10
        //  B: close=0 ⇒ mfv = -1*10 = -10  ⇒ AD = 0
        //  C: close=1 ⇒ mfv =  0*10 =   0  ⇒ AD = 0
        let mut bars = series()[..3].to_vec();
        let rows = [(2.0, 2.0), (0.0, 0.0), (2.0, 1.0)];
        for (bar, &(high_close_top, close)) in bars.iter_mut().zip(rows.iter()) {
            bar.high = 2.0;
            bar.low = 0.0;
            bar.open = high_close_top; // unused by AD; keep OHLC nesting sane
            bar.close = close;
            bar.volume = 10.0;
        }
        bars
    }

    #[test]
    fn adosc_known_vector() {
        // AD line = [10, 0, 0]. fast=1 ⇒ EMA = AD itself = [10, 0, 0].
        // slow=2 ⇒ seed = mean(10,0) = 5 at idx1; α = 2/3 ⇒
        //   idx2 = (2/3)*0 + (1/3)*5 = 5/3.
        // ADOSC = fast − slow ⇒ [None, 0-5 = -5, 0 - 5/3 = -5/3].
        let out = adosc(&adosc_fixture(), AdoscParams { fast: 1, slow: 2 }).expect("adosc");
        assert_eq!(out[0], None);
        assert!((out[1].expect("idx1") - (-5.0)).abs() < 1e-12);
        assert!((out[2].expect("idx2") - (-5.0 / 3.0)).abs() < 1e-12);
    }

    #[test]
    fn adosc_zero_span_is_invalid() {
        assert!(adosc(&series(), AdoscParams { fast: 0, slow: 10 }).is_err());
        assert!(adosc(&series(), AdoscParams { fast: 3, slow: 0 }).is_err());
    }
}
