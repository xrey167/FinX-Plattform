//! Offline `InMemoryVectorEngine` round-trip: upsert two points and run a k-NN
//! search. No network, no docker — the default in-memory engine is always
//! available.
//!
//! Run with: `cargo run -p tdw-storage-qdrant --example basic`

use serde_json::json;
use tdw_core::{VectorEngine, VectorPoint, VectorQuery};
use tdw_storage_qdrant::InMemoryVectorEngine;

#[tokio::main]
async fn main() -> tdw_core::Result<()> {
    let engine = InMemoryVectorEngine::default();

    engine
        .upsert(
            "research",
            vec![
                VectorPoint {
                    id: "a".to_string(),
                    vector: vec![1.0, 0.0],
                    payload: json!({ "note": "aligned with query" }),
                },
                VectorPoint {
                    id: "b".to_string(),
                    vector: vec![0.25, 1.0],
                    payload: json!({ "note": "off-axis" }),
                },
            ],
        )
        .await?;

    let hits = engine
        .search_knn(
            "research",
            VectorQuery {
                vector: vec![1.0, 0.0],
                top_k: 1,
            },
        )
        .await?;

    assert_eq!(hits[0].id, "a");
    println!(
        "search ok: top hit = {} (score {:.3}, payload {})",
        hits[0].id, hits[0].score, hits[0].payload
    );
    Ok(())
}
