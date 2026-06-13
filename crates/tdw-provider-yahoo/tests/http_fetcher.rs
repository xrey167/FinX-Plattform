//! Tests for the real Yahoo Finance HTTP fetcher.
//!
//! Gated by `--features http` (no reqwest dep otherwise).
//!
//! Two layers:
//!
//! 1. **Cassette test** (`cassette_replay_*`) — always runs under the
//!    feature; parses a recorded Yahoo v8 chart response shape and
//!    asserts row decoding. Verifies the deserialiser without
//!    actually hitting Yahoo.
//!
//! 2. **Live test** (`live_yahoo_*`) — additionally gated by
//!    `TDW_YAHOO_LIVE=1`; performs a real HTTP request against Yahoo
//!    Finance. Skipped by default to keep CI quiet and avoid Yahoo
//!    rate-limit / region restrictions.

#![cfg(feature = "http")]

use bytes::Bytes;
use serde_json::json;
use tdw_core::{Credentials, Fetcher};
use tdw_provider_fileset::FilesetEquityHistoricalFetcher;
use tdw_provider_testkit::{cassette_bytes, live_fetch_nonempty};
use tdw_provider_yahoo::{
    YahooHttpConsensusFetcher, YahooHttpDividendsFetcher, YahooHttpEquityHistoricalFetcher,
    YahooHttpEtfInfoFetcher, YahooHttpFuturesCurveFetcher, YahooHttpFuturesHistoricalFetcher,
    YahooHttpOptionsChainFetcher, YahooHttpPredefinedScreenerFetcher,
    YahooHttpPricePerformanceFetcher, YahooHttpProfileFetcher, YahooHttpQuoteFetcher,
    YahooHttpShareStatisticsFetcher, YahooScreenerQuery, YahooSymbolQuery,
};

fn symbol_query(symbol: &str) -> YahooSymbolQuery {
    YahooSymbolQuery::from_value(&json!({ "symbol": symbol }))
        .unwrap_or_else(|error| panic!("symbol query should transform: {error}"))
}

fn sample_query() -> tdw_provider_fileset::EquityHistoricalQuery {
    FilesetEquityHistoricalFetcher::transform_query(json!({ "symbol": "AAPL" }))
        .unwrap_or_else(|error| panic!("query should transform: {error}"))
}

/// A small slice of the real Yahoo v8 chart envelope shape, with three
/// daily bars including one bar with all-null fields (Yahoo emits this
/// when the requested range overlaps an open session).
fn cassette_bytes() -> Bytes {
    cassette_bytes!({
        "chart": {
            "result": [{
                "meta": { "symbol": "AAPL", "currency": "USD" },
                "timestamp": [1_700_006_400, 1_700_092_800, 1_700_179_200],
                "indicators": {
                    "quote": [{
                        "open":   [180.0, 182.5, null],
                        "high":   [183.2, 184.0, null],
                        "low":    [179.5, 181.8, null],
                        "close":  [182.4, 183.6, null],
                        "volume": [50_123_400i64, 45_678_900i64, null]
                    }]
                }
            }],
            "error": null
        }
    })
}

#[test]
fn cassette_replay_decodes_yahoo_chart_envelope_and_skips_null_bars() {
    let fetcher = YahooHttpEquityHistoricalFetcher::default();
    let query = sample_query();
    let rows = fetcher
        .transform_data(&query, cassette_bytes())
        .unwrap_or_else(|error| panic!("transform_data must succeed: {error}"));

    // The third (all-null) bar must be dropped.
    assert_eq!(rows.len(), 2, "rows={rows:#?}");
    assert_eq!(rows[0].symbol, "AAPL");
    assert_eq!(rows[0].date, "2023-11-15");
    assert_eq!(rows[0].open, 180.0);
    assert_eq!(rows[0].close, 182.4);
    assert_eq!(rows[0].volume, 50_123_400);
    assert_eq!(rows[1].date, "2023-11-16");
    assert_eq!(rows[1].close, 183.6);
}

#[test]
fn cassette_replay_surfaces_yahoo_error_envelope() {
    let fetcher = YahooHttpEquityHistoricalFetcher::default();
    let query = sample_query();
    let envelope = cassette_bytes!({ "chart": { "result": [], "error": { "code": "Not Found" } } });
    let err = fetcher
        .transform_data(&query, envelope)
        .expect_err("error envelope must be propagated");
    assert!(err.to_string().contains("yahoo chart error"));
}

