# tdw-provider-intrinio Readiness Worksheet

Generated during the OpenBB-parity-total G002 wave, which introduced this
key-gated Intrinio provider to close the documented-but-paid OpenBB command
block (options unusual/snapshots/surface, fundamental data-point attributes,
reported financials, forward estimates).

## Evidence Snapshot

- Manifest: `crates/tdw-provider-intrinio/Cargo.toml`.
- Targets: lib, test (`http_fetcher`, http-gated).
- Local deps: `tdw-core`, `tdw-domain` (http-gated) plus `schemars`, `serde`,
  `serde_json`, `thiserror`.
- Reverse deps: `tdw-service-api` (feature `provider-intrinio`).
- Features: `default` (offline), `http` (reqwest fetchers; the PAID
  `INTRINIO_API_KEY` is read and sent as the `api_key` query parameter; live
  calls are additionally gated by `TDW_INTRINIO_LIVE=1`). The fetchers ship and
  compile without a key; live integration tests skip when the key is absent —
  this is code-level parity for the paid block, data lights up when a key is
  supplied.
- Tests: cassette decode + normalization to the standard `tdw-domain` models for
  each endpoint (no raw Intrinio shape leaks), a `base_url_uses_tls` TLS check,
  malformed-JSON error paths, and offline query-validation unit tests.
- Docs/examples: module docs citing the Intrinio API v2 endpoints
  (`api-v2.intrinio.com`).

## Release Assessment

- Paid-key provider; offline by default, fixtures recorded from the documented
  Intrinio API v2 response shapes (clean-room from Intrinio's own public API
  docs, never OpenBB source).
- Normalizes to shared `tdw-domain` models and is wired as the provider
  candidate on the equity/fundamental attribute routes, equity/estimates
  forward routes, and derivatives/options unusual/snapshots/surface routes; no
  raw Intrinio shape leaks out.
- Dispatchability of the intrinio candidates is enforced by the
  `intrinio_catalog_routes_match_provider_endpoints` test in `tdw-service-api`.
- No clean-room exception is recorded for this crate; it depends on no OpenBB
  source.

## Verdict

Ready with follow-ups. The provider is code-complete and dispatch-wired; live
data requires a paid `INTRINIO_API_KEY`. Candidate follow-ups: README/ARCHITECTURE
docs and a full implied-volatility surface solver over the options chain (the
surface route currently exposes the chain + IV inputs).
