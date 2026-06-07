//! tdw-proto example: construct, encode, and decode market-data messages.
//! Pure in-process protobuf round-trips — no network, no `protoc`, no codegen.
//!
//! Run with:
//!
//! ```bash
//! cargo run -p tdw-proto --example basic
//! ```

use prost::Message;
use tdw_proto::{MarketDataEnvelope, OhlcvBar, Payload, Tick, TradeSide};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // A default (all-zero) message encodes to zero bytes under proto3.
    let empty = OhlcvBar::default();
    println!("default OhlcvBar -> {} bytes", empty.encode_to_vec().len());

    // Wrap an OHLCV bar in an envelope and round-trip it.
    let bar = OhlcvBar {
        symbol: "AAPL".to_string(),
        provider: "polygon".to_string(),
        timeframe: "1m".to_string(),
        ts_ns: 1_700_000_000_000_000_000,
        open: 150.25,
        high: 151.50,
        low: 149.80,
        close: 151.00,
        volume: 1_234_567.0,
        vwap: 150.75,
        trade_count: 42_000,
    };
    let env = MarketDataEnvelope {
        provider: "polygon".to_string(),
        symbol: "AAPL".to_string(),
        ingestion_id: "01HZ000000000000000000001".to_string(),
        received_at_ns: 1_700_000_000_000_100_000,
        payload: Some(Payload::Bar(bar)),
    };

    let mut buf = Vec::new();
    env.encode(&mut buf)?;
    println!("encoded envelope -> {} bytes", buf.len());

    let decoded = MarketDataEnvelope::decode(buf.as_slice())?;
    match decoded.payload {
        Some(Payload::Bar(b)) => {
            println!("decoded bar {} close={}", b.symbol, b.close);
        }
        other => return Err(format!("expected Payload::Bar, got {other:?}").into()),
    }

    // Tick payload + enum round-trip.
    let tick = Tick {
        symbol: "ETH-USD".to_string(),
        provider: "binance".to_string(),
        ts_ns: 1_700_000_001_000_000_000,
        price: 2_048.50,
        size: 0.5,
        side: TradeSide::Buy as i32,
        trade_id: "trade-abc-123".to_string(),
    };
    let tick_bytes = tick.encode_to_vec();
    let decoded_tick = Tick::decode(tick_bytes.as_slice())?;
    println!(
        "decoded tick side = {}",
        TradeSide::try_from(decoded_tick.side)
            .map(|s| s.as_str_name())
            .unwrap_or("INVALID")
    );

    Ok(())
}
