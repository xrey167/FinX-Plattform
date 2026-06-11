# tdw-charting Readiness Worksheet

Generated during the G014 "WS5 Charting" landing (gap-matrix item **L5.5**),
which introduced the pure-Rust server-side chart-spec crate.

## Evidence Snapshot

- Manifest: `crates/tdw-charting/Cargo.toml`.
- Targets: lib.
- Local deps: `tdw-domain` (the `MarketDataBar` OHLCV input shape), plus
  `schemars`, `serde`, `serde_json`.
- Reverse deps: `tdw-service-api` (the `Op::FetchData` / REST / compute dispatch
  path attaches a chart spec to the `ResultEnvelope.chart` slot when a chartable
  route is called with `chart=true`).
- Features: none.
- Tests: golden spec snapshots per builder (candlestick with/without volume,
  line with a gap, indicator overlay) plus determinism, padding, and
  non-finite-to-null projection unit tests. Golden fixtures live under
  `crates/tdw-charting/tests/golden/`.
- Docs/examples: this crate-readiness worksheet plus module-level docs citing
  the public plotly.js figure-shape documentation.

## Release Assessment

- The crate is a pure, offline, deterministic library: no async, no I/O, no
  policy, and ZERO new native dependency. It emits a **Plotly figure** as plain
  JSON (`{ "data": [...traces], "layout": {...} }`); a plotly.js client renders
  the figure browser-side.
- Builders: `candlestick(bars)` (OHLC candlestick + optional volume `bar`
  subplot on `y2`); `line(points)` (single-value `scatter` line, missing
  observations as `null` gaps); `indicator_overlay(bars, overlays)` (candlestick
  with one `scatter` line per indicator series on top). Inputs are already-parsed
  `tdw_domain::MarketDataBar`s and `(date, value)` `LinePoint`s, so the crate
  re-derives no bar shape — the caller reuses the warehouse's single tolerant
  OHLCV parser (`tdw_service_api::technical_compute::parse_bars`).
- Determinism: every trace and the layout are assembled as a `serde_json::Map`
  with fixed, sorted key order, and the data arrays follow the input bar order,
  so serialization is byte-for-byte reproducible. The golden snapshots pin the
  exact JSON.
- Clean-room: the figure object, the trace `type`s (`candlestick` / `scatter`
  with `mode: "lines"` / `bar`), and the `x` / `open` / `high` / `low` /
  `close` / `y` data keys are the public plotly.js graphing schema (cited to
  `plotly.com/javascript` in the module docs). No reference implementation was
  consulted; the clean-room audit records no exception for this crate.
- The default-feature workspace compiles this crate (charting is not
  feature-gated), so the pedantic/nursery ratchet counts it; an enumeration with
  `-W clippy::pedantic -W clippy::nursery -D warnings` reports zero warnings from
  its files.

## Golden-snapshot derivation

Expected outputs are generated once from the builders over a small fixed OHLCV
fixture (`[10.0/10.5/9.8/10.0/1000], …` over three daily bars) and pinned as
`tests/golden/*.json`; the unit tests `include_str!` each golden and assert the
pretty-printed builder output matches it exactly. The values are not hand-typed —
they are the deterministic projection of the fixture through each builder, so a
re-run regenerates byte-identical files.

## Verdict

Ready with follow-ups. Optional richer chart kinds (heatmaps, multi-pane
sub-indicator panels, Vega-Lite emission) are intentionally out of scope for this
story and are a later append; the core candlestick / line / indicator-overlay set
above is complete with golden snapshots and is wired into the daemon fetch,
compute, and REST chart paths.
