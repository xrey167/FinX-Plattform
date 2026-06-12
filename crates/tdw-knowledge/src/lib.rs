#![forbid(unsafe_code)]
#![deny(clippy::pedantic, clippy::nursery)]

pub mod extraction;
pub mod feeds;
pub mod indexer;
pub mod lessons;
pub mod proposals;
pub mod reindex;
pub mod runtime;

pub use extraction::ExtractionFreshness;
pub use feeds::FeedFreshness;
pub use runtime::{KgGraphHealth, KgProposalCounts, KgStatus, KgThesesStatus};

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::json;
use tdw_core::{VectorEngine, VectorPoint};
use tdw_embed::EmbeddingProvider;
use tdw_embed_local::HashEmbeddingProvider;
use tdw_kg::{Entity, KnowledgeGraph, Relationship, validate_entity};
use tdw_retrieve::{KnowledgeQuery, QueryFilter, Retriever};
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

/// Where a document came from — ingestion lineage for provenance and
/// re-index sweeps (distinct from the taxonomy's `Origin`, which classifies
/// entity kinds).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DocumentSource {
    /// A composed news article.
    News { source: String, url: String },
    /// Provider/dataset metadata.
    Provider { provider: String },
    /// Manually authored.
    Manual,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnowledgeDocument {
    pub id: String,
    pub body: String,
    pub entity: Entity,
    pub tags: Vec<String>,
    /// Ingestion lineage (knowledge-system B5).
    #[serde(default)]
    pub source: Option<DocumentSource>,
    /// Knowledge plane (`agent` / `platform` / `shared`).
    #[serde(default)]
    pub plane: Option<String>,
    /// Effective date, `YYYY-MM-DD`. Undated documents are structurally
    /// invisible to temporal queries (B4 leakage-safety contract).
    #[serde(default)]
    pub as_of: Option<String>,
    /// Entity ids this document mentions — written as `mentions` graph edges
    /// at index time (stub nodes are created for unknown targets).
    #[serde(default)]
    pub mentions: Vec<String>,
}

impl KnowledgeDocument {
    /// A document with the pre-B5 fields; lineage/plane/`as_of`/mentions
    /// default to absent (set them directly when ingesting).
    #[must_use]
    pub fn new(
        id: impl Into<String>,
        body: impl Into<String>,
        entity: Entity,
        tags: Vec<String>,
    ) -> Self {
        Self {
            id: id.into(),
            body: body.into(),
            entity,
            tags,
            source: None,
            plane: None,
            as_of: None,
            mentions: Vec::new(),
        }
    }
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
    /// The vector collection, namespaced by the embedder's `model_id` so two
    /// embedders of different dimension (e.g. the 8-dim hash vs a 1536-dim
    /// `OpenAI` model) never share — and silently corrupt — one collection.
    collection: String,
}

