# tdw-widgets Readiness Worksheet

Generated during the G007 "WSB1 Widgets Backend" landing, which introduced the
OpenBB Workspace bridge crate.

## Evidence Snapshot

- Manifest: `crates/tdw-widgets/Cargo.toml`.
- Targets: lib, test (the `golden_widgets_json` integration test).
- Local deps: `tdw-endpoint-catalog` (plus `schemars`, `serde`, `serde_json`).
- Reverse deps: `tdw-app-server` (optional, via its `workspace-route` feature —
  the transport derives `widgets.json` / `apps.json` from this crate).
- Features: none.
- Tests: 16 unit tests (serde round-trips for `widgets.json` / `apps.json`
  fixtures transcribed from the public docs; derivation unit tests — param
  mapping, `columnsDefs`, chart flag, formatter heuristics, title-casing, MCP
  binding, crypto symbol default; catalog assembly — one widget per Fetch route,
  unique ids, deterministic output, default-app references) plus 3 golden tests
  (full `widgets.json` / `apps.json` snapshot + marquee-route invariant).
- Docs/examples: this worksheet, module-level docs citing the public OpenBB
  Workspace doc URLs, and the product doc
  `docs/products/openbb-workspace-backend.md`.

## Release Assessment

- The crate is a pure, offline, deterministic **projection**: it performs no I/O
  and enforces no policy. It serializes the OpenBB Workspace `widgets.json` /
  `apps.json` contract and derives both documents from
  `tdw_endpoint_catalog::catalog()`, which stays the single source of truth.
- Clean-room: every contract type is a projection of **public** OpenBB Workspace
  developer documentation only — no OpenBB source code was consulted. Doc URLs
  are cited in the module docs, exactly as other clean-room-derived crates do.
  The crate compiles by default, so it is in the pedantic/nursery ratchet scope;
  it carries zero new warnings.
- The full derived `widgets.json` is pinned by a golden snapshot, so any drift in
  the derivation engine surfaces as a reviewable diff (`TDW_WIDGETS_BLESS=1` to
  refresh after an intentional change).
- Any code-level follow-up remains non-blocking unless `fmt`,
  `clippy -D warnings`, tests, the clean-room audit, `catalog-check`, or
  `crate-readiness-check` fails.

## Verdict

Ready with follow-ups. v1 derives widgets for `Fetch` routes only; `Compute`
routes (carrying no provider fetcher) are excluded and are a documented
follow-up. The curated default app ("FinX Market Overview") and the curated
label overrides are a small const-style table by design (no file-loading
system); later stories can broaden the curated layer as routes grow.
