//! Ingestion (knowledge-system B5) — the write side of the B4 retrieval contract.
//!
//! Idempotent re-index over a content-hash manifest, rule-driven
//! auto-tagging, lexical co-indexing, and durable graph stamping.
//!
//! [`KnowledgeIndexer`] wraps a [`KnowledgeIndex`] and adds the optional
//! channels the `tdw-retrieve` retriever reads: a lexical engine (same
//! payload as the vector point) and a durable [`GraphEngine`] where document
//! nodes carry `props.{as_of, plane}` — the fields the retriever's
//! `document_visible` gate filters on. Undated documents get NO `as_of` prop
//! and stay invisible to temporal queries by construction.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::json;
use tdw_core::{
    Direction, GraphEdge, GraphEngine, GraphNode, LexicalDoc, LexicalEngine, Provenance,
    TraversalFilter,
};
use tdw_kg::{Entity, EntityKind};
use tdw_news_compose::Article;
use tdw_tag_rules::{EntityContext, NeighborView, RuleEngine};

use crate::{
    KnowledgeDocument, KnowledgeError, KnowledgeIndex, Result, document_payload, is_date,
    validate_document,
};

/// What `index_at` did with a document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndexOutcome {
    /// The document was (re-)indexed.
    Indexed,
    /// The manifest already records this exact content — nothing was written.
    SkippedUnchanged,
}

/// One manifest record: the content hash last indexed and when.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestEntry {
    pub content_hash: String,
    /// The `now` date the document was last indexed with.
    pub indexed_at: String,
}

/// The idempotency ledger: document id → last-indexed content hash.
///
/// Persistence is the CALLER's concern (`to_json`/`from_json` round-trip; the
/// daemon owns paths) — this crate stays free of filesystem access.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexManifest {
    entries: BTreeMap<String, ManifestEntry>,
}

impl IndexManifest {
    /// Whether `doc_id` was already indexed with exactly this content.
    #[must_use]
    pub fn is_current(&self, doc_id: &str, content_hash: &str) -> bool {
        self.entries
            .get(doc_id)
            .is_some_and(|entry| entry.content_hash == content_hash)
    }

    /// Record a successful index of `doc_id`.
    pub fn record(&mut self, doc_id: &str, content_hash: &str, indexed_at: &str) {
        self.entries.insert(
            doc_id.to_string(),
            ManifestEntry {
                content_hash: content_hash.to_string(),
                indexed_at: indexed_at.to_string(),
            },
        );
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Serialize for persistence.
    ///
    /// # Errors
    ///
    /// Returns [`KnowledgeError::Storage`] if serialization fails.
    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(self)
            .map_err(|error| KnowledgeError::Storage(error.to_string()))
    }

    /// Restore a persisted manifest.
    ///
    /// # Errors
    ///
    /// Returns [`KnowledgeError::Storage`] on malformed input.
    pub fn from_json(raw: &str) -> Result<Self> {
        serde_json::from_str(raw).map_err(|error| KnowledgeError::Storage(error.to_string()))
    }
}

/// Stable FNV-1a 64-bit hash — deterministic across platforms and releases
/// (std's `DefaultHasher` is explicitly NOT stable).
fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// The manifest's content hash over every indexed-relevant document field
/// (body, label, tags, plane, `as_of`, mentions, source) — id is the manifest
/// KEY, not part of the hash.
#[must_use]
pub fn content_hash(document: &KnowledgeDocument) -> String {
    let canonical = json!({
        "body": document.body,
        "label": document.entity.label,
        "kind": document.entity.kind,
        "tags": document.tags,
        "plane": document.plane,
        "as_of": document.as_of,
        "mentions": document.mentions,
        "source": document.source,
    });
    format!("{:016x}", fnv1a64(canonical.to_string().as_bytes()))
}

/// The ingestion indexer: a [`KnowledgeIndex`] plus the B5 write-side
/// concerns. Channels are optional exactly like the retriever's read side.
pub struct KnowledgeIndexer {
    index: KnowledgeIndex,
    rules: RuleEngine,
    manifest: IndexManifest,
    lexical: Option<(Arc<dyn LexicalEngine>, String)>,
    graph: Option<Arc<dyn GraphEngine>>,
}

impl KnowledgeIndexer {
    /// Wrap an index; no rules, empty manifest, no extra channels.
    #[must_use]
    pub fn new(index: KnowledgeIndex) -> Self {
        Self {
            index,
            rules: RuleEngine::default(),
            manifest: IndexManifest::default(),
            lexical: None,
            graph: None,
        }
    }

    /// Attach the auto-tagging rule engine.
    #[must_use]
    pub fn with_rules(mut self, rules: RuleEngine) -> Self {
        self.rules = rules;
        self
    }

    /// Resume from a persisted manifest.
    #[must_use]
    pub fn with_manifest(mut self, manifest: IndexManifest) -> Self {
        self.manifest = manifest;
        self
    }

