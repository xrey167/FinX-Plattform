//! Tests for the real FMP HTTP fetchers.
//!
//! Gated by `--features http` (no reqwest dep otherwise).
//!
//! Cassette tests always run under the feature and parse recorded response
//! shapes without any network access. The live test is additionally gated by
//! `TDW_FMP_LIVE=1` and requires `TDW_FMP_API_KEY`.

#![cfg(feature = "http")]

use bytes::Bytes;
use serde_json::json;
use tdw_core::{Credentials, Fetcher};
use tdw_domain::StatementKind;
use tdw_provider_fmp::{
    FmpFundamentalsQuery, FmpHistoricalQuery, FmpHttpDividendsFetcher, FmpHttpEarningsFetcher,
    FmpHttpHistoricalFetcher, FmpHttpIncomeFetcher, FmpHttpKeyMetricsFetcher, FmpHttpPeersFetcher,
    FmpHttpProfileFetcher, FmpHttpQuoteSnapshotFetcher, FmpHttpRatiosFetcher, FmpHttpSplitsFetcher,
    FmpHttpStatementFetcher, FmpStatement,
};
use tdw_provider_testkit::{cassette_bytes, live_fetch_nonempty};

// ---------------------------------------------------------------------------
// Cassette helpers
// ---------------------------------------------------------------------------

fn historical_cassette() -> Bytes {
    cassette_bytes!({
        "symbol": "AAPL",
        "historical": [
            {
                "date": "2024-01-02",
                "open": 185.6,
                "high": 186.1,
                "low": 184.4,
                "close": 185.2,
                "volume": 55000000.0
            },
            {
                "date": "2024-01-03",
                "open": 184.2,
                "high": 185.9,
                "low": 183.1,
                "close": 184.8,
                "volume": 48000000.0
            }
        ]
    })
}

fn income_cassette() -> Bytes {
    cassette_bytes!([
        {
            "date": "2024-09-28",
            "symbol": "AAPL",
            "revenue": 391035000000_i64,
            "grossProfit": 180683000000_i64,
            "netIncome": 93736000000_i64
        },
        {
            "date": "2023-09-30",
            "symbol": "AAPL",
            "revenue": 383285000000_i64,
            "grossProfit": 169148000000_i64,
            "netIncome": 96995000000_i64
        }
    ])
}

// ---------------------------------------------------------------------------
// Cassette tests (no network, always run when feature enabled)
// ---------------------------------------------------------------------------

#[test]
fn cassette_parse_fmp_historical_response() {
    let fetcher = FmpHttpHistoricalFetcher::default();
    let query = FmpHttpHistoricalFetcher::transform_query(json!({"symbol": "AAPL"}))
        .unwrap_or_else(|e| panic!("transform_query must succeed: {e}"));

    let rows = fetcher
        .transform_data(&query, historical_cassette())
        .unwrap_or_else(|e| panic!("transform_data must succeed: {e}"));

    assert_eq!(rows.len(), 2, "rows={rows:#?}");
    assert_eq!(rows[0].symbol, "AAPL");
    assert_eq!(rows[0].venue, "fmp");
    assert_eq!(rows[0].ts, "2024-01-02");
    assert_eq!(rows[0].open, 185.6);
    assert_eq!(rows[0].close, 185.2);
    assert_eq!(rows[0].volume, 55_000_000.0);
    assert_eq!(rows[1].ts, "2024-01-03");
}

#[test]
fn cassette_parse_fmp_income_response() {
    let fetcher = FmpHttpIncomeFetcher::default();
    let query =
        FmpHttpIncomeFetcher::transform_query(json!({"symbol": "AAPL", "statement": "income"}))
            .unwrap_or_else(|e| panic!("transform_query must succeed: {e}"));

    let rows = fetcher
        .transform_data(&query, income_cassette())
        .unwrap_or_else(|e| panic!("transform_data must succeed: {e}"));

    assert_eq!(rows.len(), 2, "rows={rows:#?}");
    assert_eq!(rows[0].symbol, "AAPL");
    assert_eq!(rows[0].date, "2024-09-28");
    assert_eq!(rows[0].revenue, 391_035_000_000);
    assert_eq!(rows[0].gross_profit, 180_683_000_000);
    assert_eq!(rows[0].net_income, 93_736_000_000);
    assert_eq!(rows[1].date, "2023-09-30");
}