/// Build the per-embedder collection name: `tdw_knowledge__<model>` with the
/// model id sanitized to `[A-Za-z0-9_]` so it is a safe collection identifier.
///
/// Sanitization is not injective (e.g. `text-embedding-3` and `text_embedding_3`
/// collapse to one name). That is acceptable here: the only harmful collision is
/// two models of *different* vector dimension sharing a collection, which the
/// backing engine's dimension guard
/// (`QdrantHttpEngine::ensure_collection`) rejects loudly rather than corrupting.
#[must_use]
pub fn collection_name(model_id: &str) -> String {
    let safe: String = model_id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    format!("tdw_knowledge__{safe}")
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
        let collection = collection_name(embedder.model_id());
        Self {
            embedder,
            vectors,
            graph: KnowledgeGraph::default(),
            tags: TagStore::default(),
            collection,
        }
    }

    /// Index a document, stamping its tag assignments effective `now` (a
    /// `YYYY-MM-DD` date). `now` is injected — nothing here reads a clock — so
    /// indexing is deterministic in tests and honest in production (the previous
    /// implementation hardcoded a build-era date, knowledge-system B3).
    ///
    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub async fn index_document_at(
        &mut self,
        document: KnowledgeDocument,
        now: &str,
    ) -> Result<()> {
        validate_document(&document)?;
        if !is_date(now) {
            return Err(KnowledgeError::InvalidDocumentField("now"));
        }
        let embedding = self
            .embedder
            .embed(&document.body)
            .await
            .map_err(|error| KnowledgeError::Embedding(error.to_string()))?;
        self.index_document_with_vector_at(document, embedding.vector, now)
            .await
    }

    /// Batch indexing over [`EmbeddingProvider::embed_batch`] (knowledge-system
    /// B5): one embedding round-trip for the whole batch, then per-document
    /// indexing. Validation is all-or-nothing UP FRONT — a bad document fails
    /// the batch before anything is written.
    ///
    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub async fn index_documents_at(
        &mut self,
        documents: Vec<KnowledgeDocument>,
        now: &str,
    ) -> Result<()> {
        if !is_date(now) {
            return Err(KnowledgeError::InvalidDocumentField("now"));
        }
        for document in &documents {
            validate_document(document)?;
        }
        if documents.is_empty() {
            return Ok(());
        }
        let bodies: Vec<String> = documents
            .iter()
            .map(|document| document.body.clone())
            .collect();
        let embeddings = self
            .embedder
            .embed_batch(&bodies)
            .await
            .map_err(|error| KnowledgeError::Embedding(error.to_string()))?;
        for (document, embedding) in documents.into_iter().zip(embeddings) {
            self.index_document_with_vector_at(document, embedding.vector, now)
                .await?;
        }
        Ok(())
    }

    /// The shared write path behind single and batch indexing: in-process
    /// graph + tags, then the vector point with the B4 retrieval payload
    /// contract (`entity_id`, `tags`, `entity_kind`, `plane`, `as_of` as a
    /// normalized timestamp, `content_hash`). `plane`/`as_of` keys are written
    /// only when present — a missing `as_of` is what makes an undated document
    /// invisible to temporal queries.
    async fn index_document_with_vector_at(
        &mut self,
        document: KnowledgeDocument,
        vector: Vec<f32>,
        now: &str,
    ) -> Result<()> {
        self.graph.upsert_entity(document.entity.clone());
        self.graph.add_relationship(Relationship {
            from: document.entity.entity_id.clone(),
            to: format!("document:{}", document.id),
            rel_type: "described_by".to_string(),
            provenance: "tdw-knowledge".to_string(),
        });
        let active = self.tags.active_tags(&document.entity.entity_id, now);
        for tag in &document.tags {
            // Define only when missing: re-defining an existing tag with
            // `parent: None` would re-parent it to the taxonomy root.
            if !self.tags.is_defined(tag) {
                self.tags
                    .define(TagDefinition {
                        tag_id: tag.clone(),
                        parent: None,
                        ttl_days: None,
                    })
                    .map_err(|error| KnowledgeError::Tag(error.to_string()))?;
            }
            // Skip already-active assignments: the store is append-only, so a
            // re-index would otherwise duplicate every assignment (R5).
            if active.contains(tag) {
                continue;
            }
            self.tags
                .assign(TagAssignment {
                    entity_id: document.entity.entity_id.clone(),
                    tag_id: tag.clone(),
                    assigned_at: now.to_string(),
                    expires_at: None,
                    provenance: "tdw-knowledge:index".to_string(),
                })
                .map_err(|error| KnowledgeError::Tag(error.to_string()))?;
        }
        let payload = document_payload(&document);
        self.vectors
            .upsert(
                &self.collection,
                vec![VectorPoint {
                    id: document.id,
                    vector,
                    payload,
                }],
            )
            .await
            .map_err(|error| KnowledgeError::Storage(error.to_string()))
    }

    /// Crate-internal access for the [`indexer`]'s rule engine, whose
    /// assignments write into the same store the index reads.
    pub(crate) const fn tags_store_mut(&mut self) -> &mut TagStore {
        &mut self.tags
    }

    /// Vector search, delegated to a single-channel [`tdw_retrieve::Retriever`]
    /// (knowledge-system B4). Single-channel RRF preserves the engine's KNN
    /// order exactly, and the returned `score` is the channel's raw vector
    /// score, so the pre-B4 contract is unchanged. One caveat: a point whose
    /// payload carries `entity_id` but lacks `tags` now yields empty tags
    /// instead of an error (the retriever cannot distinguish missing from
    /// empty); a missing `entity_id` still errors.
    ///
    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub async fn search(&self, query: &str, top_k: usize) -> Result<Vec<KnowledgeHit>> {
        validate_query(query, top_k)?;
        let retriever = Retriever::new(
            Arc::clone(&self.embedder),
            Arc::clone(&self.vectors),
            self.collection.clone(),
        );
        let knowledge_query = KnowledgeQuery::try_new(query, top_k, QueryFilter::default(), None)
            .map_err(|error| KnowledgeError::Storage(error.to_string()))?;
        let hits = retriever
            .search(&knowledge_query)
            .await
            .map_err(|error| match error {
                tdw_core::Error::Provider(message) => KnowledgeError::Embedding(message),
                other => KnowledgeError::Storage(other.to_string()),
            })?;
        hits.into_iter()
            .map(|hit| {
                if hit.entity_id.is_empty() {
                    return Err(KnowledgeError::InvalidPayloadField("entity_id"));
                }
                let score = hit
                    .explanation
                    .channels
                    .first()
                    .map_or(0.0, |evidence| evidence.raw_score);
                Ok(KnowledgeHit {
                    id: hit.id,
                    score,
                    entity_id: hit.entity_id,
                    tags: hit.tags,
                })
            })
            .collect()
    }

    #[must_use]
    pub fn active_tags(&self, entity_id: &str, as_of: &str) -> Vec<String> {
        self.tags.active_tags(entity_id, as_of)
    }

    #[must_use]
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
    if let Some(plane) = &document.plane
        && !is_plane(plane)
    {
        return Err(KnowledgeError::InvalidDocumentField("plane"));
    }
    if let Some(as_of) = &document.as_of
        && !is_date(as_of)
    {
        return Err(KnowledgeError::InvalidDocumentField("as_of"));
    }
    if document.mentions.iter().any(|id| !is_entity_ref(id)) {
        return Err(KnowledgeError::InvalidDocumentField("mentions"));
    }
    Ok(())
}

