//! Tests for the real ECB HTTP fetcher.
//!
//! Gated by `--features http` (no reqwest dep otherwise).
//!
//! Cassette tests always run under the feature and parse a recorded ECB
//! `jsondata` response shape. The live test is additionally gated by
//! `TDW_ECB_LIVE=1`; no API key is needed (ECB SDW is public).

#![cfg(feature = "http")]

use bytes::Bytes;
use serde_json::json;
use tdw_core::Fetcher;
use tdw_provider_ecb::{
    EcbDataQuery, EcbHttpDataFetcher, EcbHttpReferenceRatesFetcher, EcbReferenceRatesQuery,
};
use tdw_provider_testkit::{cassette_bytes, live_fetch_nonempty};

fn sample_query() -> EcbDataQuery {
    EcbHttpDataFetcher::transform_query(json!({
        "flow": "EXR",
        "key": "D.USD.EUR.SP00.A",
        "start_period": "2024-01-01",
        "end_period": "2024-01-31"
    }))
    .unwrap_or_else(|error| panic!("query should transform: {error}"))
}

fn cassette_bytes() -> Bytes {
    cassette_bytes!({
        "dataSets": [{
            "series": {
                "0:0:0:0:0": {
                    "observations": {
                        "0": [1.0934, 0, null],
                        "1": [1.0945, 0, null],
                        "2": [null, 0, null]
                    }
                }
            }
        }],
        "structure": {
            "dimensions": {
                "observation": [{
                    "id": "TIME_PERIOD",
                    "values": [
                        { "id": "2024-01-02" },
                        { "id": "2024-01-03" },
                        { "id": "2024-01-04" }
                    ]
                }]
            }
        }
    })
}

#[test]
fn cassette_replay_decodes_observations_and_skips_null_values() {
    let fetcher = EcbHttpDataFetcher::default();
    let query = sample_query();
    let rows = fetcher
        .transform_data(&query, cassette_bytes())
        .unwrap_or_else(|error| panic!("transform_data must succeed: {error}"));

    assert_eq!(rows.len(), 2, "rows={rows:#?}");
    assert_eq!(rows[0].flow, "EXR");
    assert_eq!(rows[0].key, "D.USD.EUR.SP00.A");
    assert_eq!(rows[0].date, "2024-01-02");
    assert_eq!(rows[0].value, 1.0934);
    assert_eq!(rows[1].date, "2024-01-03");
    assert_eq!(rows[1].value, 1.0945);
}

#[test]
fn cassette_replay_monthly_estr_shape() {
    let fetcher = EcbHttpDataFetcher::default();
    let query = EcbHttpDataFetcher::transform_query(json!({
        "flow": "ILM",
        "key": "M.U2.EUR.RT0.MM.ESTRVOLWGTD.HSTA",
        "start_period": "2024-01",
        "end_period": "2024-06"
    }))
    .unwrap_or_else(|error| panic!("query should transform: {error}"));

    let raw = cassette_bytes!({
        "dataSets": [{
            "series": {
                "0:0:0:0:0:0:0": {
                    "observations": {
                        "0": [3.9, 0, null],
                        "1": [3.9, 0, null]
                    }
                }
            }
        }],
        "structure": {
            "dimensions": {
                "observation": [{
                    "id": "TIME_PERIOD",
                    "values": [
                        { "id": "2024-01" },
                        { "id": "2024-02" }
                    ]
                }]
            }
        }
    });

    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|error| panic!("transform_data must succeed: {error}"));

    assert_eq!(rows.len(), 2, "rows={rows:#?}");
    assert_eq!(rows[0].date, "2024-01");
    assert_eq!(rows[0].value, 3.9);
    assert_eq!(rows[0].flow, "ILM");
}

#[test]
fn transform_query_trims_flow_and_rejects_empty() {
    let query = EcbHttpDataFetcher::transform_query(json!({
        "flow": "  EXR  ",
        "key": "D.USD.EUR.SP00.A",
        "start_period": "2024-01-01",
        "end_period": "2024-01-31"
    }))
    .unwrap_or_else(|error| panic!("query should transform: {error}"));

    assert_eq!(query.flow, "EXR");

    assert!(
        EcbHttpDataFetcher::transform_query(json!({
            "flow": "",
            "key": "D.USD.EUR.SP00.A",
            "start_period": "2024-01-01",
            "end_period": "2024-01-31"
        }))
        .is_err()
    );
}

