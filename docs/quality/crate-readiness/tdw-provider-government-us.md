# tdw-provider-government-us Readiness Worksheet

Generated during the G003 "WS1b SEC and Treasury Wave" landing, which
introduced this keyless US Treasury FiscalData provider.

## Evidence Snapshot

- Manifest: `crates/tdw-provider-government-us/Cargo.toml`.
- Targets: lib, example (`basic`, http-gated), test (`http_fetcher`,
  http-gated).
- Local deps: `tdw-core`, `tdw-domain` (http-gated) plus `schemars`, `serde`,
  `serde_json`, `thiserror`.
- Reverse deps: `tdw-service-api` (feature `provider-government-us`).
- Features: `default` (offline), `http` (reqwest fetchers; keyless API, live
  calls additionally gated by `TDW_GOVERNMENT_US_LIVE=1`).
- Tests: 10 with `--features http` (fixture normalization for
  `treasury_auctions` and `treasury_prices`, null-sentinel numeric parsing,
  error envelopes) plus offline catalog/unit tests.
- Docs/examples: module docs citing the public FiscalData API, `examples/basic.rs`.

## Release Assessment

- Keyless government source (api.fiscaldata.treasury.gov); offline by default,
  fixtures recorded from documented response shapes.
- Static clean-room endpoint catalog mirrors the FRED provider pattern; the
  catalog↔ENDPOINTS sync is enforced by
  `government_us_catalog_routes_match_provider_endpoints` in `tdw-service-api`.
- String-encoded numerics and null sentinels (`""`, `"null"`, `"*"`) are
  normalized at the fetcher boundary, never stored raw.
- No clean-room exception is recorded for this crate.

## Verdict

Ready with follow-ups. Candidate follow-ups: README/ARCHITECTURE docs to match
older provider crates, and additional FiscalData datasets (debt to the penny,
interest rates) as later parity stories need them.
