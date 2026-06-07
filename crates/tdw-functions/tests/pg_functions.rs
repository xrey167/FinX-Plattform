//! Integration test for the Postgres-backed step and run stores.
//!
//! Runs only with `--features postgres`; if `TDW_POSTGRES_TEST_URL` is not set,
//! the test compiles and exits without touching a database.

#![cfg(feature = "postgres")]

use std::sync::Arc;

use serde_json::json;
use tdw_functions::{FunctionDef, FunctionRegistry, PgRunStore, PgStepStore, StepStore, Trigger};

fn database_url() -> Option<String> {
    std::env::var("TDW_POSTGRES_TEST_URL").ok()
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_millis() as i64
}

fn test_prefix() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    format!("fn_pg_test_{}_{}_", std::process::id(), nanos)
}

async fn apply_migration(step_store: &PgStepStore) {
    step_store.migrate().await.expect("migration");
}

#[tokio::test]
async fn pg_step_store_get_put_list_roundtrip() {
    let Some(url) = database_url() else {
        eprintln!("TDW_POSTGRES_TEST_URL not set; skipping pg functions integration test");
        return;
    };

    let step_store = PgStepStore::connect(&url)
        .await
        .unwrap_or_else(|error| panic!("connect: {error}"));
    apply_migration(&step_store).await;

    let prefix = test_prefix();
    let run_id = format!("{prefix}run1");

    // Initial get returns None
    let result = step_store
        .get(&run_id, "step_a")
        .await
        .expect("get before put");
    assert!(result.is_none());

    // Put stores the value
    step_store
        .put(&run_id, "step_a", &json!({"answer": 42}))
        .await
        .expect("put step_a");

    // Get returns it
    let value = step_store
        .get(&run_id, "step_a")
        .await
        .expect("get after put")
        .expect("should be Some");
    assert_eq!(value, json!({"answer": 42}));

    // Second put for same key is ignored (first-write-wins)
    step_store
        .put(&run_id, "step_a", &json!({"answer": 999}))
        .await
        .expect("second put ignored");
    let value2 = step_store
        .get(&run_id, "step_a")
        .await
        .expect("get after second put")
        .expect("Some");
    assert_eq!(value2, json!({"answer": 42}), "first write must win");

    // Put another step and verify list
    step_store
        .put(&run_id, "step_b", &json!("done"))
        .await
        .expect("put step_b");
    let mut listed = step_store.list(&run_id).await.expect("list");
    listed.sort_by_key(|(name, _)| name.clone());
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].0, "step_a");
    assert_eq!(listed[1].0, "step_b");
}

#[tokio::test]
async fn pg_registry_invoke_and_resume() {
    let Some(url) = database_url() else {
        eprintln!("TDW_POSTGRES_TEST_URL not set; skipping pg functions integration test");
        return;
    };

    let step_store = PgStepStore::connect(&url)
        .await
        .unwrap_or_else(|error| panic!("connect step: {error}"));
    apply_migration(&step_store).await;

    let run_store = PgRunStore::connect(&url)
        .await
        .unwrap_or_else(|error| panic!("connect run: {error}"));

    let prefix = test_prefix();

    let mut registry = FunctionRegistry::with_stores(Arc::new(step_store), Arc::new(run_store));

    registry
        .register(FunctionDef::from_fn(
            "pg-test-fn",
            vec![Trigger::Event("test".to_string())],
            |ctx, _payload| {
                Box::pin(async move {
                    let v = ctx
                        .step("only-step", || async { Ok(json!({"pg": true})) })
                        .await?;
                    Ok(v)
                })
            },
        ))
        .expect("register");

    let run_id = format!("{prefix}run_pg");
    let now = now_ms();

    let result = registry
        .invoke("pg-test-fn", json!({}), run_id.clone(), now)
        .await
        .expect("invoke");
    assert_eq!(result, json!({"pg": true}));

    // Resume: step is memoized, result is re-returned
    let resumed = registry.resume(&run_id, now).await.expect("resume");
    assert_eq!(resumed, json!({"pg": true}));
}