#[tokio::test]
async fn live_yahoo_returns_recent_bars_when_env_var_set() {
    if std::env::var("TDW_YAHOO_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_YAHOO_LIVE != 1; skipping live yahoo integration test");
        return;
    }

    let fetcher = YahooHttpEquityHistoricalFetcher::default();
    let query = sample_query();
    let rows = live_fetch_nonempty!(fetcher, query);

    assert!(
        !rows.is_empty(),
        "live response must include at least one bar"
    );
    assert_eq!(rows[0].symbol, "AAPL");
}

// ===========================================================================
// L2.4 expansion cassette tests — one per new fetcher. Each replays a small
// slice of the documented Yahoo JSON envelope shape and asserts the decode +
// L1.4 normalization, without hitting Yahoo.
// ===========================================================================

#[test]
fn registry_entries_advertise_distinct_endpoints() {
    assert_eq!(
        YahooHttpProfileFetcher::registry_entry().endpoint,
        "equity_profile"
    );
    assert_eq!(
        YahooHttpQuoteFetcher::registry_entry().endpoint,
        "equity_quote"
    );
    assert_eq!(
        YahooHttpPricePerformanceFetcher::registry_entry().endpoint,
        "price_performance"
    );
    assert_eq!(
        YahooHttpDividendsFetcher::registry_entry().endpoint,
        "dividends"
    );
    assert_eq!(
        YahooHttpShareStatisticsFetcher::registry_entry().endpoint,
        "share_statistics"
    );
    assert_eq!(
        YahooHttpConsensusFetcher::registry_entry().endpoint,
        "analyst_consensus"
    );
    assert_eq!(
        YahooHttpFuturesHistoricalFetcher::registry_entry().endpoint,
        "futures_historical"
    );
    assert_eq!(
        YahooHttpFuturesCurveFetcher::registry_entry().endpoint,
        "futures_curve"
    );
    assert_eq!(
        YahooHttpOptionsChainFetcher::registry_entry().endpoint,
        "options_chains"
    );
    for entry in [
        YahooHttpProfileFetcher::registry_entry(),
        YahooHttpQuoteFetcher::registry_entry(),
        YahooHttpOptionsChainFetcher::registry_entry(),
    ] {
        assert_eq!(entry.provider, "yahoo");
    }
}

#[test]
fn profile_cassette_decodes_company_profile() {
    let fetcher = YahooHttpProfileFetcher::default();
    let query = symbol_query("AAPL");
    let raw = cassette_bytes!({
        "quoteSummary": {
            "result": [{
                "assetProfile": { "sector": "Technology", "website": "https://apple.com" },
                "price": {
                    "longName": "Apple Inc.",
                    "currency": "USD",
                    "exchangeName": "NasdaqGS",
                    "marketCap": { "raw": 3_500_000_000_000i64 }
                }
            }],
            "error": null
        }
    });
    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|error| panic!("profile must decode: {error}"));
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].ticker, "AAPL");
    assert_eq!(rows[0].name, "Apple Inc.");
    assert_eq!(rows[0].currency, "USD");
    assert_eq!(rows[0].exchange, "NasdaqGS");
    assert!((rows[0].market_cap_millions - 3_500_000.0).abs() < 1.0);
}

#[test]
fn quote_cassette_decodes_quote_snapshot() {
    let fetcher = YahooHttpQuoteFetcher::default();
    let query = symbol_query("AAPL");
    let raw = cassette_bytes!({
        "quoteResponse": {
            "result": [{
                "symbol": "AAPL",
                "regularMarketPrice": 189.3,
                "regularMarketChange": 1.2,
                "regularMarketChangePercent": 0.638,
                "regularMarketPreviousClose": 188.1,
                "regularMarketTime": 1_717_200_000i64
            }],
            "error": null
        }
    });
    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|error| panic!("quote must decode: {error}"));
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].symbol, "AAPL");
    assert!((rows[0].current_price - 189.3).abs() < 1e-9);
    assert_eq!(rows[0].ts_ms, 1_717_200_000_000);
}

