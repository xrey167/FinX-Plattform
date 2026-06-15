//! Exponential smoothing: Holt-Winters (additive) and an ETS model selector.
//!
//! Holt's (1957) linear-trend method maintains a level `ℓ` and a trend `b`,
//! updated each step by the smoothing coefficients `α` and `β`; Winters (1960)
//! adds an additive seasonal component `s` of period `m` smoothed by `γ`. The
//! recursions (additive seasonality) are
//!
//! ```text
//! ℓ_t = α (y_t − s_{t−m}) + (1 − α)(ℓ_{t−1} + b_{t−1})
//! b_t = β (ℓ_t − ℓ_{t−1}) + (1 − β) b_{t−1}
//! s_t = γ (y_t − ℓ_t)     + (1 − γ) s_{t−m}
//! ```
//!
//! and the `h`-step forecast is `ŷ_{t+h} = ℓ_t + h·b_t + s_{t−m+((h−1) mod m)+1}`
//! (the seasonal term is dropped when the model is non-seasonal). Initialization
//! follows the standard heuristic: the level is the mean of the first season (or
//! the first value when non-seasonal), the trend is the average of the
//! first-season-over-first-season slope, and the initial seasonal indices are the
//! first season's deviations from that mean.
//!
//! The ETS selector fits the additive variants (level-only `α`; level+trend
//! `α,β`; and, when a period is given, level+trend+seasonal `α,β,γ`) over a small
//! coefficient grid and returns the fit with the lowest in-sample one-step
//! squared error.

use crate::error::ForecastError;
use crate::params::{EtsParams, ExpoParams, ForecastPoint, ForecastResult};

/// A fitted Holt-Winters state: the final level and trend, the seasonal indices
/// (empty when non-seasonal), the seasonal period, and the in-sample one-step
/// sum of squared errors (used by the ETS selector).
#[derive(Clone, Debug)]
struct HoltWintersFit {
    level: f64,
    trend: f64,
    seasonal: Vec<f64>,
    period: usize,
    /// In-sample length. Needed to align the forecast seasonal phase: the
    /// seasonal indices are keyed by absolute time `t % period`, so projecting
    /// forward must continue from the last in-sample index `n - 1`.
    n: usize,
    sse: f64,
}

fn check_unit_interval(name: &'static str, value: f64) -> Result<(), ForecastError> {
    if (0.0..=1.0).contains(&value) {
        Ok(())
    } else {
        Err(ForecastError::ParamOutOfRange { name, value })
    }
}

#[allow(clippy::cast_precision_loss)]
fn mean(slice: &[f64]) -> f64 {
    slice.iter().sum::<f64>() / slice.len() as f64
}

/// Fit the additive Holt-Winters recursions and return the final state.
///
/// `period == 0` or `1` selects the non-seasonal (Holt linear-trend) model;
/// `period >= 2` enables the additive seasonal component.
fn fit(
    y: &[f64],
    alpha: f64,
    beta: f64,
    gamma: f64,
    period: usize,
) -> Result<HoltWintersFit, ForecastError> {
    if y.is_empty() {
        return Err(ForecastError::EmptySeries);
    }
    check_unit_interval("alpha", alpha)?;
    check_unit_interval("beta", beta)?;
    check_unit_interval("gamma", gamma)?;

    let seasonal_on = period >= 2;
    let n = y.len();
    if seasonal_on && n < 2 * period {
        // Two full seasons are needed to initialize a seasonal model honestly.
        return Err(ForecastError::TooShort {
            needed: 2 * period,
            got: n,
        });
    }

    // Initialization.
    let (mut level, mut trend, mut seasonal, start) = if seasonal_on {
        let first = &y[..period];
        let second = &y[period..2 * period];
        let l0 = mean(first);
        // Trend: average per-step change between the first two seasons.
        #[allow(clippy::cast_precision_loss)]
        let b0 = (mean(second) - l0) / period as f64;
        let s0: Vec<f64> = first.iter().map(|&v| v - l0).collect();
        (l0, b0, s0, period)
    } else {
        let l0 = y[0];
        // Seed a trend only when trend smoothing is active. With `beta == 0`
        // the trend never updates from its seed (the recursion below reduces to
        // `trend = trend`), so a non-zero seed would turn the level-only (SES)
        // variant into a permanent-drift forecast — the ETS selector's
        // `fit(y, alpha, 0.0, 0.0, 1)` candidate must stay flat.
        let b0 = if n >= 2 && beta > 0.0 {
            y[1] - y[0]
        } else {
            0.0
        };
        (l0, b0, Vec::new(), 1)
    };

    let mut sse = 0.0;
    // Roll the recursions forward, accumulating the one-step forecast error.
    for (t, &yt) in y.iter().enumerate().skip(start) {
        let season_idx = if seasonal_on { t % period } else { 0 };
        let s_prev = if seasonal_on {
            seasonal[season_idx]
        } else {
            0.0
        };
        // One-step-ahead forecast made at t−1 for t.
        let forecast = level + trend + s_prev;
        let err = yt - forecast;
        sse += err * err;

        let prev_level = level;
        let deseason = if seasonal_on { yt - s_prev } else { yt };
        level = alpha.mul_add(deseason, (1.0 - alpha) * (prev_level + trend));
        trend = beta.mul_add(level - prev_level, (1.0 - beta) * trend);
        if seasonal_on {
            seasonal[season_idx] = gamma.mul_add(yt - level, (1.0 - gamma) * s_prev);
        }
    }

    Ok(HoltWintersFit {
        level,
        trend,
        seasonal,
        period,
        n,
        sse,
    })
}

