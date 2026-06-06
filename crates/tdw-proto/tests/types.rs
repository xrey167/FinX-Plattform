//! Characterization tests for the prost-generated market-data types in
//! `tdw-proto`. Each test constructs a message, encodes it to bytes via
//! `prost::Message::encode`, then decodes with `prost::Message::decode` and
//! asserts field preservation.  Empty-message behaviour is also checked because
//! proto3 encoding elides default-valued fields.

use prost::Message;
use tdw_proto::{
    MarketDataEnvelope, OhlcvBar, OrderBookSnapshot, Payload, PriceLevel, Tick, TradeSide,
};

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn encode<M: Message>(msg: &M) -> Vec<u8> {
    let mut buf = Vec::new();
    msg.encode(&mut buf)
        .expect("encode must not fail for valid messages");
    buf
}

fn decode<M: Message + Default>(bytes: &[u8]) -> M {
    M::decode(bytes).expect("decode must not fail for well-formed bytes")
}

// ---------------------------------------------------------------------------
// OhlcvBar
// ---------------------------------------------------------------------------

#[test]
fn ohlcv_bar_default_encodes_to_empty_bytes() {
    let bar = OhlcvBar::default();
    let bytes = encode(&bar);
    // All fields are proto3 defaults → wire representation is empty.
    assert!(
        bytes.is_empty(),
        "default OhlcvBar should produce zero bytes"
    );
}

#[test]
fn ohlcv_bar_round_trip_preserves_fields() {
    let bar = OhlcvBar {
        symbol: "AAPL".to_string(),
        provider: "polygon".to_string(),
        timeframe: "1m".to_string(),
        ts_ns: 1_700_000_000_000_000_000_i64,
        open: 150.25,
        high: 151.50,
        low: 149.80,
        close: 151.00,
        volume: 1_234_567.0,
        vwap: 150.75,
        trade_count: 42_000,
    };

    let decoded: OhlcvBar = decode(&encode(&bar));

    assert_eq!(decoded.symbol, bar.symbol, "symbol");
    assert_eq!(decoded.provider, bar.provider, "provider");
    assert_eq!(decoded.timeframe, bar.timeframe, "timeframe");
    assert_eq!(decoded.ts_ns, bar.ts_ns, "ts_ns");
    assert_eq!(decoded.open, bar.open, "open");
    assert_eq!(decoded.high, bar.high, "high");
    assert_eq!(decoded.low, bar.low, "low");
    assert_eq!(decoded.close, bar.close, "close");
    assert_eq!(decoded.volume, bar.volume, "volume");
    assert_eq!(decoded.vwap, bar.vwap, "vwap");
    assert_eq!(decoded.trade_count, bar.trade_count, "trade_count");
}

#[test]
fn ohlcv_bar_negative_ts_ns_round_trip() {
    // Timestamps before the Unix epoch (e.g. test fixtures) must survive encoding.
    let bar = OhlcvBar {
        symbol: "BTC-USD".to_string(),
        ts_ns: -1_i64,
        close: 0.001,
        ..OhlcvBar::default()
    };
    let decoded: OhlcvBar = decode(&encode(&bar));
    assert_eq!(decoded.ts_ns, -1_i64, "negative ts_ns");
    assert_eq!(decoded.close, 0.001, "close");
}

// ---------------------------------------------------------------------------
// Tick
// ---------------------------------------------------------------------------

#[test]
fn tick_default_encodes_to_empty_bytes() {
    let tick = Tick::default();
    let bytes = encode(&tick);
    assert!(bytes.is_empty(), "default Tick should produce zero bytes");
}

#[test]
fn tick_round_trip_preserves_fields() {
    let tick = Tick {
        symbol: "ETH-USD".to_string(),
        provider: "binance".to_string(),
        ts_ns: 1_700_000_001_000_000_000_i64,
        price: 2_048.50,
        size: 0.5,
        side: TradeSide::Buy as i32,
        trade_id: "trade-abc-123".to_string(),
    };

    let decoded: Tick = decode(&encode(&tick));

    assert_eq!(decoded.symbol, tick.symbol, "symbol");
    assert_eq!(decoded.provider, tick.provider, "provider");
    assert_eq!(decoded.ts_ns, tick.ts_ns, "ts_ns");
    assert_eq!(decoded.price, tick.price, "price");
    assert_eq!(decoded.size, tick.size, "size");
    assert_eq!(decoded.side, tick.side, "side");
    assert_eq!(decoded.trade_id, tick.trade_id, "trade_id");
}

#[test]
fn tick_sell_side_round_trip() {
    let tick = Tick {
        symbol: "SOL-USD".to_string(),
        side: TradeSide::Sell as i32,
        price: 100.0,
        size: 1.0,
        ..Tick::default()
    };
    let decoded: Tick = decode(&encode(&tick));
    assert_eq!(decoded.side, TradeSide::Sell as i32, "sell side");
}