#[test]
fn quote_cassette_surfaces_error_envelope() {
    let fetcher = YahooHttpQuoteFetcher::default();
    let query = symbol_query("AAPL");
    let raw =
        cassette_bytes!({ "quoteResponse": { "result": [], "error": { "code": "Not Found" } } });
    let err = fetcher
        .transform_data(&query, raw)
        .expect_err("error envelope must propagate");
    assert!(err.to_string().contains("yahoo quote error"));
}

#[test]
fn performance_cassette_derives_period_returns() {
    let fetcher = YahooHttpPricePerformanceFetcher::default();
    let query = symbol_query("AAPL");
    let raw = cassette_bytes!({
        "quoteSummary": {
            "result": [{
                "price": {
                    "regularMarketPrice": { "raw": 110.0 },
                    "regularMarketPreviousClose": { "raw": 100.0 }
                },
                "summaryDetail": {
                    "fiftyDayAverage": { "raw": 100.0 },
                    "twoHundredDayAverage": { "raw": 88.0 },
                    "fiftyTwoWeekLow": { "raw": 55.0 },
                    "fiftyTwoWeekHigh": { "raw": 120.0 }
                },
                "defaultKeyStatistics": { "52WeekChange": { "raw": 0.42 } }
            }],
            "error": null
        }
    });
    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|error| panic!("performance must decode: {error}"));
    assert_eq!(rows.len(), 1);
    // 1-day return = (110 - 100) / 100 = +0.10.
    assert!((rows[0].one_day.unwrap_or_default() - 0.10).abs() < 1e-9);
    // 1-year return comes from the dedicated 52WeekChange statistic.
    assert!((rows[0].one_year.unwrap_or_default() - 0.42).abs() < 1e-9);
}

#[test]
fn dividends_cassette_decodes_sorted_corporate_actions() {
    let fetcher = YahooHttpDividendsFetcher::default();
    let query = symbol_query("AAPL");
    let raw = cassette_bytes!({
        "chart": {
            "result": [{
                "meta": { "currency": "USD" },
                "events": {
                    "dividends": {
                        "1700006400": { "amount": 0.24, "date": 1_700_006_400i64 },
                        "1692230400": { "amount": 0.23, "date": 1_692_230_400i64 }
                    }
                }
            }],
            "error": null
        }
    });
    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|error| panic!("dividends must decode: {error}"));
    assert_eq!(rows.len(), 2);
    // Sorted ascending by ex_date: 2023-08-17 then 2023-11-15.
    assert_eq!(rows[0].ex_date, "2023-08-17");
    assert_eq!(rows[0].action_type, "dividend");
    assert!((rows[0].cash_amount - 0.23).abs() < 1e-9);
    assert_eq!(rows[1].ex_date, "2023-11-15");
    assert_eq!(rows[0].currency, "USD");
}

#[test]
fn share_statistics_cassette_decodes_ownership_record() {
    let fetcher = YahooHttpShareStatisticsFetcher::default();
    let query = symbol_query("AAPL");
    let raw = cassette_bytes!({
        "quoteSummary": {
            "result": [{
                "defaultKeyStatistics": {
                    "sharesOutstanding": { "raw": 15_000_000_000i64 },
                    "floatShares": { "raw": 14_900_000_000i64 },
                    "heldPercentInsiders": { "raw": 0.0007 },
                    "heldPercentInstitutions": { "raw": 0.61 }
                }
            }],
            "error": null
        }
    });
    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|error| panic!("share_statistics must decode: {error}"));
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].kind, "share_statistics");
    assert!((rows[0].percentage.unwrap_or_default() - 0.61).abs() < 1e-9);
    assert!((rows[0].shares.unwrap_or_default() - 14_900_000_000.0).abs() < 1.0);
}

#[test]
fn consensus_cassette_decodes_estimate() {
    let fetcher = YahooHttpConsensusFetcher::default();
    let query = symbol_query("AAPL");
    let raw = cassette_bytes!({
        "quoteSummary": {
            "result": [{
                "financialData": {
                    "targetMeanPrice": { "raw": 250.0 },
                    "targetLowPrice": { "raw": 200.0 },
                    "targetHighPrice": { "raw": 300.0 },
                    "numberOfAnalystOpinions": { "raw": 34.0 },
                    "recommendationKey": "buy",
                    "financialCurrency": "USD"
                }
            }],
            "error": null
        }
    });
    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|error| panic!("consensus must decode: {error}"));
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].kind, "consensus");
    assert_eq!(rows[0].recommendation.as_deref(), Some("buy"));
    assert_eq!(rows[0].number_of_analysts, Some(34));
    assert!((rows[0].value.unwrap_or_default() - 250.0).abs() < 1e-9);
}