/// Project `horizon` future points from a fitted state.
fn project(fitted: &HoltWintersFit, horizon: usize, model: &str) -> ForecastResult {
    let seasonal_on = fitted.period >= 2 && !fitted.seasonal.is_empty();
    let points = (1..=horizon)
        .map(|h| {
            #[allow(clippy::cast_precision_loss)]
            let drift = h as f64 * fitted.trend;
            let seasonal = if seasonal_on {
                // Seasonal indices are phase-keyed by absolute time `t % m`.
                // Forecast step `h` targets time index `n - 1 + h`, so its phase
                // is `(n - 1 + h) % m`. Using `(h - 1) % m` only coincides with
                // this when the series length is a whole multiple of the period,
                // and otherwise shifts every forecast out of phase.
                let idx = (fitted.n - 1 + h) % fitted.period;
                fitted.seasonal[idx]
            } else {
                0.0
            };
            ForecastPoint {
                step: h,
                value: fitted.level + drift + seasonal,
            }
        })
        .collect();
    ForecastResult {
        model: model.to_string(),
        points,
    }
}

/// Holt-Winters forecast from explicit coefficients.
///
/// # Errors
///
/// [`ForecastError::ZeroHorizon`] when `horizon == 0`;
/// [`ForecastError::ParamOutOfRange`] when a coefficient is outside `[0, 1]`;
/// [`ForecastError::EmptySeries`] / [`ForecastError::TooShort`] for an
/// insufficient series.
pub fn holt_winters(
    y: &[f64],
    horizon: usize,
    alpha: f64,
    beta: f64,
    gamma: f64,
    period: usize,
) -> Result<ForecastResult, ForecastError> {
    if horizon == 0 {
        return Err(ForecastError::ZeroHorizon);
    }
    let fitted = fit(y, alpha, beta, gamma, period)?;
    Ok(project(&fitted, horizon, "expo"))
}

/// Holt-Winters route dispatch from typed params.
///
/// # Errors
///
/// Propagates [`holt_winters`].
pub fn expo_from_params(p: &ExpoParams) -> Result<ForecastResult, ForecastError> {
    holt_winters(&p.y, p.horizon, p.alpha, p.beta, p.gamma, p.period)
}

/// ETS additive model selection: fit the candidate additive variants over a
/// coefficient grid and return the forecast of the lowest-SSE fit.
///
/// The grid is `α,β,γ ∈ {0.1,…,0.9}`; the level-only and level+trend variants are
/// always considered, and the level+trend+seasonal variant is added when
/// `period >= 2` and the series spans two full seasons.
///
/// # Errors
///
/// [`ForecastError::ZeroHorizon`] when `horizon == 0`;
/// [`ForecastError::EmptySeries`] when the series is empty.
pub fn ets(y: &[f64], horizon: usize, period: usize) -> Result<ForecastResult, ForecastError> {
    if horizon == 0 {
        return Err(ForecastError::ZeroHorizon);
    }
    if y.is_empty() {
        return Err(ForecastError::EmptySeries);
    }

    let grid = [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9];
    let mut best: Option<HoltWintersFit> = None;
    let seasonal_ok = period >= 2 && y.len() >= 2 * period;

    for &alpha in &grid {
        // Level-only (no trend, no season): beta = gamma = 0, period = 1.
        consider(&mut best, fit(y, alpha, 0.0, 0.0, 1));
        for &beta in &grid {
            // Level + trend.
            consider(&mut best, fit(y, alpha, beta, 0.0, 1));
            if seasonal_ok {
                for &gamma in &grid {
                    // Level + trend + additive seasonal.
                    consider(&mut best, fit(y, alpha, beta, gamma, period));
                }
            }
        }
    }

    let fitted = best.ok_or(ForecastError::EmptySeries)?;
    Ok(project(&fitted, horizon, "ets"))
}

/// Keep the lower-SSE fit (ignoring fits that failed to initialize).
//
// Portability (G003 Gemini #446): written as a nested `if let { if … }` rather
// than a `let`-chain (`if let Ok(fit) = … && cond`). The `let`-chain only
// stabilized on recent toolchains, so the nested form keeps this portable to
// the project's MSRV; the `collapsible_if` lint (which prefers the let-chain) is
// suppressed here for that reason.
#[allow(clippy::collapsible_if)]
fn consider(best: &mut Option<HoltWintersFit>, candidate: Result<HoltWintersFit, ForecastError>) {
    if let Ok(fit) = candidate {
        if best.as_ref().is_none_or(|b| fit.sse < b.sse) {
            *best = Some(fit);
        }
    }
}

