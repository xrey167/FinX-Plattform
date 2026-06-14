# tdw-provider-congress-gov Readiness Worksheet

Generated during the OpenBB-parity P4W12 close-out, which introduced this
free-key `congress.gov` US legislative-data provider.

## Evidence Snapshot

- Manifest: `crates/tdw-provider-congress-gov/Cargo.toml`.
- Targets: lib, test (`http_fetcher`, http-gated).
- Local deps: `tdw-core`, `tdw-domain` (http-gated) plus `schemars`, `serde`,
  `serde_json`, `thiserror`.
- Reverse deps: `tdw-service-api` (feature `provider-congress-gov`).
- Features: `default` (offline), `http` (reqwest fetchers; free api.data.gov key
  `CONGRESS_GOV_API_KEY` read via `read_required_key`, live calls additionally
  gated by `TDW_CONGRESS_GOV_LIVE=1`).
- Tests: cassette decode + normalization for `uscongress/bills` (list),
  `uscongress/bill_info` (sponsor/policy-area mapping), and
  `uscongress/bill_text_urls` (text-version/format flattening); a
  `base_url_uses_tls` TLS check; a malformed-JSON error path; unknown-command
  and missing-bill_number rejection; plus offline catalog/query unit tests.
- Docs/examples: module docs citing the public `congress.gov` v3 API
  (`api.congress.gov/v3`).

## Release Assessment

- Free-key government source (`api.congress.gov/v3`, key from api.data.gov);
  offline by default, fixtures recorded from documented response shapes.
- Static clean-room endpoint catalog mirrors the CFTC provider pattern; the
  catalog↔ENDPOINTS sync is enforced by
  `congress_gov_catalog_routes_match_provider_endpoints` in `tdw-service-api`.
- The three routes normalize to the standard `tdw_domain::CongressBill` /
  `BillTextUrl` models; no raw `congress.gov` shape leaks out.
- No clean-room exception is recorded for this crate; it depends on no OpenBB
  source.

## Verdict

Ready with follow-ups. Candidate follow-ups: README/ARCHITECTURE docs and an
`examples/basic.rs` to match older provider crates, and additional congress.gov
datasets (amendments, members, committee reports) as later parity stories need
them.