#[test]
fn futures_historical_cassette_decodes_bars() {
    let fetcher = YahooHttpFuturesHistoricalFetcher::default();
    let query = symbol_query("ES=F");
    let raw = cassette_bytes!({
        "chart": {
            "result": [{
                "timestamp": [1_700_006_400, 1_700_092_800],
                "indicators": {
                    "quote": [{
                        "open":   [4500.0, 4510.0],
                        "high":   [4520.0, 4530.0],
                        "low":    [4490.0, 4500.0],
                        "close":  [4515.0, 4525.0],
                        "volume": [1_200_000i64, 1_300_000i64]
                    }]
                }
            }],
            "error": null
        }
    });
    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|error| panic!("futures bars must decode: {error}"));
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].symbol, "ES=F");
    assert_eq!(rows[0].date, "2023-11-15");
}

#[test]
fn futures_curve_cassette_decodes_points() {
    let fetcher = YahooHttpFuturesCurveFetcher::default();
    let query = symbol_query("ES=F");
    let raw = cassette_bytes!({
        "quoteResponse": {
            "result": [{
                "symbol": "ESM26.CME",
                "underlyingSymbol": "ES=F",
                "regularMarketPrice": 4515.0,
                "expireDate": 1_781_222_400i64
            }],
            "error": null
        }
    });
    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|error| panic!("curve must decode: {error}"));
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].underlying, "ES=F");
    assert_eq!(rows[0].contract_symbol, "ESM26.CME");
    assert!((rows[0].price.unwrap_or_default() - 4515.0).abs() < 1e-9);
    assert!(rows[0].expiration.is_some());
}

#[test]
fn options_cassette_decodes_calls_and_puts() {
    let fetcher = YahooHttpOptionsChainFetcher::default();
    let query = symbol_query("AAPL");
    let raw = cassette_bytes!({
        "optionChain": {
            "result": [{
                "underlyingSymbol": "AAPL",
                "options": [{
                    "expirationDate": 1_700_006_400i64,
                    "calls": [{
                        "contractSymbol": "AAPL231115C00180000",
                        "strike": 180.0,
                        "bid": 12.3,
                        "ask": 12.5,
                        "lastPrice": 12.4,
                        "volume": 1500,
                        "openInterest": 20000,
                        "impliedVolatility": 0.28
                    }],
                    "puts": [{
                        "contractSymbol": "AAPL231115P00180000",
                        "strike": 180.0,
                        "bid": 5.1,
                        "ask": 5.3,
                        "lastPrice": 5.2
                    }]
                }]
            }],
            "error": null
        }
    });
    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|error| panic!("options must decode: {error}"));
    assert_eq!(rows.len(), 2);
    let call = rows
        .iter()
        .find(|c| c.option_type == "call")
        .unwrap_or_else(|| panic!("call row present"));
    assert_eq!(call.underlying_symbol, "AAPL");
    assert_eq!(call.expiration, "2023-11-15");
    assert!((call.strike - 180.0).abs() < 1e-9);
    assert_eq!(call.open_interest, Some(20000));
    assert!(rows.iter().any(|c| c.option_type == "put"));
}

#[tokio::test]
async fn live_yahoo_profile_quote_options_when_env_var_set() {
    if std::env::var("TDW_YAHOO_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_YAHOO_LIVE != 1; skipping live yahoo expansion test");
        return;
    }
    let creds = Credentials::default();

    let profile = YahooHttpProfileFetcher::default();
    let pq = symbol_query("AAPL");
    let raw = profile
        .extract_data(&pq, &creds)
        .await
        .unwrap_or_else(|error| panic!("live profile extract: {error}"));
    let rows = profile
        .transform_data(&pq, raw)
        .unwrap_or_else(|error| panic!("live profile decode: {error}"));
    assert_eq!(rows[0].ticker, "AAPL");

    let quote = YahooHttpQuoteFetcher::default();
    let qq = symbol_query("AAPL");
    let rows = live_fetch_nonempty!(quote, qq);
    assert!(!rows.is_empty(), "live quote must return a snapshot");

    let options = YahooHttpOptionsChainFetcher::default();
    let oq = symbol_query("AAPL");
    let rows = live_fetch_nonempty!(options, oq);
    assert!(!rows.is_empty(), "live options chain must return contracts");
}