    /// Attach the lexical co-index (same payload contract as the vector point).
    #[must_use]
    pub fn with_lexical(
        mut self,
        engine: Arc<dyn LexicalEngine>,
        index_name: impl Into<String>,
    ) -> Self {
        self.lexical = Some((engine, index_name.into()));
        self
    }

    /// Attach the durable graph (document nodes with `props.{as_of, plane}`,
    /// `described_by` and `mentions` edges).
    #[must_use]
    pub fn with_graph(mut self, engine: Arc<dyn GraphEngine>) -> Self {
        self.graph = Some(engine);
        self
    }

    /// The wrapped index (search, active tags, neighbors).
    #[must_use]
    pub const fn index(&self) -> &KnowledgeIndex {
        &self.index
    }

    /// The idempotency manifest — persist via [`IndexManifest::to_json`].
    #[must_use]
    pub const fn manifest(&self) -> &IndexManifest {
        &self.manifest
    }

    /// Index one document effective `now` (`YYYY-MM-DD`, injected).
    ///
    /// Pipeline: manifest check (skip when content unchanged) → auto-tag
    /// rules (graph context pre-fetched, assignments land in the tag store
    /// AND on the document's payload tags) → vector + in-process graph/tags →
    /// lexical co-index → durable graph stamping → manifest record.
    ///
    /// # Errors
    ///
    /// Returns the first failing step's error; the manifest records only
    /// fully-indexed documents, so a failed document is retried next sweep.
    pub async fn index_at(
        &mut self,
        mut document: KnowledgeDocument,
        now: &str,
    ) -> Result<IndexOutcome> {
        validate_document(&document)?;
        if !is_date(now) {
            return Err(KnowledgeError::InvalidDocumentField("now"));
        }
        let hash = content_hash(&document);
        if self.manifest.is_current(&document.id, &hash) {
            return Ok(IndexOutcome::SkippedUnchanged);
        }

        // Auto-tagging: rule-assigned tags also join the document's payload
        // tags so the retrieval channels see them.
        for assignment in self.apply_rules(&document, now).await? {
            if !document.tags.contains(&assignment.tag_id) {
                document.tags.push(assignment.tag_id);
            }
        }
        document.tags.sort();
        document.tags.dedup();

        self.index.index_document_at(document.clone(), now).await?;

        if let Some((lexical, index_name)) = &self.lexical {
            lexical
                .index(
                    index_name,
                    vec![LexicalDoc {
                        id: document.id.clone(),
                        body: document.body.clone(),
                        fields: document_payload(&document),
                    }],
                )
                .await
                .map_err(|error| KnowledgeError::Storage(error.to_string()))?;
        }

        if let Some(graph) = self.graph.clone() {
            write_durable_graph(graph.as_ref(), &document).await?;
        }

        self.manifest.record(&document.id, &hash, now);
        Ok(IndexOutcome::Indexed)
    }

    /// Index a batch effective `now`. Documents that fail validation fail the
    /// whole batch up front; after that, each document indexes independently
    /// and the first write error aborts (already-indexed documents stay
    /// recorded in the manifest).
    ///
    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub async fn index_batch_at(
        &mut self,
        documents: Vec<KnowledgeDocument>,
        now: &str,
    ) -> Result<Vec<IndexOutcome>> {
        for document in &documents {
            validate_document(document)?;
        }
        let mut outcomes = Vec::with_capacity(documents.len());
        for document in documents {
            outcomes.push(self.index_at(document, now).await?);
        }
        Ok(outcomes)
    }

    /// Evaluate the rule engine against the document's entity with pre-fetched
    /// graph context (rules stay sync/deterministic — B3).
    async fn apply_rules(
        &mut self,
        document: &KnowledgeDocument,
        now: &str,
    ) -> Result<Vec<tdw_tags::TagAssignment>> {
        let fields = json!({
            "body": document.body,
            "plane": document.plane,
            "as_of": document.as_of,
            "mentions": document.mentions,
            "source": document.source,
        });
        let active_tags = self.index.active_tags(&document.entity.entity_id, now);
        let neighbors = match &self.graph {
            Some(graph) => fetch_neighbors(graph.as_ref(), &document.entity.entity_id).await?,
            None => Vec::new(),
        };
        let ctx = EntityContext {
            entity_id: &document.entity.entity_id,
            label: &document.entity.label,
            fields: &fields,
            active_tags: &active_tags,
            neighbors: &neighbors,
        };
        self.rules
            .apply_at(&ctx, now, self.index.tags_store_mut())
            .map_err(|error| KnowledgeError::Tag(error.to_string()))
    }
}

/// One-hop neighborhood as the sync rule engine's pre-fetched view.
async fn fetch_neighbors(graph: &dyn GraphEngine, entity_id: &str) -> Result<Vec<NeighborView>> {
    let traversal = TraversalFilter {
        direction: Direction::Both,
        max_hops: 1,
        ..TraversalFilter::default()
    };
    Ok(graph
        .neighbors(entity_id, &traversal)
        .await
        .map_err(|error| KnowledgeError::Storage(error.to_string()))?
        .into_iter()
        .map(|(edge, node)| NeighborView {
            edge_type: edge.rel,
            entity_id: node.id,
            kind: serde_json::to_value(node.kind)
                .ok()
                .and_then(|value| value.as_str().map(ToOwned::to_owned)),
            tags: Vec::new(),
        })
        .collect())
}

