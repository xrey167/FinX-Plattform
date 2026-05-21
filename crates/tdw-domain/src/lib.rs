#![forbid(unsafe_code)]

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use validator::Validate;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct EquityHistoricalData {
    #[validate(length(min = 1))]
    pub symbol: String,
    #[validate(length(min = 1))]
    pub date: String,
    #[validate(range(min = 0.0))]
    pub open: f64,
    #[validate(range(min = 0.0))]
    pub high: f64,
    #[validate(range(min = 0.0))]
    pub low: f64,
    #[validate(range(min = 0.0))]
    pub close: f64,
    #[validate(range(min = 0))]
    pub volume: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct Instrument {
    #[validate(length(min = 1))]
    pub symbol: String,
    #[validate(length(min = 1))]
    pub name: String,
    #[validate(length(min = 1))]
    pub venue: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct ResearchNote {
    #[validate(length(min = 1))]
    pub id: String,
    #[validate(length(min = 1))]
    pub title: String,
    #[validate(length(min = 1))]
    pub body: String,
    pub tags: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equity_historical_validates_symbol_and_price_shape() {
        let row = EquityHistoricalData {
            symbol: "AAPL".to_string(),
            date: "2026-05-21".to_string(),
            open: 100.0,
            high: 101.0,
            low: 99.0,
            close: 100.5,
            volume: 1_000,
        };

        assert!(row.validate().is_ok());
    }
}
