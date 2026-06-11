# tdw-provider-famafrench Readiness Worksheet

Generated during the OpenBB-parity P2W6 "Ken French research factors" landing,
which introduced this keyless Ken French Data Library provider.

## Evidence Snapshot

- Manifest: `crates/tdw-provider-famafrench/Cargo.toml`.
- Targets: lib, test (`http_fetcher`, http-gated), example (`basic`, http-gated).
- Local deps: `tdw-core`, `tdw-domain` (non-optional — the offline factor-table
  parser returns `tdw_domain::FactorReturn`) plus `schemars`, `serde`,
  `serde_json`, `thiserror`.
- Reverse deps: `tdw-service-api` (feature `provider-famafrench`).
- Features: `default` (offline parser + catalog), `http` (reqwest fetcher +
  pure-Rust `zip` in-memory unzip; keyless ftp tree, live calls additionally
  gated by `TDW_FAMAFRENCH_LIVE=1`).
- Dependency policy: the data ships as a ZIP-of-CSV. The `zip` dep is pinned to
  `default-features = false, features = ["deflate"]`, pulling `flate2` with the
  pure-Rust `miniz_oxide` backend — NO `libz-sys` / system zlib / C toolchain.
  `cargo deny check` records no new denial; the only effect is a duplicate `zip`
  version warning (2.4.2 alongside a pre-existing 7.2.0), which is allowed.
- Tests: offline parser tests over hardcoded Ken French CSV fixtures (the exact
  header-preamble + factor-column-header + percent-valued date rows layout) —
  asserting the percent→fraction conversion, the header skip, the missing-value
  sentinels (`-99.99` / `-999`), monthly-date normalization, the single-column
  momentum shape, and the appended-annual-table cutoff — plus a malformed
  no-header error path; the http-gated cassette test builds an in-memory ZIP and
  drives the fetcher's unzip + parse path, plus a non-ZIP-bytes error path and an
  env-gated live test.
- Docs/examples: module docs and the catalog cite the Ken French Data Library's
  own public documentation (`mba.tuck.dartmouth.edu/.../data_library.html` and
  the `ftp/*_CSV.zip` archive names); `examples/basic.rs` shows the offline parse
  path.

## Release Assessment

- Keyless academic source (the Ken French Data Library's public ftp tree of
  ZIP-of-CSV research-factor archives); offline by default, fixtures hand-built
  from the documented file layout.
- Raw ZIP/CSV shapes are normalized to `tdw_domain::FactorReturn` at the parser
  boundary (percent → decimal fraction, missing-value sentinels → `None`), never
  stored raw.
- Static clean-room dataset table maps `(factor_set, frequency)` selections onto
  the documented archive/member names; the catalog↔`ENDPOINTS` sync is enforced
  by `famafrench_catalog_routes_match_provider_endpoints` in `tdw-service-api`.
- No clean-room exception is recorded for this crate; it depends on no OpenBB
  source — the implementation derives only from the Data Library's own docs.

## Verdict

Ready with follow-ups. Candidate follow-ups: README/ARCHITECTURE docs to match
older provider crates, and exposing the regional / portfolio research datasets
(e.g. developed/emerging-market factors, portfolios-formed-on tables) as later
parity stories need them.
