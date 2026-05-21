#![forbid(unsafe_code)]

use tdw_domain::{BOM_SCHEMA_NAMES, MarketDataBar};
use validator::Validate;

#[test]
fn market_data_bar_golden_fixture_validates() {
    let fixture = include_str!("golden/market_data_bar.json");
    let row: MarketDataBar = serde_json::from_str(fixture)
        .unwrap_or_else(|error| panic!("market data fixture should parse: {error}"));

    row.validate()
        .unwrap_or_else(|error| panic!("market data fixture should validate: {error}"));
    assert_eq!(BOM_SCHEMA_NAMES.len(), 11);
}
