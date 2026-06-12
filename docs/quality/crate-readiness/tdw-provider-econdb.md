# tdw-provider-econdb Readiness Worksheet

Generated during the OpenBB-parity P3W4 "EconDB macro time-series" landing, which
introduced this optional-token EconDB public-API provider. EconDB adds
standardized cross-country macro series (e.g. real GDP by country) fetched by
ticker, complementing the US-centric FRED cluster and the IMF SDMX cluster.

## Evidence Snapshot

- Manifest: `crates/tdw-provider-econdb/Cargo.toml`.
- Targets: lib, test (`http_fetcher`, http-gated).
- Local deps: `tdw-core`, `tdw-domain` (http-gated) plus `schemars`, `serde`,
  `serde_json`, `thiserror`.
- Reverse deps: `tdw-service-api` (feature `provider-econdb`).
- Features: `default` (offline), `http` (reqwest fetcher; keyless EconDB public
  series API, optional token `TDW_ECONDB_API_KEY`; live calls additionally gated
  by `TDW_ECONDB_LIVE=1`).
- Tests: cassette decode covering BOTH documented `data` shapes (the parallel
  `dates`/`values` arrays AND the list-of-`{date,value}` records), the
  numeric-string→`f64` coercion, a null/missing value → `None` row, a `limit`
  cap, an empty-envelope (no `data`) → no-rows path, plus malformed-JSON and
  non-object-root error paths; offline catalog/query unit tests cover command
  resolution and ticker validation (path-injection rejects). `base_url_uses_tls`.
- Docs/examples: module docs citing the public EconDB API
  (`www.econdb.com/api`, `series/{TICKER}/?format=json`).

## Release Assessment

- Optional-token public source (EconDB public series API); offline by default,
  fixtures recorded from the documented response shapes. The optional token is
  read through the shared `tdw_core::http_support::read_optional_key` helper
  (`TDW_ECONDB_API_KEY`), never `std::env::var` directly, and is omitted when
  unset so public series work keyless (mirroring the CFTC optional-token path).
- Static clean-room endpoint catalog maps the single `economy/econdb/series`
  command to the EconDB series endpoint; the catalog↔`ENDPOINTS` sync is enforced
  by `econdb_catalog_routes_match_provider_endpoints` in `tdw-service-api`.
- The series response (metadata + a `data` payload rendered as either parallel
  `dates`/`values` arrays or a list of `{date,value}` records, with values as
  JSON numbers or numeric strings) is normalized defensively to
  `tdw_domain::MacroSeries` at the fetcher boundary, never stored raw;
  `geography` populates `MacroSeries.country` so cross-country series stay
  queryable. A caller-supplied `ticker` is validated to a safe grammar (ASCII
  alphanumerics + `_ - .`) to close path-injection on the `{TICKER}` segment.
- The `BASE_URL` uses TLS (`https`); a `base_url_uses_tls` regression test guards
  against a plaintext transport regression.
- No clean-room exception is recorded for this crate; it is implemented solely
  from the EconDB public API documentation (`econdb.com/api`) and depends on no
  OpenBB source.

## Verdict

Ready with follow-ups. Candidate follow-ups: README/ARCHITECTURE docs and an
`examples/basic.rs` to match older provider crates, an EconDB series-discovery /
country-profile route, and additional standardized indicator routes as later
parity stories verify their public contracts.
