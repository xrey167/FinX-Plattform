//! Returns-based quantitative metrics.
//!
//! Every function consumes a per-period returns slice (see the crate-level docs
//! for the returns-not-prices convention) plus a typed param struct, and returns
//! a single summary figure or typed row. Formula citations are in each
//! function's docs.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::params::{
    CapmParams, OmegaParams, SharpeParams, SortinoParams, VarParams, VolatilityParams,
};
use crate::stats::{excess_kurtosis, mean, percentile, skewness, std_dev};
use crate::{QuantError, Result};

#[allow(clippy::cast_precision_loss)]
const fn len_f64(n: usize) -> f64 {
    n as f64
}

/// Validate that an annualization-period factor is strictly positive.
fn require_periods(periods: f64) -> Result<()> {
    if periods > 0.0 {
        Ok(())
    } else {
        Err(QuantError::InvalidPeriods)
    }
}

/// One CAPM-regression row: the single-factor market-model alpha and beta of the
/// asset's excess returns against the benchmark's excess returns.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CapmRow {
    /// Jensen's alpha: the regression intercept (per-period abnormal return).
    pub alpha: f64,
    /// Beta: the regression slope (sensitivity to the benchmark).
    pub beta: f64,
}

/// One maximum-drawdown row: the worst peak-to-trough decline and the Calmar
/// ratio derived from it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DrawdownRow {
    /// Maximum drawdown as a negative fraction of the running peak (e.g. `-0.25`
    /// for a 25% peak-to-trough loss); `0.0` for a never-declining series.
    pub max_drawdown: f64,
    /// Calmar ratio: annualized return divided by the absolute maximum drawdown;
    /// `0.0` when there is no drawdown (the ratio is otherwise undefined).
    pub calmar_ratio: f64,
}

/// One Jarque-Bera normality row: the test statistic and its χ²₂ p-value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct JarqueBeraRow {
    /// The Jarque-Bera statistic `JB = (n/6)·(S² + K²/4)` (`S` skewness, `K`
    /// excess kurtosis); larger values indicate stronger departure from
    /// normality.
    pub statistic: f64,
    /// Upper-tail p-value under the χ²₂ null (`p = exp(−JB/2)`); small values
    /// reject normality.
    pub p_value: f64,
}

/// Sharpe ratio (Sharpe 1966): mean excess return over its standard deviation,
/// annualized by `√periods`.
///
/// `Sharpe = √periods · (mean(r) − rf) / stdev(r)`. The standard deviation is
/// the unbiased sample estimator over the *raw* returns (a constant `rf` shift
/// does not change dispersion). Zero dispersion ⇒ `0.0`.
///
/// # Errors
///
/// Returns [`QuantError::InvalidPeriods`] when `periods <= 0`.
pub fn sharpe_ratio(returns: &[f64], params: SharpeParams) -> Result<f64> {
    require_periods(params.periods)?;
    let sigma = std_dev(returns);
    if sigma == 0.0 {
        return Ok(0.0);
    }
    let excess = mean(returns) - params.risk_free_rate;
    Ok(params.periods.sqrt() * excess / sigma)
}

/// Sortino ratio (Sortino & Price 1994): mean excess return over the downside
/// deviation below `target_return`, annualized by `√periods`.
///
/// The downside deviation is `√( (1/n) Σ min(r_i − target, 0)² )` — the
/// root-mean-square of below-target shortfalls over the full sample count `n`
/// (the standard Sortino denominator). Zero downside deviation ⇒ `0.0`.
///
/// # Errors
///
/// Returns [`QuantError::InvalidPeriods`] when `periods <= 0`.
pub fn sortino_ratio(returns: &[f64], params: SortinoParams) -> Result<f64> {
    require_periods(params.periods)?;
    if returns.is_empty() {
        return Ok(0.0);
    }
    let downside_sq: f64 = returns
        .iter()
        .map(|r| {
            let shortfall = (r - params.target_return).min(0.0);
            shortfall * shortfall
        })
        .sum();
    let downside_dev = (downside_sq / len_f64(returns.len())).sqrt();
    if downside_dev == 0.0 {
        return Ok(0.0);
    }
    let excess = mean(returns) - params.risk_free_rate;
    Ok(params.periods.sqrt() * excess / downside_dev)
}

/// Omega ratio (Keating & Shadwick 2002): the ratio of probability-weighted
/// gains above `threshold` to probability-weighted losses below it.
///
/// For a discrete return sample this reduces to
/// `Σ max(r_i − threshold, 0) / Σ max(threshold − r_i, 0)`. When there are no
/// losses below the threshold the ratio is unbounded; this is reported as `0.0`
/// (the documented sentinel for an undefined denominator), so callers must read
/// it together with the gains mass if they need the unbounded case.
#[must_use]
pub fn omega_ratio(returns: &[f64], params: OmegaParams) -> f64 {
    let mut gains = 0.0;
    let mut losses = 0.0;
    for &r in returns {
        let diff = r - params.threshold;
        if diff > 0.0 {
            gains += diff;
        } else {
            losses -= diff;
        }
    }
    if losses == 0.0 {
        return 0.0;
    }
    gains / losses
}

