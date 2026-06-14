//! `ARIMA(p, d, q)` estimated by the `Hannan-Rissanen` two-stage least-squares
//! procedure.
//!
//! An `ARIMA(p, d, q)` model differences the series `d` times to render it
//! (approximately) stationary, then fits an autoregressive-moving-average
//! `ARMA(p, q)` model to the differenced series:
//!
//! ```text
//! w_t = c + Σ_{i=1..p} φ_i · w_{t−i} + Σ_{j=1..q} θ_j · ε_{t−j} + ε_t
//! ```
//!
//! where `w_t = (1 − B)^d y_t` is the `d`-th difference. The forecast is produced
//! on the differenced scale and then **integrated** (de-differenced) back to the
//! level of the original series.
//!
//! # Estimation: `Hannan-Rissanen`
//!
//! Exact maximum likelihood for an `ARMA` model needs a `Kalman` filter and a
//! numeric optimizer, which is neither dependency-free nor deterministic in a way
//! that fits this crate's posture. Instead we use the classic two-stage
//! `Hannan-Rissanen` (1982) least-squares estimator, which reuses the OLS core
//! from `tdw-analytics-econometrics`:
//!
//! 1. **Stage 1** — fit a long autoregression `AR(m)` to `w` by OLS and keep its
//!    one-step residuals `ε̂_t` as a proxy for the unobserved innovations. The
//!    long-AR order `m` is chosen generously from the sample size.
//! 2. **Stage 2** — regress `w_t` on its own `p` lags and the `q` lagged stage-1
//!    residuals `ε̂_{t−1..t−q}` by OLS. The slope estimates are the `AR`
//!    coefficients `φ` and the `MA` coefficients `θ`; the intercept is the mean
//!    term `c`.
//!
//! This is fully deterministic (no RNG, no iterative optimizer) and reproducible
//! from the inputs. It is a consistent estimator and a strong, well-understood
//! classical approximation; it is **not** exact MLE and is documented as such.
//!
//! # Forecasting
//!
//! Multi-step forecasts on the differenced scale roll the `ARMA` recursion
//! forward: future innovations are set to their expectation (zero), past
//! innovations use the in-sample stage-2 residuals, and predicted values feed the
//! `AR` lags. The differenced forecasts are then integrated `d` times using the
//! last `d` retained level/partial-sum anchors to recover the forecast in the
//! units of the original series. An optional residual-variance band supplies
//! symmetric forecast intervals.

use tdw_analytics_econometrics::ols::{design_from_columns, ols};

use crate::error::ForecastError;
use crate::params::{ArimaParams, ForecastPoint, ForecastResult};

/// A fitted `ARIMA(p, d, q)` model: the chosen order, the estimated mean term and
/// `AR` / `MA` coefficients, and the in-sample residual standard deviation.
#[derive(Clone, Debug, PartialEq)]
pub struct ArimaFit {
    /// Autoregressive order `p`.
    pub p: usize,
    /// Differencing order `d`.
    pub d: usize,
    /// Moving-average order `q`.
    pub q: usize,
    /// The intercept / mean term `c` of the differenced `ARMA` model.
    pub intercept: f64,
    /// The `p` autoregressive coefficients `φ_1..φ_p`.
    pub ar: Vec<f64>,
    /// The `q` moving-average coefficients `θ_1..θ_q`.
    pub ma: Vec<f64>,
    /// In-sample residual standard deviation of the fitted `ARMA` model (on the
    /// differenced scale).
    pub sigma: f64,
    /// Number of effective `ARMA` observations the stage-2 fit used.
    pub n_obs: usize,
    /// Akaike information criterion of the fitted model (lower is better).
    pub aic: f64,
    /// Corrected Akaike information criterion (`AICc`).
    pub aicc: f64,
    /// Bayesian information criterion.
    pub bic: f64,
}

