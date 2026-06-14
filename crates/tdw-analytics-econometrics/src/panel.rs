//! Panel (longitudinal) regression estimators over a long-format panel.
//!
//! Every estimator consumes the same long-format triple — an `entity` id, a
//! response `y`, and a regressor row `x` per observation — and routes its final
//! coefficient solve through the crate's single OLS solver (see [`crate::ols`]),
//! reusing its standard errors, `R²`, and Durbin-Watson machinery. The
//! estimators differ only in the transformation applied to `y` and `X` before
//! that solve:
//!
//! - [`pooled_ols`] — plain OLS ignoring the panel structure (the naive
//!   benchmark).
//! - [`fixed_effects`] — the one-way *within* estimator: subtract each entity's
//!   mean from `y` and every regressor, then OLS through the origin (the entity
//!   intercepts are swept out by the demeaning).
//! - [`between`] — OLS on the entity-level means (one row per entity).
//! - [`first_difference`] — OLS on within-entity first differences `Δyₜ`, `Δxₜ`.
//! - [`random_effects`] — a Swamy-Arora feasible-GLS quasi-demeaning: each entity
//!   group is partially demeaned by `θ`, where `θ = 1 − √(σ²_e / (σ²_e + Tᵢ·σ²_u))`
//!   blends the within and between variance components.
//! - [`fama_macbeth`] — one cross-sectional OLS per time period, then the simple
//!   average of the period coefficient vectors (with the cross-period standard
//!   error of each average).
//!
//! # Clean-room provenance
//!
//! The within / between / first-difference / quasi-demeaning transforms and the
//! Fama-MacBeth two-pass average are textbook panel econometrics (Wooldridge,
//! *Econometric Analysis of Cross Section and Panel Data*; Baltagi, *Econometric
//! Analysis of Panel Data*; Fama & `MacBeth` 1973). No reference implementation
//! was consulted.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::EconometricsError;
use crate::ols::{Coefficient, OlsSummary, design_from_columns, ols};

#[allow(clippy::cast_precision_loss)]
const fn usize_to_f64(n: usize) -> f64 {
    n as f64
}

/// A panel-regression result: the estimated coefficients (in regressor order,
/// intercept first when the estimator fits one) plus the model diagnostics and
/// the estimator's effective sample size.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PanelSummary {
    /// Estimated coefficients in design-column order.
    pub coefficients: Vec<Coefficient>,
    /// Coefficient of determination of the transformed regression.
    pub r_squared: f64,
    /// Number of entities (cross-section units) in the panel.
    pub n_entities: usize,
    /// Number of observations the final regression was fit over.
    pub n_obs: usize,
}

impl PanelSummary {
    fn from_ols(fit: &OlsSummary, n_entities: usize, n_obs: usize) -> Self {
        Self {
            coefficients: fit.coefficients.clone(),
            r_squared: fit.r_squared,
            n_entities,
            n_obs,
        }
    }
}

/// Validate that the long-format inputs are positionally aligned and non-empty,
/// returning the regressor width `k`.
fn validate(entity: &[i64], y: &[f64], x: &[Vec<f64>]) -> Result<usize, EconometricsError> {
    let n = y.len();
    if n == 0 || entity.len() != n || x.len() != n {
        return Err(EconometricsError::RowMismatch {
            y: n,
            x: x.len().max(entity.len()),
        });
    }
    let k = x.first().map_or(0, Vec::len);
    if k == 0 || x.iter().any(|row| row.len() != k) {
        return Err(EconometricsError::EmptyDesign);
    }
    Ok(k)
}

/// Transpose a row-major regressor list `x[obs][k]` into `k` column vectors.
fn to_columns(x: &[Vec<f64>], k: usize) -> Vec<Vec<f64>> {
    (0..k)
        .map(|j| x.iter().map(|row| row[j]).collect())
        .collect()
}

/// Count distinct entity ids (preserving nothing but the count).
fn entity_count(entity: &[i64]) -> usize {
    entity
        .iter()
        .copied()
        .collect::<std::collections::BTreeSet<_>>()
        .len()
}

