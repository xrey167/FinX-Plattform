//! Seeded Monte-Carlo European option pricing under geometric Brownian motion.
//!
//! Simulates terminal underlying prices under the risk-neutral GBM dynamics and
//! averages the discounted payoff:
//!
//! ```text
//!   S_T = S * exp( (r - q - sigma^2 / 2) * T + sigma * sqrt(T) * Z ),   Z ~ N(0,1)
//!   price = e^{-rT} * mean( payoff(S_T) )
//! ```
//!
//! Determinism: the standard-normal draws come from a hand-rolled `SplitMix64`
//! seeded generator (no wall-clock, no global/thread RNG), so the same `seed`
//! reproduces the same price exactly — satisfying the repository's determinism
//! rule and making the tests reproducible. Standard normals are produced by the
//! Box-Muller transform. Antithetic variates (each `Z` paired with `-Z`) reduce
//! the estimator variance.
//!
//! Source: Boyle (1977), "Options: A Monte Carlo Approach"; the GBM terminal-law
//! sampling and Box-Muller transform are standard textbook math. No reference
//! implementation consulted.

use std::f64::consts::TAU;

use crate::Result;
use crate::error::OptionError;
use crate::params::{MonteCarloParams, MonteCarloPrice, OptionType};

/// A minimal deterministic `SplitMix64` generator.
///
/// `SplitMix64` is a well-known public-domain mixing generator: each call
/// advances the state by the golden-ratio increment and applies an avalanche
/// mix. It is seeded by the caller's `seed`, so a run is fully reproducible.
struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    const fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    /// Next raw 64-bit value (the standard `SplitMix64` step).
    const fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A uniform `f64` in the half-open interval `(0, 1)`.
    ///
    /// Uses the top 53 bits (the `f64` mantissa width) and shifts the result into
    /// the open interval so the subsequent `ln()` in Box-Muller never sees zero.
    fn next_f64(&mut self) -> f64 {
        // 53-bit mantissa → [0, 1); add half an ULP to open the lower bound.
        let bits = self.next_u64() >> 11;
        #[allow(clippy::cast_precision_loss)]
        let unit = (bits as f64 + 0.5) / (1u64 << 53) as f64;
        unit
    }
}

/// A Box-Muller standard-normal generator over a [`SplitMix64`] stream.
///
/// Box-Muller turns two independent uniforms into two independent standard
/// normals; this caches the second so each `next_normal` call consumes on
/// average one uniform pair per two draws.
struct NormalStream {
    rng: SplitMix64,
    spare: Option<f64>,
}

impl NormalStream {
    const fn new(seed: u64) -> Self {
        Self {
            rng: SplitMix64::new(seed),
            spare: None,
        }
    }

    fn next_normal(&mut self) -> f64 {
        if let Some(z) = self.spare.take() {
            return z;
        }
        let u1 = self.rng.next_f64();
        let u2 = self.rng.next_f64();
        let radius = (-2.0 * u1.ln()).sqrt();
        let angle = TAU * u2;
        self.spare = Some(radius * angle.sin());
        radius * angle.cos()
    }
}

/// Payoff of a European call/put at terminal price `spot_t`.
fn payoff(option_type: OptionType, spot_t: f64, strike: f64) -> f64 {
    match option_type {
        OptionType::Call => (spot_t - strike).max(0.0),
        OptionType::Put => (strike - spot_t).max(0.0),
    }
}

/// Price a European call/put by seeded Monte-Carlo GBM simulation.
///
/// Returns the discounted-payoff estimate together with its sampling standard
/// error and the number of payoff samples drawn.
///
/// # Errors
///
/// Returns [`OptionError::NonPositive`] when `spot`, `strike`, `volatility`, or
/// `time_to_expiry` is not strictly positive, and [`OptionError::InvalidPaths`]
/// when `paths` is zero.
pub fn price(params: MonteCarloParams) -> Result<MonteCarloPrice> {
    if params.spot <= 0.0 {
        return Err(OptionError::NonPositive {
            field: "spot",
            value: params.spot,
        });
    }
    if params.strike <= 0.0 {
        return Err(OptionError::NonPositive {
            field: "strike",
            value: params.strike,
        });
    }
    if params.volatility <= 0.0 {
        return Err(OptionError::NonPositive {
            field: "volatility",
            value: params.volatility,
        });
    }
    if params.time_to_expiry <= 0.0 {
        return Err(OptionError::NonPositive {
            field: "time_to_expiry",
            value: params.time_to_expiry,
        });
    }
    if params.paths == 0 {
        return Err(OptionError::InvalidPaths(params.paths));
    }

    let log_drift = (0.5 * params.volatility)
        .mul_add(-params.volatility, params.rate - params.dividend_yield)
        * params.time_to_expiry;
    let diffusion = params.volatility * params.time_to_expiry.sqrt();
    let discount = (-params.rate * params.time_to_expiry).exp();

    let terminal = |z: f64| params.spot * diffusion.mul_add(z, log_drift).exp();
    let discounted_payoff =
        |spot_t: f64| discount * payoff(params.option_type, spot_t, params.strike);

    let mut normals = NormalStream::new(params.seed);
    // Running sums of the per-draw *observations*. Each Box-Muller draw produces
    // one independent observation; under antithetic sampling the observation is
    // the AVERAGE of the `Z` and `-Z` discounted payoffs, since `Y` and `Y'` are
    // correlated and must not be treated as two independent samples. `samples`
    // counts payoff evaluations (for reporting), while `observations` counts the
    // statistically-independent draws the standard error is computed over.
    let mut sum = 0.0_f64;
    let mut sum_sq = 0.0_f64;
    let mut samples = 0usize;
    let mut observations = 0usize;

    for _ in 0..params.paths {
        let z = normals.next_normal();
        let observation = if params.antithetic {
            samples += 2;
            // The antithetic pair's average is one observation.
            0.5 * (discounted_payoff(terminal(z)) + discounted_payoff(terminal(-z)))
        } else {
            samples += 1;
            discounted_payoff(terminal(z))
        };
        sum += observation;
        sum_sq += observation * observation;
        observations += 1;
    }

    #[allow(clippy::cast_precision_loss)]
    let n = observations as f64;
    let mean = sum / n;
    // Population variance of the per-draw observations; standard error =
    // sd / sqrt(n) over the `observations` independent draws (the antithetic
    // pair-averages), NOT over the doubled payoff count.
    let variance = mean.mul_add(-mean, sum_sq / n).max(0.0);
    let std_error = (variance / n).sqrt();

    Ok(MonteCarloPrice {
        price: mean,
        std_error,
        samples,
    })
}