#[test]
fn transform_query_rejects_missing_fields() {
    assert!(
        EcbHttpDataFetcher::transform_query(json!({ "flow": "EXR" })).is_err(),
        "missing key/start_period/end_period must error"
    );
}

// ---------------------------------------------------------------------------
// Catalog-facing reference-rates fetcher (currency/reference_rates)
// ---------------------------------------------------------------------------

/// A wildcard EXR `jsondata` response: two currencies (USD, GBP) across two days.
/// The series dimension carries CURRENCY so each row resolves to its pair.
fn reference_rates_cassette() -> Bytes {
    cassette_bytes!({
        "dataSets": [{
            "series": {
                "0:0:0:0:0": {
                    "observations": {
                        "0": [1.0934, 0, null],
                        "1": [1.0945, 0, null]
                    }
                },
                "0:1:0:0:0": {
                    "observations": {
                        "0": [0.8534, 0, null],
                        "1": [0.8540, 0, null]
                    }
                }
            }
        }],
        "structure": {
            "dimensions": {
                "series": [
                    { "id": "FREQ", "values": [{ "id": "D" }] },
                    { "id": "CURRENCY", "values": [{ "id": "USD" }, { "id": "GBP" }] },
                    { "id": "CURRENCY_DENOM", "values": [{ "id": "EUR" }] },
                    { "id": "EXR_TYPE", "values": [{ "id": "SP00" }] },
                    { "id": "EXR_SUFFIX", "values": [{ "id": "A" }] }
                ],
                "observation": [{
                    "id": "TIME_PERIOD",
                    "values": [
                        { "id": "2024-01-02" },
                        { "id": "2024-01-03" }
                    ]
                }]
            }
        }
    })
}

#[test]
fn cassette_reference_rates_resolves_per_currency_macro_series() {
    let fetcher = EcbHttpReferenceRatesFetcher::default();
    let query = EcbHttpReferenceRatesFetcher::transform_query(json!({}))
        .unwrap_or_else(|error| panic!("query should transform: {error}"));
    let rows = fetcher
        .transform_data(&query, reference_rates_cassette())
        .unwrap_or_else(|error| panic!("transform_data must succeed: {error}"));

    // Two currencies x two days, sorted by (currency, date): GBP then USD.
    assert_eq!(rows.len(), 4, "rows={rows:#?}");
    assert_eq!(rows[0].series_id, "GBP");
    assert_eq!(rows[0].date, "2024-01-02");
    assert_eq!(rows[0].value, Some(0.8534));
    assert_eq!(rows[0].frequency.as_deref(), Some("daily"));
    assert_eq!(rows[2].series_id, "USD");
    assert_eq!(rows[2].value, Some(1.0934));
}

#[test]
fn reference_rates_transform_query_accepts_optional_window() {
    let query = EcbHttpReferenceRatesFetcher::transform_query(json!({
        "start_date": "2024-01-01",
        "end_date": "2024-01-31"
    }))
    .unwrap_or_else(|error| panic!("query should transform: {error}"));
    assert!(query.params.start_date.is_some());
    assert!(query.params.end_date.is_some());

    // An inverted window is rejected by the shared StandardParams validation.
    assert!(
        EcbHttpReferenceRatesFetcher::transform_query(json!({
            "start_date": "2024-02-01",
            "end_date": "2024-01-01"
        }))
        .is_err()
    );
}

#[tokio::test]
async fn live_ecb_reference_rates_returns_rows_when_env_var_set() {
    if std::env::var("TDW_ECB_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_ECB_LIVE != 1; skipping live ECB reference-rates integration test");
        return;
    }
    let fetcher = EcbHttpReferenceRatesFetcher::default();
    let query = EcbReferenceRatesQuery::from_value(&json!({}))
        .unwrap_or_else(|error| panic!("query should build: {error}"));
    let rows = live_fetch_nonempty!(fetcher, query);
    assert!(
        !rows.is_empty(),
        "live response must include reference rates"
    );
}

#[tokio::test]
async fn live_ecb_returns_recent_observations_when_env_var_set() {
    if std::env::var("TDW_ECB_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_ECB_LIVE != 1; skipping live ECB integration test");
        return;
    }

    let fetcher = EcbHttpDataFetcher::default();
    let query = sample_query();
    let rows = live_fetch_nonempty!(fetcher, query);

    assert!(
        !rows.is_empty(),
        "live response must include at least one observation"
    );
    assert_eq!(rows[0].flow, "EXR");
}
