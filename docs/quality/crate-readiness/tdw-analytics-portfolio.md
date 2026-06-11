# tdw-analytics-portfolio Readiness Worksheet

Generated during the P3W1 "analytics-portfolio" landing (gap-matrix deferred item
**D6**), which introduced the pure-Rust portfolio-analytics crate on the L4.2
quant base.

## Evidence Snapshot

- Manifest: `crates/tdw-analytics-portfolio/Cargo.toml`.
- Targets: lib.
- Local deps: `tdw-domain`, plus `schemars`, `serde`, `serde_json`, `thiserror`.
- Reverse deps: `tdw-endpoint-catalog` (the `portfolio/*` Compute routes derive
  their params/model schemas from this crate's typed structs) and
  `tdw-service-api` (the `Op::FetchData` compute path via `portfolio_compute`).
- Features: none.
- Tests: 16 unit tests — hand-computed golden-value checks per metric over fixed
  returns/weights fixtures, plus empty-input and edge-case tests.
- Docs/examples: this worksheet plus module-level docs citing each metric's
  standard definition and the returns-not-prices input convention.

## Release Assessment

- A pure, offline, deterministic numeric library: no async, no I/O, no policy.
  Metrics consume a per-period returns or position-value series (`&[f64]`).
- Metrics implemented (5 routes): `cumulative_returns` (compounded wealth path),
  `drawdown` (running peak-to-trough series), `max_drawdown` (worst magnitude +
  peak/trough indices), `allocation` (normalize position values to weights summing
  to 1), `contribution` (the `weight_i · return_i` identity whose sum is the
  portfolio return).
- Clean-room: every formula is standard textbook portfolio math documented in the
  owning module; no reference implementation was consulted; the clean-room audit
  records no exception for this crate.
- The default-feature workspace compiles this crate (analytics are not
  feature-gated), so the pedantic/nursery ratchet counts it; an enumeration with
  `-W clippy::pedantic -W clippy::nursery` reports zero warnings from its files.

## Golden-vector derivation

Expected outputs are hand-derived from the formulas over fixed fixtures:
- cumulative_returns of `[0.10, -0.05, 0.10, -0.05]` compounds to the wealth path
  `[1.10, 1.045, 1.1495, 1.092025]`.
- drawdown of that path: peak tracks the running max, so the trough steps are
  `-0.05` after each `-0.05` return; max_drawdown magnitude `= 0.05`.
- allocation of `[1, 3]` ⇒ weights `[0.25, 0.75]`.
- contribution of weights·returns ⇒ `[0.10, 0.045, 0.1495]`, summing to the
  portfolio return.

## Verdict

Ready. The core portfolio-metric set is complete with hand-computed numeric tests
and is callable as a daemon op (`portfolio/*` Compute routes). Risk/attribution
extensions are an optional later append.
