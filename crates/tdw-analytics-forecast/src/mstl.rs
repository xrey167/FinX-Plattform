//! Additive seasonal-trend decomposition (a simple, honest STL/MSTL variant).
//!
//! Full STL (Cleveland et al. 1990) iterates `LOESS`
//! smoothers over the seasonal and trend cycles. LOESS-STL is heavy; this module
//! instead implements the classical **moving-average seasonal decomposition**
//! (the `decompose()`/`seasonal_decompose` approach), which is the standard
//! dependency-free additive decomposition and is documented here as a deliberate
//! simplification:
//!
//! 1. **Trend** — a centered moving average of window `m` (the seasonal period).
//!    For an even period the average is a 2×m centered MA (average two adjacent
//!    m-windows) so the result is phase-aligned; for an odd period it is a plain
//!    centered m-window MA. The first/last `⌊m/2⌋` trend points (where the window
//!    overhangs the series) are linearly extrapolated from the nearest two
//!    defined interior trend points, so the component vector is full length,
//!    finite, and slope-consistent at the edges (this recovers a linear trend
//!    exactly, unlike a flat back-/forward-fill).
//! 2. **Seasonal** — detrend (`y − trend`), average the detrended values within
//!    each seasonal phase, then center the phase means to sum to zero over one
//!    period. The seasonal component repeats those centered phase means.
//! 3. **Residual** — `y − trend − seasonal`.
//!
//! The decomposition is additive: `y[t] = trend[t] + seasonal[t] + residual[t]`
//! holds exactly at every index (the linear edge-extrapolation of the trend keeps
//! the identity intact because the residual absorbs any edge approximation).

use crate::error::ForecastError;
use crate::params::{Decomposition, MstlParams};

/// Centered moving-average trend of window `period`. Interior points (where the
/// window fits) carry the average; the `⌊period/2⌋` edge points on each side are
/// filled from the nearest interior value so the vector is full-length and
/// finite.
#[allow(clippy::cast_precision_loss)]
fn centered_moving_average(y: &[f64], period: usize) -> Vec<f64> {
    let n = y.len();
    let half = period / 2;
    let mut trend = vec![0.0; n];

    // Compute the centered MA at every interior index where the window fits.
    let mut first_defined: Option<usize> = None;
    let mut last_defined = 0usize;
    for (t, slot) in trend.iter_mut().enumerate() {
        if t < half || t + half >= n {
            continue;
        }
        let value = if period.is_multiple_of(2) {
            // 2×m centered MA: half-weight the two endpoints of the (2·half+1)
            // window so an even period stays phase-aligned.
            let mut sum = 0.5 * (y[t - half] + y[t + half]);
            for v in &y[t - half + 1..t + half] {
                sum += v;
            }
            sum / period as f64
        } else {
            let sum: f64 = y[t - half..=t + half].iter().sum();
            sum / period as f64
        };
        *slot = value;
        if first_defined.is_none() {
            first_defined = Some(t);
        }
        last_defined = t;
    }

    extrapolate_edges(&mut trend, first_defined, last_defined);
    trend
}

/// Fill the undefined leading/trailing trend points by linear extrapolation from
/// the nearest two defined interior points (a per-index slope continued outward),
/// recovering a linear trend exactly. With only a single defined interior point
/// the edges are filled flat from it.
fn extrapolate_edges(trend: &mut [f64], first_defined: Option<usize>, last_defined: usize) {
    let Some(first) = first_defined else { return };

    // Leading edge: indices 0..first.
    if last_defined > first {
        let per_index = trend[first + 1] - trend[first];
        let anchor = trend[first];
        for (t, slot) in trend.iter_mut().enumerate().take(first) {
            #[allow(clippy::cast_precision_loss)]
            let steps = first as f64 - t as f64;
            *slot = steps.mul_add(-per_index, anchor);
        }
    } else {
        let anchor = trend[first];
        for slot in trend.iter_mut().take(first) {
            *slot = anchor;
        }
    }

    // Trailing edge: indices last_defined+1..n.
    if last_defined > first {
        let per_index = trend[last_defined] - trend[last_defined - 1];
        let anchor = trend[last_defined];
        for (offset, slot) in trend.iter_mut().skip(last_defined + 1).enumerate() {
            #[allow(clippy::cast_precision_loss)]
            let steps = offset as f64 + 1.0;
            *slot = steps.mul_add(per_index, anchor);
        }
    } else {
        let anchor = trend[last_defined];
        for slot in trend.iter_mut().skip(last_defined + 1) {
            *slot = anchor;
        }
    }
}

