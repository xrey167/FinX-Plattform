//! Collection rebuild for embedder switches (knowledge-system B6).
//!
//! Vector collections are namespaced per embedder model id, so flipping the
//! configured embedder starts from an EMPTY collection. `reindex_collection`
//! repopulates it from the durable documents in the lexical store (the one
//! channel that keeps full bodies), re-embedding every body with the target
//! embedder and preserving each document's payload verbatim — switching
//! never loses data and can never mix dimensions.

use std::sync::Arc;

use tdw_core::{LexicalEngine, VectorEngine, VectorPoint};
use tdw_embed::EmbeddingProvider;

use crate::{KnowledgeError, Result, collection_name};

/// Page size for the lexical scan and the embedding batches.
pub const REINDEX_BATCH: usize = 64;

/// Rebuild the target embedder's vector collection from the lexical store.
///
/// Pages through `lexical_index` (stable engine order), embeds each page's
/// bodies with one `embed_batch` round-trip, and upserts the vectors into
/// the embedder's own namespaced collection with the stored payload fields
/// carried over verbatim (`entity_id`, `tags`, `entity_kind`, `plane`,
/// `as_of`, `content_hash` — the B4 retrieval contract survives unchanged).
/// Returns the number of documents re-embedded.
///
/// The vector engine's dimension guard rejects any write into a collection
/// holding vectors of a different dimension, so a half-configured embedder
/// fails loudly instead of corrupting a collection.
///
/// # Errors
///
/// Returns the failing engine's or embedder's error; pages already written
/// stay written (re-running reindex is idempotent — upserts by id).
pub async fn reindex_collection(
    embedder: &Arc<dyn EmbeddingProvider>,
    vectors: &Arc<dyn VectorEngine>,
    lexical: &Arc<dyn LexicalEngine>,
    lexical_index: &str,
) -> Result<usize> {
    let collection = collection_name(embedder.model_id());
    let mut offset = 0usize;
    let mut total = 0usize;
    loop {
        let page = lexical
            .documents(lexical_index, offset, REINDEX_BATCH)
            .await
            .map_err(|error| KnowledgeError::Storage(error.to_string()))?;
        if page.is_empty() {
            return Ok(total);
        }
        let bodies: Vec<String> = page.iter().map(|doc| doc.body.clone()).collect();
        let embeddings = embedder
            .embed_batch(&bodies)
            .await
            .map_err(|error| KnowledgeError::Embedding(error.to_string()))?;
        let points: Vec<VectorPoint> = page
            .iter()
            .zip(embeddings)
            .map(|(doc, embedding)| VectorPoint {
                id: doc.id.clone(),
                vector: embedding.vector,
                payload: doc.fields.clone(),
            })
            .collect();
        vectors
            .upsert(&collection, points)
            .await
            .map_err(|error| KnowledgeError::Storage(error.to_string()))?;
        total += page.len();
        offset += page.len();
    }
}