/// Pooled OLS: ignore the panel structure and fit `y = α + Xβ + ε` directly.
///
/// # Errors
///
/// Propagates [`EconometricsError`] from input validation or the OLS solve.
pub fn pooled_ols(
    entity: &[i64],
    y: &[f64],
    x: &[Vec<f64>],
) -> Result<PanelSummary, EconometricsError> {
    let k = validate(entity, y, x)?;
    let columns = to_columns(x, k);
    let design = design_from_columns(&columns, true)?;
    let fit = ols(y, &design)?;
    Ok(PanelSummary::from_ols(&fit, entity_count(entity), y.len()))
}

/// Group the observation indices by entity id, preserving input order within
/// each group.
fn group_indices(entity: &[i64]) -> BTreeMap<i64, Vec<usize>> {
    let mut groups: BTreeMap<i64, Vec<usize>> = BTreeMap::new();
    for (i, &e) in entity.iter().enumerate() {
        groups.entry(e).or_default().push(i);
    }
    groups
}

/// One-way fixed-effects (within) estimator: demean `y` and every regressor
/// within each entity, then OLS through the origin.
///
/// # Errors
///
/// Propagates [`EconometricsError`] from input validation or the OLS solve
/// (a within design with no remaining variation is [`EconometricsError::Singular`]).
pub fn fixed_effects(
    entity: &[i64],
    y: &[f64],
    x: &[Vec<f64>],
) -> Result<PanelSummary, EconometricsError> {
    let k = validate(entity, y, x)?;
    let columns = to_columns(x, k);
    let groups = group_indices(entity);

    let mut y_within = y.to_vec();
    let mut cols_within = columns;
    for indices in groups.values() {
        demean_in_place(&mut y_within, indices);
        for col in &mut cols_within {
            demean_in_place(col, indices);
        }
    }
    // No intercept: the entity means (and the grand mean) are removed already.
    let design = design_from_columns(&cols_within, false)?;
    let fit = ols(&y_within, &design)?;
    Ok(PanelSummary::from_ols(&fit, groups.len(), y.len()))
}

/// Subtract the mean of the entries at `indices` from each of them, in place.
fn demean_in_place(values: &mut [f64], indices: &[usize]) {
    if indices.is_empty() {
        return;
    }
    let mean: f64 = indices.iter().map(|&i| values[i]).sum::<f64>() / usize_to_f64(indices.len());
    for &i in indices {
        values[i] -= mean;
    }
}

/// Between estimator: OLS on the entity-level means (one row per entity).
///
/// # Errors
///
/// Propagates [`EconometricsError`]; needs more entities than regressors+1.
pub fn between(
    entity: &[i64],
    y: &[f64],
    x: &[Vec<f64>],
) -> Result<PanelSummary, EconometricsError> {
    let k = validate(entity, y, x)?;
    let columns = to_columns(x, k);
    let groups = group_indices(entity);

    let mut y_means = Vec::with_capacity(groups.len());
    let mut col_means: Vec<Vec<f64>> = vec![Vec::with_capacity(groups.len()); k];
    for indices in groups.values() {
        y_means.push(group_mean(y, indices));
        for j in 0..k {
            col_means[j].push(group_mean(&columns[j], indices));
        }
    }
    let design = design_from_columns(&col_means, true)?;
    let fit = ols(&y_means, &design)?;
    Ok(PanelSummary::from_ols(&fit, groups.len(), y_means.len()))
}

/// Mean of the entries at `indices`.
fn group_mean(values: &[f64], indices: &[usize]) -> f64 {
    if indices.is_empty() {
        return 0.0;
    }
    indices.iter().map(|&i| values[i]).sum::<f64>() / usize_to_f64(indices.len())
}

