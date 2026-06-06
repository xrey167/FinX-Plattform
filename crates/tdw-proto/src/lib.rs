#![forbid(unsafe_code)]
#![doc = "Generated protobuf bindings for FinX-Finance market data types."]

pub mod finance {
    include!(concat!(env!("OUT_DIR"), "/tdw.finance.rs"));
}

pub use finance::{
    MarketDataEnvelope, OhlcvBar, OrderBookSnapshot, PriceLevel, Tick, TradeSide,
    market_data_envelope::Payload,
};
