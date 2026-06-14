#![forbid(unsafe_code)]
#![deny(clippy::pedantic, clippy::nursery)]
//! Pure-Rust, offline, deterministic classical-forecasting math (ecosystem item
//! **G003**).
//!
//! Hand-rolled implementations of the standard textbook statistical-forecasting
//! models over a caller-supplied time series. The crate performs no I/O and no
//! async work and uses no global/thread RNG: it is a deterministic numeric
//! library a daemon compute route or a UDF can call directly. Every figure is
//! reproducible from its inputs.
//!
//! # Models
//!
//! - [`baseline`] — the reference baselines: the plain naive method, the
//!   seasonal-naive method (tile the last full season forward), and
//!   random-walk-with-drift (`ŷ_{n+h} = y_n + h·d`, drift = mean first
//!   difference).
//! - [`expo`] — Holt-Winters additive exponential smoothing (level + trend +
//!   optional additive seasonal) and an ETS additive model selector that fits the
//!   additive variants over a coefficient grid and keeps the lowest in-sample
//!   one-step SSE.
//! - [`theta`] — the Theta method (`θ = 2`; Assimakopoulos & Nikolopoulos 2000),
//!   a strong dependency-free baseline that carries a regression drift forward.
//! - [`mstl`] — an additive moving-average seasonal-trend decomposition (a simple,
//!   documented STL/MSTL variant: centered-MA trend + per-phase seasonal means +
//!   residual).
//! - [`linregr`] — a lag-feature linear-regression forecaster with optional
//!   exogenous covariates that **reuses the OLS core from
//!   `tdw-analytics-econometrics`**.
//! - [`arima`] — `ARIMA(p, d, q)` estimated by the deterministic
//!   `Hannan-Rissanen` two-stage least-squares procedure (reusing the
//!   econometrics OLS core), with `d`-order differencing and integration of the
//!   forecast back to the series level.
//! - [`autoarima`] — automatic `ARIMA` order selection via a `Hyndman-Khandakar`
//!   stepwise `(p, q)` search plus a variance-ratio differencing decision, picking
//!   the lowest-information-criterion model (default `AICc`). This is ecosystem
//!   item **G008**.
//! - [`metrics`] — the standard forecast-accuracy measures (RMSE, MAE, MAPE,
//!   SMAPE).
//! - [`backtest`] — an expanding-window historical-forecasts harness that re-fits
//!   a model at each origin and scores the realized-vs-forecast pairs with the
//!   precision metrics.
//! - [`anomaly`] — a quantile-band anomaly detector (flag points outside the
//!   empirical `[q_lo, q_hi]` band).
//!
//! # Conventions
//!
//! Series are in time order (oldest first). Forecast horizons and seasonal
//! periods are positive integers; smoothing coefficients and quantiles lie in
//! `[0, 1]`. See [`params`] for the per-route request and output structs (all
//! `serde` + `JsonSchema` so the catalog can publish their schemas).
//!
//! ```
//! use tdw_analytics_forecast::baseline;
//! let fc = baseline::random_walk_drift(&[2.0, 4.0, 6.0, 8.0], 2).expect("forecast");
//! // Drift = mean first difference = 2; continues 10, 12.
//! assert!((fc.points[0].value - 10.0).abs() < 1e-9);
//! assert!((fc.points[1].value - 12.0).abs() < 1e-9);
//! ```
//!
//! # Honest simplifications (vs darts / statsforecast)
//!
//! - **MSTL** ([`mstl`]) ships the classical moving-average additive
//!   decomposition rather than the full iterative LOESS-STL. This is the standard
//!   dependency-free decomposition and is documented as a deliberate
//!   simplification in the module docs.
//! - **ETS** ([`expo::ets`]) selects over the *additive* error-trend-seasonal
//!   variants by in-sample one-step SSE over a coarse coefficient grid, not the
//!   full multiplicative ETS taxonomy with information-criterion likelihood
//!   selection.
//! - **Theta** ([`theta`]) implements the classic `θ = 2` closed form, not the
//!   generalized multi-theta optimization.
//! - **`ARIMA`** ([`arima`]) is estimated by `Hannan-Rissanen` two-stage least
//!   squares, a deterministic consistent estimator, rather than exact
//!   maximum-likelihood with a `Kalman` filter (which needs a numeric optimizer
//!   and is not reproducible in the same dependency-free way). This is documented
//!   in the module docs.
//! - Deep-learning forecasters (`RNN`/`LSTM`/`TFT`/`NBEATS`) remain intentionally
//!   **out of scope**: they require a heavy native dependency that breaks this
//!   crate's zero-heavy-dep, deterministic posture. See the deferral note at
//!   `docs/roadmap/dl-forecasting-deferral.md`.
//!
//! # Clean-room provenance
//!
//! Every formula here is public textbook math cited to its standard definition in
//! the owning module's docs (Holt 1957 and Winters 1960 for exponential
//! smoothing; Assimakopoulos & Nikolopoulos 2000 and Hyndman & Billah 2003 for
//! the Theta method; the classical moving-average decomposition for MSTL; the
//! Gauss-Markov / normal-equations OLS reused from `tdw-analytics-econometrics`
//! for the regression forecaster; Hannan & Rissanen 1982 for the two-stage
//! `ARIMA` estimator and Hyndman & Khandakar 2008 for the automatic order search;
//! Hyndman & Koehler 2006 for the accuracy measures). No reference implementation
//! was consulted.

pub mod anomaly;
pub mod arima;
pub mod autoarima;
pub mod backtest;
pub mod baseline;
pub mod error;
pub mod expo;
pub mod linregr;
pub mod metrics;
pub mod mstl;
pub mod params;
pub mod theta;

pub use error::ForecastError;

/// Result type for forecasting computations.
pub type Result<T> = std::result::Result<T, ForecastError>;
