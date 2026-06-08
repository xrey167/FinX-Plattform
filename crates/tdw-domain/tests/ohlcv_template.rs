//! Public-API coverage for `Ohlcv::into_bar_template`, the const helper the
//! provider crates use to splice raw OHLCV numbers into a `MarketDataBar` via
//! struct-update syntax.

use tdw_domain::{MarketDataBar, Ohlcv, TimeGranularity};

#[test]
fn into_bar_template_carries_ohlcv_and_leaves_metadata_empty() {
    let ohlcv = Ohlcv {
        open: 1.0,
        high: 2.5,
        low: 0.5,
        close: 2.0,
        volume: 1_000.0,
    };

    let template = ohlcv.into_bar_template();

    assert_eq!(template.open, 1.0);
    assert_eq!(template.high, 2.5);
    assert_eq!(template.low, 0.5);
    assert_eq!(template.close, 2.0);
    assert_eq!(template.volume, 1_000.0);

    // Metadata fields are placeholders awaiting struct-update completion.
    assert!(template.symbol.is_empty());
    assert!(template.venue.is_empty());
    assert!(template.ts.is_empty());
    assert!(template.source.is_empty());
    assert_eq!(template.granularity, TimeGranularity::Day);
}

#[test]
fn into_bar_template_supports_struct_update_completion() {
    let ohlcv = Ohlcv {
        open: 10.0,
        high: 12.0,
        low: 9.0,
        close: 11.0,
        volume: 42.0,
    };

    let bar = MarketDataBar {
        symbol: "AAPL".to_string(),
        venue: "XNAS".to_string(),
        ts: "2026-06-08T00:00:00Z".to_string(),
        source: "test".to_string(),
        granularity: TimeGranularity::Minute,
        ..ohlcv.into_bar_template()
    };

    assert_eq!(bar.symbol, "AAPL");
    assert_eq!(bar.granularity, TimeGranularity::Minute);
    assert_eq!(bar.close, 11.0);
}
