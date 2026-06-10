# tdw-provider-federal-reserve Readiness Worksheet

Generated during the G003 "WS1b SEC and Treasury Wave" landing, which
introduced this keyless Federal Reserve data-portal provider.

## Evidence Snapshot

- Manifest: `crates/tdw-provider-federal-reserve/Cargo.toml`.
- Targets: lib, example (`basic`, http-gated), test (`http_fetcher`,
  http-gated).
- Local deps: `tdw-core`, `tdw-domain` (http-gated) plus `schemars`, `serde`,
  `serde_json`, `thiserror`.
- Reverse deps: `tdw-service-api` (feature `provider-federal-reserve`).
- Features: `default` (offline), `http` (reqwest fetchers; keyless portals,
  live calls additionally gated by `TDW_FEDERAL_RESERVE_LIVE=1`).
- Tests: 12 with `--features http` (fixture normalization for H.6 money
  measures, primary-dealer statistics, FOMC document index; error envelopes)
  plus offline catalog/unit tests.
- Docs/examples: module docs citing the public Federal Reserve data portals,
  `examples/basic.rs`.

## Release Assessment

- Keyless government source (federalreserve.gov data portals); offline by
  default, fixtures recorded from documented response shapes.
- Static clean-room endpoint catalog mirrors the FRED provider pattern; the
  catalog↔ENDPOINTS sync is enforced by
  `federal_reserve_catalog_routes_match_provider_endpoints` in
  `tdw-service-api`.
- Command-injecting dispatch bindings (one fetcher per cluster) keep the
  provider surface small while serving three distinct routes
  (`economy/money_measures`, `fixedincome/government/dealer_stats`,
  `regulators/fed/fomc_documents`).
- No clean-room exception is recorded for this crate.

## Verdict

Ready with follow-ups. Candidate follow-ups: README/ARCHITECTURE docs to match
older provider crates, SOMA central-bank holdings, and additional H.x releases
as later parity stories need them.
