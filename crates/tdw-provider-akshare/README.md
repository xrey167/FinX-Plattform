# tdw-provider-akshare

AkShare historical OHLCV provider for the TDW platform. Talks to the community
AkShare REST bridge (`https://akshare.akfamily.xyz`) and returns canonical
`tdw_domain::MarketDataBar` rows via a single `tdw_core::Fetcher`.

| Market           | Endpoint path                       | Venue tag     | Symbol format     |
| ---------------- | ----------------------------------- | ------------- | ----------------- |
| China A-shares   | `/api/public/stock_zh_a_hist`       | `akshare_a`   | 6 ASCII digits    |
| Hong Kong        | `/api/public/stock_hk_hist`         | `akshare_hk`  | 5 ASCII digits    |

Daily bars only (`TimeGranularity::Day`). **No API key is required** — AkShare is
an open bridge — but live calls are still opt-in so CI stays offline.

The crate compiles and tests offline by default: the `AkShareQuery`/
`AkShareMarket` validation types and a deterministic `fetch_stub` helper are
always present; the live `AkShareHttpFetcher` and `reqwest` dependency only exist
under the `http` feature.

## Feature flags

| Feature  | Default | Effect                                                                                          |
| -------- | ------- | ----------------------------------------------------------------------------------------------- |
| `http`   | off     | Pulls in `reqwest`/`tokio`/`tdw-core`/`tdw-domain` and compiles the live `http_fetcher` module. |

```bash
cargo build -p tdw-provider-akshare --features http
```

## Environment variables

| Variable             | Required for           | Purpose                                                       |
| -------------------- | ---------------------- | ------------------------------------------------------------ |
| _(none — no API key)_ | —                      | AkShare needs no credentials.                                |
| `TDW_AKSHARE_LIVE=1` | live integration test  | Opt-in gate; without it the live test skips so CI is offline. |

## Quickstart

```rust
use tdw_core::{Credentials, Fetcher};
use tdw_provider_akshare::AkShareHttpFetcher;

# async fn run() -> tdw_core::Result<()> {
let fetcher = AkShareHttpFetcher::default();
let obb = fetcher
    .fetch(
        serde_json::json!({
            "symbol": "000001",
            "market": "AShares",
            "start_date": "20240101",
            "end_date": "20240131"
        }),
        &Credentials::default(),
    )
    .await?;
for bar in obb.rows {
    println!("{} {} close={}", bar.ts, bar.venue, bar.close);
}
# Ok(())
# }
```

`market` accepts `"AShares"` (default) or `"HongKong"` / `"hk"` / `"HK"`.
Dates are `YYYYMMDD`.

For an offline run that mirrors the cassette tests, see
[`examples/basic.rs`](examples/basic.rs):

```bash
cargo run -p tdw-provider-akshare --features http --example basic
```

## Further reading

- [`ARCHITECTURE.md`](ARCHITECTURE.md) — module map, data flow, clean-room design.
- Platform docs index: [`docs/index.md`](../../docs/index.md).
