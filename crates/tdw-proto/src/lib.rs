#![forbid(unsafe_code)]
#![doc = "Generated protobuf bindings for TDW market data types."]

pub mod finance {
    // Vendored prost-build output (see `finance.gen.rs`). Included via a relative
    // path so the crate needs no build-time codegen or system `protoc`.
    include!("finance.gen.rs");
}

pub use finance::{
    MarketDataEnvelope, OhlcvBar, OrderBookSnapshot, PriceLevel, Tick, TradeSide,
    market_data_envelope::Payload,
};
