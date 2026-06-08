# tdw-provider-tmx

TMX (Toronto/TSX) Money data provider for the TDW (Trading Data Warehouse)
platform.

Exposes offline query/validation types, a deterministic mock fetcher, and a
pure JSON parser — plus two real HTTP `Fetcher`s for the TMX Money quote API.
Network access is feature-gated so the workspace test set runs fully offline.

## What it provides

- `TmxQuoteQuery` / `TmxBatchQuoteQuery` — validated query types
  (batch capped at `BATCH_MAX_SYMBOLS = 10`).
- `TmxQuote` — the equity quote data model.
- `TmxMockQuoteFetcher` — synchronous, deterministic offline fetcher
  (`fetch_mock`) that never touches the network.
- `parse_quote_response` — pure helper that decodes a TMX `getquote` JSON
  body into `Vec<TmxQuote>`.
- `TmxHttpQuoteFetcher` / `TmxHttpBatchQuoteFetcher` — real HTTP fetchers
  (under `http`) producing `EquityHistoricalData`.

## Feature flags

| Feature | Default | Effect |
| ------- | ------- | ------ |
| `http`  | off     | Compiles `http_fetcher` and pulls in `reqwest`, `tokio`, `async-trait`, `bytes`, `tdw-core`, `tdw-domain`. |

With `http` off, the query types, `TmxQuote`, `TmxMockQuoteFetcher`, and
`parse_quote_response` are all still available.

## Environment variables

None. The TMX Money quote endpoint used here is public; the only live-call
gating is the `http` feature plus the integration test's own switch.

## Quickstart

```rust
use serde_json::json;
use tdw_provider_tmx::{TmxMockQuoteFetcher, TmxQuoteQuery};

// Validate a query.
let q = TmxQuoteQuery::from_params(&json!({ "symbol": "TD" }))?;
assert_eq!(q.symbol, "TD");

// Deterministic offline fetch (no network, no feature).
let quotes = TmxMockQuoteFetcher::fetch_mock(&json!({ "symbol": "TD" }))?;
assert_eq!(quotes[0].exchange, "TSX");
# Ok::<(), tdw_provider_tmx::TmxError>(())
```

With the `http` feature, drive the real fetcher:

```rust,ignore
use tdw_core::{Credentials, Fetcher};
use tdw_provider_tmx::TmxHttpQuoteFetcher;

let fetcher = TmxHttpQuoteFetcher::default();
let obb = fetcher
    .fetch(serde_json::json!({ "symbol": "TD" }), &Credentials::default())
    .await?;
println!("{} rows", obb.rows.len());
```

## Example

```bash
cargo run -p tdw-provider-tmx --example basic
```

See [`examples/basic.rs`](examples/basic.rs) — uses the offline mock fetcher
and the pure `parse_quote_response` helper, so it needs no feature flags or
network.

## Configuration

Provider registration and feature-gate conventions are documented in
[`docs/CONFIGURATION.md`](../../docs/CONFIGURATION.md).
