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
    BASE_URL, FmpFundamentalsQuery, FmpHistoricalQuery, FmpHttpAnalystEstimatesFetcher,
    FmpHttpDiscoveryFetcher, FmpHttpDividendsFetcher, FmpHttpEarningsFetcher,
    FmpHttpEmployeeCountFetcher, FmpHttpEsgScoreFetcher, FmpHttpEtfCountriesFetcher,
    FmpHttpEtfEquityExposureFetcher, FmpHttpEtfInfoFetcher, FmpHttpEtfPricePerformanceFetcher,
    FmpHttpEtfSearchFetcher, FmpHttpEtfSectorsFetcher, FmpHttpExecutiveCompensationFetcher,
    FmpHttpFilingsFetcher, FmpHttpGovernmentTradesFetcher, FmpHttpHistoricalFetcher,
    FmpHttpHistoricalMarketCapFetcher, FmpHttpIncomeFetcher, FmpHttpInsiderTradingFetcher,
    FmpHttpInstitutionalOwnershipFetcher, FmpHttpKeyExecutivesFetcher, FmpHttpKeyMetricsFetcher,
    FmpHttpLatestFilingsFetcher, FmpHttpPeersFetcher, FmpHttpPriceTargetFetcher,
    FmpHttpProfileFetcher, FmpHttpQuoteSnapshotFetcher, FmpHttpRatiosFetcher,
    FmpHttpRevenueSegmentFetcher, FmpHttpScreenerFetcher, FmpHttpSearchFetcher,
    FmpHttpSplitCalendarFetcher, FmpHttpSplitsFetcher, FmpHttpStatementFetcher,
    FmpHttpTranscriptFetcher, FmpStatement,
};
use tdw_provider_testkit::{cassette_bytes, live_fetch_nonempty};

#[test]
fn base_url_uses_tls() {
    assert!(
        BASE_URL.starts_with("https://"),
        "FMP base URL must use TLS, got {BASE_URL}"
    );
}

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
fn cassette_price_target_normalises_to_estimate() {
    let fetcher = FmpHttpPriceTargetFetcher::default();
    let query = FmpHttpPriceTargetFetcher::transform_query(json!({"symbol": "AAPL"}))
        .unwrap_or_else(|e| panic!("transform_query: {e}"));

    let raw = cassette_bytes!([
        {
            "symbol": "AAPL",
            "targetHigh": 300.0,
            "targetLow": 150.0,
            "targetConsensus": 240.0,
            "targetMedian": 235.0
        }
    ]);

    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|e| panic!("transform_data: {e}"));

    assert_eq!(rows.len(), 1);
    let r = &rows[0];
    assert_eq!(r.symbol, "AAPL");
    assert_eq!(r.kind, "price_target");
    assert_eq!(r.value, Some(240.0));
    assert_eq!(r.mean, Some(240.0));
    assert_eq!(r.high, Some(300.0));
    assert_eq!(r.low, Some(150.0));
}

#[test]
fn cassette_price_target_falls_back_to_median_when_consensus_missing() {
    let fetcher = FmpHttpPriceTargetFetcher::default();
    let query = FmpHttpPriceTargetFetcher::transform_query(json!({"symbol": "AAPL"}))
        .unwrap_or_else(|e| panic!("transform_query: {e}"));

    // No targetConsensus → value/mean fall back to targetMedian.
    let raw = cassette_bytes!([
        {"symbol": "AAPL", "targetHigh": 300.0, "targetLow": 150.0, "targetMedian": 235.0}
    ]);

    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|e| panic!("transform_data: {e}"));

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].value, Some(235.0));
    assert_eq!(rows[0].mean, Some(235.0));
}

#[test]
fn cassette_price_target_empty_array_yields_no_rows() {
    let fetcher = FmpHttpPriceTargetFetcher::default();
    let query = FmpHttpPriceTargetFetcher::transform_query(json!({"symbol": "AAPL"}))
        .unwrap_or_else(|e| panic!("transform_query: {e}"));
    let rows = fetcher
        .transform_data(&query, cassette_bytes!([]))
        .unwrap_or_else(|e| panic!("transform_data: {e}"));
    assert!(rows.is_empty(), "empty array must yield no estimates");
}

#[test]
fn price_target_malformed_json_produces_provider_error() {
    let fetcher = FmpHttpPriceTargetFetcher::default();
    let query = FmpHttpPriceTargetFetcher::transform_query(json!({"symbol": "AAPL"}))
        .unwrap_or_else(|e| panic!("query: {e}"));
    let err = fetcher
        .transform_data(&query, Bytes::from(b"not json".to_vec()))
        .expect_err("malformed JSON must error");
    assert!(err.to_string().contains("fmp price_target parse_json"));
}