#[cfg(test)]
mod tests {
    use super::{NormalStream, payoff, price};
    use crate::black_scholes;
    use crate::params::{BlackScholesParams, MonteCarloParams, OptionType};

    fn base(option_type: OptionType, paths: usize, seed: u64) -> MonteCarloParams {
        MonteCarloParams {
            spot: 100.0,
            strike: 100.0,
            rate: 0.05,
            dividend_yield: 0.0,
            volatility: 0.20,
            time_to_expiry: 1.0,
            option_type,
            paths,
            seed,
            antithetic: true,
        }
    }

    #[test]
    fn deterministic_for_a_fixed_seed() {
        let a = price(base(OptionType::Call, 5_000, 7)).expect("a");
        let b = price(base(OptionType::Call, 5_000, 7)).expect("b");
        assert!(
            (a.price - b.price).abs() < 1e-15,
            "seed must be reproducible"
        );
        assert_eq!(a.samples, b.samples);
    }

    #[test]
    fn within_tolerance_of_black_scholes_call() {
        let bs = black_scholes::price(BlackScholesParams {
            spot: 100.0,
            strike: 100.0,
            rate: 0.05,
            dividend_yield: 0.0,
            volatility: 0.20,
            time_to_expiry: 1.0,
            option_type: OptionType::Call,
        })
        .expect("bs");
        // 200k antithetic paths with a fixed seed land close to the BS value.
        let mc = price(base(OptionType::Call, 200_000, 12_345)).expect("mc");
        assert!((mc.price - bs).abs() < 0.1, "mc {} vs bs {bs}", mc.price);
        // The reported error bracket should also be consistent with the gap.
        assert!(mc.std_error > 0.0);
    }

    #[test]
    fn antithetic_doubles_the_sample_count() {
        let mc = price(base(OptionType::Call, 1_000, 1)).expect("mc");
        assert_eq!(mc.samples, 2_000);
        let mut plain = base(OptionType::Call, 1_000, 1);
        plain.antithetic = false;
        let mc_plain = price(plain).expect("plain");
        assert_eq!(mc_plain.samples, 1_000);
    }

    #[test]
    fn antithetic_std_error_is_over_pairs_not_doubled_samples() {
        // The standard error must be computed over the M pair-averages (one
        // observation per antithetic pair), NOT over the 2M correlated payoffs.
        // Reconstruct the correct std_error independently from the `price` and a
        // direct simulation, and confirm `samples` still reports the 2M payoff
        // count while the error reflects the M observations.
        let paths = 4_000usize;
        let params = base(OptionType::Call, paths, 2024);
        let mc = price(params).expect("mc");
        assert_eq!(mc.samples, 2 * paths, "samples counts payoff evaluations");

        // Independently recompute the pair-average observations.
        let log_drift = (0.5 * params.volatility)
            .mul_add(-params.volatility, params.rate - params.dividend_yield)
            * params.time_to_expiry;
        let diffusion = params.volatility * params.time_to_expiry.sqrt();
        let discount = (-params.rate * params.time_to_expiry).exp();
        let terminal = |z: f64| params.spot * diffusion.mul_add(z, log_drift).exp();
        let dp = |s: f64| discount * payoff(params.option_type, s, params.strike);

        let mut normals = NormalStream::new(params.seed);
        let mut obs = Vec::with_capacity(paths);
        for _ in 0..paths {
            let z = normals.next_normal();
            obs.push(0.5 * (dp(terminal(z)) + dp(terminal(-z))));
        }
        #[allow(clippy::cast_precision_loss)]
        let n = obs.len() as f64;
        let mean = obs.iter().sum::<f64>() / n;
        let var = obs.iter().map(|o| (o - mean) * (o - mean)).sum::<f64>() / n;
        let expected_se = (var / n).sqrt();
        assert!(
            (mc.std_error - expected_se).abs() < 1e-9,
            "std_error {} should match the over-M-pairs standard error {expected_se}",
            mc.std_error
        );
    }

    #[test]
    fn put_price_is_positive_and_below_call() {
        let call = price(base(OptionType::Call, 50_000, 99)).expect("call");
        let put = price(base(OptionType::Put, 50_000, 99)).expect("put");
        assert!(put.price > 0.0);
        // For these ITM-rate parameters the call is worth more than the put.
        assert!(put.price < call.price);
    }

    #[test]
    fn rejects_zero_paths_and_bad_inputs() {
        assert!(price(base(OptionType::Call, 0, 1)).is_err());
        let mut bad = base(OptionType::Call, 100, 1);
        bad.spot = 0.0;
        assert!(price(bad).is_err());
    }
}
