#![forbid(unsafe_code)]

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use validator::Validate;

pub const BOM_SCHEMA_NAMES: [&str; 11] = [
    "market_data",
    "orders",
    "positions",
    "news_sentiment",
    "fundamentals",
    "strategy",
    "risk",
    "time_calendar",
    "ops",
    "reference_data",
    "costs_fees",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum AssetClass {
    Equity,
    Etf,
    Future,
    Option,
    Forex,
    Crypto,
    Index,
    Fund,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum OrderSide {
    Buy,
    Sell,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum OrderStatus {
    New,
    PartiallyFilled,
    Filled,
    Cancelled,
    Rejected,
    Expired,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum TimeGranularity {
    Tick,
    Minute,
    Hour,
    Day,
    Week,
    Month,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct MarketDataBar {
    #[validate(length(min = 1))]
    pub symbol: String,
    #[validate(length(min = 1))]
    pub venue: String,
    pub granularity: TimeGranularity,
    #[validate(length(min = 1))]
    pub ts: String,
    #[validate(range(min = 0.0))]
    pub open: f64,
    #[validate(range(min = 0.0))]
    pub high: f64,
    #[validate(range(min = 0.0))]
    pub low: f64,
    #[validate(range(min = 0.0))]
    pub close: f64,
    #[validate(range(min = 0.0))]
    pub volume: f64,
    pub source: String,
}

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
pub struct OrderEvent {
    #[validate(length(min = 1))]
    pub order_id: String,
    #[validate(length(min = 1))]
    pub account_id: String,
    #[validate(length(min = 1))]
    pub symbol: String,
    pub side: OrderSide,
    pub status: OrderStatus,
    #[validate(range(min = 0.0))]
    pub quantity: f64,
    #[validate(range(min = 0.0))]
    pub filled_quantity: f64,
    pub limit_price: Option<f64>,
    #[validate(length(min = 1))]
    pub event_ts: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct PositionSnapshot {
    #[validate(length(min = 1))]
    pub account_id: String,
    #[validate(length(min = 1))]
    pub symbol: String,
    #[validate(length(min = 1))]
    pub as_of: String,
    pub quantity: f64,
    pub average_price: f64,
    pub market_value: f64,
    pub unrealized_pnl: f64,
    pub currency: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct NewsSentiment {
    #[validate(length(min = 1))]
    pub id: String,
    #[validate(length(min = 1))]
    pub headline: String,
    pub body: String,
    #[validate(length(min = 1))]
    pub published_at: String,
    pub symbols: Vec<String>,
    #[validate(range(min = -1.0, max = 1.0))]
    pub sentiment_score: f64,
    pub source: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct FundamentalMetric {
    #[validate(length(min = 1))]
    pub symbol: String,
    #[validate(length(min = 1))]
    pub fiscal_period: String,
    #[validate(length(min = 1))]
    pub metric: String,
    pub value: f64,
    pub currency: Option<String>,
    #[validate(length(min = 1))]
    pub reported_at: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct StrategySignal {
    #[validate(length(min = 1))]
    pub strategy_id: String,
    #[validate(length(min = 1))]
    pub symbol: String,
    pub side: OrderSide,
    pub score: f64,
    #[validate(range(min = 0.0))]
    pub target_weight: f64,
    #[validate(length(min = 1))]
    pub generated_at: String,
    pub horizon: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct RiskMetric {
    #[validate(length(min = 1))]
    pub account_id: String,
    #[validate(length(min = 1))]
    pub metric: String,
    pub value: f64,
    pub limit: Option<f64>,
    #[validate(length(min = 1))]
    pub as_of: String,
    pub currency: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct TradingCalendarEvent {
    #[validate(length(min = 1))]
    pub calendar_id: String,
    #[validate(length(min = 1))]
    pub venue: String,
    #[validate(length(min = 1))]
    pub session_date: String,
    pub open_ts: Option<String>,
    pub close_ts: Option<String>,
    pub is_trading_day: bool,
    pub note: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct OperationalEvent {
    #[validate(length(min = 1))]
    pub event_id: String,
    #[validate(length(min = 1))]
    pub component: String,
    #[validate(length(min = 1))]
    pub severity: String,
    #[validate(length(min = 1))]
    pub observed_at: String,
    pub message: String,
    pub correlation_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct ReferenceInstrument {
    #[validate(length(min = 1))]
    pub symbol: String,
    #[validate(length(min = 1))]
    pub name: String,
    #[validate(length(min = 1))]
    pub venue: String,
    pub asset_class: AssetClass,
    pub currency: String,
    pub isin: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct Instrument {
    #[validate(length(min = 1))]
    pub symbol: String,
    #[validate(length(min = 1))]
    pub name: String,
    #[validate(length(min = 1))]
    pub venue: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct CostFeeEvent {
    #[validate(length(min = 1))]
    pub event_id: String,
    #[validate(length(min = 1))]
    pub account_id: String,
    pub order_id: Option<String>,
    #[validate(length(min = 1))]
    pub fee_type: String,
    pub amount: f64,
    pub currency: String,
    #[validate(length(min = 1))]
    pub charged_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
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
    fn exposes_all_bom_schema_names() {
        assert_eq!(BOM_SCHEMA_NAMES.len(), 11);
        assert!(BOM_SCHEMA_NAMES.contains(&"market_data"));
        assert!(BOM_SCHEMA_NAMES.contains(&"costs_fees"));
    }

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

    #[test]
    fn market_data_bar_validates_clean_fixture() {
        let row = MarketDataBar {
            symbol: "AAPL".to_string(),
            venue: "XNAS".to_string(),
            granularity: TimeGranularity::Day,
            ts: "2026-05-21T00:00:00Z".to_string(),
            open: 100.0,
            high: 101.0,
            low: 99.0,
            close: 100.5,
            volume: 1_000.0,
            source: "fixture".to_string(),
        };

        assert!(row.validate().is_ok());
    }
}
