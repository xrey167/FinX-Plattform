# tdw-analytics-forecast Readiness Worksheet

Generated during the openbb-ecosystem-p1 **G003** landing (the classical
forecasting suite), which introduced the pure-Rust, offline, deterministic
statistical-forecasting + backtesting crate.

## Evidence Snapshot

- Manifest: `crates/tdw-analytics-forecast/Cargo.toml`.
- Targets: lib.
- Local deps: `tdw-domain` and `tdw-analytics-econometrics` (the linear-regression
  forecaster reuses the econometrics OLS core — it is NOT reimplemented), plus
  `schemars`, `serde`, `serde_json`, `thiserror`. No heavy numeric dependency:
  every model is hand-rolled textbook math, so `statsmodels`/`statsforecast`/
  `darts`/`torch` equivalents are intentionally NOT dependencies.
- Reverse deps: `tdw-endpoint-catalog` (the `forecast/*` Compute routes derive
  their params/model schemas from this crate's typed structs) and
  `tdw-service-api` (the `Op::FetchData` / `Op::ToolCall` compute path via
  `forecast_compute`).
- Features: none.
- Tests: 38 unit tests plus a crate-level doctest — real numeric assertions on
  known fixtures (seasonal-naive reproduces the period exactly; rwd drift equals
  the mean first difference; Holt-Winters/ETS extrapolate a linear trend; the
  Theta method beats naive on a trended holdout; MSTL additive decomposition
  recovers a clean seasonal and satisfies `y = trend + seasonal + residual`;
  RMSE/MAE/MAPE/SMAPE matched against hand-calculated values; the quantile
  detector flags an injected spike). The `forecast_compute` daemon wiring adds 8
  more tests in `tdw-service-api`.
- Docs/examples: this crate-readiness worksheet plus module-level docs citing each
  model's standard definition, and a runnable crate-level doctest for
  random-walk-with-drift.

## Release Assessment

- The crate is a pure, offline, deterministic numeric library: no async, no I/O,
  no policy, no global/thread RNG. Every figure is reproducible from its inputs
  (the repository determinism rule); there is no stochastic model in the classical
  set, so no seed is needed.
- Models implemented (10 routes): `seasonalnaive` and `rwd` (baselines); `expo`
  (Holt-Winters additive level+trend+optional-seasonal) and `ets` (additive
  error-trend-seasonal model selection); `theta` (the Theta method, theta = 2);
  `mstl` (additive moving-average seasonal-trend decomposition); `linregr`
  (lag-feature linear-regression forecast reusing the econometrics OLS);
  `backtest` (expanding-window historical-forecasts harness) and `metrics`
  (RMSE/MAE/MAPE/SMAPE); and `anomaly` (quantile-band detection).
- Clean-room: every formula is public textbook math cited to its standard
  definition in the owning module's docs (Holt 1957; Winters 1960;
  Assimakopoulos & Nikolopoulos 2000 and Hyndman & Billah 2003 for Theta; the
  classical moving-average decomposition for MSTL; the normal-equations OLS reused
  from `tdw-analytics-econometrics` for the regression forecaster; Hyndman &
  Koehler 2006 for the accuracy measures). No reference implementation was
  consulted; the clean-room audit records no exception for this crate.
- The default-feature workspace compiles this crate (analytics are not
  feature-gated), so the pedantic/nursery ratchet counts it; an enumeration with
  `-W clippy::pedantic -W clippy::nursery` reports zero warnings from its files.

## Honest simplifications (vs darts / statsforecast)

- **MSTL** ships the classical moving-average additive decomposition (centered-MA
  trend with linear edge-extrapolation + per-phase seasonal means + residual)
  rather than the full iterative LOESS-STL. This is the standard dependency-free
  decomposition and is documented as a deliberate simplification in the module.
- **ETS** selects over the additive error-trend-seasonal variants by in-sample
  one-step SSE over a coarse coefficient grid, not the full multiplicative ETS
  taxonomy with information-criterion likelihood selection.
- **Theta** implements the classic theta = 2 closed form, not generalized
  multi-theta optimization.
- Deep-learning forecasters (RNN/LSTM/TFT/NBEATS) and AutoARIMA are out of scope
  (ecosystem item G008): they require a heavy dependency that would break this
  crate's zero-heavy-dep, deterministic posture.

## Golden-vector derivation

Expected outputs in the unit tests are hand-derived from the textbook formulas:

- Seasonal-naive on `[1,2,3,4,10,20,30,40]`, period 4, horizon 8 tiles the last
  season `[10,20,30,40]` exactly twice.
- Random-walk-drift on `[2,4,6,8,10]`: drift = mean first difference =
  `(10 − 2)/4 = 2`, so the 3-step forecast is `12, 14, 16`.
- Precision metrics on `actual=[100,200,300]`, `forecast=[110,190,330]`:
  RMSE = `sqrt(1100/3) = 19.1485…`, MAE = `50/3 = 16.6667…`,
  MAPE = `100/3 · 0.25 = 8.3333…`%, SMAPE = `100/3 · (10/105 + 10/195 + 30/315)`%.
- The Theta method beats the naive baseline (lower one-step holdout RMSE) on a
  clean slope-2 trend.
- The quantile detector with band `[0.05, 0.95]` flags an injected spike of 100
  against a series oscillating near 10.

No external reference values were used directly (this is a clean-room build); the
textbook figures above are the authoritative expectations.

## Verdict

Ready with follow-ups. Optional long-tail extensions (full LOESS-STL, AutoARIMA,
multiplicative ETS with IC selection, probabilistic prediction intervals, and the
deep-learning forecasters) are intentionally out of scope for this story and are a
later append; the classical set above is complete with real numeric tests and is
callable as a daemon op (`forecast/*` Compute routes) and an MCP tool
(`forecast.*`).
