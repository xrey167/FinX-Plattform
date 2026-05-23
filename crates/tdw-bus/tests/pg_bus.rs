//! Integration test for the Postgres-backed event bus.

#![cfg(feature = "postgres")]

use tdw_bus::PgEventBus;
use tdw_event::sample_event;
use tdw_storage_postgres::PgEngine;

fn database_url() -> Option<String> {
    std::env::var("TDW_POSTGRES_TEST_URL").ok()
}

fn table_name() -> String {
    format!("tdw_bus_test_{}", std::process::id())
}

#[tokio::test]
async fn pg_bus_publishes_and_reads_back_in_order() {
    let Some(url) = database_url() else {
        eprintln!("TDW_POSTGRES_TEST_URL not set; skipping pg_bus integration test");
        return;
    };
    let engine = PgEngine::connect(&url)
        .await
        .unwrap_or_else(|error| panic!("postgres connect: {error}"));
    let bus = PgEventBus::new(engine.clone()).with_table(table_name());

    use tdw_core::RelationalEngine;
    let drop_sql = format!("DROP TABLE IF EXISTS {}", table_name());
    engine
        .execute(&drop_sql, serde_json::Value::Null)
        .await
        .unwrap_or_else(|error| panic!("drop: {error}"));
    bus.ensure_schema()
        .await
        .unwrap_or_else(|error| panic!("ensure_schema: {error}"));

    let first = bus
        .publish(sample_event("service"))
        .await
        .unwrap_or_else(|error| panic!("publish first: {error}"));
    let second = bus
        .publish(sample_event("worker"))
        .await
        .unwrap_or_else(|error| panic!("publish second: {error}"));
    let third = bus
        .publish(sample_event("agent"))
        .await
        .unwrap_or_else(|error| panic!("publish third: {error}"));

    assert!(first < second && second < third);

    let all = bus
        .read_from(0)
        .await
        .unwrap_or_else(|error| panic!("read_from 0: {error}"));
    assert_eq!(all.len(), 3);
    assert_eq!(all[0].sequence, first);
    assert_eq!(all[2].sequence, third);

    let tail = bus
        .read_from(second)
        .await
        .unwrap_or_else(|error| panic!("read_from second: {error}"));
    assert_eq!(tail.len(), 2);
    assert_eq!(tail[0].sequence, second);

    let window = bus
        .retention_window()
        .await
        .unwrap_or_else(|error| panic!("retention_window: {error}"));
    let window = window.expect("non-empty bus must have a window");
    assert_eq!(window.oldest_sequence, first);
    assert_eq!(window.newest_sequence, third);

    engine
        .execute(&drop_sql, serde_json::Value::Null)
        .await
        .unwrap_or_else(|error| panic!("cleanup drop: {error}"));
}
