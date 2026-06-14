//! Tests for the real IMF SDMX-JSON `CompactData` HTTP fetcher.
//!
//! Gated by `--features http` (no reqwest dep otherwise).
//!
//! Cassette tests always run under the feature and parse recorded IMF
//! `CompactData` response shapes without network access. They cover the
//! single-`Series`/single-`Obs` shape, the array-of-`Series`/array-of-`Obs`
//! shape, the string→`f64` `@OBS_VALUE` coercion, and a malformed-input error
//! path. The live test is additionally gated by `TDW_IMF_LIVE=1` (keyless API).

#![cfg(feature = "http")]

use bytes::Bytes;
use serde_json::json;
use tdw_core::Fetcher;
use tdw_provider_imf::{ImfHttpMacroSeriesFetcher, ImfUtilsHttpDiscoveryFetcher};
use tdw_provider_testkit::{cassette_bytes, live_fetch_nonempty};

/// A single-`Series` envelope whose `Obs` is itself a single object (not an
/// array). IMF renders both the `Series` and the `Obs` node as a bare object
/// when there is exactly one element. `@OBS_VALUE` is a JSON string.
fn single_series_cassette() -> Bytes {
    cassette_bytes!({
        "CompactData": {
            "DataSet": {
                "Series": {
                    "@FREQ": "M",
                    "@REF_AREA": "US",
                    "@INDICATOR": "PMP_IX",
                    "Obs": {
                        "@TIME_PERIOD": "2024-01",
                        "@OBS_VALUE": "123.4"
                    }
                }
            }
        }
    })
}

/// An array-of-`Series` envelope, each series carrying an array of `Obs`. One
/// observation has an empty `@OBS_VALUE` (missing value) that must decode to a
/// `None` value while still emitting a row for its date.
fn array_series_cassette() -> Bytes {
    cassette_bytes!({
        "CompactData": {
            "DataSet": {
                "Series": [
                    {
                        "@FREQ": "A",
                        "@REF_AREA": "US",
                        "@INDICATOR": "TXG_FOB_USD",
                        "Obs": [
                            { "@TIME_PERIOD": "2022", "@OBS_VALUE": "100.5" },
                            { "@TIME_PERIOD": "2023", "@OBS_VALUE": "" }
                        ]
                    },
                    {
                        "@FREQ": "A",
                        "@REF_AREA": "GB",
                        "@INDICATOR": "TXG_FOB_USD",
                        "Obs": [
                            { "@TIME_PERIOD": "2022", "@OBS_VALUE": "200.25" }
                        ]
                    }
                ]
            }
        }
    })
}

fn ifs_query() -> serde_json::Value {
    json!({
        "command": "economy/imf/international_financial_statistics",
        "key": "M.US.PMP_IX"
    })
}

#[test]
fn single_series_single_obs_decodes_and_coerces_string_value() {
    let fetcher = ImfHttpMacroSeriesFetcher::default();
    let query = ImfHttpMacroSeriesFetcher::transform_query(ifs_query())
        .unwrap_or_else(|error| panic!("query should transform: {error}"));

    let rows = fetcher
        .transform_data(&query, single_series_cassette())
        .unwrap_or_else(|error| panic!("transform_data must succeed: {error}"));

    assert_eq!(rows.len(), 1, "one obs -> one row; rows={rows:#?}");
    assert_eq!(rows[0].series_id, "IFS.M.US.PMP_IX");
    assert_eq!(rows[0].date, "2024-01");
    // The JSON string "123.4" is parsed to an f64.
    assert_eq!(rows[0].value, Some(123.4));
    assert_eq!(rows[0].country.as_deref(), Some("US"));
    assert_eq!(rows[0].frequency.as_deref(), Some("M"));
}