/// Difference the series `d` times in place, returning the differenced series and
/// the per-order "anchors" needed to integrate a forecast back to the original
/// level.
///
/// The `k`-th anchor is the last value of the `k`-th-order partial-difference
/// series; integrating a `d`-th difference forecast walks these anchors back up.
fn difference(y: &[f64], d: usize) -> (Vec<f64>, Vec<f64>) {
    let mut current = y.to_vec();
    let mut anchors = Vec::with_capacity(d);
    for _ in 0..d {
        // The anchor for this level is the last value before differencing.
        anchors.push(*current.last().unwrap_or(&0.0));
        let mut next = Vec::with_capacity(current.len().saturating_sub(1));
        for w in current.windows(2) {
            next.push(w[1] - w[0]);
        }
        current = next;
    }
    (current, anchors)
}

/// Integrate (de-difference) a `d`-th-difference forecast `diff_fc` back to the
/// original level using the `anchors` captured by [`difference`].
///
/// `anchors[k]` is the last observed value of the `k`-th partial difference. We
/// cumulatively sum from the innermost difference outward: each level's running
/// value starts at its anchor and accumulates the level below it.
fn integrate(diff_fc: &[f64], anchors: &[f64]) -> Vec<f64> {
    // Start with the d-th difference forecast and integrate outward d times.
    let mut current = diff_fc.to_vec();
    for &anchor in anchors.iter().rev() {
        let mut running = anchor;
        let mut out = Vec::with_capacity(current.len());
        for &v in &current {
            running += v;
            out.push(running);
        }
        current = out;
    }
    current
}

/// Fit a plain `AR(order)` model to `w` by OLS, returning `(intercept, coeffs,
/// residuals)`. The residuals are aligned to `w[order..]`.
fn fit_ar(w: &[f64], order: usize) -> Result<(f64, Vec<f64>, Vec<f64>), ForecastError> {
    let n = w.len();
    let rows = n - order;
    let mut response = Vec::with_capacity(rows);
    let mut lag_cols: Vec<Vec<f64>> = vec![Vec::with_capacity(rows); order];
    for t in order..n {
        response.push(w[t]);
        for (i, col) in lag_cols.iter_mut().enumerate() {
            col.push(w[t - 1 - i]);
        }
    }
    let design = design_from_columns(&lag_cols, true)
        .map_err(|e| ForecastError::Regression(e.to_string()))?;
    let fit = ols(&response, &design).map_err(|e| ForecastError::Regression(e.to_string()))?;
    let beta: Vec<f64> = fit.coefficients.iter().map(|c| c.estimate).collect();
    let intercept = beta[0];
    let coeffs: Vec<f64> = beta[1..].to_vec();
    let residuals: Vec<f64> = response
        .iter()
        .enumerate()
        .map(|(r, &actual)| {
            let mut fitted = intercept;
            for (i, c) in coeffs.iter().enumerate() {
                fitted += c * lag_cols[i][r];
            }
            actual - fitted
        })
        .collect();
    Ok((intercept, coeffs, residuals))
}

/// Stage-1 long AR with graceful order reduction, returning the one-step
/// innovation proxies aligned to the full differenced series `w` (zero-padded at
/// the front for the burn-in lags).
///
/// A degenerate or very smooth differenced series can make the long-AR design
/// rank-deficient. Rather than abort, step the order down until OLS succeeds; in
/// the limit (order `1` still singular) use the demeaned series as the innovation
/// proxy so stage 2 can always proceed. Padding always uses the **effective**
/// order that actually fit, so the returned vector is exactly `w.len()` long.
fn long_ar_innovations(w: &[f64], order: usize) -> Result<Vec<f64>, ForecastError> {
    let n = w.len();
    let mut ord = order.max(1);
    loop {
        match fit_ar(w, ord) {
            Ok((_intercept, _coeffs, eps)) => {
                let mut full = vec![0.0; n];
                for (k, &e) in eps.iter().enumerate() {
                    full[ord + k] = e;
                }
                return Ok(full);
            }
            Err(ForecastError::Regression(_)) if ord > 1 => ord -= 1,
            Err(ForecastError::Regression(_)) => {
                // Final fallback: the demeaned series as the innovation proxy.
                #[allow(clippy::cast_precision_loss)]
                let mean = w.iter().sum::<f64>() / n.max(1) as f64;
                let mut full = vec![0.0; n];
                for (t, &v) in w.iter().enumerate() {
                    full[t] = v - mean;
                }
                return Ok(full);
            }
            Err(other) => return Err(other),
        }
    }
}