/// First-difference estimator: within each entity, difference consecutive
/// observations (in input order) and OLS the differences through the origin.
///
/// # Errors
///
/// Propagates [`EconometricsError`]; an entity must contribute at least one
/// difference (two observations) for the regression to have rows.
pub fn first_difference(
    entity: &[i64],
    y: &[f64],
    x: &[Vec<f64>],
) -> Result<PanelSummary, EconometricsError> {
    let k = validate(entity, y, x)?;
    let columns = to_columns(x, k);
    let groups = group_indices(entity);

    let mut dy: Vec<f64> = Vec::new();
    let mut dx: Vec<Vec<f64>> = vec![Vec::new(); k];
    for indices in groups.values() {
        for w in indices.windows(2) {
            let (prev, cur) = (w[0], w[1]);
            dy.push(y[cur] - y[prev]);
            for j in 0..k {
                dx[j].push(columns[j][cur] - columns[j][prev]);
            }
        }
    }
    if dy.is_empty() {
        return Err(EconometricsError::InsufficientRows { rows: 0, cols: k });
    }
    let design = design_from_columns(&dx, false)?;
    let fit = ols(&dy, &design)?;
    Ok(PanelSummary::from_ols(&fit, groups.len(), dy.len()))
}

/// Random-effects (Swamy-Arora feasible GLS) estimator.
///
/// Estimates the idiosyncratic variance `σ²_e` from the within (fixed-effects)
/// residuals and the between variance `σ²_b` from the between regression, recovers
/// the entity-effect variance `σ²_u = max(σ²_b − σ²_e / T̄, 0)`, and forms the
/// per-entity quasi-demeaning weight `θᵢ = 1 − √(σ²_e / (σ²_e + Tᵢ·σ²_u))`. The
/// transformed data `yᵢₜ − θᵢ·ȳᵢ` (and likewise for `X`, including the intercept)
/// is then fit by OLS.
///
/// # Errors
///
/// Propagates [`EconometricsError`] from any of the component regressions.
pub fn random_effects(
    entity: &[i64],
    y: &[f64],
    x: &[Vec<f64>],
) -> Result<PanelSummary, EconometricsError> {
    let k = validate(entity, y, x)?;
    let columns = to_columns(x, k);
    let groups = group_indices(entity);
    let n = y.len();
    let n_entities = groups.len();

    // Idiosyncratic variance σ²_e from the within (fixed-effects) regression:
    // σ²_e = RSS_within / (N − n_entities − k).
    let sigma2_e = within_residual_variance(y, &columns, &groups, k)?;

    // Between variance σ²_1 = σ²_e + T̄·σ²_u from the between regression on the
    // entity means: σ²_1 = RSS_between / (n_entities − k − 1).
    let sigma2_between = between_residual_variance(y, &columns, &groups, k)?;

    // σ²_u = (σ²_1 − σ²_e) / T̄, floored at zero (the variance cannot be negative).
    let t_bar = usize_to_f64(n) / usize_to_f64(n_entities);
    let sigma2_u = ((sigma2_between - sigma2_e) / t_bar).max(0.0);

    // Quasi-demean each entity group by θᵢ and fit OLS with an intercept.
    let mut y_star = y.to_vec();
    let mut cols_star = columns;
    // The intercept column is quasi-demeaned too: a constant 1 becomes 1 − θ.
    let mut const_star = vec![1.0_f64; n];
    for indices in groups.values() {
        let ti = usize_to_f64(indices.len());
        let denom = ti.mul_add(sigma2_u, sigma2_e);
        let theta = if denom > 0.0 {
            1.0 - (sigma2_e / denom).sqrt()
        } else {
            0.0
        };
        quasi_demean(&mut y_star, indices, theta);
        for col in &mut cols_star {
            quasi_demean(col, indices, theta);
        }
        for &i in indices {
            const_star[i] = 1.0 - theta;
        }
    }
    // Prepend the quasi-demeaned constant as the first regressor (no auto
    // intercept, since the constant is transformed and can no longer be all-ones).
    let mut design_cols = Vec::with_capacity(k + 1);
    design_cols.push(const_star);
    design_cols.extend(cols_star);
    let design = design_from_columns(&design_cols, false)?;
    let fit = ols(&y_star, &design)?;
    Ok(PanelSummary::from_ols(&fit, n_entities, n))
}