/// Additive moving-average seasonal-trend decomposition.
///
/// # Errors
///
/// [`ForecastError::InvalidPeriod`] when `period < 2`; [`ForecastError::TooShort`]
/// when the series is shorter than two full seasons (the seasonal phase means
/// would be unreliable).
pub fn decompose(y: &[f64], period: usize) -> Result<Decomposition, ForecastError> {
    if period < 2 {
        return Err(ForecastError::InvalidPeriod(period));
    }
    let n = y.len();
    if n < 2 * period {
        return Err(ForecastError::TooShort {
            needed: 2 * period,
            got: n,
        });
    }

    let trend = centered_moving_average(y, period);

    // Detrended series, averaged per seasonal phase.
    let mut phase_sum = vec![0.0; period];
    let mut phase_count = vec![0usize; period];
    for (t, (&yt, &tr)) in y.iter().zip(trend.iter()).enumerate() {
        let phase = t % period;
        phase_sum[phase] += yt - tr;
        phase_count[phase] += 1;
    }
    #[allow(clippy::cast_precision_loss)]
    let mut phase_mean: Vec<f64> = phase_sum
        .iter()
        .zip(phase_count.iter())
        .map(|(&s, &c)| if c > 0 { s / c as f64 } else { 0.0 })
        .collect();
    // Center the phase means to sum to zero over one period (additive convention).
    #[allow(clippy::cast_precision_loss)]
    let phase_offset = phase_mean.iter().sum::<f64>() / period as f64;
    for m in &mut phase_mean {
        *m -= phase_offset;
    }

    let seasonal: Vec<f64> = (0..n).map(|t| phase_mean[t % period]).collect();
    let residual: Vec<f64> = (0..n).map(|t| y[t] - trend[t] - seasonal[t]).collect();

    Ok(Decomposition {
        trend,
        seasonal,
        residual,
        period,
    })
}

/// MSTL route dispatch from typed params.
///
/// # Errors
///
/// Propagates [`decompose`].
pub fn mstl_from_params(p: &MstlParams) -> Result<Decomposition, ForecastError> {
    decompose(&p.y, p.period)
}

#[cfg(test)]
mod tests {
    use super::decompose;

    #[test]
    fn decomposition_is_additive_and_recovers_a_clean_seasonal() {
        // Construct y = trend(2t) + season[+3,-3] exactly, period 2.
        // t:0..7 trend = 0,2,4,6,8,10,12,14; season +3,-3 repeated.
        let trend: Vec<f64> = (0..8).map(|t| 2.0 * f64::from(t)).collect();
        let season = [3.0, -3.0];
        let y: Vec<f64> = trend
            .iter()
            .enumerate()
            .map(|(t, &tr)| tr + season[t % 2])
            .collect();

        let d = decompose(&y, 2).expect("decompose");
        assert_eq!(d.period, 2);
        // Additive identity holds at every index.
        for (t, &yt) in y.iter().enumerate() {
            let recon = d.trend[t] + d.seasonal[t] + d.residual[t];
            assert!(
                (recon - yt).abs() < 1e-9,
                "additive at {t}: {recon} vs {yt}"
            );
        }
        // Seasonal component sums to ~0 over one period.
        let period_sum = d.seasonal[0] + d.seasonal[1];
        assert!(period_sum.abs() < 1e-9, "seasonal centered: {period_sum}");
        // The recovered seasonal amplitude is +3/-3 (phase 0 high, phase 1 low).
        assert!(
            (d.seasonal[0] - 3.0).abs() < 1e-9,
            "season0 {}",
            d.seasonal[0]
        );
        assert!(
            (d.seasonal[1] + 3.0).abs() < 1e-9,
            "season1 {}",
            d.seasonal[1]
        );
    }

    #[test]
    fn decomposition_trend_tracks_a_pure_trend() {
        // No seasonality, slope 1: the centered MA trend should equal y in the
        // interior and the seasonal component should be ~0.
        let y: Vec<f64> = (0..10).map(f64::from).collect();
        let d = decompose(&y, 2).expect("decompose");
        // Interior trend equals the line.
        assert!((d.trend[4] - 4.0).abs() < 1e-9, "trend {}", d.trend[4]);
        // Seasonal is negligible.
        assert!(d.seasonal.iter().all(|s| s.abs() < 1e-9));
    }

    #[test]
    fn decompose_rejects_short_series() {
        assert!(matches!(
            decompose(&[1.0, 2.0, 3.0], 4),
            Err(crate::error::ForecastError::TooShort { .. })
        ));
    }
}