/// Choose the generous stage-1 long-AR order from the differenced sample size.
///
/// `Hannan-Rissanen` wants a long AR that captures the `MA` structure; a common
/// rule is `O(log n)`-ish but bounded. We use `max(p + q, ⌈√n⌉)` bounded so the
/// OLS design always keeps ample residual degrees of freedom.
fn long_ar_order(n: usize, p: usize, q: usize) -> usize {
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_sign_loss,
        clippy::cast_possible_truncation
    )]
    let root = (n as f64).sqrt().ceil() as usize;
    let want = (p + q).max(root).max(1);
    // Keep at least n/2 residual rows for the stage-1 OLS to stay well-posed.
    want.min(n / 3).max(1)
}

/// Fit `ARIMA(p, d, q)` to `y` by the `Hannan-Rissanen` two-stage procedure.
///
/// # Errors
///
/// [`ForecastError::TooShort`] when the series is too short for the requested
/// order after differencing; [`ForecastError::Regression`] when an OLS stage
/// fails (e.g. a rank-deficient design from a degenerate series).
// `p`, `d`, `q`, `w`, `m`, `n` are the standard ARIMA / time-series symbols; the
// single-character math notation is clearer here than invented long names.
#[allow(clippy::many_single_char_names)]
pub fn fit(y: &[f64], p: usize, d: usize, q: usize) -> Result<ArimaFit, ForecastError> {
    let (w, _anchors) = difference(y, d);
    let n = w.len();
    // Need enough differenced points to estimate a long AR plus the stage-2
    // design. Require a comfortable floor.
    let min_needed = (p + q).max(1) * 2 + d + 4;
    if y.len() < min_needed {
        return Err(ForecastError::TooShort {
            needed: min_needed,
            got: y.len(),
        });
    }
    if n < (p + q).max(1) + 3 {
        return Err(ForecastError::TooShort {
            needed: (p + q).max(1) + 3,
            got: n,
        });
    }

    // Stage 1: a long AR to recover proxy innovations — only needed when there is
    // an MA part (q > 0); a pure AR(p) model needs no innovation proxy.
    let (eps_full, m) = if q > 0 {
        let m = long_ar_order(n, p, q);
        (long_ar_innovations(&w, m)?, m)
    } else {
        (vec![0.0; n], 0)
    };

    // Stage 2: regress w_t on p own-lags and q lagged stage-1 residuals.
    let start = m.max(p).max(q);
    let rows = n - start;
    if rows < p + q + 2 {
        return Err(ForecastError::TooShort {
            needed: start + p + q + 2,
            got: n,
        });
    }
    let mut response = Vec::with_capacity(rows);
    let mut cols: Vec<Vec<f64>> = vec![Vec::with_capacity(rows); p + q];
    for t in start..n {
        response.push(w[t]);
        for i in 0..p {
            cols[i].push(w[t - 1 - i]);
        }
        for j in 0..q {
            cols[p + j].push(eps_full[t - 1 - j]);
        }
    }

    let (intercept, ar, ma) = if p + q == 0 {
        // A pure ARIMA(0, d, 0): the mean of the differenced series.
        #[allow(clippy::cast_precision_loss)]
        let mean = response.iter().sum::<f64>() / response.len() as f64;
        (mean, Vec::new(), Vec::new())
    } else {
        let design = design_from_columns(&cols, true)
            .map_err(|e| ForecastError::Regression(e.to_string()))?;
        let fit = ols(&response, &design).map_err(|e| ForecastError::Regression(e.to_string()))?;
        let beta: Vec<f64> = fit.coefficients.iter().map(|c| c.estimate).collect();
        let intercept = beta[0];
        let ar: Vec<f64> = beta[1..=p].to_vec();
        let ma: Vec<f64> = beta[1 + p..].to_vec();
        (intercept, ar, ma)
    };

    // In-sample residuals of the stage-2 ARMA fit.
    let mut residuals = Vec::with_capacity(rows);
    for (r, t) in (start..n).enumerate() {
        let mut fitted = intercept;
        for (i, phi) in ar.iter().enumerate() {
            fitted += phi * w[t - 1 - i];
        }
        for (j, theta) in ma.iter().enumerate() {
            fitted += theta * eps_full[t - 1 - j];
        }
        residuals.push(response[r] - fitted);
    }
    #[allow(clippy::cast_precision_loss)]
    let n_obs_f = residuals.len() as f64;
    let rss: f64 = residuals.iter().map(|e| e * e).sum();
    let sigma2 = rss / n_obs_f;
    let sigma = sigma2.max(0.0).sqrt();

    // Information criteria from the Gaussian log-likelihood. k = p + q + 1
    // (intercept) + 1 (variance).
    #[allow(clippy::cast_precision_loss)]
    let k = (p + q + 2) as f64;
    let ln_like = if sigma2 > 0.0 {
        -0.5 * n_obs_f * ((2.0 * std::f64::consts::PI * sigma2).ln() + 1.0)
    } else {
        // A perfect fit: the log-likelihood diverges; clamp to a large finite
        // value so AIC comparisons still order models sensibly.
        n_obs_f * 50.0
    };
    let aic = 2.0_f64.mul_add(k, -2.0 * ln_like);
    let aicc = if n_obs_f - k - 1.0 > 0.0 {
        aic + (2.0 * k * (k + 1.0)) / (n_obs_f - k - 1.0)
    } else {
        f64::INFINITY
    };
    let bic = k.mul_add(n_obs_f.ln(), -2.0 * ln_like);

    Ok(ArimaFit {
        p,
        d,
        q,
        intercept,
        ar,
        ma,
        sigma,
        n_obs: residuals.len(),
        aic,
        aicc,
        bic,
    })
}