#[test]
fn cassette_analyst_estimates_emits_forward_rows_per_period() {
    let fetcher = FmpHttpAnalystEstimatesFetcher::default();
    let query = FmpHttpAnalystEstimatesFetcher::transform_query(json!({"symbol": "AAPL"}))
        .unwrap_or_else(|e| panic!("transform_query: {e}"));

    // Two periods: the first carries all three forward metrics; the second omits
    // EBITDA (null) so only forward_eps + forward_sales are emitted for it.
    let raw = cassette_bytes!([
        {
            "symbol": "AAPL",
            "date": "2026-09-30",
            "estimatedRevenueLow": 400.0,
            "estimatedRevenueHigh": 440.0,
            "estimatedRevenueAvg": 420.0,
            "estimatedEpsLow": 6.0,
            "estimatedEpsHigh": 7.0,
            "estimatedEpsAvg": 6.5,
            "estimatedEbitdaLow": 130.0,
            "estimatedEbitdaHigh": 150.0,
            "estimatedEbitdaAvg": 140.0,
            "numberAnalystEstimatedRevenue": 30,
            "numberAnalystsEstimatedEps": 28
        },
        {
            "symbol": "AAPL",
            "date": "2027-09-30",
            "estimatedRevenueLow": 430.0,
            "estimatedRevenueHigh": 470.0,
            "estimatedRevenueAvg": 450.0,
            "estimatedEpsLow": 6.5,
            "estimatedEpsHigh": 7.5,
            "estimatedEpsAvg": 7.0,
            "estimatedEbitdaAvg": null
        }
    ]);

    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|e| panic!("transform_data: {e}"));

    // Period 1: eps + sales + ebitda = 3; period 2: eps + sales = 2 → 5 total.
    assert_eq!(rows.len(), 5);

    let eps = rows
        .iter()
        .find(|r| r.kind == "forward_eps" && r.fiscal_period.as_deref() == Some("2026-09-30"))
        .expect("forward_eps row for 2026 period");
    assert_eq!(eps.symbol, "AAPL");
    assert_eq!(eps.value, Some(6.5));
    assert_eq!(eps.mean, Some(6.5));
    assert_eq!(eps.low, Some(6.0));
    assert_eq!(eps.high, Some(7.0));
    assert_eq!(eps.number_of_analysts, Some(28));

    let sales = rows
        .iter()
        .find(|r| r.kind == "forward_sales" && r.fiscal_period.as_deref() == Some("2026-09-30"))
        .expect("forward_sales row for 2026 period");
    assert_eq!(sales.value, Some(420.0));
    assert_eq!(sales.low, Some(400.0));
    assert_eq!(sales.high, Some(440.0));
    assert_eq!(sales.number_of_analysts, Some(30));

    let ebitda = rows
        .iter()
        .find(|r| r.kind == "forward_ebitda" && r.fiscal_period.as_deref() == Some("2026-09-30"))
        .expect("forward_ebitda row for 2026 period");
    assert_eq!(ebitda.value, Some(140.0));

    // The 2027 period had a null EBITDA avg → no forward_ebitda row emitted.
    assert!(
        !rows
            .iter()
            .any(|r| r.kind == "forward_ebitda" && r.fiscal_period.as_deref() == Some("2027-09-30")),
        "missing EBITDA avg must not emit a forward_ebitda row"
    );
}

#[test]
fn cassette_analyst_estimates_empty_array_yields_no_rows() {
    let fetcher = FmpHttpAnalystEstimatesFetcher::default();
    let query = FmpHttpAnalystEstimatesFetcher::transform_query(json!({"symbol": "AAPL"}))
        .unwrap_or_else(|e| panic!("transform_query: {e}"));
    let rows = fetcher
        .transform_data(&query, cassette_bytes!([]))
        .unwrap_or_else(|e| panic!("transform_data: {e}"));
    assert!(rows.is_empty(), "empty array must yield no estimates");
}

#[test]
fn analyst_estimates_malformed_json_produces_provider_error() {
    let fetcher = FmpHttpAnalystEstimatesFetcher::default();
    let query = FmpHttpAnalystEstimatesFetcher::transform_query(json!({"symbol": "AAPL"}))
        .unwrap_or_else(|e| panic!("query: {e}"));
    let err = fetcher
        .transform_data(&query, Bytes::from(b"not json".to_vec()))
        .expect_err("malformed JSON must error");
    assert!(err.to_string().contains("fmp analyst_estimates parse_json"));
}

#[test]
fn cassette_discovery_normalises_to_quote_snapshots() {
    let fetcher = FmpHttpDiscoveryFetcher::default();
    let query = FmpHttpDiscoveryFetcher::transform_query(json!({"direction": "gainers"}))
        .unwrap_or_else(|e| panic!("transform_query: {e}"));

    let raw = cassette_bytes!([
        {"symbol": "AAPL", "name": "Apple Inc.", "change": 5.10, "price": 202.0, "changesPercentage": 2.59},
        {"symbol": "MSFT", "name": "Microsoft", "change": 3.20, "price": 415.0, "changesPercentage": 0.78}
    ]);

    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|e| panic!("transform_data: {e}"));

    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].symbol, "AAPL");
    assert_eq!(rows[0].current_price, 202.0);
    assert_eq!(rows[0].change, 5.10);
    assert_eq!(rows[0].change_percent, 2.59);
    // The movers feed carries no previous close or timestamp.
    assert_eq!(rows[0].prev_close, 0.0);
    assert_eq!(rows[0].ts_ms, 0);
    assert_eq!(rows[1].symbol, "MSFT");
}

#[test]
fn discovery_transform_query_maps_direction_param() {
    use tdw_provider_fmp::FmpDiscoveryDirection;
    let gainers = FmpHttpDiscoveryFetcher::transform_query(json!({"direction": "gainers"}))
        .unwrap_or_else(|e| panic!("query: {e}"));
    assert_eq!(gainers.direction, FmpDiscoveryDirection::Gainers);
    let losers = FmpHttpDiscoveryFetcher::transform_query(json!({"direction": "losers"}))
        .unwrap_or_else(|e| panic!("query: {e}"));
    assert_eq!(losers.direction, FmpDiscoveryDirection::Losers);
    let actives = FmpHttpDiscoveryFetcher::transform_query(json!({"direction": "actives"}))
        .unwrap_or_else(|e| panic!("query: {e}"));
    assert_eq!(actives.direction, FmpDiscoveryDirection::Actives);
    // Missing/unknown direction defaults to gainers.
    let default = FmpHttpDiscoveryFetcher::transform_query(json!({}))
        .unwrap_or_else(|e| panic!("query: {e}"));
    assert_eq!(default.direction, FmpDiscoveryDirection::Gainers);
}

