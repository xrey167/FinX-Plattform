#![forbid(unsafe_code)]

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tdw_core::OBBject;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
struct Row {
    symbol: String,
    date: String,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    volume: u64,
}

#[test]
fn obbject_golden_fixture_round_trips() {
    let fixture = include_str!("golden/obbject_equity_historical.json");
    let original: Value = serde_json::from_str(fixture)
        .unwrap_or_else(|error| panic!("golden fixture should parse as json: {error}"));
    let object: OBBject<Row> = serde_json::from_value(original.clone())
        .unwrap_or_else(|error| panic!("golden fixture should parse as OBBject: {error}"));

    assert_eq!(object.provider, "fileset");
    assert_eq!(object.endpoint, "equity_historical");
    assert_eq!(object.rows[0].symbol, "AAPL");

    let round_trip = serde_json::to_value(object)
        .unwrap_or_else(|error| panic!("OBBject should serialize back to json: {error}"));
    assert_eq!(round_trip, original);
}