/// Produce an `h`-step point forecast from a fitted [`ArimaFit`] and the original
/// series `y`.
///
/// The `ARMA` recursion is rolled forward on the differenced scale (future
/// innovations are zero in expectation; the last in-sample residuals seed the
/// `MA` term) and then integrated `d` times back to the level of `y`.
///
/// # Errors
///
/// [`ForecastError::ZeroHorizon`] when `horizon == 0`.
// `w`, `m`, `n` are the standard differenced-series / order / length symbols.
#[allow(clippy::many_single_char_names)]
pub fn forecast(
    fit_model: &ArimaFit,
    y: &[f64],
    horizon: usize,
) -> Result<ForecastResult, ForecastError> {
    if horizon == 0 {
        return Err(ForecastError::ZeroHorizon);
    }
    let (w, anchors) = difference(y, fit_model.d);
    let n = w.len();

    // Recompute the in-sample residuals so the MA term has its recent history.
    // Reuse the stage-1 long AR to seed proxy innovations consistently.
    let m = long_ar_order(n, fit_model.p, fit_model.q);
    let eps_full = if fit_model.q > 0 {
        long_ar_innovations(&w, m)?
    } else {
        vec![0.0; n]
    };

    // Extend the differenced series and residuals forward.
    let mut ext_w = w;
    let mut ext_eps = eps_full;
    let mut diff_fc = Vec::with_capacity(horizon);
    for _ in 0..horizon {
        let cur = ext_w.len();
        let mut value = fit_model.intercept;
        for (i, phi) in fit_model.ar.iter().enumerate() {
            if cur > i {
                value += phi * ext_w[cur - 1 - i];
            }
        }
        for (j, theta) in fit_model.ma.iter().enumerate() {
            if cur > j {
                value += theta * ext_eps[cur - 1 - j];
            }
        }
        diff_fc.push(value);
        ext_w.push(value);
        // Future innovations are zero in expectation.
        ext_eps.push(0.0);
    }

    let levels = integrate(&diff_fc, &anchors);
    let points = levels
        .iter()
        .enumerate()
        .map(|(i, &value)| ForecastPoint { step: i + 1, value })
        .collect();

    Ok(ForecastResult {
        model: format!("arima({},{},{})", fit_model.p, fit_model.d, fit_model.q),
        points,
    })
}

