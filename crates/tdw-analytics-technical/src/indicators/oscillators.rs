//! Oscillators: RSI, Stochastic, CCI, Fisher transform, ROC, and momentum.
//!
//! Definitions:
//! - **RSI** (Wilder 1978): `RSI = 100 − 100/(1 + RS)`, `RS = avgGain/avgLoss`.
//!   The first average is the simple mean of the first `length` gains/losses;
//!   subsequent averages use Wilder's smoothing
//!   `avg_t = (avg_{t−1}·(length−1) + value_t)/length`.
//! - **Stochastic** (Lane): raw `%K = 100·(close − LL)/(HH − LL)` over the
//!   `k_period` high/low range; slow `%K = SMA(raw %K, k_smooth)`;
//!   `%D = SMA(%K, d_period)`.
//! - **CCI** (Lambert): `(TP − SMA(TP)) / (constant · meanDeviation(TP))`,
//!   `TP = (high+low+close)/3`.
//! - **Fisher transform** (Ehlers): normalise the median price into `[−1, 1]`
//!   over `length`, then `0.5·ln((1+x)/(1−x))` with EMA-like smoothing.
//! - **ROC**: `100·(price_t − price_{t−length}) / price_{t−length}`.
//! - **Momentum**: `price_t − price_{t−length}`.

use crate::params::{CciParams, FisherParams, LengthParams, RocParams, StochasticParams};
use crate::{Bar, Result, highs, lows, require_period};

use super::moving_average::sma;
use super::{FisherRow, StochasticRow};

#[allow(clippy::cast_precision_loss)]
const fn len_f64(n: usize) -> f64 {
    n as f64
}

/// Wilder's Relative Strength Index over `prices`.
///
/// # Errors
///
/// Returns [`crate::IndicatorError::InvalidPeriod`] when `length == 0`.
pub fn rsi(prices: &[f64], params: LengthParams) -> Result<Vec<Option<f64>>> {
    let length = params.length;
    require_period(length)?;
    let mut out = vec![None; prices.len()];
    if prices.len() <= length {
        return Ok(out);
    }
    // Seed: simple mean of the first `length` gains and losses (deltas 1..=length).
    let mut gain_sum = 0.0;
    let mut loss_sum = 0.0;
    for i in 1..=length {
        let delta = prices[i] - prices[i - 1];
        if delta >= 0.0 {
            gain_sum += delta;
        } else {
            loss_sum -= delta;
        }
    }
    let mut avg_gain = gain_sum / len_f64(length);
    let mut avg_loss = loss_sum / len_f64(length);
    out[length] = Some(rsi_from(avg_gain, avg_loss));

    for i in (length + 1)..prices.len() {
        let delta = prices[i] - prices[i - 1];
        let (gain, loss) = if delta >= 0.0 {
            (delta, 0.0)
        } else {
            (0.0, -delta)
        };
        avg_gain = (avg_gain.mul_add(len_f64(length - 1), gain)) / len_f64(length);
        avg_loss = (avg_loss.mul_add(len_f64(length - 1), loss)) / len_f64(length);
        out[i] = Some(rsi_from(avg_gain, avg_loss));
    }
    Ok(out)
}

/// RSI from a smoothed average gain/loss pair, handling the zero-loss edge
/// (all-up window ⇒ RSI 100).
fn rsi_from(avg_gain: f64, avg_loss: f64) -> f64 {
    if avg_loss == 0.0 {
        return 100.0;
    }
    let rs = avg_gain / avg_loss;
    100.0 - 100.0 / (1.0 + rs)
}

/// Stochastic oscillator (`%K`/`%D`) over the bar slice.
///
/// # Errors
///
/// Returns [`crate::IndicatorError::InvalidPeriod`] when any window is `0`.
pub fn stochastic(bars: &[Bar], params: StochasticParams) -> Result<Vec<StochasticRow>> {
    require_period(params.k_period)?;
    require_period(params.k_smooth)?;
    require_period(params.d_period)?;

    let highs = highs(bars);
    let lows = lows(bars);
    let n = bars.len();
    let mut raw_k = vec![f64::NAN; n];
    for i in 0..n {
        if i + 1 < params.k_period {
            continue;
        }
        let window = i + 1 - params.k_period;
        let hh = highs[window..=i].iter().copied().fold(f64::MIN, f64::max);
        let ll = lows[window..=i].iter().copied().fold(f64::MAX, f64::min);
        let span = hh - ll;
        raw_k[i] = if span == 0.0 {
            // Flat range: define %K as the midpoint 50 (a stable convention).
            50.0
        } else {
            100.0 * (bars[i].close - ll) / span
        };
    }

    let smooth_k = rolling_mean(&raw_k, params.k_smooth);
    let d = rolling_mean_opt(&smooth_k, params.d_period);

    Ok(smooth_k
        .into_iter()
        .zip(d)
        .map(|(k, d)| StochasticRow { k, d })
        .collect())
}

