//! Offline example for `tdw-provider-huggingface`.
//!
//! Mirrors the cassette path: builds a text-generation query with
//! `transform_query`, then decodes an inline HF Inference response with
//! `transform_data` — no network.
//!
//! Run with:
//! ```bash
//! cargo run -p tdw-provider-huggingface --example basic --features http
//! ```

use bytes::Bytes;
use serde_json::json;
use tdw_core::Fetcher;
use tdw_provider_huggingface::HuggingFaceHttpTextGenerationFetcher;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let fetcher = HuggingFaceHttpTextGenerationFetcher::default();

    let query = HuggingFaceHttpTextGenerationFetcher::transform_query(json!({
        "model_id": "gpt2",
        "inputs": "Hello",
        "max_new_tokens": 8,
    }))?;

    // Inline fixture identical in shape to a recorded HF generation response
    // (the Inference API returns a JSON array of generations).
    let raw = Bytes::from(
        json!([{ "generated_text": "Hello, world! This is a generated continuation." }])
            .to_string()
            .into_bytes(),
    );

    let rows = fetcher.transform_data(&query, raw)?;
    println!("decoded {} HuggingFace generation(s):", rows.len());
    for row in &rows {
        println!("  {} -> {}", row.model_id, row.generated_text);
    }

    Ok(())
}