/// Write the durable-graph projection of one document: the entity node, a
/// `document:<id>` node carrying `props.{as_of, plane}` (the fields the B4
/// retriever's `document_visible` gate reads — an undated document gets NO
/// `as_of` prop and stays invisible to temporal queries), a `described_by`
/// edge, and `mentions` edges with Primitive stub nodes for unknown targets
/// (existing nodes are never clobbered).
async fn write_durable_graph(graph: &dyn GraphEngine, document: &KnowledgeDocument) -> Result<()> {
    let storage = |error: tdw_core::Error| KnowledgeError::Storage(error.to_string());
    let as_of_ts = document.as_of.as_deref().map(tdw_tags::date_to_timestamp);

    let mut document_props = json!({});
    if let Some(plane) = &document.plane {
        document_props["plane"] = json!(plane);
    }
    if let Some(timestamp) = &as_of_ts {
        document_props["as_of"] = json!(timestamp);
    }
    let mut nodes = vec![
        document.entity.to_graph_node(),
        GraphNode {
            id: format!("document:{}", document.id),
            kind: EntityKind::Document,
            label: document.entity.label.clone(),
            aliases: Vec::new(),
            props: document_props,
            valid_from: as_of_ts.clone(),
            valid_to: None,
        },
    ];
    for mention in &document.mentions {
        // Stubs only — a mention must never overwrite a real node.
        if graph.node(mention).await.map_err(storage)?.is_none() {
            nodes.push(GraphNode {
                id: mention.clone(),
                kind: EntityKind::Primitive,
                label: mention.clone(),
                aliases: Vec::new(),
                props: json!({"mention_stub": true}),
                valid_from: None,
                valid_to: None,
            });
        }
    }
    graph.upsert_nodes(nodes).await.map_err(storage)?;

    let provenance = Provenance::Ingest {
        source: "tdw-knowledge:index".to_string(),
    };
    let mut edges = vec![GraphEdge {
        from: document.entity.entity_id.clone(),
        to: format!("document:{}", document.id),
        rel: "described_by".to_string(),
        props: serde_json::Value::Null,
        provenance: provenance.clone(),
        valid_from: as_of_ts.clone(),
        valid_to: None,
    }];
    for mention in &document.mentions {
        edges.push(GraphEdge {
            from: document.entity.entity_id.clone(),
            to: mention.clone(),
            rel: "mentions".to_string(),
            props: serde_json::Value::Null,
            provenance: provenance.clone(),
            valid_from: as_of_ts.clone(),
            valid_to: None,
        });
    }
    graph.upsert_edges(edges).await.map_err(storage)
}

/// Convert a composed news [`Article`] into an ingestible document on `plane`.
///
/// Id/entity come from a stable hash of the canonical URL, body =
/// title + summary, `as_of` from the publication timestamp (UTC date), and
/// one `mentions` target per symbol (`instrument:<SYM>`).
#[must_use]
pub fn article_to_document(article: &Article, plane: &str) -> KnowledgeDocument {
    let url_hash = fnv1a64(article.url.as_bytes());
    KnowledgeDocument {
        id: format!("news-{url_hash:016x}"),
        body: format!("{}\n{}", article.title, article.summary),
        entity: Entity {
            entity_id: format!("news:{url_hash:016x}"),
            kind: EntityKind::Document,
            label: article.title.clone(),
            aliases: Vec::new(),
        },
        tags: Vec::new(),
        source: Some(crate::DocumentSource::News {
            source: article.source.clone(),
            url: article.url.clone(),
        }),
        plane: Some(plane.to_string()),
        as_of: Some(date_from_epoch_ms(article.published_ts_ms)),
        mentions: article
            .symbols
            .iter()
            .map(|symbol| format!("instrument:{symbol}"))
            .collect(),
    }
}

/// UTC calendar date (`YYYY-MM-DD`) of a Unix epoch in milliseconds.
/// Pre-epoch timestamps floor toward earlier days (`div_euclid`).
#[must_use]
pub fn date_from_epoch_ms(ts_ms: i64) -> String {
    let (year, month, day) = civil_from_days(ts_ms.div_euclid(86_400_000));
    format!("{year:04}-{month:02}-{day:02}")
}

/// Days-since-epoch → proleptic Gregorian civil date (Howard Hinnant's
/// `civil_from_days` algorithm, exact over the full i64 day range we use).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_point = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_point + 2) / 5 + 1;
    let month = if month_point < 10 {
        month_point + 3
    } else {
        month_point - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // 1..=12 / 1..=31
    (year, month as u32, day as u32)
}
