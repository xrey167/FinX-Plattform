# tdw-provider-seeking-alpha

Seeking Alpha data provider for the TDW (Trading Data Warehouse) platform.

Exposes offline query/validation types plus two real HTTP `Fetcher`s for the
**Seeking Alpha** API served via RapidAPI
(`https://seeking-alpha.p.rapidapi.com`): analyst articles and stock ratings.
Network access is feature-gated so the workspace test set runs fully offline.

## What it provides

- `SeekingAlphaArticlesQuery` / `SeekingAlphaRatingsQuery` — validated queries
  (ticker normalised; article `size` clamped to `MAX_ARTICLE_SIZE = 40`).
- `SeekingAlphaArticlesHttpFetcher` — `GET /analysis/v2/list` →
  `SeekingAlphaArticle` rows.
- `SeekingAlphaRatingsHttpFetcher` — `GET /symbols/v1/summary` →
  `SeekingAlphaRatings` rows.

## Feature flags

| Feature | Default | Effect |
| ------- | ------- | ------ |
| `http`  | off     | Compiles `http_fetcher` and pulls in `reqwest`, `tokio`, `async-trait`, `bytes`, `tdw-core`, `tdw-domain`. |

With `http` off, only the query/validation types, data models, and error enum
compile.

## Environment variables

| Variable | Purpose |
| -------- | ------- |
| `TDW_SEEKING_ALPHA_API_KEY` | RapidAPI key, sent as the `x-rapidapi-key` header. Required for live calls. Exported as `RAPIDAPI_KEY_ENV`. |

The RapidAPI host (`seeking-alpha.p.rapidapi.com`) is sent as the
`x-rapidapi-host` header and exported as `RAPIDAPI_HOST`.

## Quickstart

```rust
use tdw_provider_seeking_alpha::{SeekingAlphaArticlesQuery, SeekingAlphaRatingsQuery};

let articles = SeekingAlphaArticlesQuery::new("aapl", 5)?;
assert_eq!(articles.ticker, "AAPL");

let ratings = SeekingAlphaRatingsQuery::new("msft")?;
assert_eq!(ratings.ticker, "MSFT");
# Ok::<(), tdw_provider_seeking_alpha::SeekingAlphaProviderError>(())
```

With the `http` feature:

```rust,ignore
use tdw_core::{Credentials, Fetcher};
use tdw_provider_seeking_alpha::SeekingAlphaArticlesHttpFetcher;

std::env::set_var("TDW_SEEKING_ALPHA_API_KEY", "…");
let fetcher = SeekingAlphaArticlesHttpFetcher::default();
let obb = fetcher
    .fetch(serde_json::json!({ "ticker": "AAPL", "size": 5 }), &Credentials::default())
    .await?;
println!("{} articles", obb.rows.len());
```

## Example

```bash
cargo run -p tdw-provider-seeking-alpha --example basic --features http
```

See [`examples/basic.rs`](examples/basic.rs) — runs `transform_data` against
inline RapidAPI fixtures, no network or key required.

## Configuration

Provider registration and feature-gate conventions are documented in
[`docs/CONFIGURATION.md`](../../docs/CONFIGURATION.md).