// ---------------------------------------------------------------------------
// PriceLevel
// ---------------------------------------------------------------------------

#[test]
fn price_level_default_encodes_to_empty_bytes() {
    let level = PriceLevel::default();
    let bytes = encode(&level);
    assert!(
        bytes.is_empty(),
        "default PriceLevel should produce zero bytes"
    );
}

#[test]
fn price_level_round_trip() {
    let level = PriceLevel {
        price: 50_000.0,
        quantity: 0.123_456_789,
    };
    let decoded: PriceLevel = decode(&encode(&level));
    assert_eq!(decoded.price, level.price, "price");
    assert_eq!(decoded.quantity, level.quantity, "quantity");
}

// ---------------------------------------------------------------------------
// OrderBookSnapshot
// ---------------------------------------------------------------------------

#[test]
fn order_book_snapshot_default_encodes_to_empty_bytes() {
    let snap = OrderBookSnapshot::default();
    let bytes = encode(&snap);
    assert!(
        bytes.is_empty(),
        "default OrderBookSnapshot should produce zero bytes"
    );
}

#[test]
fn order_book_snapshot_round_trip_preserves_levels() {
    let snap = OrderBookSnapshot {
        symbol: "BTC-USD".to_string(),
        provider: "deribit".to_string(),
        ts_ns: 1_700_000_002_000_000_000_i64,
        bids: vec![
            PriceLevel {
                price: 49_900.0,
                quantity: 1.0,
            },
            PriceLevel {
                price: 49_850.0,
                quantity: 2.5,
            },
        ],
        asks: vec![
            PriceLevel {
                price: 50_000.0,
                quantity: 0.5,
            },
            PriceLevel {
                price: 50_050.0,
                quantity: 3.0,
            },
        ],
        sequence: 999_999,
    };

    let decoded: OrderBookSnapshot = decode(&encode(&snap));

    assert_eq!(decoded.symbol, snap.symbol, "symbol");
    assert_eq!(decoded.provider, snap.provider, "provider");
    assert_eq!(decoded.ts_ns, snap.ts_ns, "ts_ns");
    assert_eq!(decoded.sequence, snap.sequence, "sequence");
    assert_eq!(decoded.bids.len(), 2, "bid count");
    assert_eq!(decoded.asks.len(), 2, "ask count");
    assert_eq!(decoded.bids[0].price, 49_900.0, "best bid price");
    assert_eq!(decoded.asks[0].price, 50_000.0, "best ask price");
}

#[test]
fn order_book_snapshot_empty_levels_round_trip() {
    let snap = OrderBookSnapshot {
        symbol: "ETH-USD".to_string(),
        ts_ns: 1,
        bids: vec![],
        asks: vec![],
        sequence: 0,
        ..OrderBookSnapshot::default()
    };
    let decoded: OrderBookSnapshot = decode(&encode(&snap));
    assert!(decoded.bids.is_empty(), "bids empty");
    assert!(decoded.asks.is_empty(), "asks empty");
}

// ---------------------------------------------------------------------------
// MarketDataEnvelope + Payload oneof
// ---------------------------------------------------------------------------

#[test]
fn envelope_default_encodes_to_empty_bytes() {
    let env = MarketDataEnvelope::default();
    let bytes = encode(&env);
    assert!(
        bytes.is_empty(),
        "default MarketDataEnvelope should produce zero bytes"
    );
}

#[test]
fn envelope_with_bar_payload_round_trip() {
    let bar = OhlcvBar {
        symbol: "AAPL".to_string(),
        provider: "polygon".to_string(),
        timeframe: "5m".to_string(),
        ts_ns: 1_700_000_003_000_000_000_i64,
        open: 170.0,
        high: 171.0,
        low: 169.5,
        close: 170.5,
        volume: 50_000.0,
        vwap: 170.25,
        trade_count: 1_200,
    };
    let env = MarketDataEnvelope {
        provider: "polygon".to_string(),
        symbol: "AAPL".to_string(),
        ingestion_id: "01HZ000000000000000000001".to_string(),
        received_at_ns: 1_700_000_003_000_100_000_i64,
        payload: Some(Payload::Bar(bar.clone())),
    };

    let decoded: MarketDataEnvelope = decode(&encode(&env));

    assert_eq!(decoded.provider, env.provider, "provider");
    assert_eq!(decoded.symbol, env.symbol, "symbol");
    assert_eq!(decoded.ingestion_id, env.ingestion_id, "ingestion_id");
    assert_eq!(decoded.received_at_ns, env.received_at_ns, "received_at_ns");

    match decoded.payload {
        Some(Payload::Bar(b)) => {
            assert_eq!(b.symbol, bar.symbol, "bar.symbol");
            assert_eq!(b.close, bar.close, "bar.close");
            assert_eq!(b.trade_count, bar.trade_count, "bar.trade_count");
        }
        other => panic!("expected Payload::Bar, got {other:?}"),
    }
}