#[test]
fn cassette_screener_normalises_to_screener_rows() {
    let fetcher = FmpHttpScreenerFetcher::default();
    let query = FmpHttpScreenerFetcher::transform_query(json!({
        "sector": "Technology", "market_cap_more_than": 1000000000.0, "limit": 10
    }))
    .unwrap_or_else(|e| panic!("transform_query: {e}"));
    assert_eq!(query.sector.as_deref(), Some("Technology"));
    assert_eq!(query.market_cap_more_than, Some(1_000_000_000.0));
    assert_eq!(query.limit, Some(10));

    let raw = cassette_bytes!([
        {
            "symbol": "AAPL",
            "companyName": "Apple Inc.",
            "marketCap": 3450000000000_i64,
            "sector": "Technology",
            "industry": "Consumer Electronics",
            "beta": 1.24,
            "price": 202.0,
            "lastAnnualDividend": 0.99,
            "volume": 55000000_i64,
            "exchange": "NASDAQ Global Select",
            "exchangeShortName": "NASDAQ",
            "country": "US",
            "isEtf": false,
            "isActivelyTrading": true
        }
    ]);

    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|e| panic!("transform_data: {e}"));

    assert_eq!(rows.len(), 1);
    let r = &rows[0];
    assert_eq!(r.symbol, "AAPL");
    assert_eq!(r.company_name.as_deref(), Some("Apple Inc."));
    assert_eq!(r.market_cap, Some(3_450_000_000_000.0));
    assert_eq!(r.sector.as_deref(), Some("Technology"));
    assert_eq!(r.beta, Some(1.24));
    assert_eq!(r.last_annual_dividend, Some(0.99));
    assert_eq!(r.exchange_short_name.as_deref(), Some("NASDAQ"));
    assert_eq!(r.is_etf, Some(false));
    assert_eq!(r.is_actively_trading, Some(true));
}

#[test]
fn screener_malformed_json_produces_provider_error() {
    let fetcher = FmpHttpScreenerFetcher::default();
    let query =
        FmpHttpScreenerFetcher::transform_query(json!({})).unwrap_or_else(|e| panic!("query: {e}"));
    let err = fetcher
        .transform_data(&query, Bytes::from(b"not json".to_vec()))
        .expect_err("malformed JSON must error");
    assert!(err.to_string().contains("fmp screener parse_json"));
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
// P4W1 cassette tests — equity/fundamental breadth (no network)
// ---------------------------------------------------------------------------

#[test]
fn cassette_parse_fmp_key_executives_response() {
    let fetcher = FmpHttpKeyExecutivesFetcher::default();
    let query = FmpHttpKeyExecutivesFetcher::transform_query(json!({"symbol": "AAPL"}))
        .unwrap_or_else(|e| panic!("query: {e}"));
    let raw = cassette_bytes!([
        {
            "title": "Chief Executive Officer",
            "name": "Mr. Timothy D. Cook",
            "pay": 16_239_562.0,
            "currencyPay": "USD",
            "gender": "male",
            "yearBorn": 1960,
            "titleSince": 2011,
            "symbol": "AAPL"
        },
        {
            "title": "Chief Financial Officer",
            "name": "Mr. Luca Maestri",
            "symbol": "AAPL"
        }
    ]);
    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|e| panic!("transform_data: {e}"));
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].symbol, "AAPL");
    assert_eq!(rows[0].name, "Mr. Timothy D. Cook");
    assert_eq!(rows[0].title.as_deref(), Some("Chief Executive Officer"));
    assert_eq!(rows[0].currency.as_deref(), Some("USD"));
    assert_eq!(rows[0].year_born, Some(1960));
}

#[test]
fn cassette_parse_fmp_executive_compensation_response() {
    let fetcher = FmpHttpExecutiveCompensationFetcher::default();
    let query = FmpHttpExecutiveCompensationFetcher::transform_query(json!({"symbol": "AAPL"}))
        .unwrap_or_else(|e| panic!("query: {e}"));
    let raw = cassette_bytes!([
        {
            "symbol": "AAPL",
            "nameAndPosition": "Timothy D. Cook Chief Executive Officer",
            "year": 2023,
            "acceptedDate": "2024-01-11",
            "salary": 3_000_000.0,
            "bonus": 0.0,
            "stock_award": 46_970_283.0,
            "option_award": 0.0,
            "incentive_plan_compensation": 10_713_360.0,
            "all_other_compensation": 2_466_545.0,
            "total": 63_209_845.0
        }
    ]);
    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|e| panic!("transform_data: {e}"));
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].symbol, "AAPL");
    assert_eq!(rows[0].fiscal_year, Some(2023));
    assert_eq!(rows[0].total, Some(63_209_845.0));
    assert_eq!(rows[0].stock_award, Some(46_970_283.0));
}

#[test]
fn cassette_parse_fmp_revenue_segment_response() {
    let fetcher = FmpHttpRevenueSegmentFetcher::default();
    let query = FmpHttpRevenueSegmentFetcher::transform_query(
        json!({"symbol": "AAPL", "structure": "product"}),
    )
    .unwrap_or_else(|e| panic!("query: {e}"));
    let raw = cassette_bytes!([
        { "2023-09-30": { "iPhone": 200_583_000_000.0, "Mac": 29_357_000_000.0 } },
        { "2022-09-24": { "iPhone": 205_489_000_000.0 } }
    ]);
    let mut rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|e| panic!("transform_data: {e}"));
    rows.sort_by(|a, b| {
        (a.date.clone(), a.segment.clone()).cmp(&(b.date.clone(), b.segment.clone()))
    });
    assert_eq!(rows.len(), 3);
    assert!(
        rows.iter()
            .all(|r| r.symbol == "AAPL" && r.kind == "product")
    );
    let iphone_2023 = rows
        .iter()
        .find(|r| r.date == "2023-09-30" && r.segment == "iPhone")
        .expect("iPhone 2023 row present");
    assert_eq!(iphone_2023.revenue, Some(200_583_000_000.0));
}

#[test]
fn cassette_parse_fmp_transcript_response() {
    let fetcher = FmpHttpTranscriptFetcher::default();
    let query = FmpHttpTranscriptFetcher::transform_query(
        json!({"symbol": "AAPL", "year": 2024, "quarter": 1}),
    )
    .unwrap_or_else(|e| panic!("query: {e}"));
    let raw = cassette_bytes!([
        {
            "symbol": "AAPL",
            "quarter": 1,
            "year": 2024,
            "date": "2024-02-01 17:00:00",
            "content": "Operator: Good day, and welcome to the Apple Q1 call."
        }
    ]);
    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|e| panic!("transform_data: {e}"));
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].symbol, "AAPL");
    assert_eq!(rows[0].year, Some(2024));
    assert_eq!(rows[0].quarter, Some(1));
    assert!(rows[0].content.starts_with("Operator:"));
}

