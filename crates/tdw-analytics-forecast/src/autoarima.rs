//! `AutoARIMA`: automatic `ARIMA(p, d, q)` order selection.
//!
//! Implements a deterministic, dependency-free variant of the
//! `Hyndman-Khandakar` (2008) stepwise algorithm:
//!
//! 1. **Choose `d`** by successive differencing: difference the series until a
//!    stationarity heuristic is satisfied (or the bound `d_max` is reached). The
//!    heuristic is a variance-ratio rule — difference while a further difference
//!    still materially reduces the series variance, which detects the
//!    non-stationary unit-root behavior of a trended/random-walk series without a
//!    heavy hypothesis-test dependency.
//! 2. **Stepwise search over `(p, q)`** at the chosen `d`: start from a small seed
//!    set of orders, fit each by `Hannan-Rissanen`, then repeatedly step to the
//!    neighbors of the current best (`p ± 1`, `q ± 1`) and keep the lowest
//!    information criterion until no neighbor improves on it. The default
//!    criterion is the corrected `AICc`.
//!
//! Orders are bounded by `p ≤ p_max`, `q ≤ q_max`, `d ≤ d_max`. Tie-breaking is
//! deterministic: among equal-criterion candidates the **simpler** model (smaller
//! `p + q`, then smaller `p`, then smaller `q`) wins, and the seed/neighbor
//! evaluation order is fixed, so the selection is reproducible from the inputs.
//!
//! The return carries the chosen order, the fitted [`ArimaFit`], the selecting
//! information criterion, and the integrated point forecast.

use std::collections::BTreeSet;

use crate::arima::{self, ArimaFit};
use crate::error::ForecastError;
use crate::params::{AutoArimaParams, ForecastResult};

/// Which information criterion drives the stepwise search.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Criterion {
    /// Akaike information criterion.
    Aic,
    /// Corrected Akaike information criterion (the default; better for small
    /// samples).
    #[default]
    Aicc,
    /// Bayesian information criterion.
    Bic,
}

impl Criterion {
    /// Read the criterion value off a fitted model.
    const fn value(self, fit: &ArimaFit) -> f64 {
        match self {
            Self::Aic => fit.aic,
            Self::Aicc => fit.aicc,
            Self::Bic => fit.bic,
        }
    }
}

/// The outcome of an `AutoARIMA` search: the chosen order, the fitted model, the
/// selecting criterion value, and the forecast.
#[derive(Clone, Debug, PartialEq)]
pub struct AutoArimaResult {
    /// The fitted model at the chosen order.
    pub fit: ArimaFit,
    /// The information-criterion value that selected this order.
    pub criterion: f64,
    /// The integrated point forecast.
    pub forecast: ForecastResult,
}

/// Population variance of a slice (denominator `n`); `0.0` for an empty slice.
fn variance(x: &[f64]) -> f64 {
    let n = x.len();
    if n == 0 {
        return 0.0;
    }
    #[allow(clippy::cast_precision_loss)]
    let nf = n as f64;
    let mean = x.iter().sum::<f64>() / nf;
    x.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / nf
}

/// One difference pass: `Δx_t = x_t − x_{t−1}`.
fn diff_once(x: &[f64]) -> Vec<f64> {
    x.windows(2).map(|w| w[1] - w[0]).collect()
}

/// Decide the differencing order `d` by successive differencing.
///
/// A non-stationary (trended / unit-root) series has a variance that keeps
/// falling sharply under differencing; a stationary one does not. We difference
/// while a further difference reduces the variance by more than a fixed fraction,
/// bounded by `d_max`. This is the variance-ratio stationarity heuristic.
fn select_d(y: &[f64], d_max: usize) -> usize {
    let mut current = y.to_vec();
    let mut d = 0;
    while d < d_max && current.len() > 3 {
        let v0 = variance(&current);
        let differenced = diff_once(&current);
        let v1 = variance(&differenced);
        // If differencing materially shrinks the variance, the series was
        // non-stationary at this order: take the difference and continue.
        // Once a difference stops helping (or starts inflating the variance, the
        // sign of over-differencing) we stop.
        if v0 > 0.0 && v1 < 0.9 * v0 {
            current = differenced;
            d += 1;
        } else {
            break;
        }
    }
    d
}

/// Try to fit `ARIMA(p, d, q)`; map any error to `None` so the search can skip an
/// infeasible order rather than abort.
fn try_fit(y: &[f64], p: usize, d: usize, q: usize) -> Option<ArimaFit> {
    arima::fit(y, p, d, q).ok()
}