// ---------------------------------------------------------------------------
// yfinance discovery screener + ETF info cassette tests (openbb-parity P4W3)
// ---------------------------------------------------------------------------

#[test]
fn predefined_screener_cassette_decodes_screener_rows() {
    let fetcher = YahooHttpPredefinedScreenerFetcher::default();
    let query = YahooScreenerQuery::from_value(&json!({ "scr_ids": "growth_technology_stocks" }))
        .unwrap_or_else(|error| panic!("screener query should transform: {error}"));
    let raw = cassette_bytes!({
        "finance": {
            "result": [{
                "quotes": [
                    {
                        "symbol": "NVDA", "longName": "NVIDIA Corporation",
                        "regularMarketPrice": { "raw": 120.5 },
                        "regularMarketVolume": { "raw": 300_000_000.0 },
                        "marketCap": { "raw": 2_900_000_000_000.0 },
                        "sector": "Technology", "industry": "Semiconductors",
                        "fullExchangeName": "NasdaqGS", "quoteType": "EQUITY",
                        "beta": { "raw": 1.7 }
                    },
                    { "symbol": "", "longName": "junk" }
                ]
            }],
            "error": null
        }
    });
    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|error| panic!("rows should decode: {error}"));
    assert_eq!(rows.len(), 1, "blank symbol dropped: {rows:#?}");
    assert_eq!(rows[0].symbol, "NVDA");
    assert_eq!(rows[0].company_name.as_deref(), Some("NVIDIA Corporation"));
    assert_eq!(rows[0].sector.as_deref(), Some("Technology"));
    assert_eq!(rows[0].is_etf, Some(false));
}

#[test]
fn predefined_screener_cassette_surfaces_error_envelope() {
    let fetcher = YahooHttpPredefinedScreenerFetcher::default();
    let query = YahooScreenerQuery::from_value(&json!({ "scr_ids": "aggressive_small_caps" }))
        .unwrap_or_else(|error| panic!("screener query should transform: {error}"));
    let raw = cassette_bytes!({ "finance": { "result": [], "error": { "code": "Bad Request" } } });
    let result = fetcher.transform_data(&query, raw);
    assert!(result.is_err(), "error envelope must surface as Err");
}

#[test]
fn etf_info_cassette_decodes_fund_profile() {
    let fetcher = YahooHttpEtfInfoFetcher::default();
    let query = symbol_query("SPY");
    let raw = cassette_bytes!({
        "quoteSummary": {
            "result": [{
                "price": {
                    "longName": "SPDR S&P 500 ETF Trust",
                    "currency": "USD", "exchangeName": "PCX"
                },
                "fundProfile": {
                    "family": "SPDR State Street Global Advisors",
                    "legalType": "Exchange Traded Fund",
                    "feesExpensesInvestment": { "annualReportExpenseRatio": { "raw": 0.000945 } }
                }
            }],
            "error": null
        }
    });
    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|error| panic!("rows should decode: {error}"));
    assert_eq!(rows.len(), 1, "rows={rows:#?}");
    assert_eq!(rows[0].symbol, "SPY");
    assert_eq!(rows[0].name, "SPDR S&P 500 ETF Trust");
    assert_eq!(
        rows[0].issuer.as_deref(),
        Some("SPDR State Street Global Advisors")
    );
    assert!((rows[0].expense_ratio.unwrap_or_default() - 0.000945).abs() < 1e-9);
}

#[test]
fn screener_query_rejects_garbage_scr_ids() {
    assert!(YahooScreenerQuery::from_value(&json!({ "scr_ids": "bad id!" })).is_err());
    assert!(YahooScreenerQuery::from_value(&json!({ "scr_ids": "" })).is_err());
    assert!(YahooScreenerQuery::from_value(&json!({})).is_err());
    let ok = YahooScreenerQuery::from_value(
        &json!({ "scr_ids": "growth_technology_stocks", "count": 5 }),
    )
    .unwrap_or_else(|error| panic!("valid screener query: {error}"));
    assert_eq!(ok.scr_ids, "growth_technology_stocks");
    assert_eq!(ok.count, 5);
}
