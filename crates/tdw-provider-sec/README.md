# tdw-provider-sec

SEC EDGAR data provider for the TDW (Trading Data Warehouse) platform.

This crate exposes offline query/validation types plus two real HTTP
`Fetcher`s for the public **SEC EDGAR** data API (`https://data.sec.gov`).
No API key is required — EDGAR is a public endpoint — but live network
access is gated behind a Cargo feature so the workspace test set stays
fully offline by default.

## What it provides

- `SecHistoricalQuery` / `SecFilingsQuery` — validated, normalised query
  types (symbol upper-cased, CIK digit-checked and zero-padded to 10).
- `SecFilingsHttpFetcher` — `GET /submissions/CIK{cik}.json` → `SecFiling`
  rows (accession number, form, filing date).
- `SecXbrlHttpFetcher` — `GET /api/xbrl/companyfacts/CIK{cik}.json` →
  `MarketDataBar` rows built from annual (`10-K`) `us-gaap/Revenue` facts.

## Feature flags

| Feature | Default | Effect |
| ------- | ------- | ------ |
| `http`  | off     | Compiles the `http_fetcher` module and pulls in `reqwest`, `tokio`, `async-trait`, `bytes`, `serde_json`, `tdw-core`, `tdw-domain`. |

With `http` **off**, only the pure query/validation types and error enum
compile — useful for callers that just need the schema.

## Environment variables

| Variable | Purpose |
| -------- | ------- |
| `TDW_SEC_LIVE=1` | Opt-in switch read by the integration test to perform a real network call. Unset → the live test is skipped. |

No API key variable exists: SEC EDGAR is unauthenticated. The fetcher sends
a descriptive `User-Agent` and sleeps 100 ms after each request to honour
EDGAR's ~10 req/sec soft limit.

## Quickstart

```rust
use tdw_provider_sec::{SecFilingsQuery, SecHistoricalQuery};

// Offline: build and validate queries (no feature needed).
let filings = SecFilingsQuery::new("320193")?; // Apple, CIK 320193
assert_eq!(filings.padded_cik(), "0000320193");

let hist = SecHistoricalQuery::new("aapl")?;
assert_eq!(hist.symbol, "AAPL");
# Ok::<(), tdw_provider_sec::SecProviderError>(())
```

With the `http` feature, drive a fetcher end-to-end:

```rust,ignore
use tdw_core::{Credentials, Fetcher};
use tdw_provider_sec::SecFilingsHttpFetcher;

let fetcher = SecFilingsHttpFetcher::default();
let obb = fetcher
    .fetch(serde_json::json!({ "cik": "320193" }), &Credentials::default())
    .await?;
println!("{} filings", obb.rows.len());
```

## Example

```bash
cargo run -p tdw-provider-sec --example basic --features http
```

See [`examples/basic.rs`](examples/basic.rs) — it runs the `transform_data`
path against an inline EDGAR fixture, no network required.

## Configuration

Provider registration and feature-gate conventions are documented in
[`docs/CONFIGURATION.md`](../../docs/CONFIGURATION.md).