#[test]
fn array_of_series_with_array_obs_decodes_all_rows() {
    let fetcher = ImfHttpMacroSeriesFetcher::default();
    let query = ImfHttpMacroSeriesFetcher::transform_query(json!({
        "command": "economy/imf/direction_of_trade",
        "key": "A.US+GB.TXG_FOB_USD"
    }))
    .unwrap_or_else(|error| panic!("query should transform: {error}"));

    let rows = fetcher
        .transform_data(&query, array_series_cassette())
        .unwrap_or_else(|error| panic!("transform_data must succeed: {error}"));

    // Two series: US has two obs (one missing value), GB has one.
    assert_eq!(rows.len(), 3, "rows={rows:#?}");
    assert_eq!(rows[0].series_id, "DOT.A.US.TXG_FOB_USD");
    assert_eq!(rows[0].date, "2022");
    assert_eq!(rows[0].value, Some(100.5));
    // Empty @OBS_VALUE -> a row with a None value, not a dropped row.
    assert_eq!(rows[1].date, "2023");
    assert_eq!(rows[1].value, None);
    assert_eq!(rows[2].series_id, "DOT.A.GB.TXG_FOB_USD");
    assert_eq!(rows[2].value, Some(200.25));
    assert_eq!(rows[2].country.as_deref(), Some("GB"));
}

#[test]
fn missing_compact_data_node_yields_empty_not_error() {
    let fetcher = ImfHttpMacroSeriesFetcher::default();
    let query = ImfHttpMacroSeriesFetcher::transform_query(ifs_query())
        .unwrap_or_else(|error| panic!("query should transform: {error}"));
    // An out-of-range window returns an envelope with no CompactData/DataSet.
    let envelope = cassette_bytes!({ "@xmlns:xsi": "http://www.w3.org/2001/XMLSchema-instance" });
    let rows = fetcher
        .transform_data(&query, envelope)
        .unwrap_or_else(|error| panic!("empty envelope must decode to no rows: {error}"));
    assert!(rows.is_empty(), "rows={rows:#?}");
}

#[test]
fn transform_data_rejects_malformed_json() {
    let fetcher = ImfHttpMacroSeriesFetcher::default();
    let query = ImfHttpMacroSeriesFetcher::transform_query(ifs_query())
        .unwrap_or_else(|error| panic!("query should transform: {error}"));
    let garbage = Bytes::from_static(b"{not valid json");
    let err = fetcher
        .transform_data(&query, garbage)
        .expect_err("malformed JSON must be propagated");
    assert!(err.to_string().contains("parse_json"), "got: {err}");
}

#[test]
fn transform_data_rejects_non_object_root() {
    let fetcher = ImfHttpMacroSeriesFetcher::default();
    let query = ImfHttpMacroSeriesFetcher::transform_query(ifs_query())
        .unwrap_or_else(|error| panic!("query should transform: {error}"));
    let array_root = cassette_bytes!([1, 2, 3]);
    let err = fetcher
        .transform_data(&query, array_root)
        .expect_err("non-object root must be propagated");
    assert!(
        err.to_string().contains("expected a JSON object"),
        "got: {err}"
    );
}

#[test]
fn transform_query_rejects_unknown_command_and_missing_key() {
    assert!(
        ImfHttpMacroSeriesFetcher::transform_query(json!({ "command": "bogus", "key": "M.US.X" }))
            .is_err()
    );
    assert!(
        ImfHttpMacroSeriesFetcher::transform_query(json!({
            "command": "economy/imf/international_financial_statistics"
        }))
        .is_err()
    );
}

