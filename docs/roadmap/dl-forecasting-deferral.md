# Deep-learning forecasting: a deliberate deferral

**Status:** Deferred (not built into the core). Ecosystem item **G008**.
**Decision date:** 2026-06-14.

## Summary

The platform's computed-forecast menu now covers the **keyless, deterministic
majority** of OpenBB's forecasting surface:

- **G003** shipped the classical statistical suite in the pure-Rust
  `tdw-analytics-forecast` crate: the naive / seasonal-naive / random-walk-drift
  baselines, Holt-Winters and an additive ETS selector, the Theta method, a
  moving-average MSTL decomposition, a lag-feature linear-regression forecaster,
  an expanding-window backtest harness, the RMSE / MAE / MAPE / SMAPE accuracy
  measures, and a quantile-band anomaly scan.
- **G008** adds `ARIMA(p, d, q)` — estimated by the deterministic
  Hannan-Rissanen two-stage least-squares procedure — and `AutoARIMA`, a
  Hyndman-Khandakar-style stepwise order search, both exposed as the compute
  routes `forecast/arima` and `forecast/autoarima`.

What remains of OpenBB's forecasting surface is its **deep-learning tail**: the
RNN / LSTM / GRU, NBEATS, NHITS, TCN, TFT, and Transformer forecasters. These are
**deliberately excluded** from the core. This document records that decision and
the conditions under which it could be revisited.

## Why deep-learning forecasters are deferred

OpenBB's neural forecasters are implemented on top of `darts`, which in turn
pulls in `torch` (PyTorch). That dependency chain is fundamentally at odds with
the posture the rest of the analytics surface is built on:

1. **Pure-Rust, zero-heavy-dependency.** Every forecaster in
   `tdw-analytics-forecast` is hand-rolled textbook math over a caller-supplied
   series, with no native build-time dependency beyond the workspace's own crates.
   `torch` is a large native library (hundreds of megabytes, platform-specific
   binary wheels, a C++/CUDA toolchain) — a build-and-distribution landmine for a
   daemon that is meant to be a small, self-contained binary.
2. **Determinism and reproducibility.** The analytics crates guarantee that every
   figure is reproducible from its inputs: no global RNG, no nondeterministic
   kernels. Neural-network training is stochastic by construction (random weight
   initialization, dropout, non-deterministic GPU reductions). Bolting it into the
   core would break the "every number is reproducible from its inputs" contract.
3. **Offline, no-I/O compute routes.** The forecast routes are `Compute`
   derivations with no provider candidates and no network or disk access. A neural
   forecaster wants accelerators, model checkpoints, and a training loop — a
   different operational shape entirely.

Adding `torch` / `darts` to the core daemon to chase the DL tail would trade away
the determinism, the small footprint, and the clean-room posture that make the
classical suite trustworthy — for forecasters whose marginal accuracy on the
financial series these routes target is, at best, situational.

## The future path (if demanded)

Deep-learning forecasting is deferred, **not forbidden forever**. If real demand
materializes, the supported way to add it keeps the core clean:

- **An optional, pure-Rust feature.** A `candle`- or `burn`-backed forecaster
  behind an off-by-default cargo feature, so the default build stays dependency-
  light and the neural path is opt-in. `candle` and `burn` are pure-Rust ML
  stacks that avoid the `torch` native-dependency problem.
- **Or a sidecar service.** A separate process (its own runtime, its own
  dependencies, its own accelerator access) that the daemon calls over a typed
  boundary, keeping the heavy stack out of the core binary entirely.

**Never** `torch` / `darts` in the core daemon, and **never** as a default
dependency.

## Scope of this wave

This wave (**G008**) does **not** add `torch`, `darts`, `candle`, or `burn` as
dependencies. It ships `ARIMA` / `AutoARIMA` in pure Rust and records this
deferral. The DL tail is the deliberate, documented exclusion — see the
`tdw-analytics-forecast` crate-level docs, which point here.
