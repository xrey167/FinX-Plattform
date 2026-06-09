# tdw-provider-binance

Binance crypto market-data provider for the TDW platform. Exposes both a REST
ticker-price `tdw_core::Fetcher` and a trade-stream `tdw_core::Streamer`.

| Surface          | Type                              | Endpoint                                            | Row / item        |
| ---------------- | --------------------------------- | --------------------------------------------------- | ----------------- |
| Ticker price     | `BinanceHttpTickerPriceFetcher`   | `GET https://api.binance.com/api/v3/ticker/price`   | `BinanceTickerPrice` |
| Trade stream     | `BinanceTradeStreamer`            | `wss://stream.binance.com:9443/ws/{symbol}@trade`   | `tdw_domain::Tick`   |

No API key is required (both surfaces use public Binance market data). The trade
decoder `decode_trade_frame` is a pure, always-available function — the live
socket is only opened under the `ws` feature; otherwise `subscribe`/`snapshot`
return a deterministic single tick so the workspace test set stays offline.

## Feature flags

| Feature  | Default | Effect                                                                            |
| -------- | ------- | -------------------------------------------------------------------------------- |
| `http`   | off     | Pulls in `reqwest`/`tokio` and compiles `BinanceHttpTickerPriceFetcher`.          |
| `ws`     | off     | Pulls in `tokio-tungstenite`/`futures-util`/`tokio` and the live `subscribe` path. |

Note: unlike most provider crates, `tdw-core`/`tdw-domain`/`serde_json` are
non-optional here because the streamer and decoder are always compiled.

```bash
cargo build -p tdw-provider-binance --features http
cargo build -p tdw-provider-binance --features ws
```

## Environment variables

| Variable             | Required for          | Purpose                                                          |
| -------------------- | --------------------- | -------------------------------------------------------------- |
| _(none — no API key)_ | —                     | Public market data needs no credentials.                       |
| `TDW_BINANCE_LIVE=1` | live HTTP integration test | Opt-in gate; without it the live ticker test skips.       |

## Quickstart

```rust
use tdw_core::{Credentials, Fetcher};
use tdw_provider_binance::BinanceHttpTickerPriceFetcher;

# async fn run() -> tdw_core::Result<()> {
let fetcher = BinanceHttpTickerPriceFetcher::default();
let obb = fetcher
    .fetch(serde_json::json!({ "symbol": "BTCUSDT" }), &Credentials::default())
    .await?;
for row in obb.rows {
    println!("{} = {}", row.symbol, row.price);
}
# Ok(())
# }
```

Decode a recorded trade frame entirely offline (no feature needed):

```rust
use tdw_provider_binance::decode_trade_frame;

let frame = r#"{"e":"trade","s":"BTCUSDT","p":"68000.50","q":"0.01","T":1700000000000}"#;
let ticks = decode_trade_frame(frame).expect("frame decodes");
assert_eq!(ticks[0].symbol, "BTCUSDT");
```

For an offline run that mirrors the cassette + decoder tests, see
[`examples/basic.rs`](examples/basic.rs):

```bash
cargo run -p tdw-provider-binance --features http --example basic
```

## Further reading

- [`ARCHITECTURE.md`](ARCHITECTURE.md) — module map, data flow, clean-room design.
- Platform docs index: [`docs/index.md`](../../docs/index.md).