/// Quasi-demean: subtract `theta` times the group mean from each group entry.
fn quasi_demean(values: &mut [f64], indices: &[usize], theta: f64) {
    let mean = group_mean(values, indices);
    for &i in indices {
        values[i] -= theta * mean;
    }
}

/// Idiosyncratic variance `σ²_e = RSS_within / (N − n_entities − k)` from the
/// within (demeaned, intercept-free) regression. The dof subtracts the
/// `n_entities` swept entity means and the `k` slopes.
fn within_residual_variance(
    y: &[f64],
    columns: &[Vec<f64>],
    groups: &BTreeMap<i64, Vec<usize>>,
    k: usize,
) -> Result<f64, EconometricsError> {
    let mut y_within = y.to_vec();
    let mut cols_within = columns.to_vec();
    for indices in groups.values() {
        demean_in_place(&mut y_within, indices);
        for col in &mut cols_within {
            demean_in_place(col, indices);
        }
    }
    let design = design_from_columns(&cols_within, false)?;
    let fit = ols(&y_within, &design)?;
    let beta: Vec<f64> = fit.coefficients.iter().map(|c| c.estimate).collect();
    let rss = rss_of(&y_within, &cols_within, &beta, false);
    let n = y.len();
    let dof = n.saturating_sub(groups.len() + k).max(1);
    Ok(rss / usize_to_f64(dof))
}

/// Between variance `σ²_1 = RSS_between / (n_entities − k − 1)` from the OLS on
/// the entity means (intercept-bearing). This estimates `σ²_e + T̄·σ²_u`.
fn between_residual_variance(
    y: &[f64],
    columns: &[Vec<f64>],
    groups: &BTreeMap<i64, Vec<usize>>,
    k: usize,
) -> Result<f64, EconometricsError> {
    let mut y_means = Vec::with_capacity(groups.len());
    let mut col_means: Vec<Vec<f64>> = vec![Vec::with_capacity(groups.len()); k];
    for indices in groups.values() {
        y_means.push(group_mean(y, indices));
        for (j, col_mean) in col_means.iter_mut().enumerate() {
            col_mean.push(group_mean(&columns[j], indices));
        }
    }
    let design = design_from_columns(&col_means, true)?;
    let fit = ols(&y_means, &design)?;
    let beta: Vec<f64> = fit.coefficients.iter().map(|c| c.estimate).collect();
    let rss = rss_of(&y_means, &col_means, &beta, true);
    let dof = groups.len().saturating_sub(k + 1).max(1);
    Ok(rss / usize_to_f64(dof))
}

/// Residual sum of squares of a fitted regression reconstructed from its
/// coefficient vector. When `intercept` the coefficient vector leads with the
/// intercept (so `columns[j]` pairs with `beta[j + 1]`); otherwise `columns[j]`
/// pairs with `beta[j]`.
fn rss_of(response: &[f64], columns: &[Vec<f64>], beta: &[f64], intercept: bool) -> f64 {
    let offset = usize::from(intercept);
    let mut rss = 0.0;
    for (row, &actual) in response.iter().enumerate() {
        let mut predicted = if intercept { beta[0] } else { 0.0 };
        for (j, col) in columns.iter().enumerate() {
            predicted += beta[j + offset] * col[row];
        }
        let resid = actual - predicted;
        rss += resid * resid;
    }
    rss
}

