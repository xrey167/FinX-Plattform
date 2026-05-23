//! Integration test for the real Postgres backend.
//!
//! Gated two ways:
//!   - Compiled only with `--features postgres` (no sqlx dep otherwise).
//!   - Runs only when `TDW_POSTGRES_TEST_URL` is set in the environment.
//!     CI workflows that bring up a postgres docker container should set
//!     this; default `cargo test --workspace` leaves it unset and the
//!     test silently skips.

#![cfg(feature = "postgres")]

use serde_json::Value;
use tdw_core::RelationalEngine;
use tdw_storage_postgres::PgEngine;

fn url() -> Option<String> {
    std::env::var("TDW_POSTGRES_TEST_URL").ok()
}

#[tokio::test]
async fn pg_engine_executes_and_fetches_against_real_postgres() {
    let Some(database_url) = url() else {
        eprintln!("TDW_POSTGRES_TEST_URL not set; skipping postgres integration test");
        return;
    };

    let engine = PgEngine::connect(&database_url)
        .await
        .unwrap_or_else(|error| panic!("connect must succeed: {error}"));

    // Idempotent table for the smoke; drop+recreate to keep test
    // hermetic regardless of prior runs.
    engine
        .execute("DROP TABLE IF EXISTS tdw_g010_smoke", Value::Null)
        .await
        .unwrap_or_else(|error| panic!("drop must succeed: {error}"));
    let created = engine
        .execute(
            "CREATE TABLE tdw_g010_smoke (id BIGSERIAL PRIMARY KEY, label TEXT NOT NULL, value DOUBLE PRECISION)",
            Value::Null,
        )
        .await
        .unwrap_or_else(|error| panic!("create must succeed: {error}"));
    assert_eq!(created, 0, "DDL reports zero rows affected");

    let inserted = engine
        .execute(
            "INSERT INTO tdw_g010_smoke (label, value) VALUES ($1, $2), ($3, $4)",
            serde_json::json!(["alpha", 1.5, "beta", 2.5]),
        )
        .await
        .unwrap_or_else(|error| panic!("insert must succeed: {error}"));
    assert_eq!(inserted, 2);

    let rows = engine
        .fetch_json(
            "SELECT label, value FROM tdw_g010_smoke WHERE value > $1 ORDER BY label",
            serde_json::json!([1.0]),
        )
        .await
        .unwrap_or_else(|error| panic!("fetch_json must succeed: {error}"));

    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["label"], "alpha");
    assert_eq!(rows[0]["value"], 1.5);
    assert_eq!(rows[1]["label"], "beta");
    assert_eq!(rows[1]["value"], 2.5);

    engine
        .execute("DROP TABLE tdw_g010_smoke", Value::Null)
        .await
        .unwrap_or_else(|error| panic!("cleanup drop must succeed: {error}"));
}

#[tokio::test]
async fn pg_engine_rejects_object_shaped_params() {
    let Some(database_url) = url() else {
        return;
    };

    let engine = PgEngine::connect(&database_url)
        .await
        .unwrap_or_else(|error| panic!("connect must succeed: {error}"));

    let err = engine
        .execute("SELECT 1", serde_json::json!({"k": 1}))
        .await
        .expect_err("object-shaped params must be rejected");
    assert!(err.to_string().contains("array of primitives"));
}
