//! Tests for the real Ken French Data Library HTTP fetcher.
//!
//! Gated by `--features http` (no reqwest / zip dep otherwise).
//!
//! The cassette test builds an in-memory ZIP archive in the exact Data Library
//! shape (one `.CSV` member, a descriptive header preamble, the factor column
//! header, percent-valued date rows) and drives the fetcher's `transform_data`
//! unzip + parse path without network access. The live test is additionally
//! gated by `TDW_FAMAFRENCH_LIVE=1` (keyless ftp tree).

#![cfg(feature = "http")]

use std::io::Write;

use bytes::Bytes;
use serde_json::json;
use tdw_core::Fetcher;
use tdw_provider_famafrench::FamaFrenchHttpFetcher;
use tdw_provider_testkit::live_fetch_nonempty;
use zip::write::SimpleFileOptions;

/// Build an in-memory ZIP archive carrying one `.CSV` member with the given
/// name and contents, mirroring the Data Library's ZIP-of-CSV shape.
fn zip_with_csv(member: &str, csv: &str) -> Bytes {
    let mut cursor = std::io::Cursor::new(Vec::new());
    {
        let mut writer = zip::ZipWriter::new(&mut cursor);
        writer
            .start_file(member, SimpleFileOptions::default())
            .expect("start zip member");
        writer.write_all(csv.as_bytes()).expect("write csv member");
        writer.finish().expect("finish zip");
    }
    Bytes::from(cursor.into_inner())
}

const THREE_FACTOR_DAILY_CSV: &str = "\
This file was created by CMPT_ME_BEME_RETS using the 202404 CRSP database.

,Mkt-RF,SMB,HML,RF
20240603,0.55,-0.21,0.13,0.022
20240604,-0.34,0.10,-0.05,0.022

Annual Factors: January-December
,Mkt-RF,SMB,HML,RF
2023,21.00,-3.00,-9.00,5.00
";

#[test]
fn cassette_unzips_and_parses_three_factor_daily() {
    let fetcher = FamaFrenchHttpFetcher::default();
    let query = FamaFrenchHttpFetcher::transform_query(json!({
        "factor_set": "3factor",
        "frequency": "daily"
    }))
    .unwrap_or_else(|error| panic!("query should transform: {error}"));

    let raw = zip_with_csv(
        "F-F_Research_Data_Factors_daily.CSV",
        THREE_FACTOR_DAILY_CSV,
    );
    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|error| panic!("transform_data must succeed: {error}"));

    // Only the daily rows; the appended annual table is ignored.
    assert_eq!(rows.len(), 2, "rows={rows:#?}");
    assert_eq!(rows[0].date, "2024-06-03");
    // Percent -> fraction.
    assert!((rows[0].mkt_rf.expect("mkt_rf") - 0.0055).abs() < 1e-12);
    assert!((rows[0].rf.expect("rf") - 0.00022).abs() < 1e-12);
    assert_eq!(rows[0].rmw, None);
    assert_eq!(rows[1].date, "2024-06-04");
}

#[test]
fn cassette_surfaces_non_zip_bytes_as_error() {
    let fetcher = FamaFrenchHttpFetcher::default();
    let query = FamaFrenchHttpFetcher::transform_query(json!({
        "factor_set": "3factor",
        "frequency": "daily"
    }))
    .unwrap_or_else(|error| panic!("query should transform: {error}"));

    let err = fetcher
        .transform_data(&query, Bytes::from_static(b"not a zip archive"))
        .expect_err("non-zip bytes must be rejected");
    assert!(err.to_string().contains("open zip"), "err={err}");
}

#[test]
fn transform_query_rejects_unknown_tokens() {
    assert!(FamaFrenchHttpFetcher::transform_query(json!({ "factor_set": "4factor" })).is_err());
    assert!(FamaFrenchHttpFetcher::transform_query(json!({ "frequency": "weekly" })).is_err());
}

#[tokio::test]
async fn live_famafrench_returns_rows_when_env_set() {
    if std::env::var("TDW_FAMAFRENCH_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_FAMAFRENCH_LIVE != 1; skipping live Ken French factors test");
        return;
    }
    let fetcher = FamaFrenchHttpFetcher::default();
    let query = FamaFrenchHttpFetcher::transform_query(json!({
        "factor_set": "3factor",
        "frequency": "daily"
    }))
    .unwrap_or_else(|error| panic!("query should transform: {error}"));
    let rows = live_fetch_nonempty!(fetcher, query);
    assert!(!rows.is_empty(), "live famafrench must include rows");
}
