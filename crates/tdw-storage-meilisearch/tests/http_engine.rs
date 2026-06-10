//! Integration test for the real Meilisearch backend.
//!
//! Gated two ways:
//!   - Compiled only with `--features meilisearch` (no reqwest dep
//!     otherwise).
//!   - Runs only when `TDW_MEILISEARCH_TEST_URL` is set in the
//!     environment. Default `cargo test --workspace` leaves it unset
//!     and the test silently skips.
//!
//! Optional env vars:
//!   - `TDW_MEILISEARCH_TEST_API_KEY` (default: unset)

#![cfg(feature = "meilisearch")]

use serde_json::json;
use tdw_core::{LexicalDoc, LexicalEngine, TextQuery};
use tdw_storage_meilisearch::MeilisearchHttpEngine;

fn endpoint() -> Option<String> {
    std::env::var("TDW_MEILISEARCH_TEST_URL").ok()
}

fn index_name() -> String {
    format!("tdw_g010_smoke_{}", std::process::id())
}

#[tokio::test]
async fn meilisearch_engine_indexes_and_searches_against_real_meilisearch() {
    let Some(url) = endpoint() else {
        eprintln!("TDW_MEILISEARCH_TEST_URL not set; skipping meilisearch integration test");
        return;
    };
    let api_key = std::env::var("TDW_MEILISEARCH_TEST_API_KEY").ok();
    let engine = MeilisearchHttpEngine::new(&url, api_key)
        .unwrap_or_else(|error| panic!("connect must succeed: {error}"));

    let index = index_name();

    let docs = vec![
        LexicalDoc {
            id: "1".to_string(),
            body: "alpha launches the rocket".to_string(),
            fields: json!({"category": "space"}),
        },
        LexicalDoc {
            id: "2".to_string(),
            body: "beta files a quarterly report".to_string(),
            fields: json!({"category": "finance"}),
        },
        LexicalDoc {
            id: "3".to_string(),
            body: "gamma launches another rocket from alpha base".to_string(),
            fields: json!({"category": "space"}),
        },
    ];

    engine
        .index(&index, docs)
        .await
        .unwrap_or_else(|error| panic!("index must succeed: {error}"));

    let results = engine
        .search_text(
            &index,
            TextQuery {
                text: "rocket".to_string(),
                top_k: 5,
                filter: tdw_core::PayloadFilter::default(),
            },
        )
        .await
        .unwrap_or_else(|error| panic!("search must succeed: {error}"));

    assert!(
        results.len() >= 2,
        "expected at least two hits, got {results:?}"
    );
    let ids: Vec<&str> = results.iter().map(|hit| hit.id.as_str()).collect();
    assert!(
        ids.contains(&"1") && ids.contains(&"3"),
        "rocket query must surface both rocket docs; got ids={ids:?}"
    );
}

#[tokio::test]
async fn meilisearch_engine_rejects_empty_query() {
    let Some(url) = endpoint() else {
        return;
    };
    let engine = MeilisearchHttpEngine::new(&url, None)
        .unwrap_or_else(|error| panic!("connect must succeed: {error}"));

    let err = engine
        .search_text(
            "any",
            TextQuery {
                text: "   ".to_string(),
                top_k: 5,
                filter: tdw_core::PayloadFilter::default(),
            },
        )
        .await
        .expect_err("blank query must be rejected");
    assert!(err.to_string().contains("must not be empty"));
}
