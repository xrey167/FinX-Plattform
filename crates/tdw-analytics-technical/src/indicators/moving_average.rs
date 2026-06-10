//! Moving averages and the MACD.
//!
//! Definitions are the standard textbook forms:
//! - **SMA**: arithmetic mean of the trailing `length` closes.
//! - **EMA**: `EMA_t = α·price_t + (1−α)·EMA_{t−1}` with `α = 2/(length+1)`,
//!   seeded by the SMA of the first `length` samples (the common seeding used by
//!   `OpenBB` / `pandas-ta` / `TA-Lib`).
//! - **WMA**: linearly weighted mean, weight `i` (1..=length) on the `i`-th most
//!   recent close, normalised by `length·(length+1)/2`.
//! - **HMA** (Hull, 2005): `WMA(2·WMA(price, length/2) − WMA(price, length),
//!   floor(sqrt(length)))`.
//! - **MACD** (Appel): `EMA(fast) − EMA(slow)`, signal `= EMA(macd, signal)`,
//!   histogram `= macd − signal`. Defaults 12/26/9.

use crate::params::{HmaParams, LengthParams, MacdParams};
use crate::{Result, require_period};

use super::MacdRow;

/// Convert a small window length to `f64` for averaging. Lengths are bar counts
/// (tiny), so this is exact in `f64`.
#[allow(clippy::cast_precision_loss)]
const fn len_f64(n: usize) -> f64 {
    n as f64
}

/// Simple moving average of `prices` over `length`.
///
/// Output length equals input length; the first `length−1` positions are
/// [`None`].
///
/// # Errors
///
/// Returns [`crate::IndicatorError::InvalidPeriod`] when `length == 0`.
pub fn sma(prices: &[f64], params: LengthParams) -> Result<Vec<Option<f64>>> {
    let length = params.length;
    require_period(length)?;
    let mut out = vec![None; prices.len()];
    if prices.len() < length {
        return Ok(out);
    }
    let mut window_sum: f64 = prices[..length].iter().sum();
    out[length - 1] = Some(window_sum / len_f64(length));
    for i in length..prices.len() {
        window_sum += prices[i] - prices[i - length];
        out[i] = Some(window_sum / len_f64(length));
    }
    Ok(out)
}

/// Exponential moving average of `prices` over `length`, SMA-seeded.
///
/// # Errors
///
/// Returns [`crate::IndicatorError::InvalidPeriod`] when `length == 0`.
pub fn ema(prices: &[f64], params: LengthParams) -> Result<Vec<Option<f64>>> {
    let length = params.length;
    require_period(length)?;
    let mut out = vec![None; prices.len()];
    if prices.len() < length {
        return Ok(out);
    }
    let alpha = 2.0 / (len_f64(length) + 1.0);
    let seed: f64 = prices[..length].iter().sum::<f64>() / len_f64(length);
    let mut prev = seed;
    out[length - 1] = Some(seed);
    for (i, &price) in prices.iter().enumerate().skip(length) {
        prev = alpha.mul_add(price - prev, prev);
        out[i] = Some(prev);
    }
    Ok(out)
}

/// Linearly weighted moving average of `prices` over `length`.
///
/// # Errors
///
/// Returns [`crate::IndicatorError::InvalidPeriod`] when `length == 0`.
pub fn wma(prices: &[f64], params: LengthParams) -> Result<Vec<Option<f64>>> {
    let length = params.length;
    require_period(length)?;
    let mut out = vec![None; prices.len()];
    if prices.len() < length {
        return Ok(out);
    }
    let denom = len_f64(length * (length + 1)) / 2.0;
    for i in (length - 1)..prices.len() {
        let mut acc = 0.0;
        for (offset, &price) in prices[i + 1 - length..=i].iter().enumerate() {
            // Most recent close (offset = length-1) gets the largest weight.
            acc += price * len_f64(offset + 1);
        }
        out[i] = Some(acc / denom);
    }
    Ok(out)
}

/// Hull moving average of `prices`.
///
/// # Errors
///
/// Returns [`crate::IndicatorError::InvalidPeriod`] when `length == 0`.
pub fn hma(prices: &[f64], params: HmaParams) -> Result<Vec<Option<f64>>> {
    let length = params.length;
    require_period(length)?;
    let half = (length / 2).max(1);
    // `sqrt(length)` is a small non-negative window size; the floor cast is the
    // textbook HMA definition and cannot truncate/sign-flip a real bar count.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let sqrt_len = (len_f64(length).sqrt() as usize).max(1);

    let wma_half = wma(prices, LengthParams::new(half))?;
    let wma_full = wma(prices, LengthParams::new(length))?;

    // raw_t = 2·WMA_half − WMA_full, defined where both inputs are defined.
    let raw: Vec<f64> = wma_half
        .iter()
        .zip(wma_full.iter())
        .map(|(h, f)| match (h, f) {
            (Some(h), Some(f)) => 2.0f64.mul_add(*h, -*f),
            _ => f64::NAN,
        })
        .collect();

    // WMA of the raw series over sqrt(length), skipping the NaN prefix so the
    // smoothing window only spans defined raw values.
    let first_defined = raw.iter().position(|v| v.is_finite());
    let mut out = vec![None; prices.len()];
    let Some(start) = first_defined else {
        return Ok(out);
    };
    let defined = &raw[start..];
    let smoothed = wma(defined, LengthParams::new(sqrt_len))?;
    for (i, value) in smoothed.into_iter().enumerate() {
        out[start + i] = value;
    }
    Ok(out)
}

