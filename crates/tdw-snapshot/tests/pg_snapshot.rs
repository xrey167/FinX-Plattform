//! Integration test for the Postgres-backed snapshot store.
//!
//! Gated two ways:
//!   - Compiled only with `--features postgres`.
//!   - Runs only when `TDW_POSTGRES_TEST_URL` is set.

#![cfg(feature = "postgres")]

use tdw_snapshot::PgSnapshotStore;
use tdw_storage_postgres::PgEngine;

fn database_url() -> Option<String> {
    std::env::var("TDW_POSTGRES_TEST_URL").ok()
}

fn table_name() -> String {
    format!("tdw_snapshot_test_{}", std::process::id())
}

#[tokio::test]
async fn pg_snapshot_commits_versions_monotonically_and_recovers_by_lookup() {
    let Some(url) = database_url() else {
        eprintln!("TDW_POSTGRES_TEST_URL not set; skipping pg_snapshot integration test");
        return;
    };

    let engine = PgEngine::connect(&url)
        .await
        .unwrap_or_else(|error| panic!("postgres connect: {error}"));
    let store = PgSnapshotStore::new(engine.clone()).with_table(table_name());

    use tdw_core::RelationalEngine;
    let drop_sql = format!("DROP TABLE IF EXISTS {}", table_name());
    engine
        .execute(&drop_sql, serde_json::Value::Null)
        .await
        .unwrap_or_else(|error| panic!("drop must succeed: {error}"));
    store
        .ensure_schema()
        .await
        .unwrap_or_else(|error| panic!("ensure_schema: {error}"));

    // First commit for table "alpha" -> version 1.
    let first = store
        .commit(
            "alpha",
            "2026-05-23T00:00:00Z",
            vec!["row-a".into(), "row-b".into()],
        )
        .await
        .unwrap_or_else(|error| panic!("commit first: {error}"));
    assert_eq!(first.table, "alpha");
    assert_eq!(first.version, 1);
    assert_eq!(first.row_ids, vec!["row-a", "row-b"]);

    // Second commit same table -> version 2.
    let second = store
        .commit("alpha", "2026-05-23T01:00:00Z", vec!["row-c".into()])
        .await
        .unwrap_or_else(|error| panic!("commit second: {error}"));
    assert_eq!(second.version, 2);

    // First commit for a different table is its own version 1.
    let beta = store
        .commit("beta", "2026-05-23T02:00:00Z", vec!["row-x".into()])
        .await
        .unwrap_or_else(|error| panic!("commit beta: {error}"));
    assert_eq!(beta.table, "beta");
    assert_eq!(beta.version, 1);

    // latest("alpha") returns the second commit.
    let latest = store
        .latest("alpha")
        .await
        .unwrap_or_else(|error| panic!("latest: {error}"));
    let latest = latest.expect("alpha must have a latest snapshot");
    assert_eq!(latest.version, 2);
    assert_eq!(latest.row_ids, vec!["row-c"]);

    // as_of_version("alpha", 1) returns the first commit.
    let v1 = store
        .as_of_version("alpha", 1)
        .await
        .unwrap_or_else(|error| panic!("as_of_version: {error}"));
    let v1 = v1.expect("alpha v1 must exist");
    assert_eq!(v1.row_ids, vec!["row-a", "row-b"]);

    // Unknown table yields None.
    let unknown = store
        .latest("unknown")
        .await
        .unwrap_or_else(|error| panic!("latest unknown: {error}"));
    assert!(unknown.is_none());

    engine
        .execute(&drop_sql, serde_json::Value::Null)
        .await
        .unwrap_or_else(|error| panic!("cleanup drop: {error}"));
}
