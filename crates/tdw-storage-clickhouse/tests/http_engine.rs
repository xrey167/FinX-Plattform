//! Integration test for the real ClickHouse backend.
//!
//! Gated two ways:
//!   - Compiled only with `--features clickhouse` (no reqwest dep otherwise).
//!   - Runs only when `TDW_CLICKHOUSE_TEST_URL` is set in the
//!     environment. CI workflows that bring up a clickhouse docker
//!     container should set this; default `cargo test --workspace`
//!     leaves it unset and the test silently skips.
//!
//! Optional env vars:
//!   - `TDW_CLICKHOUSE_TEST_USER`     (default: unset — anonymous)
//!   - `TDW_CLICKHOUSE_TEST_PASSWORD` (default: unset)

#![cfg(feature = "clickhouse")]

use serde_json::Value;
use tdw_core::OlapEngine;
use tdw_storage_clickhouse::ClickHouseHttpEngine;

fn endpoint() -> Option<String> {
    std::env::var("TDW_CLICKHOUSE_TEST_URL").ok()
}

#[tokio::test]
async fn clickhouse_engine_executes_and_queries_against_real_server() {
    let Some(url) = endpoint() else {
        eprintln!("TDW_CLICKHOUSE_TEST_URL not set; skipping clickhouse integration test");
        return;
    };

    let user = std::env::var("TDW_CLICKHOUSE_TEST_USER").ok();
    let password = std::env::var("TDW_CLICKHOUSE_TEST_PASSWORD").ok();
    let engine = ClickHouseHttpEngine::new(&url, user, password)
        .unwrap_or_else(|error| panic!("connect must succeed: {error}"));

    // Idempotent setup: drop + create the smoke table.
    engine
        .execute("DROP TABLE IF EXISTS tdw_g010_smoke")
        .await
        .unwrap_or_else(|error| panic!("drop must succeed: {error}"));
    engine
        .execute("CREATE TABLE tdw_g010_smoke (label String, value Float64) ENGINE = Memory")
        .await
        .unwrap_or_else(|error| panic!("create must succeed: {error}"));
    engine
        .execute("INSERT INTO tdw_g010_smoke VALUES ('alpha', 1.5), ('beta', 2.5)")
        .await
        .unwrap_or_else(|error| panic!("insert must succeed: {error}"));

    let payload = engine
        .query_json(
            "SELECT label, value FROM tdw_g010_smoke ORDER BY label",
            Value::Null,
        )
        .await
        .unwrap_or_else(|error| panic!("query_json must succeed: {error}"));

    // ClickHouse FORMAT JSON returns { meta: [...], data: [...], rows: N, ... }
    let data = payload
        .get("data")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("response must contain `data` array, got: {payload}"));
    assert_eq!(data.len(), 2);
    assert_eq!(data[0]["label"], "alpha");
    assert_eq!(data[1]["label"], "beta");

    engine
        .execute("DROP TABLE tdw_g010_smoke")
        .await
        .unwrap_or_else(|error| panic!("cleanup drop must succeed: {error}"));
}

#[tokio::test]
async fn clickhouse_engine_rejects_param_binding() {
    let Some(url) = endpoint() else {
        return;
    };

    let engine = ClickHouseHttpEngine::new(&url, None, None)
        .unwrap_or_else(|error| panic!("connect must succeed: {error}"));

    let err = engine
        .query_json("SELECT 1", serde_json::json!([1, 2, 3]))
        .await
        .expect_err("param binding must be rejected in this slice");
    assert!(err.to_string().contains("not supported"));
}