/// Fit `ARIMA(p, d, q)` and forecast `horizon` steps in one call.
///
/// # Errors
///
/// Propagates [`fit`] and [`forecast`].
pub fn arima(
    y: &[f64],
    p: usize,
    d: usize,
    q: usize,
    horizon: usize,
) -> Result<ForecastResult, ForecastError> {
    if horizon == 0 {
        return Err(ForecastError::ZeroHorizon);
    }
    let model = fit(y, p, d, q)?;
    forecast(&model, y, horizon)
}

/// `ARIMA` route dispatch from typed params.
///
/// # Errors
///
/// Propagates [`arima`].
pub fn arima_from_params(p: &ArimaParams) -> Result<ForecastResult, ForecastError> {
    arima(&p.y, p.p, p.d, p.q, p.horizon)
}

#[cfg(test)]
mod tests {
    use super::{arima, difference, fit, forecast, integrate};

    /// A deterministic AR(1) series `y_t = c + φ·y_{t−1} + e_t` driven by a fixed,
    /// reproducible pseudo-random innovation stream from a seeded linear
    /// congruential generator (no thread RNG): the sequence is identical on every
    /// run and platform yet rich enough that the lag design is well-conditioned.
    fn ar1_series(n: usize, c: f64, phi: f64) -> Vec<f64> {
        let mut state: u64 = 0x2545_F491_4F6C_DD1D;
        let mut next_noise = move || {
            // SplitMix64-style step → a value in roughly [−1, 1].
            state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^= z >> 31;
            #[allow(clippy::cast_precision_loss)]
            let unit = (z >> 11) as f64 / (1u64 << 53) as f64; // [0, 1)
            2.0_f64.mul_add(unit, -1.0)
        };
        let mut y = Vec::with_capacity(n);
        let mut prev = c / (1.0 - phi); // start at the stationary mean
        for _ in 0..n {
            let next = phi.mul_add(prev, c) + next_noise();
            y.push(next);
            prev = next;
        }
        y
    }

    #[test]
    fn difference_then_integrate_round_trips() {
        // Differencing then integrating the SAME steps reconstructs the future
        // levels exactly. Split a clean series into a "history" and a "future":
        // difference the history, integrate the future's true differences, and
        // confirm the original future levels return — for d = 1 and d = 2.
        let full: Vec<f64> = (0..24)
            .map(|v| {
                let x = f64::from(v);
                0.5_f64.mul_add(x * x, 1.5_f64.mul_add(x, 3.0)) // a quadratic: needs d=2
            })
            .collect();
        let hist = &full[..20];
        let future = &full[20..];

        for d in 1..=2 {
            let (_w, anchors) = difference(hist, d);
            // The true d-th differences over the boundary, computed from the full
            // series so they line up with the history's anchors.
            let mut level = full.clone();
            for _ in 0..d {
                level = level.windows(2).map(|w| w[1] - w[0]).collect();
            }
            // The future d-th differences start at index (20 − d) in `level`.
            let fut_diffs: Vec<f64> = level[20 - d..].to_vec();
            let reconstructed = integrate(&fut_diffs[..future.len()], &anchors);
            for (got, want) in reconstructed.iter().zip(future.iter()) {
                assert!(
                    (got - want).abs() < 1e-6,
                    "d={d} reconstruct {got} vs {want}"
                );
            }
        }
    }