/// The B4 retrieval payload contract for one document.
///
/// `entity_id`, `tags`, `entity_kind`, `content_hash`, `provenance_class`
/// (K-X3 trust-dial stamp), plus `plane`/`as_of` ONLY when present (`as_of`
/// normalized to a timestamp).  Shared by the vector point and the lexical
/// co-index so the two channels can never drift.
///
/// ## Provenance-class stamp (K-X3)
///
/// The `provenance_class` field encodes the trust bucket this document belongs
/// to.  The mapping is purely structural — derived from the document's entity
/// kind and source lineage, never from caller input:
///
/// | Condition                               | `provenance_class`   |
/// |-----------------------------------------|----------------------|
/// | `entity_kind == finding`                | `"user_authored"`    |
/// | all other documents                     | `"document_ingested"`|
///
/// `rule_derived` and `agent_proposed` classes are reserved for future
/// document kinds stamped by those paths; they do not appear in this function
/// today.  The field is always present so no reindex is needed for filtering
/// on new indexes; old index points without it fall back to
/// `DocumentIngested` in the retriever (see `tdw-retrieve::effective_trust_class`).
#[must_use]
pub fn document_payload(document: &KnowledgeDocument) -> serde_json::Value {
    let provenance_class = provenance_class_token(document.entity.kind);
    let mut payload = json!({
        "entity_id": document.entity.entity_id,
        "tags": document.tags,
        "entity_kind": kind_token(document.entity.kind),
        "content_hash": indexer::content_hash(document),
        "provenance_class": provenance_class,
    });
    if let Some(plane) = &document.plane {
        payload["plane"] = json!(plane);
    }
    if let Some(as_of) = &document.as_of {
        payload["as_of"] = json!(tdw_tags::date_to_timestamp(as_of));
    }
    payload
}

/// Map a document's entity kind to its K-X3 `provenance_class` payload token.
///
/// `Finding` nodes are user-authored (written with `Provenance::Agent { gated:
/// false }` via `tdw.kg.finding`).  All other indexable entity kinds are
/// externally sourced and enter the index via the ingestion path.
fn provenance_class_token(kind: tdw_kg::EntityKind) -> &'static str {
    if kind == tdw_kg::EntityKind::Finding {
        "user_authored"
    } else {
        "document_ingested"
    }
}

/// The taxonomy kind's lowercase serde token (e.g. `instrument`) — the
/// `entity_kind` payload value the retrieval contract filters on.
fn kind_token(kind: tdw_kg::EntityKind) -> String {
    serde_json::to_value(kind)
        .ok()
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
        .unwrap_or_default()
}

/// Plane tokens are lowercase identifiers (`agent` / `platform` / `shared`).
fn is_plane(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|character| character.is_ascii_lowercase() || character == '_')
}

/// Mention targets use the graph id grammar (`instrument:AAPL`).
fn is_entity_ref(value: &str) -> bool {
    !value.is_empty()
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, ':' | '.' | '_' | '-')
        })
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