/// ETS route dispatch from typed params.
///
/// # Errors
///
/// Propagates [`ets`].
pub fn ets_from_params(p: &EtsParams) -> Result<ForecastResult, ForecastError> {
    ets(&p.y, p.horizon, p.period)
}

#[cfg(test)]
mod tests {
    use super::{ets, holt_winters};

    #[test]
    fn holt_winters_extrapolates_a_linear_trend() {
        // y = 1,2,3,...,10 is a perfect line with slope 1. A level+trend model
        // should extrapolate 11,12,13 within tolerance.
        let y: Vec<f64> = (1..=10).map(f64::from).collect();
        let fc = holt_winters(&y, 3, 0.6, 0.4, 0.0, 0).expect("forecast");
        let values: Vec<f64> = fc.points.iter().map(|p| p.value).collect();
        assert!((values[0] - 11.0).abs() < 0.5, "h1 {}", values[0]);
        assert!((values[1] - 12.0).abs() < 0.6, "h2 {}", values[1]);
        assert!((values[2] - 13.0).abs() < 0.8, "h3 {}", values[2]);
        // The trend must be increasing.
        assert!(values[1] > values[0] && values[2] > values[1]);
    }

    #[test]
    fn holt_winters_seasonal_tracks_seasonality() {
        // Level 10 with an additive season [+5,-5] repeated; slope 0.
        let y = vec![15.0, 5.0, 15.0, 5.0, 15.0, 5.0, 15.0, 5.0];
        let fc = holt_winters(&y, 2, 0.4, 0.0, 0.4, 2).expect("forecast");
        let values: Vec<f64> = fc.points.iter().map(|p| p.value).collect();
        // Next two steps continue the alternation around 10.
        assert!((values[0] - 15.0).abs() < 1.5, "h1 {}", values[0]);
        assert!((values[1] - 5.0).abs() < 1.5, "h2 {}", values[1]);
    }

    #[test]
    fn holt_winters_seasonal_phase_aligns_when_length_not_multiple_of_period() {
        // Regression: a series whose length is NOT a whole multiple of the
        // period must still forecast the correct seasonal phase. With nine
        // points (period 2), the last in-sample index (8) is a "high" (15), so
        // the next step (index 9) must be a "low" (~5) and the step after a
        // "high" (~15). A phase keyed off `(h - 1) % m` would wrongly repeat the
        // high first; the correct `(n - 1 + h) % m` keeps the phase aligned.
        let y = vec![15.0, 5.0, 15.0, 5.0, 15.0, 5.0, 15.0, 5.0, 15.0];
        let fc = holt_winters(&y, 2, 0.4, 0.0, 0.4, 2).expect("forecast");
        let values: Vec<f64> = fc.points.iter().map(|p| p.value).collect();
        assert!(
            (values[0] - 5.0).abs() < 1.5,
            "h1 {} should be ~5 (low)",
            values[0]
        );
        assert!(
            (values[1] - 15.0).abs() < 1.5,
            "h2 {} should be ~15 (high)",
            values[1]
        );
    }

    #[test]
    fn holt_winters_level_only_does_not_drift() {
        // Regression: with no trend smoothing (beta == 0) the model is SES and
        // must NOT carry a trend. Previously the trend was seeded to the first
        // difference y[1]-y[0] and frozen by beta==0, so a flat series produced
        // a spurious permanent drift. This series jumps 0->10 then stays flat;
        // the forecast must hold near the level, not ramp upward.
        let y = vec![0.0, 10.0, 10.0, 10.0, 10.0, 10.0, 10.0];
        let fc = holt_winters(&y, 3, 0.5, 0.0, 0.0, 0).expect("forecast");
        let v: Vec<f64> = fc.points.iter().map(|p| p.value).collect();
        // No drift: every horizon sits within a tight band of the first.
        assert!((v[2] - v[0]).abs() < 1.0, "level-only drifted: {v:?}");
        // Anchored near the recent level (~10), not ramping away.
        assert!(v[0] > 5.0 && v[0] < 11.0, "h1 {} not near level", v[0]);
    }

    #[test]
    fn holt_winters_rejects_out_of_range_alpha() {
        assert!(matches!(
            holt_winters(&[1.0, 2.0, 3.0], 1, 1.5, 0.1, 0.0, 0),
            Err(crate::error::ForecastError::ParamOutOfRange { name: "alpha", .. })
        ));
    }

    #[test]
    fn ets_selects_a_trended_fit_for_a_line() {
        // ETS over a clean upward line should extrapolate upward.
        let y: Vec<f64> = (1..=12).map(f64::from).collect();
        let fc = ets(&y, 2, 0).expect("forecast");
        let values: Vec<f64> = fc.points.iter().map(|p| p.value).collect();
        assert!(values[0] > 12.0, "next {} should exceed last 12", values[0]);
        assert!(values[1] > values[0]);
        assert_eq!(fc.model, "ets");
    }
}
