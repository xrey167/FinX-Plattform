#![forbid(unsafe_code)]
#![doc = "Generated protobuf bindings for TDW market data types."]

#![deny(clippy::pedantic, clippy::nursery)]
pub mod finance {
    // Vendored prost-build output (see `finance.gen.rs`). Included via a relative
    // path so the crate needs no build-time codegen or system `protoc`.
    include!("finance.gen.rs");
}

pub use finance::{
    MarketDataEnvelope, OhlcvBar, OrderBookSnapshot, PriceLevel, Tick, TradeSide,
    market_data_envelope::Payload,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn market_data_envelope_round_trips_ohlcv_payload() {
        let envelope = MarketDataEnvelope {
            provider: "polygon".to_string(),
            symbol: "AAPL".to_string(),
            ingestion_id: "01HX0000000000000000000000".to_string(),
            received_at_ns: 1_700_000_000_000,
            payload: Some(Payload::Bar(OhlcvBar {
                symbol: "AAPL".to_string(),
                provider: "polygon".to_string(),
                timeframe: "1d".to_string(),
                ts_ns: 1_700_000_000_000,
                open: 180.0,
                high: 181.0,
                low: 179.5,
                close: 180.5,
                volume: 42_000.0,
                vwap: 180.25,
                trade_count: 125,
            })),
        };

        let payload = envelope.payload.expect("payload");

        let Payload::Bar(bar) = payload else {
            panic!("expected OHLCV bar payload");
        };
        assert_eq!(bar.symbol, "AAPL");
        assert!((bar.close - 180.5).abs() < f64::EPSILON);
    }

    #[test]
    fn order_book_snapshot_holds_bid_and_ask_levels() {
        let snapshot = OrderBookSnapshot {
            symbol: "BTCUSDT".to_string(),
            provider: "binance".to_string(),
            ts_ns: 1_700_000_000_001,
            bids: vec![PriceLevel {
                price: 50_000.0,
                quantity: 1.25,
            }],
            asks: vec![PriceLevel {
                price: 50_001.0,
                quantity: 0.75,
            }],
            sequence: 42,
        };

        assert!((snapshot.bids[0].price - 50_000.0).abs() < f64::EPSILON);
        assert!((snapshot.asks[0].quantity - 0.75).abs() < f64::EPSILON);
    }
}
