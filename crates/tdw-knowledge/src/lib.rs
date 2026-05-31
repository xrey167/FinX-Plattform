#![forbid(unsafe_code)]

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tdw_core::{VectorEngine, VectorPoint, VectorQuery};
use tdw_embed::EmbeddingProvider;
use tdw_embed_local::HashEmbeddingProvider;
use tdw_kg::{Entity, KnowledgeGraph, Relationship, validate_entity};
use tdw_storage_qdrant::InMemoryVectorEngine;
use tdw_tags::{TagAssignment, TagDefinition, TagStore};
use thiserror::Error;

pub type Result<T> = std::result::Result<T, KnowledgeError>;

#[derive(Debug, Error)]
pub enum KnowledgeError {
    #[error("embedding error: {0}")]
    Embedding(String),
    #[error("storage error: {0}")]
    Storage(String),
    #[error("tag error: {0}")]
    Tag(String),
    #[error("invalid vector payload field: {0}")]
    InvalidPayloadField(&'static str),
    #[error("invalid knowledge document field: {0}")]
    InvalidDocumentField(&'static str),
    #[error("invalid knowledge query: {0}")]
    InvalidQuery(&'static str),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnowledgeDocument {
    pub id: String,
    pub body: String,
    pub entity: Entity,
    pub tags: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KnowledgeHit {
    pub id: String,
    pub score: f32,
    pub entity_id: String,
    pub tags: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SymbolRef {
    pub kind: String,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyntaxSummary {
    pub symbols: Vec<SymbolRef>,
}

pub struct KnowledgeIndex {
    embedder: Arc<dyn EmbeddingProvider>,
    vectors: Arc<dyn VectorEngine>,
    graph: KnowledgeGraph,
    tags: TagStore,
}

impl Default for KnowledgeIndex {
    /// Build a fully offline, deterministic index over the hash embedder and an
    /// in-process vector engine, preserving the original default behavior.
    fn default() -> Self {
        Self::new(
            Arc::new(HashEmbeddingProvider::default()),
            Arc::new(InMemoryVectorEngine::default()),
        )
    }
}

impl KnowledgeIndex {
    /// Build a `KnowledgeIndex` over an injected embedder and vector engine.
    ///
    /// This is the durability/backends seam: callers pass a shared
    /// [`VectorEngine`] (e.g. the daemon's `AppState.vector`, which may be a
    /// real Qdrant engine) so indexed knowledge persists across the engine the
    /// rest of the daemon uses. The knowledge graph and tag store remain
    /// in-process (`Default`).
    #[must_use]
    pub fn new(embedder: Arc<dyn EmbeddingProvider>, vectors: Arc<dyn VectorEngine>) -> Self {
        Self {
            embedder,
            vectors,
            graph: KnowledgeGraph::default(),
            tags: TagStore::default(),
        }
    }

    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub async fn index_document(&mut self, document: KnowledgeDocument) -> Result<()> {
        validate_document(&document)?;
        let embedding = self
            .embedder
            .embed(&document.body)
            .await
            .map_err(|error| KnowledgeError::Embedding(error.to_string()))?;
        self.graph.upsert_entity(document.entity.clone());
        self.graph.add_relationship(Relationship {
            from: document.entity.entity_id.clone(),
            to: format!("document:{}", document.id),
            rel_type: "described_by".to_string(),
            provenance: "tdw-knowledge".to_string(),
        });
        for tag in &document.tags {
            self.tags
                .define(TagDefinition {
                    tag_id: tag.clone(),
                    parent: None,
                    ttl_days: None,
                })
                .map_err(|error| KnowledgeError::Tag(error.to_string()))?;
            self.tags
                .assign(TagAssignment {
                    entity_id: document.entity.entity_id.clone(),
                    tag_id: tag.clone(),
                    assigned_at: "2026-05-22".to_string(),
                    expires_at: None,
                    provenance: "tdw-knowledge:index".to_string(),
                })
                .map_err(|error| KnowledgeError::Tag(error.to_string()))?;
        }
        self.vectors
            .upsert(
                "tdw_knowledge",
                vec![VectorPoint {
                    id: document.id,
                    vector: embedding.vector,
                    payload: json!({
                        "entity_id": document.entity.entity_id,
                        "tags": document.tags,
                    }),
                }],
            )
            .await
            .map_err(|error| KnowledgeError::Storage(error.to_string()))
    }

    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub async fn search(&self, query: &str, top_k: usize) -> Result<Vec<KnowledgeHit>> {
        validate_query(query, top_k)?;
        let embedding = self
            .embedder
            .embed(query)
            .await
            .map_err(|error| KnowledgeError::Embedding(error.to_string()))?;
        let hits = self
            .vectors
            .search_knn(
                "tdw_knowledge",
                VectorQuery {
                    vector: embedding.vector,
                    top_k,
                },
            )
            .await
            .map_err(|error| KnowledgeError::Storage(error.to_string()))?;
        hits.into_iter()
            .map(|hit| {
                Ok(KnowledgeHit {
                    id: hit.id,
                    score: hit.score,
                    entity_id: payload_str(&hit.payload, "entity_id")?.to_string(),
                    tags: payload_string_array(&hit.payload, "tags")?,
                })
            })
            .collect()
    }

    pub fn active_tags(&self, entity_id: &str, as_of: &str) -> Vec<String> {
        self.tags.active_tags(entity_id, as_of)
    }

    pub fn neighbors(&self, entity_id: &str) -> Vec<String> {
        self.graph
            .neighbors(entity_id)
            .into_iter()
            .map(|entity| entity.entity_id.clone())
            .collect()
    }
}

/// # Errors
///
/// Returns an error variant if the underlying operation fails.
pub fn validate_document(document: &KnowledgeDocument) -> Result<()> {
    if !is_identifier(&document.id) {
        return Err(KnowledgeError::InvalidDocumentField("id"));
    }
    if document.body.trim().is_empty() {
        return Err(KnowledgeError::InvalidDocumentField("body"));
    }
    validate_entity(&document.entity)
        .map_err(|_| KnowledgeError::InvalidDocumentField("entity"))?;
    if document.tags.iter().any(|tag| !is_tag_id(tag)) {
        return Err(KnowledgeError::InvalidDocumentField("tags"));
    }
    Ok(())
}

/// # Errors
///
/// Returns an error variant if the underlying operation fails.
pub fn validate_query(query: &str, top_k: usize) -> Result<()> {
    if query.trim().is_empty() {
        return Err(KnowledgeError::InvalidQuery("query"));
    }
    if top_k == 0 {
        return Err(KnowledgeError::InvalidQuery("top_k"));
    }
    Ok(())
}

fn payload_str<'a>(payload: &'a Value, field: &'static str) -> Result<&'a str> {
    payload
        .get(field)
        .and_then(Value::as_str)
        .ok_or(KnowledgeError::InvalidPayloadField(field))
}

fn payload_string_array(payload: &Value, field: &'static str) -> Result<Vec<String>> {
    payload
        .get(field)
        .and_then(Value::as_array)
        .ok_or(KnowledgeError::InvalidPayloadField(field))?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(ToOwned::to_owned)
                .ok_or(KnowledgeError::InvalidPayloadField(field))
        })
        .collect()
}

fn is_identifier(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
}

fn is_tag_id(value: &str) -> bool {
    !value.is_empty()
        && value.contains(':')
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, ':' | '_' | '-')
        })
}

#[must_use]
pub fn summarize_syntax(input: &str) -> SyntaxSummary {
    let symbols = input
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            trimmed.strip_prefix("create table ").map_or_else(
                || {
                    trimmed.strip_prefix("fn ").map(|rest| SymbolRef {
                        kind: "function".to_string(),
                        name: rest.split('(').next().unwrap_or_default().to_string(),
                    })
                },
                |rest| {
                    Some(SymbolRef {
                        kind: "table".to_string(),
                        name: rest
                            .split([' ', '('])
                            .next()
                            .unwrap_or_default()
                            .trim_matches('"')
                            .to_string(),
                    })
                },
            )
        })
        .filter(|symbol| !symbol.name.is_empty())
        .collect();
    SyntaxSummary { symbols }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tdw_kg::EntityKind;

    #[tokio::test]
    async fn indexes_and_searches_embedded_knowledge() {
        let mut index = KnowledgeIndex::default();
        index
            .index_document(KnowledgeDocument {
                id: "doc-1".to_string(),
                body: "AAPL equity momentum research".to_string(),
                entity: Entity {
                    entity_id: "instrument:AAPL".to_string(),
                    kind: EntityKind::Instrument,
                    label: "Apple".to_string(),
                    aliases: vec!["AAPL".to_string()],
                },
                tags: vec!["asset:equity".to_string()],
            })
            .await
            .unwrap_or_else(|error| panic!("index succeeds: {error}"));

        let hits = index
            .search("AAPL momentum", 1)
            .await
            .unwrap_or_else(|error| panic!("search succeeds: {error}"));

        assert_eq!(hits[0].id, "doc-1");
        assert_eq!(hits[0].entity_id, "instrument:AAPL");
        assert_eq!(
            index.active_tags("instrument:AAPL", "2026-05-22"),
            vec!["asset:equity".to_string()]
        );
    }

    #[tokio::test]
    async fn new_injects_custom_embedder_and_vector_engine() {
        // Proves the injection seam: an index built over an explicitly supplied
        // embedder + vector engine (here the same offline defaults, but passed
        // in via `new` rather than synthesized by `default`) indexes and
        // searches end to end.
        let mut index = KnowledgeIndex::new(
            Arc::new(HashEmbeddingProvider::default()),
            Arc::new(InMemoryVectorEngine::default()),
        );
        index
            .index_document(KnowledgeDocument {
                id: "doc-inject".to_string(),
                body: "AAPL equity momentum research".to_string(),
                entity: Entity {
                    entity_id: "instrument:AAPL".to_string(),
                    kind: EntityKind::Instrument,
                    label: "Apple".to_string(),
                    aliases: vec!["AAPL".to_string()],
                },
                tags: vec!["asset:equity".to_string()],
            })
            .await
            .unwrap_or_else(|error| panic!("injected index should index: {error}"));

        let hits = index
            .search("AAPL momentum", 1)
            .await
            .unwrap_or_else(|error| panic!("injected index should search: {error}"));
        assert_eq!(hits[0].id, "doc-inject");
        assert_eq!(hits[0].entity_id, "instrument:AAPL");
    }

    #[tokio::test]
    async fn search_rejects_malformed_vector_payloads() {
        let index = KnowledgeIndex::default();
        let vector = index
            .embedder
            .embed("AAPL")
            .await
            .unwrap_or_else(|error| panic!("embedding should succeed: {error}"))
            .vector;
        index
            .vectors
            .upsert(
                "tdw_knowledge",
                vec![VectorPoint {
                    id: "bad-payload".to_string(),
                    vector,
                    payload: json!({}),
                }],
            )
            .await
            .unwrap_or_else(|error| panic!("malformed payload fixture inserts: {error}"));

        let error = index
            .search("AAPL", 1)
            .await
            .expect_err("malformed vector payload should error");
        assert!(matches!(
            error,
            KnowledgeError::InvalidPayloadField("entity_id")
        ));
    }

    #[tokio::test]
    async fn rejects_invalid_documents_and_queries() {
        let mut index = KnowledgeIndex::default();
        let document = KnowledgeDocument {
            id: "../doc".to_string(),
            body: "AAPL".to_string(),
            entity: Entity {
                entity_id: "instrument:AAPL".to_string(),
                kind: EntityKind::Instrument,
                label: "Apple".to_string(),
                aliases: vec!["AAPL".to_string()],
            },
            tags: vec!["asset:equity".to_string()],
        };

        let error = index
            .index_document(document)
            .await
            .expect_err("invalid document id should fail");
        assert!(matches!(error, KnowledgeError::InvalidDocumentField("id")));
        assert!(matches!(
            index.search(" ", 1).await,
            Err(KnowledgeError::InvalidQuery("query"))
        ));
        assert!(matches!(
            index.search("AAPL", 0).await,
            Err(KnowledgeError::InvalidQuery("top_k"))
        ));
    }

    #[test]
    fn summarizes_schema_and_code_symbols() {
        let summary = summarize_syntax(
            r"
create table raw.market_data_bar (symbol text);
fn build_context() {}
",
        );

        assert_eq!(summary.symbols[0].kind, "table");
        assert_eq!(summary.symbols[0].name, "raw.market_data_bar");
        assert_eq!(summary.symbols[1].kind, "function");
        assert_eq!(summary.symbols[1].name, "build_context");
    }
}
