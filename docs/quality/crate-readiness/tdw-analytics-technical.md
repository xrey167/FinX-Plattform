# tdw-analytics-technical Readiness Worksheet

Generated during the G006 "WS6a Technical Analytics" landing (gap-matrix item
**L4.1**), which introduced the pure-Rust technical-indicator crate.

## Evidence Snapshot

- Manifest: `crates/tdw-analytics-technical/Cargo.toml`.
- Targets: lib.
- Local deps: `tdw-domain` (the `MarketDataBar` OHLCV input shape), plus
  `schemars`, `serde`, `serde_json`, `thiserror`.
- Reverse deps: `tdw-endpoint-catalog` (the `technical/*` Compute routes derive
  their params/model schemas from this crate's typed structs) and
  `tdw-service-api` (the `Op::FetchData` / `Op::ToolCall` compute path).
- Features: none.
- Tests: 32 unit tests — golden-vector checks per indicator (hand-derived from
  the textbook definitions over a fixed OHLCV fixture) plus param-default and
  smoothing-primitive tests.
- Docs/examples: crate-readiness worksheet plus module-level docs citing each
  indicator's standard definition.

## Release Assessment

- The crate is a pure, offline, deterministic numeric library: no async, no I/O,
  no policy. Indicators consume `&[MarketDataBar]` (or a derived close column)
  and return a date-aligned `Vec` of the input length with a documented
  leading-`None` policy (multi-line indicators return typed rows of
  `Option<f64>` components).
- Indicators implemented (~18 core): SMA, EMA, WMA, HMA; MACD (12/26/9); RSI
  (Wilder); Stochastic %K/%D; CCI; ADX (+DI/−DI, Wilder); Aroon up/down/osc;
  Bollinger Bands; Keltner Channels; Donchian Channels; ATR (Wilder); OBV; A/D;
  VWAP; Fisher transform; ROC; momentum.
- Clean-room: every formula is textbook math cited to its standard definition in
  the owning module's docs (Wilder 1978 for RSI/ATR/ADX/+DI/−DI; Lane for
  Stochastic; Lambert for CCI; Bollinger; Ehlers for the Fisher transform; Hull
  for HMA). No reference implementation was consulted; the clean-room audit
  records no exception for this crate.
- The default-feature workspace compiles this crate (analytics are not
  feature-gated), so the pedantic/nursery ratchet counts it; an enumeration with
  `-W clippy::pedantic -W clippy::nursery` reports zero warnings from its files.

## Golden-vector derivation

Expected outputs in the unit tests are hand-derived from the textbook formulas
over a small fixed OHLCV series (`fixture::series`) or short literal arrays:

- SMA/EMA/WMA: closed-form means over `[1,2,3,4,5]` etc. (e.g. EMA(3) seed = SMA
  of the first 3 samples, then α = 2/(3+1) = 0.5 recursion).
- RSI(2) over `[10,11,10,12]`: seed avgGain/avgLoss as the simple mean of the
  first 2 deltas, then Wilder-smooth — yields 50 then 100−100/6.
- ROC/Momentum: direct percent / difference over `[10,11,12]`.
- Wilder smoothing primitive: sum-seed (`wilder_smooth`) and mean-seed
  (`wilder_average`) recurrences checked against `[1,2,3]` / `[2,4,6]`.
- Bollinger over a flat series collapses all three bands to the mean; Donchian
  midpoint equals the rolling high/low midpoint.

## Verdict

Ready with follow-ups. Optional OpenBB long-tail indicators (Ichimoku, cones,
Clenow, Demark, fib, RRG) are intentionally out of scope for this story and are a
later append; the core set above is complete with numeric tests and is callable
as a daemon op (`technical/*` Compute routes) and an MCP tool (`technical.*`).