    #[test]
    fn difference_integrate_identity_on_zero_order() {
        // d = 0 is the identity: integrate(diff, []) == diff.
        let fc = [10.0, 11.0, 12.0];
        let out = integrate(&fc, &[]);
        assert_eq!(out, fc.to_vec());
    }

    #[test]
    fn ar1_recovers_a_sensible_coefficient() {
        // Strong positive AR(1): the fitted φ_1 should be positive and within a
        // reasonable band of the true 0.6.
        let y = ar1_series(120, 2.0, 0.6);
        let model = fit(&y, 1, 0, 0).expect("fit");
        assert_eq!(model.ar.len(), 1);
        let phi = model.ar[0];
        assert!(phi > 0.2 && phi < 0.95, "phi {phi} not near 0.6");
    }

    #[test]
    fn forecast_length_and_shape_are_correct() {
        let y = ar1_series(80, 1.0, 0.5);
        let model = fit(&y, 1, 0, 1).expect("fit");
        let fc = forecast(&model, &y, 5).expect("forecast");
        assert_eq!(fc.points.len(), 5);
        for (i, p) in fc.points.iter().enumerate() {
            assert_eq!(p.step, i + 1);
            assert!(p.value.is_finite(), "non-finite forecast at {i}");
        }
        assert!(fc.model.starts_with("arima(1,0,1)"));
    }

    #[test]
    fn trending_series_with_d1_extrapolates_upward() {
        // A clean linear trend differenced once is constant; ARIMA(0,1,0) (drift)
        // should continue upward.
        let y: Vec<f64> = (0..40)
            .map(|v| 2.0_f64.mul_add(f64::from(v), 5.0))
            .collect();
        let fc = arima(&y, 0, 1, 0, 3).expect("forecast");
        let v: Vec<f64> = fc.points.iter().map(|p| p.value).collect();
        assert!(v[0] > y[y.len() - 1], "h1 {} should exceed last", v[0]);
        assert!(v[1] > v[0]);
        assert!(v[2] > v[1]);
    }

    #[test]
    fn arima_010_on_a_line_recovers_the_slope() {
        // ARIMA(0,1,0) on a perfect line: the single differenced mean is the
        // slope, so each forecast advances by ~slope.
        let slope = 3.0;
        let y: Vec<f64> = (0..30).map(|v| slope * f64::from(v)).collect();
        let fc = arima(&y, 0, 1, 0, 2).expect("forecast");
        let v: Vec<f64> = fc.points.iter().map(|p| p.value).collect();
        let last = y[y.len() - 1];
        assert!((v[0] - (last + slope)).abs() < 1e-6, "h1 {}", v[0]);
        assert!(
            (v[1] - 2.0_f64.mul_add(slope, last)).abs() < 1e-6,
            "h2 {}",
            v[1]
        );
    }

    #[test]
    fn fit_rejects_too_short() {
        assert!(matches!(
            fit(&[1.0, 2.0, 3.0], 2, 1, 2),
            Err(crate::error::ForecastError::TooShort { .. })
        ));
    }

    #[test]
    fn zero_horizon_is_rejected() {
        let y = ar1_series(40, 1.0, 0.5);
        assert!(matches!(
            arima(&y, 1, 0, 0, 0),
            Err(crate::error::ForecastError::ZeroHorizon)
        ));
    }

    #[test]
    fn information_criteria_are_finite_and_ordered() {
        let y = ar1_series(100, 1.0, 0.6);
        let model = fit(&y, 1, 0, 1).expect("fit");
        assert!(model.aic.is_finite());
        assert!(model.bic.is_finite());
        // AICc >= AIC always (the correction is non-negative).
        assert!(
            model.aicc >= model.aic - 1e-9,
            "aicc {} aic {}",
            model.aicc,
            model.aic
        );
    }
}