#[test]
fn transform_query_normalises_symbol_and_rejects_path_injection() {
    let query = FmpHttpHistoricalFetcher::transform_query(json!({"symbol": "msft"}))
        .unwrap_or_else(|e| panic!("query should transform: {e}"));

    assert_eq!(query.symbol, "MSFT");

    assert!(
        FmpHttpHistoricalFetcher::transform_query(json!({"symbol": "MSFT/../../secret"})).is_err()
    );
    assert!(FmpHttpHistoricalFetcher::transform_query(json!({"symbol": ""})).is_err());
}

#[test]
fn income_transform_query_defaults_statement_to_income_and_limit_to_five() {
    let query = FmpHttpIncomeFetcher::transform_query(json!({"symbol": "aapl"}))
        .unwrap_or_else(|e| panic!("query should transform: {e}"));

    assert_eq!(query.symbol, "AAPL");
    assert_eq!(query.statement, FmpStatement::Income);
    assert_eq!(query.limit, 5);
}

#[test]
fn income_transform_query_rejects_unknown_statement() {
    assert!(
        FmpHttpIncomeFetcher::transform_query(json!({"symbol": "AAPL", "statement": "bogus"}))
            .is_err()
    );
}

#[test]
fn empty_historical_response_produces_empty_vec() {
    let fetcher = FmpHttpHistoricalFetcher::default();
    let query = FmpHistoricalQuery::new("AAPL").unwrap_or_else(|e| panic!("query: {e}"));
    let raw = cassette_bytes!({"symbol": "AAPL", "historical": []});

    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|e| panic!("transform_data must succeed: {e}"));

    assert!(rows.is_empty());
}

#[test]
fn malformed_json_produces_provider_error() {
    let fetcher = FmpHttpHistoricalFetcher::default();
    let query = FmpHistoricalQuery::new("AAPL").unwrap_or_else(|e| panic!("query: {e}"));
    let raw = Bytes::from(b"not valid json".to_vec());

    let err = fetcher
        .transform_data(&query, raw)
        .expect_err("malformed JSON must produce an error");

    assert!(err.to_string().contains("fmp parse_json"));
}

fn quote_cassette() -> Bytes {
    Bytes::from(
        json!([
            {
                "symbol": "AAPL",
                "price": 189.30,
                "change": 1.20,
                "changesPercentage": 0.638,
                "previousClose": 188.10,
                "timestamp": 1717200000_i64
            }
        ])
        .to_string()
        .into_bytes(),
    )
}

// ---------------------------------------------------------------------------
// Quote-snapshot cassette tests
// ---------------------------------------------------------------------------

#[test]
fn cassette_parse_fmp_quote_snapshot_response() {
    let fetcher = FmpHttpQuoteSnapshotFetcher::default();
    let query = FmpHttpQuoteSnapshotFetcher::transform_query(json!({"symbol": "AAPL"}))
        .unwrap_or_else(|e| panic!("transform_query must succeed: {e}"));

    let rows = fetcher
        .transform_data(&query, quote_cassette())
        .unwrap_or_else(|e| panic!("transform_data must succeed: {e}"));

    assert_eq!(rows.len(), 1, "rows={rows:#?}");
    assert_eq!(rows[0].symbol, "AAPL");
    assert_eq!(rows[0].current_price, 189.30);
    assert_eq!(rows[0].change, 1.20);
    assert_eq!(rows[0].change_percent, 0.638);
    assert_eq!(rows[0].prev_close, 188.10);
    // FMP timestamp is seconds; fetcher multiplies by 1000 for ts_ms.
    assert_eq!(rows[0].ts_ms, 1_717_200_000_000);
}

#[test]
fn quote_snapshot_transform_query_normalises_symbol_and_rejects_invalid() {
    let query = FmpHttpQuoteSnapshotFetcher::transform_query(json!({"symbol": "msft"}))
        .unwrap_or_else(|e| panic!("query should transform: {e}"));
    assert_eq!(query.symbol, "MSFT");

    assert!(
        FmpHttpQuoteSnapshotFetcher::transform_query(json!({"symbol": "MSFT/../../secret"}))
            .is_err()
    );
    assert!(FmpHttpQuoteSnapshotFetcher::transform_query(json!({"symbol": ""})).is_err());
}

#[test]
fn quote_snapshot_empty_response_produces_empty_vec() {
    let fetcher = FmpHttpQuoteSnapshotFetcher::default();
    let query = FmpHttpQuoteSnapshotFetcher::transform_query(json!({"symbol": "AAPL"}))
        .unwrap_or_else(|e| panic!("query: {e}"));
    let raw = Bytes::from(json!([]).to_string().into_bytes());

    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|e| panic!("transform_data must succeed: {e}"));
    assert!(rows.is_empty());
}

