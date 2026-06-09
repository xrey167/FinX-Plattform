//! Live websocket test for the Binance trade streamer.
//!
//! Gated by `--features ws` (no tokio-tungstenite dep otherwise). The live
//! subscribe is additionally gated by `TDW_BINANCE_LIVE=1` so unattended CI
//! never opens a socket; BTCUSDT trades continuously, so the first frame
//! arrives within seconds.

#![cfg(feature = "ws")]

use futures_util::StreamExt;
use tdw_core::{Credentials, Streamer};
use tdw_provider_binance::{BinanceTradeQuery, BinanceTradeStreamer};

#[tokio::test]
async fn live_binance_ws_streams_a_trade_tick_when_env_var_set() {
    if std::env::var("TDW_BINANCE_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_BINANCE_LIVE != 1; skipping live Binance websocket test");
        return;
    }

    let streamer = BinanceTradeStreamer;
    let query = BinanceTradeQuery::new("BTCUSDT").expect("valid symbol");
    let mut stream = streamer
        .subscribe(query, &Credentials::default())
        .await
        .expect("live ws subscribe must succeed");

    let tick = tokio::time::timeout(std::time::Duration::from_secs(30), stream.next())
        .await
        .expect("a trade tick must arrive within 30s")
        .expect("stream must not end before the first tick")
        .expect("first frame must decode");

    assert_eq!(tick.symbol, "BTCUSDT");
    assert_eq!(tick.venue, "BINANCE");
    assert!(tick.price > 0.0, "trade price must be positive");
    assert!(tick.size > 0.0, "trade size must be positive");
}