#[test]
fn cassette_parse_fmp_esg_score_response() {
    let fetcher = FmpHttpEsgScoreFetcher::default();
    let query = FmpHttpEsgScoreFetcher::transform_query(json!({"symbol": "AAPL"}))
        .unwrap_or_else(|e| panic!("query: {e}"));
    let raw = cassette_bytes!([
        {
            "symbol": "AAPL",
            "cik": "0000320193",
            "date": "2023-09-30",
            "companyName": "Apple Inc.",
            "environmentalScore": 50.0,
            "socialScore": 67.0,
            "governanceScore": 63.0,
            "ESGScore": 60.0
        }
    ]);
    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|e| panic!("transform_data: {e}"));
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].symbol, "AAPL");
    assert_eq!(rows[0].date, "2023-09-30");
    assert_eq!(rows[0].esg_score, Some(60.0));
    assert_eq!(rows[0].social_score, Some(67.0));
}

#[test]
fn cassette_parse_fmp_employee_count_response() {
    let fetcher = FmpHttpEmployeeCountFetcher::default();
    let query = FmpHttpEmployeeCountFetcher::transform_query(json!({"symbol": "AAPL"}))
        .unwrap_or_else(|e| panic!("query: {e}"));
    let raw = cassette_bytes!([
        {
            "symbol": "AAPL",
            "cik": "0000320193",
            "acceptanceTime": "2023-11-02 18:08:27",
            "periodOfReport": "2023-09-30",
            "filingDate": "2023-11-03",
            "employeeCount": 161_000,
            "source": "https://www.sec.gov/..."
        }
    ]);
    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|e| panic!("transform_data: {e}"));
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].symbol, "AAPL");
    assert_eq!(rows[0].period_of_report, "2023-09-30");
    assert_eq!(rows[0].employee_count, Some(161_000));
}

#[test]
fn cassette_parse_fmp_filings_response() {
    let fetcher = FmpHttpFilingsFetcher::default();
    let query = FmpHttpFilingsFetcher::transform_query(json!({"symbol": "AAPL"}))
        .unwrap_or_else(|e| panic!("query: {e}"));
    let raw = cassette_bytes!([
        {
            "symbol": "AAPL",
            "fillingDate": "2023-11-03 00:00:00",
            "acceptedDate": "2023-11-02 18:08:27",
            "cik": "0000320193",
            "type": "10-K",
            "link": "https://www.sec.gov/cgi-bin/browse-edgar?...",
            "finalLink": "https://www.sec.gov/Archives/....htm"
        }
    ]);
    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|e| panic!("transform_data: {e}"));
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].symbol, "AAPL");
    assert_eq!(rows[0].form_type, "10-K");
    assert_eq!(rows[0].filing_date.as_deref(), Some("2023-11-03 00:00:00"));
    assert_eq!(rows[0].cik.as_deref(), Some("0000320193"));
}

#[test]
fn cassette_parse_fmp_statement_growth_response() {
    let fetcher = FmpHttpStatementFetcher::default();
    let query = FmpHttpStatementFetcher::transform_query(
        json!({"symbol": "AAPL", "statement": "balance", "growth": true}),
    )
    .unwrap_or_else(|e| panic!("query: {e}"));
    let raw = cassette_bytes!([
        {
            "date": "2023-09-30",
            "symbol": "AAPL",
            "calendarYear": 2023,
            "period": "FY",
            "growthTotalAssets": 0.0123,
            "growthTotalLiabilities": -0.0456
        }
    ]);
    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|e| panic!("transform_data: {e}"));
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].symbol, "AAPL");
    assert_eq!(rows[0].statement, StatementKind::Balance);
    assert_eq!(
        rows[0].line_items.get("growth_total_assets").copied(),
        Some(0.0123)
    );
}

// ---------------------------------------------------------------------------
// P4W2 cassette tests: search / market-cap / split-calendar / latest-filings /
// insider / institutional / government trades
// ---------------------------------------------------------------------------

#[test]
fn cassette_parse_fmp_search_response() {
    let fetcher = FmpHttpSearchFetcher::default();
    let query = FmpHttpSearchFetcher::transform_query(json!({"query": "apple", "limit": 5}))
        .unwrap_or_else(|e| panic!("transform_query: {e}"));
    let raw = cassette_bytes!([
        {"symbol": "AAPL", "name": "Apple Inc.", "exchangeShortName": "NASDAQ"},
        {"symbol": "APLE", "name": "Apple Hospitality REIT", "stockExchange": "NYSE"},
        {"symbol": "", "name": "ignored blank symbol"}
    ]);
    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|e| panic!("transform_data: {e}"));
    assert_eq!(rows.len(), 2, "rows={rows:#?}");
    assert_eq!(rows[0].symbol, "AAPL");
    assert_eq!(rows[0].name, "Apple Inc.");
    assert_eq!(rows[0].venue, "NASDAQ");
    assert_eq!(rows[1].venue, "NYSE");
}

#[test]
fn cassette_parse_fmp_historical_market_cap_response() {
    let fetcher = FmpHttpHistoricalMarketCapFetcher::default();
    let query =
        FmpHttpHistoricalMarketCapFetcher::transform_query(json!({"symbol": "AAPL", "limit": 3}))
            .unwrap_or_else(|e| panic!("transform_query: {e}"));
    let raw = cassette_bytes!([
        {"symbol": "AAPL", "date": "2024-09-28", "marketCap": 3450000000000.0},
        {"symbol": "AAPL", "date": "2024-09-27", "marketCap": 3440000000000.0}
    ]);
    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|e| panic!("transform_data: {e}"));
    assert_eq!(rows.len(), 2, "rows={rows:#?}");
    assert_eq!(rows[0].symbol, "AAPL");
    assert_eq!(rows[0].date, "2024-09-28");
    assert_eq!(rows[0].market_cap, Some(3_450_000_000_000.0));
}