#[test]
fn discovery_decodes_dataflow_list() {
    let fetcher = ImfUtilsHttpDiscoveryFetcher::default();
    let query = ImfUtilsHttpDiscoveryFetcher::transform_query(json!({
        "command": "imf_utils/list_dataflows"
    }))
    .unwrap_or_else(|error| panic!("query should transform: {error}"));

    let raw = cassette_bytes!({
        "Structure": { "Dataflows": { "Dataflow": [
            {
                "@id": "IFS",
                "Name": { "#text": "International Financial Statistics", "@xml:lang": "en" },
                "KeyFamilyRef": { "KeyFamilyID": "ECOFIN_DSD" }
            }
        ] } }
    });
    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|error| panic!("transform_data must succeed: {error}"));
    assert_eq!(rows.len(), 1, "rows={rows:#?}");
    assert_eq!(rows[0].kind, "dataflow");
    assert_eq!(rows[0].id, "IFS");
    assert_eq!(rows[0].structure.as_deref(), Some("ECOFIN_DSD"));
}

#[test]
fn discovery_decodes_dataflow_dimensions() {
    let fetcher = ImfUtilsHttpDiscoveryFetcher::default();
    let query = ImfUtilsHttpDiscoveryFetcher::transform_query(json!({
        "command": "imf_utils/get_dataflow_dimensions",
        "dataflow": "IFS"
    }))
    .unwrap_or_else(|error| panic!("query should transform: {error}"));

    let raw = cassette_bytes!({
        "Structure": { "KeyFamilies": { "KeyFamily": {
            "@id": "ECOFIN_DSD",
            "Components": { "Dimension": [
                { "@conceptRef": "FREQ", "Name": "Frequency" },
                { "@conceptRef": "REF_AREA", "@codelist": "CL_AREA" }
            ] }
        } } }
    });
    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|error| panic!("transform_data must succeed: {error}"));
    assert_eq!(rows.len(), 2, "rows={rows:#?}");
    assert_eq!(rows[0].kind, "dimension");
    assert_eq!(rows[0].id, "FREQ");
    assert_eq!(rows[0].position, Some(1));
    assert_eq!(rows[0].dataflow.as_deref(), Some("IFS"));
}

#[test]
fn discovery_rejects_missing_dataflow_and_unknown_command() {
    // A DataStructure-backed helper requires a dataflow id.
    assert!(
        ImfUtilsHttpDiscoveryFetcher::transform_query(json!({
            "command": "imf_utils/get_dataflow_dimensions"
        }))
        .is_err()
    );
    assert!(
        ImfUtilsHttpDiscoveryFetcher::transform_query(json!({ "command": "imf_utils/bogus" }))
            .is_err()
    );
    // A path-injection dataflow id is rejected.
    assert!(
        ImfUtilsHttpDiscoveryFetcher::transform_query(json!({
            "command": "imf_utils/get_dataflow_dimensions",
            "dataflow": "../../etc"
        }))
        .is_err()
    );
}

#[tokio::test]
async fn live_imf_utils_dataflows_return_rows_when_env_set() {
    if std::env::var("TDW_IMF_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_IMF_LIVE != 1; skipping live IMF dataflow discovery test");
        return;
    }
    let fetcher = ImfUtilsHttpDiscoveryFetcher::default();
    let query = ImfUtilsHttpDiscoveryFetcher::transform_query(json!({
        "command": "imf_utils/list_dataflows"
    }))
    .unwrap_or_else(|error| panic!("query should transform: {error}"));
    let rows = live_fetch_nonempty!(fetcher, query);
    assert!(!rows.is_empty(), "live IMF dataflow list must include rows");
}

#[tokio::test]
async fn live_imf_compact_data_returns_rows_when_env_set() {
    if std::env::var("TDW_IMF_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_IMF_LIVE != 1; skipping live IMF CompactData test");
        return;
    }
    let fetcher = ImfHttpMacroSeriesFetcher::default();
    let query = ImfHttpMacroSeriesFetcher::transform_query(json!({
        "command": "economy/imf/international_financial_statistics",
        "key": "M.US.PMP_IX",
        "start_date": "2020-01-01",
        "end_date": "2021-12-31"
    }))
    .unwrap_or_else(|error| panic!("query should transform: {error}"));
    let rows = live_fetch_nonempty!(fetcher, query);
    assert!(!rows.is_empty(), "live IMF CompactData must include rows");
}
