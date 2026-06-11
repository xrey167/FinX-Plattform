# tdw-provider-cftc Readiness Worksheet

Generated during the OpenBB-parity P2W5 "Commitments of Traders" landing, which
introduced this keyless CFTC Socrata provider.

## Evidence Snapshot

- Manifest: `crates/tdw-provider-cftc/Cargo.toml`.
- Targets: lib, test (`http_fetcher`, http-gated).
- Local deps: `tdw-core`, `tdw-domain` (http-gated) plus `schemars`, `serde`,
  `serde_json`, `thiserror`.
- Reverse deps: `tdw-service-api` (feature `provider-cftc`).
- Features: `default` (offline), `http` (reqwest fetchers; keyless Socrata API,
  optional `X-App-Token` from `TDW_CFTC_APP_TOKEN`, live calls additionally
  gated by `TDW_CFTC_LIVE=1`).
- Tests: cassette decode + string→number coercion + report-date normalization
  for `regulators/cftc/cot`, distinct-market discovery for
  `regulators/cftc/cot_search`, a JSON-object api-error path, and a
  malformed-JSON error path; plus offline catalog/query unit tests.
- Docs/examples: module docs citing the public CFTC Socrata API
  (`publicreporting.cftc.gov`, resource `6dca-aqww`).

## Release Assessment

- Keyless government source (publicreporting.cftc.gov, the CFTC's own Socrata
  SODA API); offline by default, fixtures recorded from documented response
  shapes.
- Static clean-room endpoint catalog mirrors the FiscalData provider pattern;
  the catalog↔ENDPOINTS sync is enforced by
  `cftc_catalog_routes_match_provider_endpoints` in `tdw-service-api`.
- Socrata's all-string numerics and `report_date_as_yyyy_mm_dd` ISO timestamps
  are normalized at the fetcher boundary, never stored raw.
- No clean-room exception is recorded for this crate; it depends on no OpenBB
  source.

## Verdict

Ready with follow-ups. Candidate follow-ups: README/ARCHITECTURE docs and an
`examples/basic.rs` to match older provider crates, and additional CFTC datasets
(disaggregated / TFF report variants) as later parity stories need them.
