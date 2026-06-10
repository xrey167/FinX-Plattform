//! Integration test for the real Qdrant backend.
//!
//! Gated two ways:
//!   - Compiled only with `--features qdrant` (no reqwest dep otherwise).
//!   - Runs only when `TDW_QDRANT_TEST_URL` is set in the environment.
//!     CI workflows that bring up a qdrant docker container should set
//!     this; default `cargo test --workspace` leaves it unset and the
//!     test silently skips.
//!
//! Optional env vars:
//!   - `TDW_QDRANT_TEST_API_KEY` (default: unset)

#![cfg(feature = "qdrant")]

use serde_json::json;
use tdw_core::{VectorEngine, VectorPoint, VectorQuery};
use tdw_storage_qdrant::QdrantHttpEngine;

fn endpoint() -> Option<String> {
    std::env::var("TDW_QDRANT_TEST_URL").ok()
}

fn collection_name() -> String {
    format!("tdw_g010_smoke_{}", std::process::id())
}

#[tokio::test]
async fn qdrant_engine_upserts_and_searches_against_real_qdrant() {
    let Some(url) = endpoint() else {
        eprintln!("TDW_QDRANT_TEST_URL not set; skipping qdrant integration test");
        return;
    };
    let api_key = std::env::var("TDW_QDRANT_TEST_API_KEY").ok();
    let engine = QdrantHttpEngine::new(&url, api_key)
        .unwrap_or_else(|error| panic!("connect must succeed: {error}"));

    let collection = collection_name();

    // Qdrant accepts unsigned-integer or UUID point IDs as JSON values; a
    // bare numeric string like "1" is rejected with HTTP 400 because it is
    // a JSON string that is neither a valid UUID nor a numeric type. Use
    // hand-crafted deterministic v4-shaped UUIDs to stay independent of
    // any uuid crate version on the test runner.
    let points = vec![
        VectorPoint {
            id: "00000000-0000-4000-8000-000000000001".to_string(),
            vector: vec![0.1, 0.2, 0.3, 0.4],
            payload: json!({"label": "alpha"}),
        },
        VectorPoint {
            id: "00000000-0000-4000-8000-000000000002".to_string(),
            vector: vec![0.9, 0.8, 0.7, 0.6],
            payload: json!({"label": "beta"}),
        },
        VectorPoint {
            id: "00000000-0000-4000-8000-000000000003".to_string(),
            vector: vec![0.15, 0.25, 0.35, 0.45],
            payload: json!({"label": "alpha-near"}),
        },
    ];

    engine
        .upsert(&collection, points)
        .await
        .unwrap_or_else(|error| panic!("upsert must succeed: {error}"));

    let results = engine
        .search_knn(
            &collection,
            VectorQuery {
                vector: vec![0.1, 0.2, 0.3, 0.4],
                top_k: 2,
                filter: tdw_core::PayloadFilter::default(),
            },
        )
        .await
        .unwrap_or_else(|error| panic!("search must succeed: {error}"));

    assert_eq!(results.len(), 2);
    // The point identical to the query should be the top hit.
    assert_eq!(results[0].id, "00000000-0000-4000-8000-000000000001");
    assert_eq!(results[0].payload["label"], "alpha");
    // The near point should be second.
    assert_eq!(results[1].id, "00000000-0000-4000-8000-000000000003");
}

#[tokio::test]
async fn qdrant_engine_rejects_empty_upsert() {
    let Some(url) = endpoint() else {
        return;
    };
    let engine = QdrantHttpEngine::new(&url, None)
        .unwrap_or_else(|error| panic!("connect must succeed: {error}"));

    let err = engine
        .upsert("any", Vec::new())
        .await
        .expect_err("empty upsert must be rejected");
    assert!(err.to_string().contains("must include at least one point"));
}