#[test]
fn quote_snapshot_missing_numerics_fall_back_to_zero() {
    let fetcher = FmpHttpQuoteSnapshotFetcher::default();
    let query = FmpHttpQuoteSnapshotFetcher::transform_query(json!({"symbol": "AAPL"}))
        .unwrap_or_else(|e| panic!("query: {e}"));
    // Only symbol provided — all numeric fields should default to 0.0 / 0.
    let raw = Bytes::from(json!([{"symbol": "AAPL"}]).to_string().into_bytes());

    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|e| panic!("transform_data must succeed: {e}"));
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].current_price, 0.0);
    assert_eq!(rows[0].ts_ms, 0);
}

#[test]
fn quote_snapshot_malformed_json_produces_provider_error() {
    let fetcher = FmpHttpQuoteSnapshotFetcher::default();
    let query = FmpHttpQuoteSnapshotFetcher::transform_query(json!({"symbol": "AAPL"}))
        .unwrap_or_else(|e| panic!("query: {e}"));
    let raw = Bytes::from(b"not valid json".to_vec());

    let err = fetcher
        .transform_data(&query, raw)
        .expect_err("malformed JSON must produce an error");
    assert!(err.to_string().contains("fmp quote parse_json"));
}

// ---------------------------------------------------------------------------
// Fundamentals cluster cassette tests (no network, always run when feature on)
// ---------------------------------------------------------------------------

#[test]
fn cassette_statement_normalises_to_financial_statement() {
    let fetcher = FmpHttpStatementFetcher::default();
    let query = FmpHttpStatementFetcher::transform_query(json!({
        "symbol": "AAPL", "statement": "balance", "period": "annual"
    }))
    .unwrap_or_else(|e| panic!("transform_query: {e}"));

    let raw = cassette_bytes!([
        {
            "date": "2024-09-28",
            "symbol": "AAPL",
            "reportedCurrency": "USD",
            "calendarYear": "2024",
            "period": "FY",
            "fillingDate": "2024-11-01",
            "totalAssets": 364980000000_i64,
            "totalLiabilities": 308030000000_i64,
            "link": "https://example.com/filing"
        }
    ]);

    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|e| panic!("transform_data: {e}"));

    assert_eq!(rows.len(), 1);
    let stmt = &rows[0];
    assert_eq!(stmt.symbol, "AAPL");
    assert_eq!(stmt.statement, StatementKind::Balance);
    assert_eq!(stmt.fiscal_year, Some(2024));
    assert_eq!(stmt.fiscal_period.as_deref(), Some("FY"));
    assert_eq!(stmt.date.as_deref(), Some("2024-09-28"));
    assert_eq!(stmt.filing_date.as_deref(), Some("2024-11-01"));
    assert_eq!(stmt.currency.as_deref(), Some("USD"));
    // Numeric lines swept into line_items under snake_case keys; header keys and
    // the string `link` are excluded.
    assert_eq!(
        stmt.line_items.get("total_assets"),
        Some(&364_980_000_000.0)
    );
    assert_eq!(
        stmt.line_items.get("total_liabilities"),
        Some(&308_030_000_000.0)
    );
    assert!(!stmt.line_items.contains_key("calendar_year"));
    assert!(!stmt.line_items.contains_key("link"));
}

#[test]
fn cassette_statement_growth_uses_growth_endpoint() {
    let query = FmpHttpStatementFetcher::transform_query(json!({
        "symbol": "AAPL", "statement": "income", "growth": true
    }))
    .unwrap_or_else(|e| panic!("transform_query: {e}"));
    assert!(query.growth);
    assert_eq!(query.statement, FmpStatement::Income);
}

