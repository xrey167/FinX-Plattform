# tdw-provider-geckoterminal

Market-data provider for **GeckoTerminal**, the on-chain DEX analytics API.
Exposes typed query models, a `DexPool` data model, and a `tdw_core::Fetcher`
implementation for single-pool lookups, plus standalone helpers for trending
and token pools.

- **Vendor:** GeckoTerminal — public REST API v2
- **Base URL:** `https://api.geckoterminal.com/api/v2`
- **Endpoint:** `pool` — `GET /networks/{network}/pools/{pool_address}`
- **Auth:** none (public API). An `Accept: application/json;version=20230302`
  header pins the API version.

## Feature flags

| Feature | Default | Effect |
| ------- | ------- | ------ |
| `http`  | off     | Compiles `GeckoTerminalHttpFetcher` (and the `fetch_trending_raw` / `fetch_token_pools_raw` / `parse_pool_list` helpers) and pulls in `reqwest`, `tokio`, `async-trait`, `bytes`, `tdw-core`, `tdw-domain`. |

With `http` off the typed query models, network/address validation, the
`DexPool` model, and the `mock_pool` stub are available with no network
dependencies.

## Environment variables

| Variable                  | Required | Purpose |
| ------------------------- | -------- | ------- |
| `TDW_GECKOTERMINAL_LIVE`  | no       | Set to `1` to enable the live network integration test. |

No API key is required.

## Quickstart

Offline (default features):

```rust
use tdw_provider_geckoterminal::mock_pool;

let pool = mock_pool("eth", "0x88e6a0c2ddd26feeb64f039a2c41296fcb3f5640")?;
println!("{} {} reserve_usd={:?}", pool.network, pool.name, pool.reserve_in_usd);
# Ok::<(), tdw_provider_geckoterminal::GeckoTerminalError>(())
```

Live HTTP (requires `--features http`):

```rust,ignore
use tdw_core::{Credentials, Fetcher};
use tdw_provider_geckoterminal::GeckoTerminalHttpFetcher;

let fetcher = GeckoTerminalHttpFetcher::default();
let pools = fetcher
    .fetch(serde_json::json!({
        "network": "eth",
        "pool_address": "0x88e6a0c2ddd26feeb64f039a2c41296fcb3f5640"
    }), &Credentials::default())
    .await?;
```

## Example

```bash
cargo run -p tdw-provider-geckoterminal --example basic --features http
```

## Configuration

See [`docs/CONFIGURATION.md`](../../docs/CONFIGURATION.md) for the workspace-wide
provider feature-gating model and env-var conventions.
