#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used)]

// Integration coverage for every Validate-derived BOM type in tdw-domain.
//
// Each type gets:
//   * a "clean" fixture builder that returns a structurally valid instance
//   * happy-path test asserting validate() is Ok
//   * one mutated test asserting validate() rejects a constraint violation
//
// IMPORTANT: This file was authored offline and has NOT been compiled or run.
// Verify with `cargo test --package tdw-domain` before merging.

use tdw_domain::{
    AssetClass, CostFeeEvent, EquityHistoricalData, FundamentalMetric, Instrument, MarketDataBar,
    NewsSentiment, OperationalEvent, OrderEvent, OrderSide, OrderStatus, PositionSnapshot,
    ReferenceInstrument, ResearchNote, RiskMetric, StrategySignal, TimeGranularity,
    TradingCalendarEvent,
};
use validator::Validate;

fn clean_market_data_bar() -> MarketDataBar {
    MarketDataBar {
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
    }
}

fn clean_equity_historical() -> EquityHistoricalData {
    EquityHistoricalData {
        symbol: "AAPL".to_string(),
        date: "2026-05-21".to_string(),
        open: 100.0,
        high: 101.0,
        low: 99.0,
        close: 100.5,
        volume: 1_000,
    }
}

fn clean_order_event() -> OrderEvent {
    OrderEvent {
        order_id: "ORD-1".to_string(),
        account_id: "ACC-1".to_string(),
        symbol: "AAPL".to_string(),
        side: OrderSide::Buy,
        status: OrderStatus::New,
        quantity: 10.0,
        filled_quantity: 0.0,
        limit_price: Some(101.0),
        event_ts: "2026-05-21T00:00:00Z".to_string(),
    }
}

fn clean_position_snapshot() -> PositionSnapshot {
    PositionSnapshot {
        account_id: "ACC-1".to_string(),
        symbol: "AAPL".to_string(),
        as_of: "2026-05-21".to_string(),
        quantity: 10.0,
        average_price: 100.0,
        market_value: 1010.0,
        unrealized_pnl: 10.0,
        currency: "USD".to_string(),
    }
}

fn clean_news_sentiment() -> NewsSentiment {
    NewsSentiment {
        id: "news-1".to_string(),
        headline: "Apple beats earnings".to_string(),
        body: "Body text.".to_string(),
        published_at: "2026-05-21T12:00:00Z".to_string(),
        symbols: vec!["AAPL".to_string()],
        sentiment_score: 0.4,
        source: "fixture".to_string(),
    }
}

fn clean_fundamental_metric() -> FundamentalMetric {
    FundamentalMetric {
        symbol: "AAPL".to_string(),
        fiscal_period: "2026Q1".to_string(),
        metric: "revenue".to_string(),
        value: 1_234.5,
        currency: Some("USD".to_string()),
        reported_at: "2026-05-01".to_string(),
    }
}

fn clean_strategy_signal() -> StrategySignal {
    StrategySignal {
        strategy_id: "strat-momentum".to_string(),
        symbol: "AAPL".to_string(),
        side: OrderSide::Buy,
        score: 0.8,
        target_weight: 0.05,
        generated_at: "2026-05-21T00:00:00Z".to_string(),
        horizon: "1d".to_string(),
    }
}

fn clean_risk_metric() -> RiskMetric {
    RiskMetric {
        account_id: "ACC-1".to_string(),
        metric: "VaR_95".to_string(),
        value: 1_500.0,
        limit: Some(5_000.0),
        as_of: "2026-05-21".to_string(),
        currency: Some("USD".to_string()),
    }
}

fn clean_trading_calendar_event() -> TradingCalendarEvent {
    TradingCalendarEvent {
        calendar_id: "cal-us".to_string(),
        venue: "XNAS".to_string(),
        session_date: "2026-05-21".to_string(),
        open_ts: Some("2026-05-21T13:30:00Z".to_string()),
        close_ts: Some("2026-05-21T20:00:00Z".to_string()),
        is_trading_day: true,
        note: None,
    }
}

fn clean_operational_event() -> OperationalEvent {
    OperationalEvent {
        event_id: "evt-1".to_string(),
        component: "ingest".to_string(),
        severity: "warning".to_string(),
        observed_at: "2026-05-21T00:00:00Z".to_string(),
        message: "rate limited".to_string(),
        correlation_id: None,
    }
}

fn clean_reference_instrument() -> ReferenceInstrument {
    ReferenceInstrument {
        symbol: "AAPL".to_string(),
        name: "Apple Inc.".to_string(),
        venue: "XNAS".to_string(),
        asset_class: AssetClass::Equity,
        currency: "USD".to_string(),
        isin: Some("US0378331005".to_string()),
    }
}

fn clean_instrument() -> Instrument {
    Instrument {
        symbol: "AAPL".to_string(),
        name: "Apple Inc.".to_string(),
        venue: "XNAS".to_string(),
    }
}

fn clean_cost_fee_event() -> CostFeeEvent {
    CostFeeEvent {
        event_id: "fee-1".to_string(),
        account_id: "ACC-1".to_string(),
        order_id: Some("ORD-1".to_string()),
        fee_type: "commission".to_string(),
        amount: 1.25,
        currency: "USD".to_string(),
        charged_at: "2026-05-21T00:00:00Z".to_string(),
    }
}