#[test]
fn cassette_parse_fmp_split_calendar_response() {
    let fetcher = FmpHttpSplitCalendarFetcher::default();
    let query = FmpHttpSplitCalendarFetcher::transform_query(json!({"from": "2024-01-01"}))
        .unwrap_or_else(|e| panic!("transform_query: {e}"));
    let raw = cassette_bytes!([
        {"symbol": "NVDA", "date": "2024-06-10", "label": "June 10, 24", "numerator": 10.0, "denominator": 1.0},
        {"symbol": "", "date": "2024-07-01"}
    ]);
    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|e| panic!("transform_data: {e}"));
    assert_eq!(rows.len(), 1, "rows={rows:#?}");
    assert_eq!(rows[0].kind, "split");
    assert_eq!(rows[0].symbol, "NVDA");
    assert_eq!(rows[0].date.as_deref(), Some("2024-06-10"));
    assert_eq!(rows[0].price, Some(10.0));
}

#[test]
fn cassette_parse_fmp_split_calendar_zero_denominator_yields_no_ratio() {
    // A denominator of exactly 0.0 makes the split ratio undefined: the row must
    // still be emitted but with `price` (the ratio) left as None, rather than
    // falling through to the numerator-only arm.
    let fetcher = FmpHttpSplitCalendarFetcher::default();
    let query = FmpHttpSplitCalendarFetcher::transform_query(json!({"from": "2024-01-01"}))
        .unwrap_or_else(|e| panic!("transform_query: {e}"));
    let raw = cassette_bytes!([
        {"symbol": "ZERO", "date": "2024-06-10", "numerator": 10.0, "denominator": 0.0}
    ]);
    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|e| panic!("transform_data: {e}"));
    assert_eq!(rows.len(), 1, "rows={rows:#?}");
    assert_eq!(rows[0].symbol, "ZERO");
    assert_eq!(
        rows[0].price, None,
        "zero denominator must not yield a ratio"
    );
}

#[test]
fn cassette_parse_fmp_latest_filings_response() {
    let fetcher = FmpHttpLatestFilingsFetcher::default();
    let query = FmpHttpLatestFilingsFetcher::transform_query(json!({"limit": 50}))
        .unwrap_or_else(|e| panic!("transform_query: {e}"));
    let raw = cassette_bytes!([
        {"symbol": "AAPL", "type": "8-K", "date": "2024-11-01", "cik": "0000320193", "link": "https://sec.gov/a"},
        {"symbol": "MSFT", "type": "", "date": "2024-11-01"}
    ]);
    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|e| panic!("transform_data: {e}"));
    assert_eq!(rows.len(), 1, "rows={rows:#?}");
    assert_eq!(rows[0].symbol, "AAPL");
    assert_eq!(rows[0].form_type, "8-K");
    assert_eq!(rows[0].filing_date.as_deref(), Some("2024-11-01"));
}

#[test]
fn cassette_parse_fmp_insider_trading_response() {
    let fetcher = FmpHttpInsiderTradingFetcher::default();
    let query = FmpHttpInsiderTradingFetcher::transform_query(json!({"symbol": "AAPL"}))
        .unwrap_or_else(|e| panic!("transform_query: {e}"));
    let raw = cassette_bytes!([
        {
            "symbol": "AAPL", "reportingName": "COOK TIMOTHY", "typeOfOwner": "officer: CEO",
            "transactionDate": "2024-04-02", "transactionType": "S-Sale",
            "securitiesTransacted": 100.0, "price": 170.0
        }
    ]);
    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|e| panic!("transform_data: {e}"));
    assert_eq!(rows.len(), 1, "rows={rows:#?}");
    assert_eq!(rows[0].kind, "insider");
    assert_eq!(rows[0].holder.as_deref(), Some("COOK TIMOTHY"));
    assert_eq!(rows[0].transaction_type.as_deref(), Some("S-Sale"));
    assert_eq!(rows[0].shares, Some(100.0));
    assert_eq!(rows[0].value, Some(17_000.0));
}

#[test]
fn cassette_parse_fmp_institutional_ownership_response() {
    let fetcher = FmpHttpInstitutionalOwnershipFetcher::default();
    let query = FmpHttpInstitutionalOwnershipFetcher::transform_query(json!({"symbol": "AAPL"}))
        .unwrap_or_else(|e| panic!("transform_query: {e}"));
    let raw = cassette_bytes!([
        {"holder": "VANGUARD GROUP INC", "shares": 1300000000.0, "dateReported": "2024-06-30", "change": 5000000.0},
        {"holder": "", "shares": 1.0}
    ]);
    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|e| panic!("transform_data: {e}"));
    assert_eq!(rows.len(), 1, "rows={rows:#?}");
    assert_eq!(rows[0].kind, "institutional");
    assert_eq!(rows[0].holder.as_deref(), Some("VANGUARD GROUP INC"));
    assert_eq!(rows[0].shares, Some(1_300_000_000.0));
}

#[test]
fn cassette_parse_fmp_government_trades_response() {
    let fetcher = FmpHttpGovernmentTradesFetcher::default();
    let query = FmpHttpGovernmentTradesFetcher::transform_query(json!({"symbol": "AAPL"}))
        .unwrap_or_else(|e| panic!("transform_query: {e}"));
    let raw = cassette_bytes!([
        {
            "symbol": "AAPL", "representative": "Jane Senator", "office": "Senate",
            "transactionDate": "2024-03-01", "type": "Purchase", "amount": "$1,001 - $15,000"
        }
    ]);
    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|e| panic!("transform_data: {e}"));
    assert_eq!(rows.len(), 1, "rows={rows:#?}");
    assert_eq!(rows[0].kind, "government_trade");
    assert_eq!(rows[0].holder.as_deref(), Some("Jane Senator"));
    assert_eq!(rows[0].relationship.as_deref(), Some("Senate"));
    assert_eq!(rows[0].transaction_type.as_deref(), Some("Purchase"));
    assert_eq!(rows[0].value, Some(1001.0));
}