/// MACD line, signal, and histogram over `prices`.
///
/// # Errors
///
/// Returns [`crate::IndicatorError::InvalidPeriod`] when any span is `0`.
pub fn macd(prices: &[f64], params: MacdParams) -> Result<Vec<MacdRow>> {
    require_period(params.fast)?;
    require_period(params.slow)?;
    require_period(params.signal)?;

    let fast = ema(prices, LengthParams::new(params.fast))?;
    let slow = ema(prices, LengthParams::new(params.slow))?;

    // MACD line is defined where both EMAs are defined.
    let line: Vec<Option<f64>> = fast
        .iter()
        .zip(slow.iter())
        .map(|(f, s)| match (f, s) {
            (Some(f), Some(s)) => Some(f - s),
            _ => None,
        })
        .collect();

    // Signal = EMA of the (contiguous) defined MACD-line tail.
    let first = line.iter().position(Option::is_some);
    let mut out: Vec<MacdRow> = line
        .iter()
        .map(|macd| MacdRow {
            macd: *macd,
            signal: None,
            histogram: None,
        })
        .collect();

    if let Some(start) = first {
        let tail: Vec<f64> = line[start..].iter().map(|v| v.unwrap_or(0.0)).collect();
        let signal = ema(&tail, LengthParams::new(params.signal))?;
        for (i, sig) in signal.into_iter().enumerate() {
            let row = &mut out[start + i];
            row.signal = sig;
            row.histogram = match (row.macd, sig) {
                (Some(m), Some(s)) => Some(m - s),
                _ => None,
            };
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::{assert_close_opt, series};
    use crate::{Bar, closes};

    fn p(v: &[f64]) -> Vec<f64> {
        v.to_vec()
    }

    #[test]
    fn sma_matches_hand_computed_window() {
        // SMA(3) over [1,2,3,4,5]: means are 2, 3, 4 at indices 2..4.
        let out = sma(&p(&[1.0, 2.0, 3.0, 4.0, 5.0]), LengthParams::new(3)).expect("sma");
        assert_close_opt(&out, &[None, None, Some(2.0), Some(3.0), Some(4.0)], 1e-12);
    }

    #[test]
    fn ema_seed_is_sma_then_recurses() {
        // EMA(3) over [1,2,3,4,5]: seed = mean(1,2,3) = 2 at idx 2; α = 0.5.
        // idx3 = 0.5*4 + 0.5*2 = 3; idx4 = 0.5*5 + 0.5*3 = 4.
        let out = ema(&p(&[1.0, 2.0, 3.0, 4.0, 5.0]), LengthParams::new(3)).expect("ema");
        assert_close_opt(&out, &[None, None, Some(2.0), Some(3.0), Some(4.0)], 1e-12);
    }

    #[test]
    fn wma_weights_recent_more() {
        // WMA(3) over [1,2,3]: (1*1 + 2*2 + 3*3) / 6 = 14/6.
        let out = wma(&p(&[1.0, 2.0, 3.0]), LengthParams::new(3)).expect("wma");
        assert_close_opt(&out, &[None, None, Some(14.0 / 6.0)], 1e-12);
    }

    #[test]
    fn hma_length_and_finite_tail() {
        let out = hma(&closes(&series()), HmaParams { length: 4 }).expect("hma");
        assert_eq!(out.len(), series().len());
        assert!(out.last().expect("row").is_some());
    }

    #[test]
    fn macd_default_histogram_is_line_minus_signal() {
        let closes = closes(&series());
        let out = macd(
            &closes,
            MacdParams {
                fast: 2,
                slow: 4,
                signal: 2,
            },
        )
        .expect("macd");
        assert_eq!(out.len(), closes.len());
        for row in &out {
            if let (Some(m), Some(s), Some(h)) = (row.macd, row.signal, row.histogram) {
                assert!((h - (m - s)).abs() < 1e-12);
            }
        }
    }

    #[test]
    fn zero_length_is_invalid() {
        let _: Bar; // keep import used across the module's test set
        assert!(sma(&[1.0], LengthParams::new(0)).is_err());
        assert!(ema(&[1.0], LengthParams::new(0)).is_err());
        assert!(wma(&[1.0], LengthParams::new(0)).is_err());
    }
}
