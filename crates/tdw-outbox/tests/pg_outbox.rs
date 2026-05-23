//! Integration test for the Postgres-backed outbox.
//!
//! Gated two ways:
//!   - Compiled only with `--features postgres`.
//!   - Runs only when `TDW_POSTGRES_TEST_URL` is set.

#![cfg(feature = "postgres")]

use tdw_event::sample_event;
use tdw_outbox::PgOutboxStore;
use tdw_storage_postgres::PgEngine;

fn database_url() -> Option<String> {
    std::env::var("TDW_POSTGRES_TEST_URL").ok()
}

fn table_name() -> String {
    format!("tdw_outbox_test_{}", std::process::id())
}

#[tokio::test]
async fn pg_outbox_appends_marks_and_recovers_pending_records() {
    let Some(url) = database_url() else {
        eprintln!("TDW_POSTGRES_TEST_URL not set; skipping pg_outbox integration test");
        return;
    };

    let engine = PgEngine::connect(&url)
        .await
        .unwrap_or_else(|error| panic!("postgres connect: {error}"));
    let store = PgOutboxStore::new(engine.clone()).with_table(table_name());

    // Hermetic setup: drop any leftovers from a previous interrupted
    // run, then create fresh.
    let drop_sql = format!("DROP TABLE IF EXISTS {}", table_name());
    use tdw_core::RelationalEngine;
    engine
        .execute(&drop_sql, serde_json::Value::Null)
        .await
        .unwrap_or_else(|error| panic!("drop must succeed: {error}"));
    store
        .ensure_schema()
        .await
        .unwrap_or_else(|error| panic!("ensure_schema: {error}"));

    // Append two envelopes; expect monotonically increasing sequences.
    let first = store
        .append(sample_event("service"))
        .await
        .unwrap_or_else(|error| panic!("append first: {error}"));
    let second = store
        .append(sample_event("worker"))
        .await
        .unwrap_or_else(|error| panic!("append second: {error}"));
    assert!(
        second > first,
        "sequence must increase: {first} -> {second}"
    );

    // Mark the first as dispatched. Second remains pending.
    assert!(
        store
            .mark_dispatched(first)
            .await
            .unwrap_or_else(|error| panic!("mark_dispatched: {error}")),
        "mark_dispatched(first) must return true"
    );
    assert!(
        !store
            .mark_dispatched(first)
            .await
            .unwrap_or_else(|error| panic!("mark_dispatched second time: {error}")),
        "mark_dispatched(first) on already-dispatched row must return false"
    );

    // pending_after(0) must return only the second envelope.
    let pending = store
        .pending_after(0)
        .await
        .unwrap_or_else(|error| panic!("pending_after: {error}"));
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].sequence, second);

    // pending_after(second) must be empty.
    let pending = store
        .pending_after(second)
        .await
        .unwrap_or_else(|error| panic!("pending_after second: {error}"));
    assert!(pending.is_empty());

    engine
        .execute(&drop_sql, serde_json::Value::Null)
        .await
        .unwrap_or_else(|error| panic!("cleanup drop: {error}"));
}