#[test]
fn cassette_parse_fmp_government_trades_open_ended_low_buckets() {
    // Open-ended low buckets ("$1,000 or less", "under $1,001", "below $1,001")
    // have a lower bound of zero; the leading number must not be mistaken for the
    // disclosed minimum.
    let fetcher = FmpHttpGovernmentTradesFetcher::default();
    let query = FmpHttpGovernmentTradesFetcher::transform_query(json!({"symbol": "AAPL"}))
        .unwrap_or_else(|e| panic!("transform_query: {e}"));
    let raw = cassette_bytes!([
        {
            "symbol": "AAPL", "representative": "A", "office": "House",
            "transactionDate": "2024-03-01", "type": "Purchase", "amount": "$1,000 or less"
        },
        {
            "symbol": "AAPL", "representative": "B", "office": "House",
            "transactionDate": "2024-03-02", "type": "Purchase", "amount": "under $1,001"
        },
        {
            "symbol": "AAPL", "representative": "C", "office": "House",
            "transactionDate": "2024-03-03", "type": "Purchase", "amount": "below $1,001"
        }
    ]);
    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|e| panic!("transform_data: {e}"));
    assert_eq!(rows.len(), 3, "rows={rows:#?}");
    assert!(
        rows.iter().all(|r| r.value == Some(0.0)),
        "open-ended low buckets must report a zero lower bound: {rows:#?}",
    );
}

// ---------------------------------------------------------------------------
// ETF cluster cassette tests (openbb-parity P4W3)
// ---------------------------------------------------------------------------

#[test]
fn cassette_parse_fmp_etf_search_filters_by_query() {
    let fetcher = FmpHttpEtfSearchFetcher::default();
    let query = FmpHttpEtfSearchFetcher::transform_query(json!({"query": "S&P", "limit": 10}))
        .unwrap_or_else(|e| panic!("transform_query: {e}"));
    let raw = cassette_bytes!([
        {"symbol": "SPY", "name": "SPDR S&P 500 ETF Trust", "exchangeShortName": "NYSE Arca"},
        {"symbol": "QQQ", "name": "Invesco QQQ Trust", "exchangeShortName": "NASDAQ"},
        {"symbol": "", "name": "junk"}
    ]);
    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|e| panic!("transform_data: {e}"));
    assert_eq!(rows.len(), 1, "only the S&P match survives: {rows:#?}");
    assert_eq!(rows[0].symbol, "SPY");
    assert_eq!(rows[0].venue, "NYSE Arca");
}

#[test]
fn cassette_parse_fmp_etf_info_response() {
    let fetcher = FmpHttpEtfInfoFetcher::default();
    let query = FmpHttpEtfInfoFetcher::transform_query(json!({"symbol": "SPY"}))
        .unwrap_or_else(|e| panic!("transform_query: {e}"));
    let raw = cassette_bytes!([
        {
            "symbol": "SPY", "name": "SPDR S&P 500 ETF Trust", "etfCompany": "State Street",
            "expenseRatio": 0.000945, "holdingsCount": 503, "exchangeShortName": "NYSE Arca",
            "inceptionDate": "1993-01-22"
        }
    ]);
    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|e| panic!("transform_data: {e}"));
    assert_eq!(rows.len(), 1, "rows={rows:#?}");
    assert_eq!(rows[0].symbol, "SPY");
    assert_eq!(rows[0].issuer.as_deref(), Some("State Street"));
    assert_eq!(rows[0].holdings_count, Some(503));
}

#[test]
fn cassette_parse_fmp_etf_sectors_parses_percent_strings() {
    let fetcher = FmpHttpEtfSectorsFetcher::default();
    let query = FmpHttpEtfSectorsFetcher::transform_query(json!({"symbol": "SPY"}))
        .unwrap_or_else(|e| panic!("transform_query: {e}"));
    let raw = cassette_bytes!([
        {"sector": "Technology", "weightPercentage": "29.50%"},
        {"sector": "Financials", "weightPercentage": "13.10%"},
        {"sector": "", "weightPercentage": "1.00%"}
    ]);
    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|e| panic!("transform_data: {e}"));
    assert_eq!(rows.len(), 2, "blank sector dropped: {rows:#?}");
    assert_eq!(rows[0].fund_symbol, "SPY");
    assert_eq!(rows[0].sector, "Technology");
    assert_eq!(rows[0].weight_pct, Some(29.50));
}

#[test]
fn cassette_parse_fmp_etf_countries_parses_percent_strings() {
    let fetcher = FmpHttpEtfCountriesFetcher::default();
    let query = FmpHttpEtfCountriesFetcher::transform_query(json!({"symbol": "SPY"}))
        .unwrap_or_else(|e| panic!("transform_query: {e}"));
    let raw = cassette_bytes!([
        {"country": "United States", "weightPercentage": "98.70%"},
        {"country": "Ireland", "weightPercentage": "1.30%"}
    ]);
    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|e| panic!("transform_data: {e}"));
    assert_eq!(rows.len(), 2, "rows={rows:#?}");
    assert_eq!(rows[0].country, "United States");
    assert_eq!(rows[0].weight_pct, Some(98.70));
}

#[test]
fn cassette_parse_fmp_etf_price_performance_scales_to_fraction() {
    let fetcher = FmpHttpEtfPricePerformanceFetcher::default();
    let query = FmpHttpEtfPricePerformanceFetcher::transform_query(json!({"symbol": "SPY"}))
        .unwrap_or_else(|e| panic!("transform_query: {e}"));
    // FMP reports period changes as whole-number percentages.
    let raw = cassette_bytes!([
        {"symbol": "SPY", "1D": 0.5, "5D": 1.2, "1M": 3.4, "3M": 8.0, "ytd": 15.0, "1Y": 27.0}
    ]);
    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|e| panic!("transform_data: {e}"));
    assert_eq!(rows.len(), 1, "rows={rows:#?}");
    assert_eq!(rows[0].symbol, "SPY");
    // 0.5% -> 0.005 fraction.
    assert!((rows[0].one_day.unwrap_or_default() - 0.005).abs() < 1e-9);
    assert!((rows[0].one_year.unwrap_or_default() - 0.27).abs() < 1e-9);
}

