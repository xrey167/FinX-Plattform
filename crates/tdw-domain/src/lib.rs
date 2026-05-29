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
pub struct Tick {
    #[validate(length(min = 1))]
    pub symbol: String,
    #[validate(length(min = 1))]
    pub venue: String,
    #[validate(length(min = 1))]
    pub ts: String,
    #[validate(range(min = 0.0))]
    pub price: f64,
    #[validate(range(min = 0.0))]
    pub size: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct Quote {
    #[validate(length(min = 1))]
    pub symbol: String,
    #[validate(length(min = 1))]
    pub venue: String,
    #[validate(length(min = 1))]
    pub ts: String,
    #[validate(range(min = 0.0))]
    pub bid: f64,
    #[validate(range(min = 0.0))]
    pub ask: f64,
    #[validate(range(min = 0.0))]
    pub bid_size: f64,
    #[validate(range(min = 0.0))]
    pub ask_size: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct Trade {
    #[validate(length(min = 1))]
    pub symbol: String,
    #[validate(length(min = 1))]
    pub venue: String,
    #[validate(length(min = 1))]
    pub ts: String,
    #[validate(range(min = 0.0))]
    pub price: f64,
    #[validate(range(min = 0.0))]
    pub qty: f64,
    #[validate(length(min = 1))]
    pub side: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct BookLevel {
    #[validate(length(min = 1))]
    pub symbol: String,
    #[validate(length(min = 1))]
    pub venue: String,
    #[validate(length(min = 1))]
    pub ts: String,
    #[validate(length(min = 1))]
    pub side: String,
    pub level: u8,
    #[validate(range(min = 0.0))]
    pub price: f64,
    #[validate(range(min = 0.0))]
    pub size: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct CorporateAction {
    #[validate(length(min = 1))]
    pub symbol: String,
    #[validate(length(min = 1))]
    pub ex_date: String,
    #[validate(length(min = 1))]
    pub action_type: String,
    #[validate(range(min = 0.0))]
    pub split_ratio: f64,
    #[validate(range(min = 0.0))]
    pub cash_amount: f64,
    pub currency: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct FxRate {
    #[validate(length(min = 1))]
    pub base_currency: String,
    #[validate(length(min = 1))]
    pub quote_currency: String,
    #[validate(length(min = 1))]
    pub ts: String,
    #[validate(range(min = 0.0))]
    pub rate: f64,
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

/// Error returned when constructing a validated reference-data newtype id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReferenceIdError {
    /// The value had the wrong length for a fixed-width identifier.
    WrongLength {
        field: &'static str,
        expected: usize,
        actual: usize,
    },
    /// The value was empty for a non-empty identifier.
    Empty { field: &'static str },
}

impl core::fmt::Display for ReferenceIdError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::WrongLength {
                field,
                expected,
                actual,
            } => write!(
                formatter,
                "{field} must be {expected} characters, got {actual}"
            ),
            Self::Empty { field } => write!(formatter, "{field} must not be empty"),
        }
    }
}

impl std::error::Error for ReferenceIdError {}

fn fixed_len(
    value: String,
    field: &'static str,
    expected: usize,
) -> Result<String, ReferenceIdError> {
    let actual = value.chars().count();
    if actual == expected {
        Ok(value)
    } else {
        Err(ReferenceIdError::WrongLength {
            field,
            expected,
            actual,
        })
    }
}

fn non_empty_id(value: String, field: &'static str) -> Result<String, ReferenceIdError> {
    if value.trim().is_empty() {
        Err(ReferenceIdError::Empty { field })
    } else {
        Ok(value)
    }
}

macro_rules! fixed_len_id {
    ($(#[$meta:meta])* $name:ident, $field:literal, $len:literal) => {
        $(#[$meta])*
        #[derive(
            Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
        )]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// # Errors
            ///
            /// Returns [`ReferenceIdError::WrongLength`] if the value is not exactly
            #[doc = concat!(stringify!($len), " characters long.")]
            pub fn new(value: impl Into<String>) -> Result<Self, ReferenceIdError> {
                fixed_len(value.into(), $field, $len).map(Self)
            }

            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
    };
}

macro_rules! non_empty_id {
    ($(#[$meta:meta])* $name:ident, $field:literal) => {
        $(#[$meta])*
        #[derive(
            Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
        )]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// # Errors
            ///
            /// Returns [`ReferenceIdError::Empty`] if the value is empty.
            pub fn new(value: impl Into<String>) -> Result<Self, ReferenceIdError> {
                non_empty_id(value.into(), $field).map(Self)
            }

            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
    };
}

fixed_len_id!(
    /// Financial Instrument Global Identifier (12 characters).
    Figi, "figi", 12
);
fixed_len_id!(
    /// International Securities Identification Number (12 characters).
    Isin, "isin", 12
);
fixed_len_id!(
    /// CUSIP identifier (9 characters).
    Cusip, "cusip", 9
);
fixed_len_id!(
    /// SEDOL identifier (7 characters).
    Sedol, "sedol", 7
);
fixed_len_id!(
    /// ISO 10383 Market Identifier Code (4 characters).
    Mic, "mic", 4
);
fixed_len_id!(
    /// ISO 3166-1 alpha-2 country code (2 characters).
    CountryAlpha2, "country_alpha2", 2
);
fixed_len_id!(
    /// ISO 4217 currency code (3 characters).
    CurrencyCode, "currency_code", 3
);

non_empty_id!(
    /// Stable issuer identifier (non-empty).
    IssuerId, "issuer_id"
);
non_empty_id!(
    /// Classification node code, e.g. a GICS/ICB code (non-empty).
    ClassificationCode, "classification_code"
);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct Country {
    #[validate(length(equal = 2))]
    pub code_alpha2: String,
    #[validate(length(equal = 3))]
    pub code_alpha3: Option<String>,
    #[validate(length(min = 1))]
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct Currency {
    #[validate(length(equal = 3))]
    pub code: String,
    #[validate(length(min = 1))]
    pub name: String,
    pub minor_units: Option<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct Exchange {
    #[validate(length(equal = 4))]
    pub mic: String,
    pub name: Option<String>,
    #[validate(length(equal = 2))]
    pub country_alpha2: Option<String>,
    #[validate(length(equal = 4))]
    pub operating_mic: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct Issuer {
    #[validate(length(min = 1))]
    pub issuer_id: String,
    #[validate(length(min = 1))]
    pub legal_name: String,
    #[validate(length(equal = 2))]
    pub country_alpha2: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct ClassificationNode {
    #[validate(length(min = 1))]
    pub scheme: String,
    #[validate(length(min = 1))]
    pub code: String,
    pub level: u8,
    #[validate(length(min = 1))]
    pub name: String,
    pub parent_code: Option<String>,
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
    fn tick_and_trade_validate_clean_fixtures() {
        let tick = Tick {
            symbol: "AAPL".to_string(),
            venue: "XNAS".to_string(),
            ts: "2026-05-21T20:00:00Z".to_string(),
            price: 100.5,
            size: 10.0,
        };
        let trade = Trade {
            symbol: "AAPL".to_string(),
            venue: "XNAS".to_string(),
            ts: "2026-05-21T20:00:00Z".to_string(),
            price: 100.5,
            qty: 5.0,
            side: "buy".to_string(),
        };

        assert!(tick.validate().is_ok());
        assert!(trade.validate().is_ok());

        let bad_tick = Tick {
            symbol: String::new(),
            ..tick
        };
        assert!(bad_tick.validate().is_err());
    }

    #[test]
    fn quote_validates_clean_and_rejects_empty_symbol() {
        let quote = Quote {
            symbol: "AAPL".to_string(),
            venue: "XNAS".to_string(),
            ts: "2026-05-21T20:00:00Z".to_string(),
            bid: 100.4,
            ask: 100.6,
            bid_size: 12.0,
            ask_size: 8.0,
        };

        assert!(quote.validate().is_ok());

        let bad_quote = Quote {
            symbol: String::new(),
            ..quote
        };
        assert!(bad_quote.validate().is_err());
    }

    #[test]
    fn book_level_validates_clean_and_rejects_empty_side() {
        let level = BookLevel {
            symbol: "AAPL".to_string(),
            venue: "XNAS".to_string(),
            ts: "2026-05-21T20:00:00Z".to_string(),
            side: "bid".to_string(),
            level: 1,
            price: 100.4,
            size: 12.0,
        };

        assert!(level.validate().is_ok());

        let bad_side = BookLevel {
            side: String::new(),
            ..level.clone()
        };
        assert!(bad_side.validate().is_err());

        let bad_price = BookLevel {
            price: -1.0,
            ..level
        };
        assert!(bad_price.validate().is_err());
    }

    #[test]
    fn corporate_action_validates_clean_and_rejects_empty_symbol() {
        let action = CorporateAction {
            symbol: "AAPL".to_string(),
            ex_date: "2020-08-31".to_string(),
            action_type: "split".to_string(),
            split_ratio: 4.0,
            cash_amount: 0.0,
            currency: "USD".to_string(),
        };

        assert!(action.validate().is_ok());

        let bad_symbol = CorporateAction {
            symbol: String::new(),
            ..action.clone()
        };
        assert!(bad_symbol.validate().is_err());

        let bad_ratio = CorporateAction {
            split_ratio: -1.0,
            ..action
        };
        assert!(bad_ratio.validate().is_err());
    }

    #[test]
    fn fx_rate_validates_clean_and_rejects_empty_pair() {
        let rate = FxRate {
            base_currency: "EUR".to_string(),
            quote_currency: "USD".to_string(),
            ts: "2026-05-21T20:00:00Z".to_string(),
            rate: 1.08,
            source: "ecb".to_string(),
        };

        assert!(rate.validate().is_ok());

        let bad_base = FxRate {
            base_currency: String::new(),
            ..rate.clone()
        };
        assert!(bad_base.validate().is_err());

        let bad_rate = FxRate { rate: -1.0, ..rate };
        assert!(bad_rate.validate().is_err());
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

    #[test]
    fn fixed_length_newtypes_accept_valid_and_reject_wrong_length() {
        assert_eq!(
            Figi::new("BBG000B9XRY4").expect("valid figi").as_str(),
            "BBG000B9XRY4"
        );
        assert!(Isin::new("US0378331005").is_ok());
        assert!(Cusip::new("037833100").is_ok());
        assert!(Sedol::new("2046251").is_ok());
        assert!(Mic::new("XNAS").is_ok());
        assert!(CountryAlpha2::new("US").is_ok());
        assert!(CurrencyCode::new("USD").is_ok());

        assert_eq!(
            Figi::new("TOOLONGFIGI123"),
            Err(ReferenceIdError::WrongLength {
                field: "figi",
                expected: 12,
                actual: 14,
            })
        );
        assert!(matches!(
            Mic::new("XNA"),
            Err(ReferenceIdError::WrongLength { field: "mic", .. })
        ));
        assert!(CurrencyCode::new("US").is_err());
    }

    #[test]
    fn non_empty_newtypes_accept_valid_and_reject_empty() {
        assert_eq!(
            IssuerId::new("ISS-APPLE").expect("valid issuer").as_str(),
            "ISS-APPLE"
        );
        assert!(ClassificationCode::new("101010").is_ok());

        assert_eq!(
            IssuerId::new("  "),
            Err(ReferenceIdError::Empty { field: "issuer_id" })
        );
        assert!(ClassificationCode::new("").is_err());
    }

    #[test]
    fn reference_entities_validate_clean_fixtures() {
        let country = Country {
            code_alpha2: "US".to_string(),
            code_alpha3: Some("USA".to_string()),
            name: "United States".to_string(),
        };
        let exchange = Exchange {
            mic: "XNAS".to_string(),
            name: Some("Nasdaq".to_string()),
            country_alpha2: Some("US".to_string()),
            operating_mic: Some("XNAS".to_string()),
        };
        let node = ClassificationNode {
            scheme: "ICB".to_string(),
            code: "101010".to_string(),
            level: 3,
            name: "Computer Hardware".to_string(),
            parent_code: Some("1010".to_string()),
        };

        assert!(country.validate().is_ok());
        assert!(exchange.validate().is_ok());
        assert!(node.validate().is_ok());

        let bad_country = Country {
            code_alpha2: "USA".to_string(),
            ..country
        };
        assert!(bad_country.validate().is_err());
    }
}