/// Annualized volatility: the unbiased sample standard deviation of returns
/// scaled by `√periods`.
///
/// # Errors
///
/// Returns [`QuantError::InvalidPeriods`] when `periods <= 0`.
pub fn volatility(returns: &[f64], params: VolatilityParams) -> Result<f64> {
    require_periods(params.periods)?;
    Ok(params.periods.sqrt() * std_dev(returns))
}

/// Sample skewness of the returns (adjusted Fisher-Pearson; see
/// [`crate::stats::skewness`]).
#[must_use]
pub fn skew(returns: &[f64]) -> f64 {
    skewness(returns)
}

/// Sample *excess* kurtosis of the returns (normal ⇒ ≈ 0; see
/// [`crate::stats::excess_kurtosis`]).
#[must_use]
pub fn kurtosis(returns: &[f64]) -> f64 {
    excess_kurtosis(returns)
}

/// Historical Value-at-Risk: the loss at the `1 − confidence` left-tail
/// quantile of the return distribution, reported as a negative number (a loss).
///
/// With historical simulation, `VaR` is the empirical quantile of returns at
/// level `1 − confidence` (e.g. the 5% quantile for 95% confidence). Returns
/// `0.0` for an empty series.
#[must_use]
pub fn value_at_risk(returns: &[f64], params: VarParams) -> f64 {
    let alpha = 1.0 - params.confidence;
    percentile(returns, alpha).unwrap_or(0.0)
}

/// Conditional `VaR` / expected shortfall: the mean of the returns at or below
/// the historical `VaR` cutoff (the average loss in the tail beyond `VaR`).
///
/// Returns `0.0` for an empty series. Like [`value_at_risk`] the figure is in
/// return space (a loss is negative).
#[must_use]
pub fn expected_shortfall(returns: &[f64], params: VarParams) -> f64 {
    if returns.is_empty() {
        return 0.0;
    }
    let cutoff = value_at_risk(returns, params);
    let tail: Vec<f64> = returns.iter().copied().filter(|r| *r <= cutoff).collect();
    if tail.is_empty() {
        return cutoff;
    }
    mean(&tail)
}

/// Maximum drawdown and the Calmar ratio over a returns series.
///
/// The returns are compounded into a wealth index `W_t = Π (1 + r_i)`; the
/// drawdown at each step is `W_t / peak_t − 1` against the running peak, and the
/// maximum drawdown is the most negative of these (Young 1991 for the Calmar
/// definition). The Calmar ratio divides the *annualized* mean return
/// (`mean(r) · periods`, with `periods` taken as the sample length so the figure
/// is per-sample-horizon when the caller has no frequency) — here we use the
/// simple per-period mean return over the absolute max drawdown, which is the
/// `OpenBB` `calmar` convention for a raw returns column. No drawdown ⇒ Calmar
/// `0.0` (undefined ratio).
#[must_use]
pub fn max_drawdown(returns: &[f64]) -> DrawdownRow {
    if returns.is_empty() {
        return DrawdownRow::default();
    }
    let mut wealth = 1.0;
    let mut peak = 1.0;
    let mut max_dd = 0.0;
    for &r in returns {
        wealth *= 1.0 + r;
        if wealth > peak {
            peak = wealth;
        }
        if peak > 0.0 {
            let dd = wealth / peak - 1.0;
            if dd < max_dd {
                max_dd = dd;
            }
        }
    }
    let calmar = if max_dd < 0.0 {
        mean(returns) / max_dd.abs()
    } else {
        0.0
    };
    DrawdownRow {
        max_drawdown: max_dd,
        calmar_ratio: calmar,
    }
}