#[test]
fn cassette_key_metrics_normalises_to_key_metrics() {
    let fetcher = FmpHttpKeyMetricsFetcher::default();
    let query = FmpHttpKeyMetricsFetcher::transform_query(json!({"symbol": "AAPL"}))
        .unwrap_or_else(|e| panic!("transform_query: {e}"));

    let raw = cassette_bytes!([
        {
            "symbol": "AAPL",
            "date": "2024-09-28",
            "period": "FY",
            "marketCap": 3450000000000_i64,
            "peRatio": 31.2,
            "priceToSalesRatio": 8.1,
            "pbRatio": 48.0,
            "enterpriseValue": 3600000000000_i64,
            "enterpriseValueOverEBITDA": 24.5,
            "netIncomePerShare": 6.08,
            "revenuePerShare": 25.1,
            "bookValuePerShare": 4.0,
            "freeCashFlowPerShare": 6.5,
            "dividendYield": 0.0044,
            "workingCapital": 1500000000_i64
        }
    ]);

    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|e| panic!("transform_data: {e}"));

    assert_eq!(rows.len(), 1);
    let m = &rows[0];
    assert_eq!(m.symbol, "AAPL");
    assert_eq!(m.date.as_deref(), Some("2024-09-28"));
    assert_eq!(m.market_cap, Some(3_450_000_000_000.0));
    assert_eq!(m.pe_ratio, Some(31.2));
    assert_eq!(m.ev_to_ebitda, Some(24.5));
    assert_eq!(m.earnings_per_share, Some(6.08));
    assert_eq!(m.dividend_yield, Some(0.0044));
    // Untyped numeric metric flows into extra_metrics.
    assert_eq!(m.extra_metrics.get("working_capital"), Some(&1.5e9));
    assert!(!m.extra_metrics.contains_key("market_cap"));
}

#[test]
fn cassette_ratios_normalises_to_ratios() {
    let fetcher = FmpHttpRatiosFetcher::default();
    let query =
        FmpHttpRatiosFetcher::transform_query(json!({"symbol": "AAPL", "period": "quarter"}))
            .unwrap_or_else(|e| panic!("transform_query: {e}"));
    assert_eq!(query.period, tdw_provider_fmp::FmpPeriod::Quarter);

    let raw = cassette_bytes!([
        {
            "symbol": "AAPL",
            "date": "2024-09-28",
            "period": "FY",
            "currentRatio": 0.87,
            "quickRatio": 0.83,
            "grossProfitMargin": 0.462,
            "operatingProfitMargin": 0.315,
            "netProfitMargin": 0.239,
            "returnOnAssets": 0.257,
            "returnOnEquity": 1.65,
            "debtEquityRatio": 1.87,
            "interestCoverage": 28.0,
            "assetTurnover": 1.07,
            "payoutRatio": 0.15
        }
    ]);

    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|e| panic!("transform_data: {e}"));

    assert_eq!(rows.len(), 1);
    let r = &rows[0];
    assert_eq!(r.symbol, "AAPL");
    assert_eq!(r.current_ratio, Some(0.87));
    assert_eq!(r.gross_margin, Some(0.462));
    assert_eq!(r.return_on_equity, Some(1.65));
    assert_eq!(r.debt_to_equity, Some(1.87));
    assert_eq!(r.extra_ratios.get("payout_ratio"), Some(&0.15));
}

#[test]
fn cassette_peers_normalises_to_instruments() {
    let fetcher = FmpHttpPeersFetcher::default();
    let query = FmpHttpPeersFetcher::transform_query(json!({"symbol": "AAPL"}))
        .unwrap_or_else(|e| panic!("transform_query: {e}"));

    let raw = cassette_bytes!([
        {"symbol": "AAPL", "peersList": ["MSFT", "GOOGL", "HPQ"]}
    ]);

    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|e| panic!("transform_data: {e}"));

    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].symbol, "MSFT");
    assert_eq!(rows[0].name, "MSFT");
    assert_eq!(rows[0].venue, "fmp");
    assert_eq!(rows[2].symbol, "HPQ");
}

#[test]
fn cassette_profile_normalises_to_company_profile() {
    let fetcher = FmpHttpProfileFetcher::default();
    let query = FmpHttpProfileFetcher::transform_query(json!({"symbol": "AAPL"}))
        .unwrap_or_else(|e| panic!("transform_query: {e}"));

    let raw = cassette_bytes!([
        {
            "symbol": "AAPL",
            "companyName": "Apple Inc.",
            "currency": "USD",
            "exchangeShortName": "NASDAQ",
            "exchange": "NASDAQ Global Select",
            "image": "https://example.com/aapl.png",
            "mktCap": 3450000000000_i64
        }
    ]);

    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|e| panic!("transform_data: {e}"));

    assert_eq!(rows.len(), 1);
    let p = &rows[0];
    assert_eq!(p.ticker, "AAPL");
    assert_eq!(p.name, "Apple Inc.");
    assert_eq!(p.currency, "USD");
    assert_eq!(p.exchange, "NASDAQ");
    assert_eq!(p.logo_url, "https://example.com/aapl.png");
    // 3.45e12 absolute -> 3.45e6 millions.
    assert_eq!(p.market_cap_millions, 3_450_000.0);
}

