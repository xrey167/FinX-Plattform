//! Offline `tdw-embed-google` example: build the Gemini `:embedContent` request
//! body and decode a fixture vector into a validated `Embedding`.
//!
//! NO live API call and NO API key — `api_key_present` is just a flag; no key is
//! read. The real HTTP client lives behind the `http` feature.
//!
//! ```text
//! cargo run --example tdw_embed_google_basic -p tdw-embed-google
//! ```

use tdw_embed_google::{build_embedding_request, decode_embedding};

fn main() {
    // 1) Build the Gemini request body + path.
    let request = build_embedding_request("text-embedding-004", "macro note", true)
        .expect("request should build");
    assert_eq!(request.provider, "google");
    assert_eq!(request.path, "/models/text-embedding-004:embedContent");
    assert_eq!(request.body["model"], "models/text-embedding-004");
    assert_eq!(request.body["content"]["parts"][0]["text"], "macro note");
    println!("request path: {}", request.path);
    println!("request body: {}", request.body);

    // 2) Decode a *fixture* vector into the workspace Embedding contract.
    let embedding =
        decode_embedding("text-embedding-004", vec![0.1, 0.2]).expect("vector should decode");
    assert_eq!(embedding.model_id, "text-embedding-004");
    assert_eq!(embedding.vector.len(), 2);
    println!("decoded vector: {:?}", embedding.vector);

    // Guard rails.
    assert!(build_embedding_request("text-embedding-004", "x", false).is_err());
    assert!(decode_embedding("text-embedding-004", vec![f32::INFINITY]).is_err());
}
