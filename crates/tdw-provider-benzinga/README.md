# tdw-provider-benzinga

Benzinga news and earnings-calendar provider for the TDW platform. Wraps the
Benzinga REST API (`https://api.benzinga.com/api/v2`) and exposes two read
endpoints as `tdw_core::Fetcher` implementations.

| Endpoint           | Fetcher                          | Path                          | Row model               |
| ------------------ | -------------------------------- | ----------------------------- | ----------------------- |
| Company news       | `BenzingaNewsHttpFetcher`        | `GET /news`                   | `BenzingaNewsItem`      |
| Earnings calendar  | `BenzingaEarningsHttpFetcher`    | `GET /calendar/earnings`      | `BenzingaEarningsItem`  |

The crate compiles and tests offline by default: query structs, error types,
domain models, and `stub_fetch_*` helpers are always available; the
network-backed fetchers and `reqwest` dependency only exist under the `http`
feature.

## Feature flags

| Feature  | Default | Effect                                                                                          |
| -------- | ------- | ----------------------------------------------------------------------------------------------- |
| `http`   | off     | Pulls in `reqwest`/`tokio`/`tdw-core`/`tdw-domain` and compiles the live `http_fetcher` module. |

```bash
cargo build -p tdw-provider-benzinga --features http
```

## Environment variables

| Variable                | Required for          | Purpose                                                            |
| ----------------------- | --------------------- | ----------------------------------------------------------------- |
| `TDW_BENZINGA_API_KEY`  | any live HTTP call    | Sent as the `Authorization: Token <key>` header.                  |
| `TDW_BENZINGA_LIVE=1`   | live integration tests | Opt-in gate; without it the live tests skip so CI stays offline. |

## Quickstart

```rust
use tdw_core::{Credentials, Fetcher};
use tdw_provider_benzinga::BenzingaNewsHttpFetcher;

# async fn run() -> tdw_core::Result<()> {
// Requires TDW_BENZINGA_API_KEY in the environment.
let fetcher = BenzingaNewsHttpFetcher::default();
let obb = fetcher
    .fetch(
        serde_json::json!({ "symbol": "AAPL", "page_size": 5 }),
        &Credentials::default(),
    )
    .await?;
for item in obb.rows {
    println!("{} — {}", item.published_date, item.title);
}
# Ok(())
# }
```

`symbol` is required for both fetchers; news also takes `page_size` (1..=100),
earnings takes `date_from`/`date_to` (`YYYY-MM-DD`).

For an offline run that mirrors the cassette tests, see
[`examples/basic.rs`](examples/basic.rs):

```bash
cargo run -p tdw-provider-benzinga --features http --example basic
```

## Further reading

- [`ARCHITECTURE.md`](ARCHITECTURE.md) — module map, data flow, clean-room design.
- Platform docs index: [`docs/index.md`](../../docs/index.md).
