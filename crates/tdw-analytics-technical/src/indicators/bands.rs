//! Channel indicators: Bollinger, Keltner, and Donchian.
//!
//! Definitions:
//! - **Bollinger Bands** (Bollinger): middle `= SMA(close, length)`; upper/lower
//!   `= middle ± std·σ`, where `σ` is the population standard deviation of the
//!   trailing `length` closes.
//! - **Keltner Channels**: middle `= EMA(close, length)`; upper/lower
//!   `= middle ± multiplier·ATR(atr_length)`.
//! - **Donchian Channels**: upper `= max(high, length)`,
//!   lower `= min(low, length)`, middle `= (upper + lower)/2`.

use crate::params::{BollingerParams, DonchianParams, KeltnerParams, LengthParams};
use crate::{Bar, Result, closes, highs, lows, require_period};

use super::ChannelRow;
use super::moving_average::ema;
use super::volume::atr;

#[allow(clippy::cast_precision_loss)]
const fn len_f64(n: usize) -> f64 {
    n as f64
}

/// Bollinger Bands over the bar slice (computed on closes).
///
/// # Errors
///
/// Returns [`crate::IndicatorError::InvalidPeriod`] when `length == 0`.
pub fn bollinger(bars: &[Bar], params: BollingerParams) -> Result<Vec<ChannelRow>> {
    let length = params.length;
    require_period(length)?;
    let closes = closes(bars);
    let mut out = vec![ChannelRow::default(); bars.len()];
    if closes.len() < length {
        return Ok(out);
    }
    for i in (length - 1)..closes.len() {
        let window = &closes[i + 1 - length..=i];
        let mean = window.iter().sum::<f64>() / len_f64(length);
        let variance = window.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / len_f64(length);
        let sigma = variance.sqrt();
        out[i] = ChannelRow {
            upper: Some(params.std.mul_add(sigma, mean)),
            middle: Some(mean),
            lower: Some((-params.std).mul_add(sigma, mean)),
        };
    }
    Ok(out)
}

/// Keltner Channels over the bar slice.
///
/// # Errors
///
/// Returns [`crate::IndicatorError::InvalidPeriod`] when any window is `0`.
pub fn keltner(bars: &[Bar], params: KeltnerParams) -> Result<Vec<ChannelRow>> {
    require_period(params.length)?;
    require_period(params.atr_length)?;
    let mid = ema(&closes(bars), LengthParams::new(params.length))?;
    let range = atr(bars, LengthParams::new(params.atr_length))?;
    let mut out = vec![ChannelRow::default(); bars.len()];
    for (i, row) in out.iter_mut().enumerate() {
        if let (Some(m), Some(a)) = (mid[i], range[i]) {
            *row = ChannelRow {
                upper: Some(params.multiplier.mul_add(a, m)),
                middle: Some(m),
                lower: Some((-params.multiplier).mul_add(a, m)),
            };
        }
    }
    Ok(out)
}

/// Donchian Channels over the bar slice.
///
/// # Errors
///
/// Returns [`crate::IndicatorError::InvalidPeriod`] when `length == 0`.
pub fn donchian(bars: &[Bar], params: DonchianParams) -> Result<Vec<ChannelRow>> {
    let length = params.length;
    require_period(length)?;
    let highs = highs(bars);
    let lows = lows(bars);
    let mut out = vec![ChannelRow::default(); bars.len()];
    if bars.len() < length {
        return Ok(out);
    }
    for i in (length - 1)..bars.len() {
        let upper = highs[i + 1 - length..=i]
            .iter()
            .copied()
            .fold(f64::MIN, f64::max);
        let lower = lows[i + 1 - length..=i]
            .iter()
            .copied()
            .fold(f64::MAX, f64::min);
        out[i] = ChannelRow {
            upper: Some(upper),
            middle: Some(f64::midpoint(upper, lower)),
            lower: Some(lower),
        };
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::series;

    #[test]
    fn bollinger_bands_straddle_middle() {
        let out = bollinger(
            &series(),
            BollingerParams {
                length: 5,
                std: 2.0,
            },
        )
        .expect("bb");
        for row in out {
            if let (Some(u), Some(m), Some(l)) = (row.upper, row.middle, row.lower) {
                assert!(u >= m && m >= l, "bands not ordered: {u} {m} {l}");
            }
        }
    }

    #[test]
    fn bollinger_known_vector_flat_series() {
        // A flat close series has zero variance ⇒ all three bands equal the mean.
        let bars: Vec<Bar> = {
            let mut s = series();
            for b in &mut s {
                b.open = 12.0;
                b.high = 12.0;
                b.low = 12.0;
                b.close = 12.0;
            }
            s
        };
        let out = bollinger(
            &bars,
            BollingerParams {
                length: 3,
                std: 2.0,
            },
        )
        .expect("bb");
        let row = out[5];
        assert_eq!(row.middle, Some(12.0));
        assert_eq!(row.upper, Some(12.0));
        assert_eq!(row.lower, Some(12.0));
    }

    #[test]
    fn donchian_known_vector() {
        // Donchian(3) middle is the midpoint of the rolling high/low.
        let out = donchian(&series(), DonchianParams { length: 3 }).expect("dc");
        let bars = series();
        let i = 4;
        let hh = bars[2..=4].iter().map(|b| b.high).fold(f64::MIN, f64::max);
        let ll = bars[2..=4].iter().map(|b| b.low).fold(f64::MAX, f64::min);
        assert!((out[i].upper.unwrap_or(0.0) - hh).abs() < 1e-12);
        assert!((out[i].lower.unwrap_or(0.0) - ll).abs() < 1e-12);
        assert!((out[i].middle.unwrap_or(0.0) - f64::midpoint(hh, ll)).abs() < 1e-12);
    }

    #[test]
    fn keltner_bands_ordered() {
        let out = keltner(&series(), KeltnerParams::default()).expect("kc");
        for row in out {
            if let (Some(u), Some(m), Some(l)) = (row.upper, row.middle, row.lower) {
                assert!(u >= m && m >= l);
            }
        }
    }
}