/// `YYYY-MM-DD` shape check, mirroring the tag store's assignment validation —
/// enforced up front so a tagless document (whose tag loop never reaches
/// `TagStore::assign`) cannot silently swallow a garbage `now`.
fn is_date(value: &str) -> bool {
    value.len() == 10
        && value.as_bytes()[4] == b'-'
        && value.as_bytes()[7] == b'-'
        && value
            .chars()
            .enumerate()
            .all(|(index, character)| matches!(index, 4 | 7) || character.is_ascii_digit())
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
            .index_document_at(
                KnowledgeDocument {
                    id: "doc-1".to_string(),
                    body: "AAPL equity momentum research".to_string(),
                    entity: Entity {
                        entity_id: "instrument:AAPL".to_string(),
                        kind: EntityKind::Instrument,
                        label: "Apple".to_string(),
                        aliases: vec!["AAPL".to_string()],
                    },
                    tags: vec!["asset:equity".to_string()],
                    source: None,
                    plane: None,
                    as_of: None,
                    mentions: Vec::new(),
                },
                "2026-05-22",
            )
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
            .index_document_at(
                KnowledgeDocument {
                    id: "doc-inject".to_string(),
                    body: "AAPL equity momentum research".to_string(),
                    entity: Entity {
                        entity_id: "instrument:AAPL".to_string(),
                        kind: EntityKind::Instrument,
                        label: "Apple".to_string(),
                        aliases: vec!["AAPL".to_string()],
                    },
                    tags: vec!["asset:equity".to_string()],
                    source: None,
                    plane: None,
                    as_of: None,
                    mentions: Vec::new(),
                },
                "2026-05-22",
            )
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
                &index.collection,
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

    #[test]
    fn collection_name_is_namespaced_and_sanitized_per_model() {
        // Embedders of different dimension must map to DIFFERENT collections so
        // they never share — and silently corrupt — one vector collection.
        assert_eq!(
            collection_name("local-hash-8"),
            "tdw_knowledge__local_hash_8"
        );
        assert_eq!(
            collection_name("text-embedding-3-small"),
            "tdw_knowledge__text_embedding_3_small"
        );
        assert_ne!(
            collection_name("local-hash-8"),
            collection_name("text-embedding-3-small")
        );
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
            source: None,
            plane: None,
            as_of: None,
            mentions: Vec::new(),
        };

        let error = index
            .index_document_at(document, "2026-05-22")
            .await
            .expect_err("invalid document id should fail");
        assert!(matches!(error, KnowledgeError::InvalidDocumentField("id")));
        // A garbage `now` must error LOUDLY even on a tagless document, whose
        // tag loop never reaches the store's own date validation.
        let tagless = KnowledgeDocument {
            id: "doc-tagless".to_string(),
            body: "AAPL".to_string(),
            entity: Entity {
                entity_id: "instrument:AAPL".to_string(),
                kind: EntityKind::Instrument,
                label: "Apple".to_string(),
                aliases: vec!["AAPL".to_string()],
            },
            tags: Vec::new(),
            source: None,
            plane: None,
            as_of: None,
            mentions: Vec::new(),
        };
        let error = index
            .index_document_at(tagless, "not-a-date")
            .await
            .expect_err("garbage now should fail before any storage step");
        assert!(matches!(error, KnowledgeError::InvalidDocumentField("now")));
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

    #[test]
    fn validate_document_rejects_body_entity_and_tags() {
        let ok_entity = || Entity {
            entity_id: "instrument:AAPL".to_string(),
            kind: EntityKind::Instrument,
            label: "Apple".to_string(),
            aliases: vec!["AAPL".to_string()],
        };

        assert!(matches!(
            validate_document(&KnowledgeDocument {
                id: "doc-1".to_string(),
                body: "  ".to_string(),
                entity: ok_entity(),
                tags: vec![],
                source: None,
                plane: None,
                as_of: None,
                mentions: Vec::new(),
            }),
            Err(KnowledgeError::InvalidDocumentField("body"))
        ));
        // An invalid entity (empty label fails validate_entity) maps to the
        // "entity" document field.
        assert!(matches!(
            validate_document(&KnowledgeDocument {
                id: "doc-1".to_string(),
                body: "ok".to_string(),
                entity: Entity {
                    label: String::new(),
                    ..ok_entity()
                },
                tags: vec![],
                source: None,
                plane: None,
                as_of: None,
                mentions: Vec::new(),
            }),
            Err(KnowledgeError::InvalidDocumentField("entity"))
        ));
        // A tag without a ':' is not a valid tag id.
        assert!(matches!(
            validate_document(&KnowledgeDocument {
                id: "doc-1".to_string(),
                body: "ok".to_string(),
                entity: ok_entity(),
                tags: vec!["notacolon".to_string()],
                source: None,
                plane: None,
                as_of: None,
                mentions: Vec::new(),
            }),
            Err(KnowledgeError::InvalidDocumentField("tags"))
        ));
    }
}