#[test]
fn envelope_with_tick_payload_round_trip() {
    let tick = Tick {
        symbol: "ETH-USD".to_string(),
        provider: "binance".to_string(),
        ts_ns: 1_700_000_004_000_000_000_i64,
        price: 2_100.0,
        size: 0.25,
        side: TradeSide::Sell as i32,
        trade_id: String::new(),
    };
    let env = MarketDataEnvelope {
        provider: "binance".to_string(),
        symbol: "ETH-USD".to_string(),
        ingestion_id: "01HZ000000000000000000002".to_string(),
        received_at_ns: 1_700_000_004_000_050_000_i64,
        payload: Some(Payload::Tick(tick.clone())),
    };

    let decoded: MarketDataEnvelope = decode(&encode(&env));

    match decoded.payload {
        Some(Payload::Tick(t)) => {
            assert_eq!(t.price, tick.price, "tick.price");
            assert_eq!(t.side, TradeSide::Sell as i32, "tick.side");
        }
        other => panic!("expected Payload::Tick, got {other:?}"),
    }
}

#[test]
fn envelope_with_order_book_payload_round_trip() {
    let snap = OrderBookSnapshot {
        symbol: "BTC-USD".to_string(),
        provider: "cboe".to_string(),
        ts_ns: 1_700_000_005_000_000_000_i64,
        bids: vec![PriceLevel {
            price: 60_000.0,
            quantity: 1.0,
        }],
        asks: vec![PriceLevel {
            price: 60_100.0,
            quantity: 0.75,
        }],
        sequence: 42,
    };
    let env = MarketDataEnvelope {
        provider: "cboe".to_string(),
        symbol: "BTC-USD".to_string(),
        ingestion_id: "01HZ000000000000000000003".to_string(),
        received_at_ns: 1_700_000_005_000_200_000_i64,
        payload: Some(Payload::OrderBook(snap.clone())),
    };

    let decoded: MarketDataEnvelope = decode(&encode(&env));

    match decoded.payload {
        Some(Payload::OrderBook(ob)) => {
            assert_eq!(ob.sequence, 42, "sequence");
            assert_eq!(ob.bids.len(), 1, "bid count");
            assert_eq!(ob.bids[0].price, 60_000.0, "bid price");
        }
        other => panic!("expected Payload::OrderBook, got {other:?}"),
    }
}

#[test]
fn envelope_no_payload_round_trip() {
    let env = MarketDataEnvelope {
        provider: "test".to_string(),
        symbol: "X".to_string(),
        ingestion_id: String::new(),
        received_at_ns: 0,
        payload: None,
    };
    let decoded: MarketDataEnvelope = decode(&encode(&env));
    assert!(decoded.payload.is_none(), "payload should be absent");
}

// ---------------------------------------------------------------------------
// TradeSide enum
// ---------------------------------------------------------------------------

#[test]
fn trade_side_as_str_name_matches_proto_definition() {
    assert_eq!(TradeSide::Unknown.as_str_name(), "TRADE_SIDE_UNKNOWN");
    assert_eq!(TradeSide::Buy.as_str_name(), "TRADE_SIDE_BUY");
    assert_eq!(TradeSide::Sell.as_str_name(), "TRADE_SIDE_SELL");
}

#[test]
fn trade_side_from_str_name_round_trips() {
    for side in [TradeSide::Unknown, TradeSide::Buy, TradeSide::Sell] {
        let name = side.as_str_name();
        let recovered = TradeSide::from_str_name(name)
            .unwrap_or_else(|| panic!("from_str_name({name}) should return Some"));
        assert_eq!(recovered, side, "round-trip for {name}");
    }
}

#[test]
fn trade_side_from_str_name_unknown_returns_none() {
    assert!(
        TradeSide::from_str_name("TRADE_SIDE_INVALID").is_none(),
        "unrecognised name should return None"
    );
}

#[test]
fn trade_side_discriminants_match_proto_values() {
    // Proto3 wire values must be stable.
    assert_eq!(TradeSide::Unknown as i32, 0);
    assert_eq!(TradeSide::Buy as i32, 1);
    assert_eq!(TradeSide::Sell as i32, 2);
}

// ---------------------------------------------------------------------------
// Deterministic encoding
// ---------------------------------------------------------------------------

#[test]
fn ohlcv_bar_encoding_is_deterministic() {
    let bar = OhlcvBar {
        symbol: "MSFT".to_string(),
        provider: "alpha_vantage".to_string(),
        timeframe: "1d".to_string(),
        ts_ns: 1_700_100_000_000_000_000_i64,
        open: 300.0,
        high: 305.0,
        low: 298.0,
        close: 302.0,
        volume: 25_000_000.0,
        vwap: 301.5,
        trade_count: 150_000,
    };
    let a = encode(&bar);
    let b = encode(&bar);
    assert_eq!(a, b, "repeated encodes must produce identical bytes");
}
