# tdw-provider-imf Readiness Worksheet

Generated during the OpenBB-parity P3W3 "IMF macro time-series" landing, which
introduced this keyless IMF Data Services SDMX-JSON provider. IMF adds
international / cross-country macro coverage the US-centric FRED cluster does not
provide.

## Evidence Snapshot

- Manifest: `crates/tdw-provider-imf/Cargo.toml`.
- Targets: lib, test (`http_fetcher`, http-gated).
- Local deps: `tdw-core`, `tdw-domain` (http-gated) plus `schemars`, `serde`,
  `serde_json`, `thiserror`.
- Reverse deps: `tdw-service-api` (feature `provider-imf`).
- Features: `default` (offline), `http` (reqwest fetcher; keyless IMF SDMX-JSON
  API, live calls additionally gated by `TDW_IMF_LIVE=1`).
- Tests: cassette decode covering the single-`Series`/single-`Obs` shape, the
  array-of-`Series`/array-of-`Obs` shape, the string→`f64` `@OBS_VALUE`
  coercion (incl. a missing-value `None` row), an empty-envelope→no-rows path,
  plus malformed-JSON and non-object-root error paths; offline catalog/query
  unit tests cover command resolution and SDMX-key validation.
- Docs/examples: module docs citing the public IMF Data Services SDMX-JSON API
  (`dataservices.imf.org`, `CompactData/{DatabaseID}/{Key}`).

## Release Assessment

- Keyless government source (IMF Data Services SDMX-JSON `CompactData`); offline
  by default, fixtures recorded from documented response shapes.
- Static clean-room endpoint catalog maps `economy/imf/*` commands to the public
  SDMX database ids (`IFS`, `DOT`, `BOP`); the catalog↔`ENDPOINTS` sync is
  enforced by `imf_catalog_routes_match_provider_endpoints` in `tdw-service-api`.
- The deeply-nested `CompactData` envelope (single-or-array `Series`/`Obs`,
  all-string `@OBS_VALUE`) is normalized to `tdw_domain::MacroSeries` at the
  fetcher boundary, never stored raw; `@REF_AREA` populates `MacroSeries.country`
  so cross-country series are queryable.
- No clean-room exception is recorded for this crate; it is implemented solely
  from the IMF Data Services public documentation and depends on no OpenBB
  source.

## Verdict

Ready with follow-ups. Candidate follow-ups: README/ARCHITECTURE docs and an
`examples/basic.rs` to match older provider crates, an IMF `DataStructure`
metadata/discovery route, and additional SDMX databases (e.g. `GFSR`, `COFER`)
as later parity stories need them.
