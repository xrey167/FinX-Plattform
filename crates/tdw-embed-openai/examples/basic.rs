//! Offline `tdw-embed-openai` example: build the `/embeddings` request body and
//! decode a fixture vector into a validated `Embedding`.
//!
//! NO live API call and NO API key — `api_key_present` is just a flag asserting
//! the caller would have a key; no key is read. The real HTTP client lives
//! behind the `http` feature and is exercised by the env-gated live test.
//!
//! ```text
//! cargo run --example tdw_embed_openai_basic -p tdw-embed-openai
//! ```

use tdw_embed_openai::{EMBEDDINGS_PATH, build_embedding_request, decode_embedding};

fn main() {
    // 1) Build the request body that would be POSTed to /v1/embeddings.
    let request = build_embedding_request("text-embedding-3-small", "macro note", true)
        .expect("request should build");
    assert_eq!(request.provider, "openai");
    assert_eq!(request.path, EMBEDDINGS_PATH);
    assert_eq!(request.body["model"], "text-embedding-3-small");
    assert_eq!(request.body["input"], "macro note");
    println!("request body: {}", request.body);

    // 2) Decode a *fixture* vector (what the server would have returned) into the
    //    workspace Embedding contract — validated, no network.
    let embedding = decode_embedding("text-embedding-3-small", vec![0.1, -0.2, 0.3])
        .expect("vector should decode");
    assert_eq!(embedding.model_id, "text-embedding-3-small");
    assert_eq!(embedding.vector.len(), 3);
    println!("decoded vector: {:?}", embedding.vector);

    // Guard rails: missing key flag and bad vectors are rejected.
    assert!(build_embedding_request("text-embedding-3-small", "x", false).is_err());
    assert!(decode_embedding("text-embedding-3-small", vec![f32::NAN]).is_err());
}