#[test]
fn cassette_dividends_normalises_to_corporate_actions() {
    let fetcher = FmpHttpDividendsFetcher::default();
    let query = FmpHttpDividendsFetcher::transform_query(json!({"symbol": "AAPL"}))
        .unwrap_or_else(|e| panic!("transform_query: {e}"));

    let raw = cassette_bytes!({
        "symbol": "AAPL",
        "historical": [
            {"date": "2024-08-12", "dividend": 0.25},
            {"date": "2024-05-10", "dividend": 0.25}
        ]
    });

    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|e| panic!("transform_data: {e}"));

    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].symbol, "AAPL");
    assert_eq!(rows[0].action_type, "dividend");
    assert_eq!(rows[0].ex_date, "2024-08-12");
    assert_eq!(rows[0].cash_amount, 0.25);
    assert_eq!(rows[0].split_ratio, 0.0);
}

#[test]
fn cassette_splits_normalises_to_corporate_actions() {
    let fetcher = FmpHttpSplitsFetcher::default();
    let query = FmpHttpSplitsFetcher::transform_query(json!({"symbol": "AAPL"}))
        .unwrap_or_else(|e| panic!("transform_query: {e}"));

    let raw = cassette_bytes!({
        "symbol": "AAPL",
        "historical": [
            {"date": "2020-08-31", "numerator": 4.0, "denominator": 1.0}
        ]
    });

    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|e| panic!("transform_data: {e}"));

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].action_type, "split");
    assert_eq!(rows[0].ex_date, "2020-08-31");
    assert_eq!(rows[0].split_ratio, 4.0);
    assert_eq!(rows[0].cash_amount, 0.0);
}

#[test]
fn cassette_earnings_normalises_to_estimates() {
    let fetcher = FmpHttpEarningsFetcher::default();
    let query = FmpHttpEarningsFetcher::transform_query(json!({"symbol": "AAPL"}))
        .unwrap_or_else(|e| panic!("transform_query: {e}"));

    let raw = cassette_bytes!([
        {"symbol": "AAPL", "date": "2024-08-01", "eps": 1.40, "epsEstimated": 1.35},
        {"symbol": "AAPL", "date": "2024-05-02", "eps": 1.53, "epsEstimated": 1.50}
    ]);

    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|e| panic!("transform_data: {e}"));

    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].symbol, "AAPL");
    assert_eq!(rows[0].kind, "historical_eps");
    assert_eq!(rows[0].date.as_deref(), Some("2024-08-01"));
    assert_eq!(rows[0].value, Some(1.40));
    assert_eq!(rows[0].mean, Some(1.35));
}

#[test]
fn statement_transform_query_rejects_unknown_statement() {
    assert!(
        FmpHttpStatementFetcher::transform_query(json!({"symbol": "AAPL", "statement": "bogus"}))
            .is_err()
    );
}

#[test]
fn fundamentals_malformed_json_produces_provider_error() {
    let fetcher = FmpHttpRatiosFetcher::default();
    let query = FmpHttpRatiosFetcher::transform_query(json!({"symbol": "AAPL"}))
        .unwrap_or_else(|e| panic!("query: {e}"));
    let err = fetcher
        .transform_data(&query, Bytes::from(b"not json".to_vec()))
        .expect_err("malformed JSON must error");
    assert!(err.to_string().contains("fmp ratios parse_json"));
}

// ---------------------------------------------------------------------------
// Live test (gated by TDW_FMP_LIVE=1 and TDW_FMP_API_KEY)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn live_fmp_historical_returns_data_when_env_var_set() {
    if std::env::var("TDW_FMP_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_FMP_LIVE != 1; skipping live FMP historical integration test");
        return;
    }

    let fetcher = FmpHttpHistoricalFetcher::default();
    let query = FmpHttpHistoricalFetcher::transform_query(json!({"symbol": "AAPL"}))
        .unwrap_or_else(|e| panic!("transform_query must succeed: {e}"));

    let rows = live_fetch_nonempty!(fetcher, query);

    assert!(
        !rows.is_empty(),
        "live response must include at least one bar"
    );
    assert_eq!(rows[0].symbol, "AAPL");
}

#[tokio::test]
async fn live_fmp_income_returns_data_when_env_var_set() {
    if std::env::var("TDW_FMP_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_FMP_LIVE != 1; skipping live FMP income integration test");
        return;
    }

    let fetcher = FmpHttpIncomeFetcher::default();
    let query = FmpFundamentalsQuery::new("AAPL", FmpStatement::Income, 3)
        .unwrap_or_else(|e| panic!("query: {e}"));

    let rows = live_fetch_nonempty!(fetcher, query);

    assert!(
        !rows.is_empty(),
        "live income response must include at least one statement"
    );
}