/// Fama-MacBeth two-pass estimator.
///
/// Runs one cross-sectional OLS per time period, then takes the simple average of
/// the period coefficient vectors. The reported standard error of each
/// coefficient is the cross-period standard error of its mean.
///
/// When `time` is empty the per-entity observation position is used as the period
/// index (so the `t`-th observation of every entity forms one cross-section).
///
/// # Errors
///
/// - [`EconometricsError::InsufficientRows`] when no period has enough
///   cross-sectional observations to fit the regression.
/// - Propagates [`EconometricsError`] from input validation.
pub fn fama_macbeth(
    entity: &[i64],
    time: &[i64],
    y: &[f64],
    x: &[Vec<f64>],
) -> Result<PanelSummary, EconometricsError> {
    let k = validate(entity, y, x)?;
    let columns = to_columns(x, k);

    // Build the period key for each observation.
    let period_key: Vec<i64> = if time.len() == y.len() {
        time.to_vec()
    } else {
        // Fall back to per-entity observation position.
        let mut seen: BTreeMap<i64, i64> = BTreeMap::new();
        entity
            .iter()
            .map(|&ent| {
                let slot = seen.entry(ent).or_insert(0);
                let pos = *slot;
                *slot += 1;
                pos
            })
            .collect()
    };

    let mut periods: BTreeMap<i64, Vec<usize>> = BTreeMap::new();
    for (obs, &period) in period_key.iter().enumerate() {
        periods.entry(period).or_default().push(obs);
    }

    // Each period needs > k+1 observations to fit an intercept + k slopes.
    let mut period_betas: Vec<Vec<f64>> = Vec::new();
    for indices in periods.values() {
        if indices.len() <= k + 1 {
            continue;
        }
        let yp: Vec<f64> = indices.iter().map(|&i| y[i]).collect();
        let xp: Vec<Vec<f64>> = columns
            .iter()
            .map(|col| indices.iter().map(|&i| col[i]).collect())
            .collect();
        let design = design_from_columns(&xp, true)?;
        match ols(&yp, &design) {
            Ok(fit) => period_betas.push(fit.coefficients.iter().map(|c| c.estimate).collect()),
            Err(EconometricsError::Singular | EconometricsError::InsufficientRows { .. }) => {}
            Err(other) => return Err(other),
        }
    }

    let n_periods = period_betas.len();
    if n_periods == 0 {
        return Err(EconometricsError::InsufficientRows {
            rows: 0,
            cols: k + 1,
        });
    }

    let width = k + 1;
    let mut coefficients = Vec::with_capacity(width);
    for j in 0..width {
        let mean = period_betas.iter().map(|beta| beta[j]).sum::<f64>() / usize_to_f64(n_periods);
        // Cross-period standard error of the mean (Fama-MacBeth).
        let std_error = if n_periods > 1 {
            let var = period_betas
                .iter()
                .map(|beta| (beta[j] - mean).powi(2))
                .sum::<f64>()
                / usize_to_f64(n_periods - 1);
            (var / usize_to_f64(n_periods)).sqrt()
        } else {
            0.0
        };
        let t_stat = if std_error > 0.0 { mean / std_error } else { 0.0 };
        coefficients.push(Coefficient {
            estimate: mean,
            std_error,
            t_stat,
        });
    }

    Ok(PanelSummary {
        coefficients,
        r_squared: 0.0,
        n_entities: entity_count(entity),
        n_obs: y.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::{
        between, fama_macbeth, first_difference, fixed_effects, pooled_ols, random_effects,
    };

    /// A small three-entity panel where, *within* each entity, y = 2·x exactly
    /// (plus a per-entity level shift), so the within / first-difference slope is
    /// exactly 2 while the pooled slope is contaminated by the entity shift. The
    /// entities use distinct x ranges so the between regression (on entity means)
    /// is well-posed (non-tied regressor means).
    fn panel() -> (Vec<i64>, Vec<f64>, Vec<Vec<f64>>) {
        // entity 1: x=1,2,3   ⇒ y = 10 + 2x = 12,14,16   (x̄=2)
        // entity 2: x=4,5,6   ⇒ y = 20 + 2x = 28,30,32   (x̄=5)
        // entity 3: x=7,8,9   ⇒ y = 30 + 2x = 44,46,48   (x̄=8)
        let entity = vec![1, 1, 1, 2, 2, 2, 3, 3, 3];
        let x = vec![
            vec![1.0],
            vec![2.0],
            vec![3.0],
            vec![4.0],
            vec![5.0],
            vec![6.0],
            vec![7.0],
            vec![8.0],
            vec![9.0],
        ];
        let y = vec![12.0, 14.0, 16.0, 28.0, 30.0, 32.0, 44.0, 46.0, 48.0];
        (entity, y, x)
    }

    #[test]
    fn fixed_effects_recovers_within_slope() {
        let (e, y, x) = panel();
        let fit = fixed_effects(&e, &y, &x).expect("fe");
        // The within slope is exactly 2 (the entity level shift is swept out).
        assert!(
            (fit.coefficients[0].estimate - 2.0).abs() < 1e-9,
            "slope {}",
            fit.coefficients[0].estimate
        );
        assert_eq!(fit.n_entities, 3);
    }

    #[test]
    fn first_difference_recovers_within_slope() {
        let (e, y, x) = panel();
        let fit = first_difference(&e, &y, &x).expect("fd");
        // Δy = 2·Δx within each entity ⇒ slope exactly 2.
        assert!(
            (fit.coefficients[0].estimate - 2.0).abs() < 1e-9,
            "slope {}",
            fit.coefficients[0].estimate
        );
    }

    #[test]
    fn pooled_ols_slope_is_inflated_by_entity_shift() {
        let (e, y, x) = panel();
        let fit = pooled_ols(&e, &y, &x).expect("pooled");
        // Pooling mixes the rising between-entity level jumps into the slope, so
        // the pooled slope is NOT the clean within slope of 2 (it is larger).
        let slope = fit.coefficients[1].estimate;
        assert!(
            slope > 2.0,
            "pooled slope {slope} should exceed within slope 2"
        );
    }

    #[test]
    fn between_uses_entity_means() {
        let (e, y, x) = panel();
        let fit = between(&e, &y, &x).expect("between");
        // Entity means (x̄, ȳ): (2,14), (5,30), (8,46). These lie on the exact
        // line ȳ = 10/3 + (16/3)·x̄ (slope = (30−14)/(5−2) = 16/3, intercept =
        // 14 − (16/3)·2 = 10/3), so the between regression recovers them exactly.
        assert_eq!(fit.n_entities, 3);
        assert!(
            (fit.coefficients[0].estimate - 10.0 / 3.0).abs() < 1e-9,
            "intercept {}",
            fit.coefficients[0].estimate
        );
        assert!(
            (fit.coefficients[1].estimate - 16.0 / 3.0).abs() < 1e-9,
            "slope {}",
            fit.coefficients[1].estimate
        );
    }

    #[test]
    fn random_effects_slope_is_finite_and_positive() {
        let (e, y, x) = panel();
        let fit = random_effects(&e, &y, &x).expect("re");
        // RE is a GLS blend of the within (slope 2) and between (slope 4) signals;
        // its slope is finite and strictly positive.
        let slope = fit.coefficients[1].estimate;
        assert!(slope.is_finite() && slope > 0.0, "re slope {slope}");
    }

    #[test]
    fn fama_macbeth_averages_cross_sections() {
        // Three time periods, four entities per period, y = 1 + 3x + tiny noise.
        let mut entity = Vec::new();
        let mut time = Vec::new();
        let mut y = Vec::new();
        let mut x = Vec::new();
        for t in 0..3_i64 {
            for ent in 0..4_i64 {
                entity.push(ent);
                time.push(t);
                let ent_f = f64::from(i32::try_from(ent).expect("small"));
                let t_f = f64::from(i32::try_from(t).expect("small"));
                let xv = t_f.mul_add(0.5, ent_f) + 1.0;
                x.push(vec![xv]);
                y.push(3.0_f64.mul_add(xv, 1.0));
            }
        }
        let fit = fama_macbeth(&entity, &time, &y, &x).expect("fmac");
        // Intercept ≈ 1, slope ≈ 3 (exact linear relation each period).
        assert!(
            (fit.coefficients[0].estimate - 1.0).abs() < 1e-6,
            "intercept {}",
            fit.coefficients[0].estimate
        );
        assert!(
            (fit.coefficients[1].estimate - 3.0).abs() < 1e-6,
            "slope {}",
            fit.coefficients[1].estimate
        );
    }

    #[test]
    fn panel_rejects_ragged_input() {
        let entity = vec![1, 1, 2];
        let y = vec![1.0, 2.0, 3.0];
        let x = vec![vec![1.0], vec![2.0]]; // wrong length
        assert!(pooled_ols(&entity, &y, &x).is_err());
    }
}
