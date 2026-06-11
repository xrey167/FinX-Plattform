# tdw-analytics-quant Readiness Worksheet

Generated during the G015 "WS6b Quant and Econometrics" landing (gap-matrix item
**L4.2**), which introduced the pure-Rust returns-based quantitative-metrics
crate.

## Evidence Snapshot

- Manifest: `crates/tdw-analytics-quant/Cargo.toml`.
- Targets: lib.
- Local deps: `tdw-domain`, plus `schemars`, `serde`, `serde_json`, `thiserror`.
- Reverse deps: `tdw-endpoint-catalog` (the `quantitative/*` Compute routes derive
  their params/model schemas from this crate's typed structs) and
  `tdw-service-api` (the `Op::FetchData` / `Op::ToolCall` compute path).
- Features: none.
- Tests: 31 unit tests — golden-value checks per metric (hand-derived from the
  textbook definitions over a fixed four-point returns fixture), descriptive-
  statistics primitive tests, and param-default tests.
- Docs/examples: crate-readiness worksheet plus module-level docs citing each
  metric's standard definition, and a runnable crate-level doctest for the
  prices-to-returns helper.

## Release Assessment

- The crate is a pure, offline, deterministic numeric library: no async, no I/O,
  no policy. Metrics consume a per-period **returns** series (`&[f64]`); the
  crate documents the returns-not-prices convention and ships a
  `prices_to_returns` helper. Each metric summarizes the whole series into one
  figure or a small typed row (no leading-`None` per-bar policy).
- Metrics implemented (12 routes): `sharpe_ratio`, `sortino_ratio`,
  `omega_ratio`, `max_drawdown` (+ `calmar_ratio` from the same drawdown row),
  `volatility` (annualized), `skewness`, `kurtosis` (excess, bias-corrected),
  `value_at_risk` (historical), `expected_shortfall` (CVaR), `capm` (alpha+beta
  vs a benchmark), `jarque_bera` (statistic + chi-squared p-value).
- Clean-room: every formula is textbook math cited to its standard definition in
  the owning module's docs (Sharpe 1966; Sortino & Price 1994; Keating &
  Shadwick 2002 for Omega; Young 1991 for Calmar; the adjusted Fisher-Pearson
  skewness and bias-corrected excess-kurtosis estimators; Jarque & Bera 1980).
  No reference implementation was consulted; the clean-room audit records no
  exception for this crate.
- The default-feature workspace compiles this crate (analytics are not
  feature-gated), so the pedantic/nursery ratchet counts it; an enumeration with
  `-W clippy::pedantic -W clippy::nursery` reports zero warnings from its files.

## Golden-vector derivation

Expected outputs in the unit tests are hand-derived from the textbook formulas
over the fixed returns fixture `r = [0.10, -0.05, 0.10, -0.05]`:

- Mean 0.025; sample (n-1) variance 0.0075 ⇒ std 0.0866025; so unannualized
  Sharpe = 0.025 / 0.0866025 = 0.2886751, and √periods scaling is checked
  independently (√4 = 2× the base).
- Sortino (target 0): downside-sq = 2·0.0025 = 0.005, /n(4) = 0.00125, downside
  deviation 0.0353553; Sortino = 0.025 / 0.0353553 = 0.7071068 (= 1/√2).
- Omega (threshold 0): gains 0.20 over losses 0.10 = 2.0.
- Historical VaR (95%): 5% quantile of the sorted returns = -0.05; expected
  shortfall (tail mean at/below -0.05) = -0.05.
- CAPM: asset = benchmark ⇒ beta 1, alpha 0; asset = 2× benchmark ⇒ beta 2,
  alpha 0.
- Max drawdown over the compounded wealth path = -0.05; Calmar = 0.025/0.05 =
  0.5.
- Jarque-Bera: the χ²₂ survival closed form `p = exp(-JB/2)` is verified to be
  exactly the reported p-value; the small-sample guard (n<4) returns statistic 0
  and p-value 1.

The `statsmodels`/`numpy` reference values cited in the gap-matrix were *not*
used directly (no Python is available in this clean-room build); the hand-derived
exact-arithmetic figures above are the authoritative expectations.

## Verdict

Ready with follow-ups. Optional OpenBB long-tail quant routes (rolling
mean/stdev/var/skew/kurtosis/quantile windows, a standalone normality `summary`,
and a formal unit-root test) are intentionally out of scope for this story and
are a later append; the core set above is complete with numeric tests and is
callable as a daemon op (`quantitative/*` Compute routes) and an MCP tool
(`quantitative.*`).