/// CAPM single-factor market-model regression of the asset's excess returns on
/// the benchmark's excess returns.
///
/// Fits `r_asset − rf = α + β·(r_bench − rf)` by ordinary least squares:
/// `β = cov(asset_excess, bench_excess) / var(bench_excess)` and
/// `α = mean(asset_excess) − β·mean(bench_excess)`. A benchmark with zero
/// variance yields `β = 0` and `α = mean(asset_excess)`.
///
/// # Errors
///
/// Returns [`QuantError::LengthMismatch`] when `returns` and `benchmark` differ
/// in length (the two series must be positionally aligned).
pub fn capm(returns: &[f64], benchmark: &[f64], params: CapmParams) -> Result<CapmRow> {
    if returns.len() != benchmark.len() {
        return Err(QuantError::LengthMismatch {
            asset: returns.len(),
            benchmark: benchmark.len(),
        });
    }
    if returns.is_empty() {
        return Ok(CapmRow::default());
    }
    let asset_excess: Vec<f64> = returns.iter().map(|r| r - params.risk_free_rate).collect();
    let bench_excess: Vec<f64> = benchmark
        .iter()
        .map(|r| r - params.risk_free_rate)
        .collect();
    let mean_asset = mean(&asset_excess);
    let mean_bench = mean(&bench_excess);

    let mut cov = 0.0;
    let mut var_bench = 0.0;
    for (a, b) in asset_excess.iter().zip(bench_excess.iter()) {
        cov += (a - mean_asset) * (b - mean_bench);
        var_bench += (b - mean_bench).powi(2);
    }
    let beta = if var_bench == 0.0 {
        0.0
    } else {
        cov / var_bench
    };
    let alpha = mean_asset - beta * mean_bench;
    Ok(CapmRow { alpha, beta })
}

/// Jarque-Bera normality statistic and its χ²₂ p-value (Jarque & Bera 1980).
///
/// `JB = (n/6)·(S² + K²/4)` where `S` is the sample skewness and `K` the sample
/// excess kurtosis. Under the null of normality `JB` is asymptotically χ² with
/// two degrees of freedom; the survival function of the χ²₂ distribution has the
/// closed form `P(X > x) = exp(−x/2)`, so the p-value is `exp(−JB/2)` exactly
/// (no table or series approximation needed). Fewer than four points ⇒ a zero
/// statistic and a p-value of `1.0` (insufficient data to reject normality).
#[must_use]
pub fn jarque_bera(returns: &[f64]) -> JarqueBeraRow {
    if returns.len() < 4 {
        return JarqueBeraRow {
            statistic: 0.0,
            p_value: 1.0,
        };
    }
    let s = skewness(returns);
    let k = excess_kurtosis(returns);
    let n = len_f64(returns.len());
    let statistic = (n / 6.0) * k.mul_add(k / 4.0, s * s);
    let p_value = (-statistic / 2.0).exp();
    JarqueBeraRow { statistic, p_value }
}

#[cfg(test)]
mod tests {
    use super::{
        capm, expected_shortfall, jarque_bera, max_drawdown, omega_ratio, sharpe_ratio, skew,
        sortino_ratio, value_at_risk, volatility,
    };
    use crate::params::{
        CapmParams, OmegaParams, SharpeParams, SortinoParams, VarParams, VolatilityParams,
    };

    // Golden series: four returns whose moments are hand-checkable.
    // r = [0.10, -0.05, 0.10, -0.05]
    //   mean   = 0.025
    //   sample variance (n-1): deviations [0.075, -0.075, 0.075, -0.075];
    //     squared = 4 * 0.005625 = 0.0225; /3 = 0.0075; std = 0.0866025...
    fn golden() -> Vec<f64> {
        vec![0.10, -0.05, 0.10, -0.05]
    }

    #[test]
    fn sharpe_unannualized_matches_hand_value() {
        // periods=1, rf=0 ⇒ Sharpe = mean/std = 0.025 / 0.0866025403 = 0.288675...
        let s = sharpe_ratio(&golden(), SharpeParams::default()).expect("sharpe");
        assert!((s - 0.288_675_134_594_8).abs() < 1e-9, "got {s}");
    }

    #[test]
    fn sharpe_annualization_scales_by_sqrt_periods() {
        let base = sharpe_ratio(&golden(), SharpeParams::default()).expect("base");
        let ann = sharpe_ratio(
            &golden(),
            SharpeParams {
                risk_free_rate: 0.0,
                periods: 4.0,
            },
        )
        .expect("ann");
        // √4 = 2.
        let expected = 2.0 * base;
        assert!((ann - expected).abs() < 1e-9, "got {ann}");
    }

    #[test]
    fn sharpe_zero_dispersion_is_zero() {
        let s = sharpe_ratio(&[0.01, 0.01, 0.01], SharpeParams::default()).expect("sharpe");
        assert!(s.abs() < 1e-12);
    }

    #[test]
    fn sharpe_invalid_periods_errors() {
        assert!(
            sharpe_ratio(
                &golden(),
                SharpeParams {
                    risk_free_rate: 0.0,
                    periods: 0.0,
                },
            )
            .is_err()
        );
    }

    #[test]
    fn sortino_uses_downside_only_denominator() {
        // target=0: shortfalls are the two -0.05 returns; downside-sq =
        // 2*0.0025 = 0.005; /n(4) = 0.00125; sqrt = 0.0353553391.
        // Sortino = mean(0.025) / 0.0353553391 = 0.707106781 = 1/√2.
        let s = sortino_ratio(&golden(), SortinoParams::default()).expect("sortino");
        let expected = std::f64::consts::FRAC_1_SQRT_2;
        assert!((s - expected).abs() < 1e-9, "got {s}");
    }