#[tokio::test]
async fn live_fmp_quote_snapshot_returns_data_when_env_var_set() {
    if std::env::var("TDW_FMP_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_FMP_LIVE != 1; skipping live FMP quote-snapshot integration test");
        return;
    }

    let fetcher = FmpHttpQuoteSnapshotFetcher::default();
    let query = FmpHttpQuoteSnapshotFetcher::transform_query(json!({"symbol": "AAPL"}))
        .unwrap_or_else(|e| panic!("transform_query must succeed: {e}"));

    let raw = fetcher
        .extract_data(&query, &Credentials::default())
        .await
        .unwrap_or_else(|e| panic!("live extract_data must succeed: {e}"));
    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|e| panic!("live transform_data must succeed: {e}"));

    assert!(
        !rows.is_empty(),
        "live quote-snapshot response must include at least one entry"
    );
    assert_eq!(rows[0].symbol, "AAPL");
}

#[tokio::test]
async fn live_fmp_statement_returns_data_when_env_var_set() {
    if std::env::var("TDW_FMP_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_FMP_LIVE != 1; skipping live FMP statement integration test");
        return;
    }

    let fetcher = FmpHttpStatementFetcher::default();
    let query = FmpHttpStatementFetcher::transform_query(json!({
        "symbol": "AAPL", "statement": "balance", "limit": 2
    }))
    .unwrap_or_else(|e| panic!("transform_query must succeed: {e}"));

    let rows = live_fetch_nonempty!(fetcher, query);
    assert!(
        !rows.is_empty(),
        "live statement response must include at least one period"
    );
    assert_eq!(rows[0].statement, StatementKind::Balance);
}

#[tokio::test]
async fn live_fmp_key_metrics_returns_data_when_env_var_set() {
    if std::env::var("TDW_FMP_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_FMP_LIVE != 1; skipping live FMP key-metrics integration test");
        return;
    }

    let fetcher = FmpHttpKeyMetricsFetcher::default();
    let query = FmpHttpKeyMetricsFetcher::transform_query(json!({"symbol": "AAPL", "limit": 2}))
        .unwrap_or_else(|e| panic!("transform_query must succeed: {e}"));

    let rows = live_fetch_nonempty!(fetcher, query);
    assert!(!rows.is_empty(), "live key-metrics must include a row");
}

#[tokio::test]
async fn live_fmp_ratios_returns_data_when_env_var_set() {
    if std::env::var("TDW_FMP_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_FMP_LIVE != 1; skipping live FMP ratios integration test");
        return;
    }

    let fetcher = FmpHttpRatiosFetcher::default();
    let query = FmpHttpRatiosFetcher::transform_query(json!({"symbol": "AAPL", "limit": 2}))
        .unwrap_or_else(|e| panic!("transform_query must succeed: {e}"));

    let rows = live_fetch_nonempty!(fetcher, query);
    assert!(!rows.is_empty(), "live ratios must include a row");
}

#[tokio::test]
async fn live_fmp_peers_returns_data_when_env_var_set() {
    if std::env::var("TDW_FMP_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_FMP_LIVE != 1; skipping live FMP peers integration test");
        return;
    }

    let fetcher = FmpHttpPeersFetcher::default();
    let query = FmpHttpPeersFetcher::transform_query(json!({"symbol": "AAPL"}))
        .unwrap_or_else(|e| panic!("transform_query must succeed: {e}"));

    let rows = live_fetch_nonempty!(fetcher, query);
    assert!(
        !rows.is_empty(),
        "live peers must include at least one peer"
    );
}

#[tokio::test]
async fn live_fmp_profile_returns_data_when_env_var_set() {
    if std::env::var("TDW_FMP_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_FMP_LIVE != 1; skipping live FMP profile integration test");
        return;
    }

    let fetcher = FmpHttpProfileFetcher::default();
    let query = FmpHttpProfileFetcher::transform_query(json!({"symbol": "AAPL"}))
        .unwrap_or_else(|e| panic!("transform_query must succeed: {e}"));

    let rows = live_fetch_nonempty!(fetcher, query);
    assert!(!rows.is_empty(), "live profile must include an entry");
    assert_eq!(rows[0].ticker, "AAPL");
}
