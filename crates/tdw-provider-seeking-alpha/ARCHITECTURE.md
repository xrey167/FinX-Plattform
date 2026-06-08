# tdw-provider-seeking-alpha — Architecture

## Module map

| Module | Gate | Responsibility |
| ------ | ---- | -------------- |
| `lib.rs` | always | Query types, data models (`SeekingAlphaArticle`, `SeekingAlphaRatings`), error enum, validation, the offline `stub_fetch_articles` helper, and the `PROVIDER_ID` / `BASE_URL` / `RAPIDAPI_KEY_ENV` / `RAPIDAPI_HOST` / `MAX_ARTICLE_SIZE` constants. |
| `http_fetcher.rs` | `feature = "http"` | The two `Fetcher` implementations, private RapidAPI wire structs, `map_article`, and the shared `read_api_key` / `build_client` helpers. |

## Traits implemented

Both fetchers implement `tdw_core::Fetcher<Q, D>`:

| Type | `Q` | `D` | `PROVIDER` / `ENDPOINT` |
| ---- | --- | --- | ----------------------- |
| `SeekingAlphaArticlesHttpFetcher` | `SeekingAlphaArticlesQuery` | `SeekingAlphaArticle` | `seeking-alpha` / `articles` |
| `SeekingAlphaRatingsHttpFetcher` | `SeekingAlphaRatingsQuery` | `SeekingAlphaRatings` | `seeking-alpha` / `ratings` |

`PROVIDER` is wired to `crate::PROVIDER_ID` so the registry name stays in one
place.

## Data flow

```
transform_query (Value -> Q)  ->  extract_data (Q -> Bytes, async IO)
                              ->  transform_data (Bytes -> Vec<D>, pure)
```

1. `transform_query` reads `ticker` (and, for articles, `size`) from the JSON
   `Value` and validates via the `lib.rs` constructors. Article `size` must be
   `1..=MAX_ARTICLE_SIZE`.
2. `extract_data` reads `TDW_SEEKING_ALPHA_API_KEY`, sets the `x-rapidapi-key`
   and `x-rapidapi-host` headers, and issues the GET. Non-2xx becomes
   `Error::Provider`.
3. `transform_data` deserialises the RapidAPI JSON:API-style envelope
   (`data[].attributes.*`) into the flat public models.

## Offline / cassette + stub design

Two offline seams exist:

- `stub_fetch_articles` (always compiled) returns deterministic hardcoded
  articles, for callers that want data with no feature and no IO.
- `transform_data` (under `http`) is pure over `Bytes`, so the real parsing
  path is exercised against inline RapidAPI cassettes in tests and in
  `examples/basic.rs` — no network.

`with_base_url(..)` retargets `extract_data` at a local stub server in
integration tests.

## Clean-room invariants

- `#![forbid(unsafe_code)]` via workspace lints.
- No captured RapidAPI responses are committed; only synthetic fixtures shaped
  like the documented JSON:API envelope appear in tests and the example.
- `reqwest` / `tokio` are optional and gated behind `http`; the default build
  is offline and deterministic.
- The crate talks only to the documented RapidAPI Seeking Alpha endpoints via
  the standard RapidAPI headers — no scraping or private APIs.