    #[test]
    fn omega_ratio_matches_gain_loss_mass() {
        // threshold 0: gains = 0.10+0.10 = 0.20; losses = 0.05+0.05 = 0.10;
        // omega = 2.0.
        let o = omega_ratio(&golden(), OmegaParams::default());
        assert!((o - 2.0).abs() < 1e-12, "got {o}");
    }

    #[test]
    fn volatility_unannualized_is_sample_std() {
        let v = volatility(&golden(), VolatilityParams::default()).expect("vol");
        assert!((v - 0.086_602_540_378_4).abs() < 1e-9, "got {v}");
    }

    #[test]
    fn value_at_risk_is_left_tail_quantile() {
        // 95% confidence ⇒ 5% quantile. Sorted [-0.05,-0.05,0.10,0.10];
        // rank = 0.05*(3) = 0.15 between idx0(-0.05) and idx1(-0.05) ⇒ -0.05.
        let var = value_at_risk(&golden(), VarParams::default());
        assert!((var - -0.05).abs() < 1e-12, "got {var}");
    }

    #[test]
    fn expected_shortfall_averages_the_tail() {
        // cutoff -0.05 ⇒ tail = the two -0.05s ⇒ mean -0.05.
        let es = expected_shortfall(&golden(), VarParams::default());
        assert!((es - -0.05).abs() < 1e-12, "got {es}");
    }

    #[test]
    fn capm_beta_one_alpha_zero_when_asset_equals_benchmark() {
        // asset == benchmark ⇒ perfect fit: beta 1, alpha 0.
        let row = capm(&golden(), &golden(), CapmParams::default()).expect("capm");
        assert!((row.beta - 1.0).abs() < 1e-12, "beta {}", row.beta);
        assert!(row.alpha.abs() < 1e-12, "alpha {}", row.alpha);
    }

    #[test]
    fn capm_beta_two_when_asset_is_double_benchmark() {
        let bench = golden();
        let asset: Vec<f64> = bench.iter().map(|r| 2.0 * r).collect();
        let row = capm(&asset, &bench, CapmParams::default()).expect("capm");
        assert!((row.beta - 2.0).abs() < 1e-9, "beta {}", row.beta);
        assert!(row.alpha.abs() < 1e-9, "alpha {}", row.alpha);
    }

    #[test]
    fn capm_length_mismatch_errors() {
        assert!(capm(&[0.1, 0.2], &[0.1], CapmParams::default()).is_err());
    }

    #[test]
    fn max_drawdown_matches_compounded_path() {
        // [0.10,-0.05,0.10,-0.05]: wealth path
        //   1.10, 1.045, 1.1495, 1.092025; peak tracks 1.10 then 1.1495.
        //   drawdowns: at idx1 1.045/1.10-1 = -0.05; at idx3 1.092025/1.1495-1
        //   = -0.05. Max drawdown = -0.05.
        let row = max_drawdown(&golden());
        assert!(
            (row.max_drawdown - -0.05).abs() < 1e-9,
            "dd {}",
            row.max_drawdown
        );
        // calmar = mean(0.025)/0.05 = 0.5.
        assert!(
            (row.calmar_ratio - 0.5).abs() < 1e-9,
            "calmar {}",
            row.calmar_ratio
        );
    }

    #[test]
    fn max_drawdown_monotone_up_has_no_drawdown() {
        let row = max_drawdown(&[0.10, 0.10, 0.10]);
        assert!(row.max_drawdown.abs() < 1e-12);
        assert!(row.calmar_ratio.abs() < 1e-12);
    }

    #[test]
    fn skew_of_symmetric_returns_is_zero() {
        assert!(skew(&golden()).abs() < 1e-12);
    }

    #[test]
    fn jarque_bera_small_sample_is_inconclusive() {
        // n<4 ⇒ statistic 0, p-value 1 (cannot reject normality).
        let row = jarque_bera(&[0.1, 0.2, 0.3]);
        assert!(row.statistic.abs() < 1e-12);
        assert!((row.p_value - 1.0).abs() < 1e-12);
    }

    #[test]
    fn jarque_bera_p_value_is_chi2_survival() {
        // For the golden symmetric series, skew = 0, so JB = (n/6)*(K^2/4).
        // We only assert the closed-form link p = exp(-JB/2) holds.
        let row = jarque_bera(&golden());
        let expected_p = (-row.statistic / 2.0).exp();
        assert!((row.p_value - expected_p).abs() < 1e-12);
        assert!((0.0..=1.0).contains(&row.p_value));
    }
}
