//! Offline `InMemoryLexicalEngine` round-trip: index a document and run a
//! full-text search. No network, no docker — the default in-memory engine is
//! always available.
//!
//! Run with: `cargo run -p tdw-storage-meilisearch --example basic`

use serde_json::json;
use tdw_core::{LexicalDoc, LexicalEngine, TextQuery};
use tdw_storage_meilisearch::InMemoryLexicalEngine;

#[tokio::main]
async fn main() -> tdw_core::Result<()> {
    let engine = InMemoryLexicalEngine::default();

    engine
        .index(
            "research",
            vec![
                LexicalDoc {
                    id: "note-1".to_string(),
                    body: "AAPL volatility note".to_string(),
                    fields: json!({ "symbol": "AAPL" }),
                },
                LexicalDoc {
                    id: "note-2".to_string(),
                    body: "MSFT earnings recap".to_string(),
                    fields: json!({ "symbol": "MSFT" }),
                },
            ],
        )
        .await?;

    let hits = engine
        .search_text(
            "research",
            TextQuery {
                text: "volatility".to_string(),
                top_k: 5,
            },
        )
        .await?;

    assert_eq!(hits[0].id, "note-1");
    println!(
        "search ok: top hit = {} (score {:.1}, fields {})",
        hits[0].id, hits[0].score, hits[0].fields
    );
    Ok(())
}