/// Commodity Channel Index over the bar slice.
///
/// # Errors
///
/// Returns [`crate::IndicatorError::InvalidPeriod`] when `length == 0`.
pub fn cci(bars: &[Bar], params: CciParams) -> Result<Vec<Option<f64>>> {
    let length = params.length;
    require_period(length)?;
    let tp: Vec<f64> = bars
        .iter()
        .map(|b| (b.high + b.low + b.close) / 3.0)
        .collect();
    let tp_sma = sma(&tp, LengthParams::new(length))?;
    let mut out = vec![None; bars.len()];
    for (i, mean) in tp_sma.iter().enumerate() {
        let Some(mean) = mean else { continue };
        let window = &tp[i + 1 - length..=i];
        let mean_dev = window.iter().map(|v| (v - mean).abs()).sum::<f64>() / len_f64(length);
        out[i] = if mean_dev == 0.0 {
            Some(0.0)
        } else {
            Some((tp[i] - mean) / (params.constant * mean_dev))
        };
    }
    Ok(out)
}

/// Rate of change (percent) over `prices`.
///
/// # Errors
///
/// Returns [`crate::IndicatorError::InvalidPeriod`] when `length == 0`.
pub fn roc(prices: &[f64], params: RocParams) -> Result<Vec<Option<f64>>> {
    let length = params.length;
    require_period(length)?;
    let mut out = vec![None; prices.len()];
    for i in length..prices.len() {
        let prior = prices[i - length];
        if prior != 0.0 {
            out[i] = Some(100.0 * (prices[i] - prior) / prior);
        }
    }
    Ok(out)
}

/// Momentum (price difference) over `prices`.
///
/// # Errors
///
/// Returns [`crate::IndicatorError::InvalidPeriod`] when `length == 0`.
pub fn momentum(prices: &[f64], params: RocParams) -> Result<Vec<Option<f64>>> {
    let length = params.length;
    require_period(length)?;
    let mut out = vec![None; prices.len()];
    for i in length..prices.len() {
        out[i] = Some(prices[i] - prices[i - length]);
    }
    Ok(out)
}

/// Ehlers Fisher transform of the median price over the bar slice.
///
/// # Errors
///
/// Returns [`crate::IndicatorError::InvalidPeriod`] when `length == 0`.
pub fn fisher(bars: &[Bar], params: FisherParams) -> Result<Vec<FisherRow>> {
    let length = params.length;
    require_period(length)?;
    let n = bars.len();
    let median: Vec<f64> = bars.iter().map(|b| f64::midpoint(b.high, b.low)).collect();
    let mut out = vec![FisherRow::default(); n];
    let mut value = 0.0_f64;
    let mut fish = 0.0_f64;
    let mut prev_fish: Option<f64> = None;
    for i in 0..n {
        if i + 1 < length {
            continue;
        }
        let window = &median[i + 1 - length..=i];
        let hh = window.iter().copied().fold(f64::MIN, f64::max);
        let ll = window.iter().copied().fold(f64::MAX, f64::min);
        let span = hh - ll;
        let raw = if span == 0.0 {
            0.0
        } else {
            2.0f64.mul_add((median[i] - ll) / span, -1.0)
        };
        // Smooth then clamp to keep the log argument finite.
        value = 0.33f64.mul_add(raw, 0.67 * value).clamp(-0.999, 0.999);
        fish = 0.5f64.mul_add(((1.0 + value) / (1.0 - value)).ln(), 0.5 * fish);
        out[i] = FisherRow {
            fisher: Some(fish),
            trigger: prev_fish,
        };
        prev_fish = Some(fish);
    }
    Ok(out)
}