#[test]
fn cassette_parse_fmp_etf_equity_exposure_response() {
    let fetcher = FmpHttpEtfEquityExposureFetcher::default();
    let query = FmpHttpEtfEquityExposureFetcher::transform_query(json!({"symbol": "AAPL"}))
        .unwrap_or_else(|e| panic!("transform_query: {e}"));
    let raw = cassette_bytes!([
        {"etfSymbol": "SPY", "weightPercentage": 7.10, "sharesNumber": 1234567.0, "marketValue": 250000000.0},
        {"etfSymbol": "", "weightPercentage": 1.0}
    ]);
    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|e| panic!("transform_data: {e}"));
    assert_eq!(rows.len(), 1, "blank fund dropped: {rows:#?}");
    assert_eq!(rows[0].equity_symbol, "AAPL");
    assert_eq!(rows[0].fund_symbol, "SPY");
    assert_eq!(rows[0].weight_pct, Some(7.10));
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

#[tokio::test]
async fn live_fmp_price_target_returns_data_when_env_var_set() {
    if std::env::var("TDW_FMP_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_FMP_LIVE != 1; skipping live FMP price-target integration test");
        return;
    }

    let fetcher = FmpHttpPriceTargetFetcher::default();
    let query = FmpHttpPriceTargetFetcher::transform_query(json!({"symbol": "AAPL"}))
        .unwrap_or_else(|e| panic!("transform_query must succeed: {e}"));

    let rows = live_fetch_nonempty!(fetcher, query);
    assert!(!rows.is_empty(), "live price-target must include an entry");
    assert_eq!(rows[0].kind, "price_target");
}

#[tokio::test]
async fn live_fmp_analyst_estimates_returns_data_when_env_var_set() {
    if std::env::var("TDW_FMP_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_FMP_LIVE != 1; skipping live FMP analyst-estimates integration test");
        return;
    }

    let fetcher = FmpHttpAnalystEstimatesFetcher::default();
    let query = FmpHttpAnalystEstimatesFetcher::transform_query(json!({"symbol": "AAPL"}))
        .unwrap_or_else(|e| panic!("transform_query must succeed: {e}"));

    let rows = live_fetch_nonempty!(fetcher, query);
    assert!(
        !rows.is_empty(),
        "live analyst-estimates must include a row"
    );
    assert!(
        rows.iter().all(|r| r.kind.starts_with("forward_")),
        "every analyst-estimate row must carry a forward_* kind"
    );
}

#[tokio::test]
async fn live_fmp_key_executives_returns_data_when_env_var_set() {
    if std::env::var("TDW_FMP_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_FMP_LIVE != 1; skipping live FMP key-executives integration test");
        return;
    }
    let fetcher = FmpHttpKeyExecutivesFetcher::default();
    let query = FmpHttpKeyExecutivesFetcher::transform_query(json!({"symbol": "AAPL"}))
        .unwrap_or_else(|e| panic!("transform_query must succeed: {e}"));
    let rows = live_fetch_nonempty!(fetcher, query);
    assert!(!rows.is_empty(), "live key-executives must include a row");
    assert_eq!(rows[0].symbol, "AAPL");
}

#[tokio::test]
async fn live_fmp_executive_compensation_returns_data_when_env_var_set() {
    if std::env::var("TDW_FMP_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_FMP_LIVE != 1; skipping live FMP executive-compensation integration test");
        return;
    }
    let fetcher = FmpHttpExecutiveCompensationFetcher::default();
    let query = FmpHttpExecutiveCompensationFetcher::transform_query(json!({"symbol": "AAPL"}))
        .unwrap_or_else(|e| panic!("transform_query must succeed: {e}"));
    let rows = live_fetch_nonempty!(fetcher, query);
    assert!(
        !rows.is_empty(),
        "live executive-compensation must include a row"
    );
}

#[tokio::test]
async fn live_fmp_revenue_segment_returns_data_when_env_var_set() {
    if std::env::var("TDW_FMP_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_FMP_LIVE != 1; skipping live FMP revenue-segment integration test");
        return;
    }
    let fetcher = FmpHttpRevenueSegmentFetcher::default();
    let query = FmpHttpRevenueSegmentFetcher::transform_query(
        json!({"symbol": "AAPL", "structure": "geography"}),
    )
    .unwrap_or_else(|e| panic!("transform_query must succeed: {e}"));
    let rows = live_fetch_nonempty!(fetcher, query);
    assert!(!rows.is_empty(), "live revenue-segment must include a row");
    assert!(rows.iter().all(|r| r.kind == "geography"));
}

#[tokio::test]
async fn live_fmp_transcript_returns_data_when_env_var_set() {
    if std::env::var("TDW_FMP_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_FMP_LIVE != 1; skipping live FMP transcript integration test");
        return;
    }
    let fetcher = FmpHttpTranscriptFetcher::default();
    let query = FmpHttpTranscriptFetcher::transform_query(
        json!({"symbol": "AAPL", "year": 2024, "quarter": 1}),
    )
    .unwrap_or_else(|e| panic!("transform_query must succeed: {e}"));
    let rows = live_fetch_nonempty!(fetcher, query);
    assert!(!rows.is_empty(), "live transcript must include a row");
    assert_eq!(rows[0].symbol, "AAPL");
}

#[tokio::test]
async fn live_fmp_esg_score_returns_data_when_env_var_set() {
    if std::env::var("TDW_FMP_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_FMP_LIVE != 1; skipping live FMP esg-score integration test");
        return;
    }
    let fetcher = FmpHttpEsgScoreFetcher::default();
    let query = FmpHttpEsgScoreFetcher::transform_query(json!({"symbol": "AAPL"}))
        .unwrap_or_else(|e| panic!("transform_query must succeed: {e}"));
    let rows = live_fetch_nonempty!(fetcher, query);
    assert!(!rows.is_empty(), "live esg-score must include a row");
    assert_eq!(rows[0].symbol, "AAPL");
}

