# tdw-endpoint-catalog Readiness Worksheet

Generated during the G001 "WS0 Catalog Spine" landing, which introduced the
namespaced endpoint catalog crate.

## Evidence Snapshot

- Manifest: `crates/tdw-endpoint-catalog/Cargo.toml`.
- Targets: lib.
- Local deps: `tdw-core`, `tdw-domain` (plus `schemars`, `serde`).
- Reverse deps: `tdw-acp` (route-grammar validation), `tdw-service-api`
  (logical-endpoint resolution + `FetchData` dispatch), `xtask`
  (`catalog-check`).
- Features: none.
- Tests: 8 unit tests (route grammar, route uniqueness, deterministic order,
  Fetch/Compute candidate invariants, lookup, legacy-order parity).
- Docs/examples: crate-readiness worksheet plus module-level docs.

## Release Assessment

- The crate is a pure, offline, deterministic static table plus lookup and
  route-grammar helpers — no I/O, no policy, no network.
- It is the single source of truth WS0 establishes: `provider_resolve` now
  delegates to `catalog()`, and `xtask catalog-check` validates every candidate
  against the full ingest dispatch table.
- No clean-room exception is recorded for this crate.
- Any code-level follow-up remains non-blocking unless `fmt`,
  `clippy -D warnings`, tests, clean-room audit, `catalog-check`, or
  `crate-readiness-check` fails.

## Verdict

Ready with follow-ups. Later OpenBB-parity stories populate the seeded stub
routers (`fixedincome`, `etf`, `derivatives`, `currency`, `economy`,
`commodity`, `news`, `regulators`); each is an append to the relevant router's
`entries()`.