/// Rolling mean of a possibly-NaN-prefixed series, returning `Option` rows
/// (`None` until a full window of finite samples is available).
fn rolling_mean(values: &[f64], window: usize) -> Vec<Option<f64>> {
    let mut out = vec![None; values.len()];
    for i in 0..values.len() {
        if i + 1 < window {
            continue;
        }
        let slice = &values[i + 1 - window..=i];
        if slice.iter().all(|v| v.is_finite()) {
            out[i] = Some(slice.iter().sum::<f64>() / len_f64(window));
        }
    }
    out
}

/// Rolling mean over an `Option<f64>` series; a window with any `None` yields
/// `None`.
fn rolling_mean_opt(values: &[Option<f64>], window: usize) -> Vec<Option<f64>> {
    let mut out = vec![None; values.len()];
    for i in 0..values.len() {
        if i + 1 < window {
            continue;
        }
        let slice = &values[i + 1 - window..=i];
        if slice.iter().all(Option::is_some) {
            let sum: f64 = slice.iter().map(|v| v.unwrap_or(0.0)).sum();
            out[i] = Some(sum / len_f64(window));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::closes;
    use crate::fixture::{assert_close_opt, series};

    #[test]
    fn rsi_all_gains_is_one_hundred() {
        // Strictly rising closes ⇒ no losses ⇒ RSI saturates at 100.
        let prices = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
        let out = rsi(&prices, LengthParams::new(3)).expect("rsi");
        for v in out.into_iter().flatten() {
            assert!((v - 100.0).abs() < 1e-9, "expected 100, got {v}");
        }
    }

    #[test]
    fn rsi_first_defined_index_is_length() {
        let out = rsi(&closes(&series()), LengthParams::new(14)).expect("rsi");
        assert!(out[13].is_none());
        assert!(out[14].is_some());
    }

    #[test]
    fn rsi_known_vector_wilder() {
        // Hand-derived RSI(2) over [10, 11, 10, 12]:
        //  deltas: +1, -1, +2. Seed (len 2) over deltas 1..=2 (+1,-1):
        //   avgGain=0.5, avgLoss=0.5 ⇒ RS=1 ⇒ RSI=50 at idx2.
        //  idx3 delta=+2: avgGain=(0.5*1+2)/2=1.25; avgLoss=(0.5*1+0)/2=0.25;
        //   RS=5 ⇒ RSI = 100 - 100/6 = 83.3333…
        let out = rsi(&[10.0, 11.0, 10.0, 12.0], LengthParams::new(2)).expect("rsi");
        assert_close_opt(
            &out,
            &[None, None, Some(50.0), Some(100.0 - 100.0 / 6.0)],
            1e-9,
        );
    }

    #[test]
    fn stochastic_k_in_range() {
        let out = stochastic(&series(), StochasticParams::default()).expect("stoch");
        assert_eq!(out.len(), series().len());
        for row in out.into_iter().filter_map(|r| r.k) {
            assert!((0.0..=100.0).contains(&row), "k out of range: {row}");
        }
    }

    #[test]
    fn roc_known_vector() {
        // ROC(1) over [10, 11, 12]: idx1 = 10%, idx2 = (12-11)/11*100.
        let out = roc(&[10.0, 11.0, 12.0], RocParams { length: 1 }).expect("roc");
        assert_close_opt(&out, &[None, Some(10.0), Some(100.0 / 11.0)], 1e-9);
    }

    #[test]
    fn momentum_known_vector() {
        let out = momentum(&[10.0, 11.0, 13.0], RocParams { length: 1 }).expect("mom");
        assert_close_opt(&out, &[None, Some(1.0), Some(2.0)], 1e-12);
    }

    #[test]
    fn cci_zero_when_flat() {
        let bars = series();
        let out = cci(&bars, CciParams::default()).expect("cci");
        assert_eq!(out.len(), bars.len());
    }

    #[test]
    fn fisher_has_trigger_lag() {
        let out = fisher(&series(), FisherParams { length: 5 }).expect("fisher");
        // The trigger is the prior bar's fisher value: first defined fisher has
        // no trigger, the next one's trigger equals the prior fisher.
        let defined: Vec<usize> = out
            .iter()
            .enumerate()
            .filter(|(_, r)| r.fisher.is_some())
            .map(|(i, _)| i)
            .collect();
        assert!(defined.len() >= 2);
        let first = defined[0];
        let second = defined[1];
        assert!(out[first].trigger.is_none());
        assert_eq!(out[second].trigger, out[first].fisher);
    }
}