#[tokio::test]
async fn live_fmp_employee_count_returns_data_when_env_var_set() {
    if std::env::var("TDW_FMP_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_FMP_LIVE != 1; skipping live FMP employee-count integration test");
        return;
    }
    let fetcher = FmpHttpEmployeeCountFetcher::default();
    let query = FmpHttpEmployeeCountFetcher::transform_query(json!({"symbol": "AAPL"}))
        .unwrap_or_else(|e| panic!("transform_query must succeed: {e}"));
    let rows = live_fetch_nonempty!(fetcher, query);
    assert!(!rows.is_empty(), "live employee-count must include a row");
    assert_eq!(rows[0].symbol, "AAPL");
}

#[tokio::test]
async fn live_fmp_filings_returns_data_when_env_var_set() {
    if std::env::var("TDW_FMP_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_FMP_LIVE != 1; skipping live FMP filings integration test");
        return;
    }
    let fetcher = FmpHttpFilingsFetcher::default();
    let query = FmpHttpFilingsFetcher::transform_query(json!({"symbol": "AAPL"}))
        .unwrap_or_else(|e| panic!("transform_query must succeed: {e}"));
    let rows = live_fetch_nonempty!(fetcher, query);
    assert!(!rows.is_empty(), "live filings must include a row");
    assert_eq!(rows[0].symbol, "AAPL");
}

#[tokio::test]
async fn live_fmp_search_returns_data_when_env_var_set() {
    if std::env::var("TDW_FMP_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_FMP_LIVE != 1; skipping live FMP search integration test");
        return;
    }
    let fetcher = FmpHttpSearchFetcher::default();
    let query = FmpHttpSearchFetcher::transform_query(json!({"query": "apple", "limit": 5}))
        .unwrap_or_else(|e| panic!("transform_query must succeed: {e}"));
    let rows = live_fetch_nonempty!(fetcher, query);
    assert!(!rows.is_empty(), "live search must include a row");
}

#[tokio::test]
async fn live_fmp_historical_market_cap_returns_data_when_env_var_set() {
    if std::env::var("TDW_FMP_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_FMP_LIVE != 1; skipping live FMP historical-market-cap integration test");
        return;
    }
    let fetcher = FmpHttpHistoricalMarketCapFetcher::default();
    let query =
        FmpHttpHistoricalMarketCapFetcher::transform_query(json!({"symbol": "AAPL", "limit": 5}))
            .unwrap_or_else(|e| panic!("transform_query must succeed: {e}"));
    let rows = live_fetch_nonempty!(fetcher, query);
    assert!(!rows.is_empty(), "live market-cap must include a row");
    assert_eq!(rows[0].symbol, "AAPL");
}

#[tokio::test]
async fn live_fmp_split_calendar_returns_data_when_env_var_set() {
    if std::env::var("TDW_FMP_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_FMP_LIVE != 1; skipping live FMP split-calendar integration test");
        return;
    }
    let fetcher = FmpHttpSplitCalendarFetcher::default();
    let query = FmpHttpSplitCalendarFetcher::transform_query(json!({}))
        .unwrap_or_else(|e| panic!("transform_query must succeed: {e}"));
    // The split calendar may legitimately be empty for the default window, so do
    // not require non-empty; just confirm a live round trip succeeds.
    let raw = fetcher
        .extract_data(&query, &Credentials::default())
        .await
        .unwrap_or_else(|e| panic!("live extract_data must succeed: {e}"));
    fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|e| panic!("live transform_data must succeed: {e}"));
}

#[tokio::test]
async fn live_fmp_latest_filings_returns_data_when_env_var_set() {
    if std::env::var("TDW_FMP_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_FMP_LIVE != 1; skipping live FMP latest-filings integration test");
        return;
    }
    let fetcher = FmpHttpLatestFilingsFetcher::default();
    let query = FmpHttpLatestFilingsFetcher::transform_query(json!({"limit": 25}))
        .unwrap_or_else(|e| panic!("transform_query must succeed: {e}"));
    let rows = live_fetch_nonempty!(fetcher, query);
    assert!(!rows.is_empty(), "live latest-filings must include a row");
}

#[tokio::test]
async fn live_fmp_insider_trading_returns_data_when_env_var_set() {
    if std::env::var("TDW_FMP_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_FMP_LIVE != 1; skipping live FMP insider-trading integration test");
        return;
    }
    let fetcher = FmpHttpInsiderTradingFetcher::default();
    let query = FmpHttpInsiderTradingFetcher::transform_query(json!({"symbol": "AAPL"}))
        .unwrap_or_else(|e| panic!("transform_query must succeed: {e}"));
    let raw = fetcher
        .extract_data(&query, &Credentials::default())
        .await
        .unwrap_or_else(|e| panic!("live extract_data must succeed: {e}"));
    fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|e| panic!("live transform_data must succeed: {e}"));
}

#[tokio::test]
async fn live_fmp_institutional_ownership_returns_data_when_env_var_set() {
    if std::env::var("TDW_FMP_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_FMP_LIVE != 1; skipping live FMP institutional-ownership integration test");
        return;
    }
    let fetcher = FmpHttpInstitutionalOwnershipFetcher::default();
    let query = FmpHttpInstitutionalOwnershipFetcher::transform_query(json!({"symbol": "AAPL"}))
        .unwrap_or_else(|e| panic!("transform_query must succeed: {e}"));
    let raw = fetcher
        .extract_data(&query, &Credentials::default())
        .await
        .unwrap_or_else(|e| panic!("live extract_data must succeed: {e}"));
    fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|e| panic!("live transform_data must succeed: {e}"));
}

#[tokio::test]
async fn live_fmp_government_trades_returns_data_when_env_var_set() {
    if std::env::var("TDW_FMP_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_FMP_LIVE != 1; skipping live FMP government-trades integration test");
        return;
    }
    let fetcher = FmpHttpGovernmentTradesFetcher::default();
    let query = FmpHttpGovernmentTradesFetcher::transform_query(json!({"symbol": "AAPL"}))
        .unwrap_or_else(|e| panic!("transform_query must succeed: {e}"));
    let raw = fetcher
        .extract_data(&query, &Credentials::default())
        .await
        .unwrap_or_else(|e| panic!("live extract_data must succeed: {e}"));
    fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|e| panic!("live transform_data must succeed: {e}"));
}