/// Deterministic order comparison for tie-breaking: lower criterion wins; on a
/// near-tie the simpler model (smaller `p + q`, then `p`, then `q`) wins.
fn better(crit: Criterion, candidate: &ArimaFit, incumbent: &ArimaFit) -> bool {
    let cc = crit.value(candidate);
    let ci = crit.value(incumbent);
    if (cc - ci).abs() > 1e-9 {
        return cc < ci;
    }
    // Tie on the criterion: prefer the simpler model.
    let kc = candidate.p + candidate.q;
    let ki = incumbent.p + incumbent.q;
    if kc != ki {
        return kc < ki;
    }
    if candidate.p != incumbent.p {
        return candidate.p < incumbent.p;
    }
    candidate.q < incumbent.q
}

/// Search the best `(p, q)` order at fixed `d` by the `Hyndman-Khandakar`
/// stepwise neighborhood walk, returning the best fit found.
fn stepwise(y: &[f64], d: usize, p_max: usize, q_max: usize, crit: Criterion) -> Option<ArimaFit> {
    // Seed candidates in a fixed, deterministic order (Hyndman-Khandakar seeds).
    let seeds: &[(usize, usize)] = &[(0, 0), (1, 0), (0, 1), (1, 1), (2, 2)];
    let mut best: Option<ArimaFit> = None;
    let mut visited: BTreeSet<(usize, usize)> = BTreeSet::new();

    for &(p, q) in seeds {
        if p > p_max || q > q_max || !visited.insert((p, q)) {
            continue;
        }
        if let Some(candidate) = try_fit(y, p, d, q)
            && best.as_ref().is_none_or(|b| better(crit, &candidate, b))
        {
            best = Some(candidate);
        }
    }

    // Neighborhood walk: from the current best, try (p±1, q) and (p, q±1) until
    // no neighbor improves. Bounded iterations for determinism.
    let mut improved = true;
    let mut guard = 0;
    while improved && guard < 100 {
        improved = false;
        guard += 1;
        let Some(cur) = best.as_ref() else { break };
        let (cp, cq) = (cur.p, cur.q);
        // Fixed neighbor order: decrement before increment for stable ties. Each
        // entry is `(p, q, enabled)`; the decrements are gated so the subtraction
        // never underflows.
        let neighbors = [
            (cp.saturating_sub(1), cq, cp >= 1),
            (cp + 1, cq, cp < p_max),
            (cp, cq.saturating_sub(1), cq >= 1),
            (cp, cq + 1, cq < q_max),
        ];
        for (np, nq, ok) in neighbors {
            if !ok || !visited.insert((np, nq)) {
                continue;
            }
            if let Some(candidate) = try_fit(y, np, d, nq)
                && best.as_ref().is_some_and(|b| better(crit, &candidate, b))
            {
                best = Some(candidate);
                improved = true;
            }
        }
    }

    best
}

/// Run `AutoARIMA`: select `(p, d, q)`, fit the chosen model, and forecast.
///
/// `p_max` / `q_max` / `d_max` bound the search; `criterion` selects the
/// information criterion. Defaults from [`AutoArimaParams`] are `p_max = q_max =
/// 3`, `d_max = 2`, criterion `AICc`.
///
/// # Errors
///
/// [`ForecastError::ZeroHorizon`] when `horizon == 0`; [`ForecastError::TooShort`]
/// when no candidate order can be fit (the series is too short for even
/// `ARIMA(0, d, 0)`).
pub fn auto_arima(
    y: &[f64],
    horizon: usize,
    p_max: usize,
    q_max: usize,
    d_max: usize,
    criterion: Criterion,
) -> Result<AutoArimaResult, ForecastError> {
    if horizon == 0 {
        return Err(ForecastError::ZeroHorizon);
    }
    let d = select_d(y, d_max);
    let best = stepwise(y, d, p_max, q_max, criterion).ok_or(ForecastError::TooShort {
        needed: d + 4,
        got: y.len(),
    })?;
    let forecast = arima::forecast(&best, y, horizon)?;
    let criterion_value = criterion.value(&best);
    Ok(AutoArimaResult {
        fit: best,
        criterion: criterion_value,
        forecast,
    })
}

/// Map the serialized criterion knob to the [`Criterion`] enum.
fn criterion_from_str(s: &str) -> Criterion {
    match s.to_ascii_lowercase().as_str() {
        "aic" => Criterion::Aic,
        "bic" => Criterion::Bic,
        _ => Criterion::Aicc,
    }
}