fn clean_research_note() -> ResearchNote {
    ResearchNote {
        id: "note-1".to_string(),
        title: "Macro outlook".to_string(),
        body: "Lorem ipsum.".to_string(),
        tags: vec!["macro".to_string()],
    }
}

// ---------- Happy path ----------

#[test]
fn market_data_bar_clean_validates() {
    assert!(clean_market_data_bar().validate().is_ok());
}
#[test]
fn equity_historical_clean_validates() {
    assert!(clean_equity_historical().validate().is_ok());
}
#[test]
fn order_event_clean_validates() {
    assert!(clean_order_event().validate().is_ok());
}
#[test]
fn position_snapshot_clean_validates() {
    assert!(clean_position_snapshot().validate().is_ok());
}
#[test]
fn news_sentiment_clean_validates() {
    assert!(clean_news_sentiment().validate().is_ok());
}
#[test]
fn fundamental_metric_clean_validates() {
    assert!(clean_fundamental_metric().validate().is_ok());
}
#[test]
fn strategy_signal_clean_validates() {
    assert!(clean_strategy_signal().validate().is_ok());
}
#[test]
fn risk_metric_clean_validates() {
    assert!(clean_risk_metric().validate().is_ok());
}
#[test]
fn trading_calendar_event_clean_validates() {
    assert!(clean_trading_calendar_event().validate().is_ok());
}
#[test]
fn operational_event_clean_validates() {
    assert!(clean_operational_event().validate().is_ok());
}
#[test]
fn reference_instrument_clean_validates() {
    assert!(clean_reference_instrument().validate().is_ok());
}
#[test]
fn instrument_clean_validates() {
    assert!(clean_instrument().validate().is_ok());
}
#[test]
fn cost_fee_event_clean_validates() {
    assert!(clean_cost_fee_event().validate().is_ok());
}
#[test]
fn research_note_clean_validates() {
    assert!(clean_research_note().validate().is_ok());
}

// ---------- Constraint violations ----------

#[test]
fn market_data_bar_rejects_empty_symbol() {
    let mut row = clean_market_data_bar();
    row.symbol = String::new();
    assert!(row.validate().is_err());
}

#[test]
fn market_data_bar_rejects_negative_open_price() {
    let mut row = clean_market_data_bar();
    row.open = -0.01;
    assert!(row.validate().is_err());
}

#[test]
fn equity_historical_rejects_empty_date() {
    let mut row = clean_equity_historical();
    row.date = String::new();
    assert!(row.validate().is_err());
}

#[test]
fn order_event_rejects_empty_order_id() {
    let mut row = clean_order_event();
    row.order_id = String::new();
    assert!(row.validate().is_err());
}

#[test]
fn order_event_rejects_negative_quantity() {
    let mut row = clean_order_event();
    row.quantity = -1.0;
    assert!(row.validate().is_err());
}

#[test]
fn position_snapshot_rejects_empty_account_id() {
    let mut row = clean_position_snapshot();
    row.account_id = String::new();
    assert!(row.validate().is_err());
}

#[test]
fn news_sentiment_rejects_score_above_one() {
    let mut row = clean_news_sentiment();
    row.sentiment_score = 1.5;
    assert!(row.validate().is_err());
}

#[test]
fn news_sentiment_rejects_score_below_negative_one() {
    let mut row = clean_news_sentiment();
    row.sentiment_score = -1.5;
    assert!(row.validate().is_err());
}

#[test]
fn fundamental_metric_rejects_empty_metric() {
    let mut row = clean_fundamental_metric();
    row.metric = String::new();
    assert!(row.validate().is_err());
}

#[test]
fn strategy_signal_rejects_negative_target_weight() {
    let mut row = clean_strategy_signal();
    row.target_weight = -0.1;
    assert!(row.validate().is_err());
}

#[test]
fn risk_metric_rejects_empty_metric() {
    let mut row = clean_risk_metric();
    row.metric = String::new();
    assert!(row.validate().is_err());
}

#[test]
fn trading_calendar_event_rejects_empty_venue() {
    let mut row = clean_trading_calendar_event();
    row.venue = String::new();
    assert!(row.validate().is_err());
}

#[test]
fn operational_event_rejects_empty_severity() {
    let mut row = clean_operational_event();
    row.severity = String::new();
    assert!(row.validate().is_err());
}

#[test]
fn reference_instrument_rejects_empty_name() {
    let mut row = clean_reference_instrument();
    row.name = String::new();
    assert!(row.validate().is_err());
}

#[test]
fn instrument_rejects_empty_venue() {
    let mut row = clean_instrument();
    row.venue = String::new();
    assert!(row.validate().is_err());
}

#[test]
fn cost_fee_event_rejects_empty_fee_type() {
    let mut row = clean_cost_fee_event();
    row.fee_type = String::new();
    assert!(row.validate().is_err());
}

#[test]
fn research_note_rejects_empty_title() {
    let mut row = clean_research_note();
    row.title = String::new();
    assert!(row.validate().is_err());
}
