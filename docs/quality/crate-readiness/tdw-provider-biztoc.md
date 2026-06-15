# tdw-provider-biztoc Readiness Worksheet

Generated during the OpenBB-parity P4W12 close-out, which introduced this
free-key BizToc business/world-news provider.

## Evidence Snapshot

- Manifest: `crates/tdw-provider-biztoc/Cargo.toml`.
- Targets: lib, test (`http_fetcher`, http-gated).
- Local deps: `tdw-core`, `tdw-domain` (http-gated) plus `schemars`, `serde`,
  `serde_json`, `thiserror`.
- Reverse deps: `tdw-service-api` (feature `provider-biztoc`).
- Features: `default` (offline), `http` (reqwest fetcher; free RapidAPI key
  `BIZTOC_API_KEY` read via `read_required_key`, sent as `X-RapidAPI-Key` +
  `X-RapidAPI-Host` headers; live calls additionally gated by
  `TDW_BIZTOC_LIVE=1`).
- Tests: cassette decode + normalization to `tdw_domain::NewsArticle` for the
  world-news stream (id/title/url/created/excerpt/domain/tags mapping, blank-id
  URL fallback), a `base_url_uses_tls` TLS check, a page-size cap test, a
  malformed-JSON error path, invalid-page-size rejection, plus offline query
  validation unit tests.
- Docs/examples: module docs citing the BizToc RapidAPI endpoint
  (`biztoc.p.rapidapi.com`).

## Release Assessment

- Free-key aggregator served via RapidAPI; offline by default, fixtures recorded
  from the documented BizToc RapidAPI response shape.
- Normalizes to the shared `tdw_domain::NewsArticle` model and is wired as a
  second provider candidate on the existing `news/world` route (alongside
  benzinga); no raw BizToc shape leaks out.
- Dispatchability of the `news/world` biztoc candidate is enforced by the
  `catalog_candidates_all_dispatchable_under_full_providers` test in
  `tdw-service-api`.
- No clean-room exception is recorded for this crate; it depends on no OpenBB
  source.

## Verdict

Ready with follow-ups. Candidate follow-ups: README/ARCHITECTURE docs and an
`examples/basic.rs` to match older provider crates, and the BizToc
`/news/sources` and `/search?q=` endpoints as later parity stories need them.