/// `AutoARIMA` route dispatch from typed params, returning the forecast (the
/// chosen order is encoded in the [`ForecastResult`] model label).
///
/// # Errors
///
/// Propagates [`auto_arima`].
pub fn autoarima_from_params(p: &AutoArimaParams) -> Result<ForecastResult, ForecastError> {
    let result = auto_arima(
        &p.y,
        p.horizon,
        p.p_max,
        p.q_max,
        p.d_max,
        criterion_from_str(&p.criterion),
    )?;
    Ok(result.forecast)
}

#[cfg(test)]
mod tests {
    use super::{Criterion, auto_arima, select_d};

    /// Deterministic seeded pseudo-random innovation stream (`SplitMix64`), so the
    /// synthetic series are reproducible yet well-conditioned for the lag fits.
    fn noise_stream(seed: u64) -> impl FnMut() -> f64 {
        let mut state = seed;
        move || {
            state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^= z >> 31;
            #[allow(clippy::cast_precision_loss)]
            let unit = (z >> 11) as f64 / (1u64 << 53) as f64;
            2.0_f64.mul_add(unit, -1.0)
        }
    }

    /// Deterministic AR(1) generator driven by [`noise_stream`].
    fn ar1_series(n: usize, c: f64, phi: f64) -> Vec<f64> {
        let mut noise = noise_stream(0x2545_F491_4F6C_DD1D);
        let mut y = Vec::with_capacity(n);
        let mut prev = c / (1.0 - phi);
        for _ in 0..n {
            let next = phi.mul_add(prev, c) + noise();
            y.push(next);
            prev = next;
        }
        y
    }

    #[test]
    fn select_d_is_zero_for_stationary_noise() {
        // White noise around a fixed mean is already stationary: differencing only
        // inflates its variance, so d = 0.
        let mut noise = noise_stream(0xABCD_1234);
        let y: Vec<f64> = (0..80).map(|_| 5.0 + noise()).collect();
        assert_eq!(select_d(&y, 2), 0);
    }

    #[test]
    fn select_d_is_at_least_one_for_a_trend() {
        // A linear trend is a unit-root non-stationary series: d >= 1.
        let y: Vec<f64> = (0..60)
            .map(|v| 2.0_f64.mul_add(f64::from(v), 1.0))
            .collect();
        assert!(select_d(&y, 2) >= 1, "trend must pick d >= 1");
    }

    #[test]
    fn white_noise_picks_a_simple_model() {
        // Pure white noise has no autoregressive or moving-average structure, so
        // the information-criterion search should not over-fit it with a
        // high-order ARMA: p + q stays small.
        let mut noise = noise_stream(0x55AA_55AA);
        let y: Vec<f64> = (0..120).map(|_| 10.0 + noise()).collect();
        let result = auto_arima(&y, 3, 3, 3, 2, Criterion::Aicc).expect("auto_arima");
        let order = result.fit.p + result.fit.q;
        assert!(order <= 2, "white-noise order {order} should stay small");
    }

    #[test]
    fn auto_arima_forecast_length_is_correct() {
        let y = ar1_series(100, 1.0, 0.6);
        let result = auto_arima(&y, 6, 3, 3, 2, Criterion::Aicc).expect("auto_arima");
        assert_eq!(result.forecast.points.len(), 6);
        assert!(result.criterion.is_finite());
        assert!(result.forecast.model.starts_with("arima("));
    }

    #[test]
    fn auto_arima_on_a_trend_picks_differencing_and_extrapolates() {
        let y: Vec<f64> = (0..50)
            .map(|v| 3.0_f64.mul_add(f64::from(v), 2.0))
            .collect();
        let result = auto_arima(&y, 3, 3, 3, 2, Criterion::Aicc).expect("auto_arima");
        assert!(result.fit.d >= 1, "trend must difference");
        let v: Vec<f64> = result.forecast.points.iter().map(|p| p.value).collect();
        assert!(v[0] > y[y.len() - 1], "forecast should continue upward");
        assert!(v[2] > v[0]);
    }

    #[test]
    fn auto_arima_is_deterministic() {
        let y = ar1_series(90, 1.0, 0.5);
        let a = auto_arima(&y, 4, 3, 3, 2, Criterion::Aicc).expect("a");
        let b = auto_arima(&y, 4, 3, 3, 2, Criterion::Aicc).expect("b");
        assert_eq!(a.fit.p, b.fit.p);
        assert_eq!(a.fit.d, b.fit.d);
        assert_eq!(a.fit.q, b.fit.q);
        assert!((a.criterion - b.criterion).abs() < 1e-12);
    }

    #[test]
    fn zero_horizon_is_rejected() {
        let y = ar1_series(40, 1.0, 0.5);
        assert!(matches!(
            auto_arima(&y, 0, 3, 3, 2, Criterion::Aicc),
            Err(crate::error::ForecastError::ZeroHorizon)
        ));
    }
}
