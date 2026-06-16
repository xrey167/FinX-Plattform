#![forbid(unsafe_code)]
#![deny(clippy::pedantic, clippy::nursery)]

//! Hybrid retrieval (knowledge-system B4).
//!
//! The [`Retriever`] fans a query out across up to three channels — vector
//! KNN, lexical full-text, and the temporal tag index — fuses the ranked
//! lists with pure Reciprocal Rank Fusion ([`rrf`]), optionally expands the
//! top fused hits through the knowledge graph with per-hop score decay, and
//! returns [`RetrievedHit`]s that EXPLAIN themselves: which channels found
//! them at which rank, which query tags matched, and the graph path that
//! reached an expanded hit.
//!
//! Channels are optional by construction: a vector-only retriever behaves
//! exactly like the pre-B4 `KnowledgeIndex::search`, which now delegates
//! here. Temporal `as_of` filtering uses tag-date semantics (`YYYY-MM-DD`);
//! the vector/lexical payload condition maps it onto the graph layer's
//! normalized timestamps via [`tdw_tags::date_to_timestamp`], and documents
//! WITHOUT an `as_of` payload field are excluded by a temporal query (dated
//! retrieval is leakage-safe by construction, never best-effort). The
//! graph-derived paths — the tag channel and graph expansion — enforce the
//! same contract through document-node props (the `document_visible`
//! `as_of`/`plane` mirror plus the entity node's kind), so no channel is a
//! way around the payload gate.
//!
//! ## Trust-dial (K-X3)
//!
//! [`QueryFilter::provenance_classes`] restricts hits to one or more
//! [`TrustClass`] buckets.  The class is stamped as a `provenance_class`
//! payload field at index time (see `tdw-knowledge::document_payload`).
//! The mapping is:
//!
//! | [`TrustClass`]        | Payload `provenance_class`   | Provenance at write time                     |
//! |----------------------|------------------------------|----------------------------------------------|
//! | `DocumentIngested`   | `"document_ingested"`        | `Provenance::Ingest { .. }` (provider/news/manual) |
//! | `RuleDerived`        | `"rule_derived"`             | `Provenance::Rule { .. }` (tag-rule derived) |
//! | `AgentProposed`      | `"agent_proposed"`           | `Provenance::Agent { gated: true }` (eval-landed) |
//! | `UserAuthored`       | `"user_authored"`            | `Provenance::Agent { gated: false }` (findings, `entity_kind=finding`) |
//!
//! Default (no filter) = all classes → unchanged behaviour (regression-safe).
//! Old index points without the `provenance_class` field are treated as
//! `DocumentIngested` when applying a restrictive filter (conservative
//! default: externally sourced content is the most trusted class so it is
//! never silently excluded from a `DocumentIngested`-only query against a
//! pre-stamp index).  Agents calling `tdw.kg.answer` (K-M3) can reuse the
//! same [`QueryFilter`] field when that tool lands.

pub mod evals;
pub mod rrf;

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tdw_core::{
    ConfidenceScore, Direction, EdgeConfidenceInput, Error, GraphEngine, LexicalEngine,
    PayloadCondition, PayloadFilter, Result, TextQuery, TraversalFilter, VectorEngine, VectorQuery,
    compute_confidence,
};
use tdw_embed::EmbeddingProvider;
use tdw_tags::{TagEngine, date_to_timestamp};
use tdw_taxonomy::EntityKind;

pub use rrf::{Fused, RRF_K, RankedEntry, RrfKHandle, rrf_fuse};

/// Confidence ranking weight for hybrid retrieval.
///
/// Blends `confidence` into the fused RRF score:
/// `final_score = rrf_score × (1 - weight) + confidence × weight`.
///
/// Set to `0.0` to disable (pure relevance ranking); [`tdw_core::DEFAULT_CONFIDENCE_WEIGHT`]
/// is the platform default.  Values must be in `[0.0, 1.0]`.
///
/// K-M3 seam: when `tdw.kg.answer` is built, pass the confidence scores alongside
/// retrieved hits and weight answer candidates by `score.confidence`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConfidenceRankingWeight(pub f64);

impl Default for ConfidenceRankingWeight {
    fn default() -> Self {
        Self(tdw_core::DEFAULT_CONFIDENCE_WEIGHT)
    }
}

impl ConfidenceRankingWeight {
    /// Zero weight — confidence does not influence ranking.
    pub const OFF: Self = Self(0.0);

    /// Clamp the weight to `[0.0, 1.0]`.
    #[must_use]
    pub const fn clamped(self) -> Self {
        Self(self.0.clamp(0.0, 1.0))
    }

    /// Blend an RRF score with a confidence score using this weight.
    ///
    /// `blended = rrf_score × (1 - w) + confidence × w`
    #[must_use]
    pub fn blend(self, rrf_score: f64, confidence: f64) -> f64 {
        let w = self.0.clamp(0.0, 1.0);
        rrf_score.mul_add(1.0 - w, confidence * w)
    }
}

/// The relationship linking an entity node to a document node in the graph.
pub const DESCRIBED_BY: &str = "described_by";
/// Document node ids are the document id under this prefix.
pub const DOCUMENT_PREFIX: &str = "document:";
/// Hard ceiling on `top_k`. Bounds the per-channel fan-out
/// (`channel_top_k = 4 * top_k`) and expansion seeding so a hostile query
/// cannot amplify work without limit (B8 exposes this layer to agents).
pub const MAX_TOP_K: usize = 256;
/// Hard ceiling on per-seed, per-hop expansion breadth.
pub const MAX_PER_HIT_LIMIT: usize = 64;

/// A trust-dial provenance class (K-X3).
///
/// Ordered from highest trust (externally sourced, human-vetted) to lowest
/// (agent-proposed, pending operator review).  Every indexable document is
/// stamped with exactly one class in the `provenance_class` payload field.
/// The class is derived from the document kind and ingestion path — it is NOT
/// a free-form caller input.
///
/// Use [`QueryFilter::provenance_classes`] to restrict a search to a subset.
/// An absent or `null` filter means **all classes** (the pre-K-X3 default —
/// no existing behaviour is changed).
///
/// ## Which classes the doc-index retrieval path can actually produce
///
/// The **vector and lexical channels** retrieve documents indexed via
/// `tdw-knowledge::document_payload`.  That function stamps exactly two
/// classes today:
///
/// - `DocumentIngested` — all entity kinds except `Finding`
/// - `UserAuthored` — `EntityKind::Finding` (K-X6 user findings)
///
/// `RuleDerived` and `AgentProposed` are **reserved for future use**.
/// Rule-derived and agent-gated facts currently write to the knowledge graph
/// as edges (with the corresponding `Provenance` variants), not as retrievable
/// documents, so the doc-index retrieval path cannot produce those classes
/// today.  Filtering on them via `provenance_classes` is safe and well-defined
/// (it will match zero hits against the current index) but the MCP tool
/// descriptor only advertises `document_ingested` and `user_authored` to avoid
/// advertising classes the vector/lexical path cannot deliver.  This variant
/// is kept in the Rust enum so the type system is forward-compatible and
/// future document kinds can be stamped without an API break.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustClass {
    /// External-source ingested document (provider data, news articles, manual
    /// knowledge entries).  Highest trust: content is externally authored and
    /// human-selected for ingestion.
    ///
    /// Produced by `document_payload` for all `EntityKind`s except `Finding`.
    DocumentIngested,
    /// A fact derived by an inference/tag rule engine.
    ///
    /// **Reserved / future**: rule-derived facts currently write to the graph
    /// as edges, not as retrievable documents.  This class will match zero
    /// hits against the current index.  The MCP `tdw.kg.search` tool does not
    /// advertise this variant in its schema enum today.
    RuleDerived,
    /// An agent proposal that passed the eval gate (`Provenance::Agent { gated:
    /// true }`).
    ///
    /// **Reserved / future**: eval-gated proposals currently write to the graph,
    /// not to the doc index.  This class will match zero hits against the
    /// current index.  The MCP `tdw.kg.search` tool does not advertise this
    /// variant in its schema enum today.
    AgentProposed,
    /// A user-authored finding (`Provenance::Agent { gated: false }`,
    /// `EntityKind::Finding`).  Personal analyst research notes; high personal
    /// relevance, not operator-reviewed.
    ///
    /// Produced by `document_payload` when `entity.kind == EntityKind::Finding`.
    UserAuthored,
}

impl TrustClass {
    /// The `provenance_class` payload token for this class.
    #[must_use]
    pub const fn payload_token(self) -> &'static str {
        match self {
            Self::DocumentIngested => "document_ingested",
            Self::RuleDerived => "rule_derived",
            Self::AgentProposed => "agent_proposed",
            Self::UserAuthored => "user_authored",
        }
    }

    /// Recover a [`TrustClass`] from a raw payload token.  Returns `None` for
    /// unknown tokens so old index points without the field can be handled
    /// gracefully by the caller.
    #[must_use]
    pub fn from_payload_token(token: &str) -> Option<Self> {
        match token {
            "document_ingested" => Some(Self::DocumentIngested),
            "rule_derived" => Some(Self::RuleDerived),
            "agent_proposed" => Some(Self::AgentProposed),
            "user_authored" => Some(Self::UserAuthored),
            _ => None,
        }
    }
}

/// Which channel produced a piece of evidence.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Channel {
    Vector,
    Lexical,
    Tag,
    Graph,
}

/// Why a hit surfaced on one channel.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChannelEvidence {
    pub channel: Channel,
    /// 1-based rank within that channel.
    pub rank: usize,
    /// The channel's own score (scales are channel-specific).
    pub raw_score: f32,
}

/// One hop of the graph path that reached an expanded hit.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PathStep {
    pub from: String,
    pub edge_type: String,
    pub to: String,
}

/// The full why of a hit.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct HitExplanation {
    /// Per-channel evidence (empty for purely graph-expanded hits).
    pub channels: Vec<ChannelEvidence>,
    /// Query tags (after subsumption expansion) the hit actually carries.
    pub matched_tags: Vec<String>,
    /// For graph-expanded hits: the path from the seed entity.
    pub graph_path: Option<Vec<PathStep>>,
    /// For graph-expanded hits: the fused hit that seeded the expansion.
    pub seed_hit: Option<String>,
}

/// One explained retrieval result.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RetrievedHit {
    pub id: String,
    pub entity_id: String,
    pub score: f64,
    pub tags: Vec<String>,
    /// The trust class this hit qualified under (K-X3 explainability).
    ///
    /// `None` only for graph-expanded hits that arrive without a stamped
    /// payload (stub nodes, expansion neighbours).  All directly retrieved
    /// hits carry a class so callers see WHY a hit qualified.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trust_class: Option<TrustClass>,
    pub explanation: HitExplanation,
    /// Per-fact confidence score (K-R6).  `None` when the graph engine is not
    /// attached or the entity has no graph edges (e.g. pure-vector hits without
    /// graph expansion).  When present, `score` already incorporates the
    /// confidence weight via [`ConfidenceRankingWeight::blend`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<ConfidenceScore>,
}

/// Structured filters applied across every channel.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryFilter {
    /// Match any of these tags — each expanded with its taxonomy descendants
    /// when a tag engine is attached.
    #[serde(default)]
    pub tags_any: Vec<String>,
    /// Restrict to these entity kinds.
    #[serde(default)]
    pub entity_kinds: Option<Vec<EntityKind>>,
    /// Restrict to one plane (`agent` / `platform` / `shared`).
    #[serde(default)]
    pub plane: Option<String>,
    /// Temporal point of view, tag-date form (`YYYY-MM-DD`). Excludes undated
    /// documents and gates the tag channel.
    #[serde(default)]
    pub as_of: Option<String>,
    /// Trust-dial: restrict hits to these provenance classes (K-X3).
    ///
    /// `None` / empty = all classes (the pre-K-X3 default — no existing
    /// behaviour changes).  Supplying one or more classes filters to the
    /// union.  Old index points that lack the `provenance_class` payload field
    /// are treated as [`TrustClass::DocumentIngested`] (the most trusted
    /// class, so a `DocumentIngested`-only query against a pre-stamp index
    /// still returns ingested content rather than excluding it silently).
    ///
    /// Reusable by `tdw.kg.answer` (K-M3) when that tool lands — the shared
    /// filter contract is defined here once.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance_classes: Option<Vec<TrustClass>>,
}

/// Graph-expansion settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GraphExpansion {
    /// Hop budget, `1..=3`.
    pub k_hop: u8,
    /// Relationship types to follow; empty = all.
    #[serde(default)]
    pub edge_types: Vec<String>,
    /// Neighbors considered per seed per hop.
    pub per_hit_limit: usize,
    /// Score multiplier per hop, `(0, 1]`.
    pub decay: f64,
}

/// A validated hybrid query.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KnowledgeQuery {
    pub text: String,
    /// Final fused result size.
    pub top_k: usize,
    /// Per-channel fan-out depth.
    pub channel_top_k: usize,
    #[serde(default)]
    pub filter: QueryFilter,
    #[serde(default)]
    pub expand: Option<GraphExpansion>,
    /// Confidence ranking weight (K-R6).
    ///
    /// Blends per-fact confidence into the final RRF score:
    /// `final_score = rrf × (1 - w) + confidence × w`.
    ///
    /// Default: [`ConfidenceRankingWeight::default`] (low, so relevance dominates).
    /// Set to [`ConfidenceRankingWeight::OFF`] to disable.
    /// Requires the graph engine to be attached; absent graph → weight effectively 0.
    #[serde(default)]
    pub confidence_weight: ConfidenceRankingWeight,
}

impl KnowledgeQuery {
    /// A validated query; `channel_top_k` defaults to `4 * top_k`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidQuery`] for an empty text, a zero `top_k`, an
    /// out-of-range expansion, or a malformed `as_of` date.
    pub fn try_new(
        text: impl Into<String>,
        top_k: usize,
        filter: QueryFilter,
        expand: Option<GraphExpansion>,
    ) -> Result<Self> {
        let text = text.into();
        if text.trim().is_empty() {
            return Err(Error::InvalidQuery("query text must not be empty".into()));
        }
        if top_k == 0 {
            return Err(Error::InvalidQuery(
                "top_k must be greater than zero".into(),
            ));
        }
        if top_k > MAX_TOP_K {
            return Err(Error::InvalidQuery(format!(
                "top_k must be at most {MAX_TOP_K}, got {top_k}"
            )));
        }
        if let Some(as_of) = &filter.as_of
            && !is_date(as_of)
        {
            return Err(Error::InvalidQuery(format!(
                "as_of must be YYYY-MM-DD, got {as_of:?}"
            )));
        }
        if let Some(expansion) = &expand {
            if !(1..=3).contains(&expansion.k_hop) {
                return Err(Error::InvalidQuery("k_hop must be 1..=3".into()));
            }
            if expansion.per_hit_limit == 0 {
                return Err(Error::InvalidQuery("per_hit_limit must be > 0".into()));
            }
            if expansion.per_hit_limit > MAX_PER_HIT_LIMIT {
                return Err(Error::InvalidQuery(format!(
                    "per_hit_limit must be at most {MAX_PER_HIT_LIMIT}, got {}",
                    expansion.per_hit_limit
                )));
            }
            if !(expansion.decay > 0.0 && expansion.decay <= 1.0) {
                return Err(Error::InvalidQuery("decay must be in (0, 1]".into()));
            }
        }
        Ok(Self {
            text,
            top_k,
            channel_top_k: top_k * 4,
            filter,
            expand,
            confidence_weight: ConfidenceRankingWeight::default(),
        })
    }

    /// Override the confidence ranking weight (K-R6).
    ///
    /// The weight is clamped to `[0.0, 1.0]`.
    #[must_use]
    pub const fn with_confidence_weight(mut self, weight: ConfidenceRankingWeight) -> Self {
        self.confidence_weight = weight.clamped();
        self
    }
}

fn is_date(value: &str) -> bool {
    value.len() == 10
        && value.as_bytes()[4] == b'-'
        && value.as_bytes()[7] == b'-'
        && value
            .chars()
            .enumerate()
            .all(|(index, character)| matches!(index, 4 | 7) || character.is_ascii_digit())
}

/// Per-document metadata gathered while the channels run.
#[derive(Clone, Debug, Default)]
struct DocMeta {
    entity_id: String,
    tags: Vec<String>,
    /// The `provenance_class` payload token from the index, if present.
    provenance_class_token: Option<String>,
}

/// One expansion seed: a fused hit whose entity anchors the BFS.
struct Seed {
    doc: String,
    entity: String,
    score: f64,
}

/// Ranked lists plus per-document metadata from one channel fan-out.
/// `channels[i]` was produced by the channel `kinds[i]`.
#[derive(Debug, Default)]
struct ChannelRun {
    channels: Vec<Vec<RankedEntry>>,
    kinds: Vec<Channel>,
    meta: BTreeMap<String, DocMeta>,
}

/// The hybrid retriever. Channels beyond the vector one are attached with the
/// builder methods; a bare `new` behaves exactly like vector-only search.
pub struct Retriever {
    embedder: Arc<dyn EmbeddingProvider>,
    vectors: Arc<dyn VectorEngine>,
    collection: String,
    lexical: Option<(Arc<dyn LexicalEngine>, String)>,
    tags: Option<Arc<dyn TagEngine>>,
    graph: Option<Arc<dyn GraphEngine>>,
    /// Hot-applied RRF fusion constant `k` (knowledge-system K-R3).
    ///
    /// `None` ⇒ the compile-time default [`RRF_K`].  When a self-tuning handle
    /// is attached via [`Retriever::with_rrf_k_handle`], `search` reads it on
    /// every query so an accepted tune takes effect on the next search.
    rrf_k: Option<RrfKHandle>,
}

impl Retriever {
    /// Vector-only retriever over an embedder + vector engine + collection.
    #[must_use]
    pub fn new(
        embedder: Arc<dyn EmbeddingProvider>,
        vectors: Arc<dyn VectorEngine>,
        collection: impl Into<String>,
    ) -> Self {
        Self {
            embedder,
            vectors,
            collection: collection.into(),
            lexical: None,
            tags: None,
            graph: None,
            rrf_k: None,
        }
    }

    /// Attach a hot-applied RRF fusion-constant handle (knowledge-system K-R3).
    ///
    /// Once attached, every `search` reads `handle.load()` to obtain the live
    /// `k`, so the self-tuning worker can change fusion behavior at runtime by
    /// writing through the shared [`RrfKHandle`] — no retriever rebuild.  The
    /// handle's value is clamped at the tuner source, so `search` applies it
    /// verbatim.
    #[must_use]
    pub fn with_rrf_k_handle(mut self, handle: RrfKHandle) -> Self {
        self.rrf_k = Some(handle);
        self
    }

    /// The effective RRF `k` for the next fusion: the live handle value when a
    /// self-tuning handle is attached, else the compile-time [`RRF_K`].
    #[must_use]
    fn effective_rrf_k(&self) -> f64 {
        self.rrf_k.as_ref().map_or(RRF_K, RrfKHandle::load)
    }

    /// Attach the lexical channel.
    #[must_use]
    pub fn with_lexical(
        mut self,
        engine: Arc<dyn LexicalEngine>,
        index: impl Into<String>,
    ) -> Self {
        self.lexical = Some((engine, index.into()));
        self
    }

    /// Attach the tag engine (enables subsumption expansion + the tag channel).
    #[must_use]
    pub fn with_tags(mut self, engine: Arc<dyn TagEngine>) -> Self {
        self.tags = Some(engine);
        self
    }

    /// Attach the graph engine (enables the tag channel's entity→document
    /// resolution and graph expansion).
    #[must_use]
    pub fn with_graph(mut self, engine: Arc<dyn GraphEngine>) -> Self {
        self.graph = Some(engine);
        self
    }

    /// Run the hybrid search.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidQuery`] for a malformed query, or the failing
    /// engine's error — channel failures are loud, never silently skipped.
    pub async fn search(&self, query: &KnowledgeQuery) -> Result<Vec<RetrievedHit>> {
        // 1. Taxonomy subsumption: a query tag also matches its descendants.
        let mut expanded_tags = query.filter.tags_any.clone();
        if let Some(tags) = &self.tags {
            for tag in &query.filter.tags_any {
                expanded_tags.extend(
                    tags.descendants(tag)
                        .await
                        .map_err(|error| Error::Storage(error.to_string()))?,
                );
            }
        }
        expanded_tags.sort();
        expanded_tags.dedup();

        // 2. Channel fan-out, 3. fuse, 4. explained hits (trust-dial applied).
        let run = self.run_channels(query, &expanded_tags).await?;
        let fused = rrf_fuse(&run.channels, self.effective_rrf_k());
        let mut hits =
            assemble_hits(&fused, &run, &expanded_tags, self.graph.as_deref(), query).await?;

        // 5. Graph expansion of the top fused hits.
        if let (Some(expansion), Some(graph)) = (&query.expand, &self.graph) {
            let seeds: Vec<Seed> = fused
                .iter()
                .take(query.top_k.min(8))
                .filter_map(|candidate| {
                    run.meta.get(&candidate.id).map(|doc_meta| Seed {
                        doc: candidate.id.clone(),
                        entity: doc_meta.entity_id.clone(),
                        score: candidate.score,
                    })
                })
                .collect();
            for seed in seeds {
                if seed.entity.is_empty() {
                    continue;
                }
                self.expand_seed(graph.as_ref(), expansion, &query.filter, &seed, &mut hits)
                    .await?;
            }
            // Trust-dial gate on expanded hits (K-X3 fix): graph-expanded
            // neighbours bypass `assemble_hits` and are inserted directly.
            // Apply the same provenance-class gate here so a restrictive filter
            // cannot be circumvented by expansion.  Expanded stubs carry no
            // payload round-trip (trust_class: None) → effective class is
            // `DocumentIngested` (the conservative default).
            if query.filter.provenance_classes.is_some() {
                let allowed = query.filter.provenance_classes.as_deref();
                hits.retain(|_, hit| {
                    let class = hit.trust_class.unwrap_or(TrustClass::DocumentIngested);
                    trust_class_passes(class, allowed)
                });
            }
        }

        // 6. Final order: score desc, id asc; truncate.
        let mut result: Vec<RetrievedHit> = hits.into_values().collect();
        result.sort_by(|left, right| {
            right
                .score
                .partial_cmp(&left.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| left.id.cmp(&right.id))
        });
        result.truncate(query.top_k);
        Ok(result)
    }

    /// Fan the query out over the attached channels, collecting per-document
    /// metadata as the ranked lists stream by.
    async fn run_channels(
        &self,
        query: &KnowledgeQuery,
        expanded_tags: &[String],
    ) -> Result<ChannelRun> {
        let payload_filter = build_payload_filter(&query.filter, expanded_tags);
        let mut run = ChannelRun::default();

        // Vector channel.
        let embedding = self
            .embedder
            .embed(&query.text)
            .await
            .map_err(|error| Error::Provider(error.to_string()))?;
        let knn = self
            .vectors
            .search_knn(
                &self.collection,
                VectorQuery {
                    vector: embedding.vector,
                    top_k: query.channel_top_k,
                    filter: payload_filter.clone(),
                },
            )
            .await?;
        run.channels.push(
            knn.iter()
                .map(|hit| {
                    record_meta(&mut run.meta, &hit.id, &hit.payload);
                    RankedEntry {
                        id: hit.id.clone(),
                        raw_score: hit.score,
                    }
                })
                .collect(),
        );
        run.kinds.push(Channel::Vector);

        // Lexical channel.
        if let Some((lexical, index)) = &self.lexical {
            let docs = lexical
                .search_text(
                    index,
                    TextQuery {
                        text: query.text.clone(),
                        top_k: query.channel_top_k,
                        filter: payload_filter,
                    },
                )
                .await?;
            run.channels.push(
                docs.iter()
                    .map(|doc| {
                        record_meta(&mut run.meta, &doc.id, &doc.fields);
                        RankedEntry {
                            id: doc.id.clone(),
                            raw_score: doc.score,
                        }
                    })
                    .collect(),
            );
            run.kinds.push(Channel::Lexical);
        }

        // Tag channel: entities holding any expanded tag at as_of, mapped to
        // their documents through the graph. Runs only when temporal (as_of)
        // — tag activity is a dated property.
        if let (Some(tags), Some(graph), Some(as_of)) =
            (&self.tags, &self.graph, &query.filter.as_of)
            && !expanded_tags.is_empty()
        {
            let mut entries = Vec::new();
            for tag in expanded_tags {
                for entity in tags
                    .entities_with_tag(tag, as_of)
                    .await
                    .map_err(|error| Error::Storage(error.to_string()))?
                {
                    // The payload contract's entity_kind condition, applied
                    // via the entity's graph node.
                    if let Some(kinds) = &query.filter.entity_kinds {
                        let Some(entity_node) = graph.node(&entity).await? else {
                            continue;
                        };
                        if !kinds.contains(&entity_node.kind) {
                            continue;
                        }
                    }
                    for (doc_id, class_token) in
                        entity_documents(graph.as_ref(), &entity, &query.filter).await?
                    {
                        run.meta.entry(doc_id.clone()).or_insert_with(|| DocMeta {
                            entity_id: entity.clone(),
                            tags: vec![tag.clone()],
                            // K-X3: class_token is read from the graph document
                            // node's props.provenance_class (stamped at index time).
                            // Pre-stamp nodes carry None → effective DocumentIngested.
                            provenance_class_token: class_token,
                        });
                        entries.push(RankedEntry {
                            id: doc_id,
                            raw_score: 1.0,
                        });
                    }
                }
                if entries.len() >= query.channel_top_k {
                    break;
                }
            }
            entries.truncate(query.channel_top_k);
            run.channels.push(entries);
            run.kinds.push(Channel::Tag);
        }
        Ok(run)
    }

    /// BFS from one seed entity, contributing decayed scores to the documents
    /// of reached neighbors, with the reaching path recorded for explanation.
    async fn expand_seed(
        &self,
        graph: &dyn GraphEngine,
        expansion: &GraphExpansion,
        filter: &QueryFilter,
        seed: &Seed,
        hits: &mut BTreeMap<String, RetrievedHit>,
    ) -> Result<()> {
        let traversal = TraversalFilter {
            rels: (!expansion.edge_types.is_empty()).then(|| expansion.edge_types.clone()),
            kinds: filter.entity_kinds.clone(),
            as_of: filter.as_of.as_deref().map(date_to_timestamp),
            direction: Direction::Both,
            max_hops: 1,
        };
        let mut visited = std::collections::BTreeSet::from([seed.entity.clone()]);
        let mut frontier: Vec<(String, Vec<PathStep>)> = vec![(seed.entity.clone(), Vec::new())];
        for hop in 1..=expansion.k_hop {
            let mut next = Vec::new();
            for (anchor, path) in &frontier {
                let mut reached = graph.neighbors(anchor, &traversal).await?;
                reached.truncate(expansion.per_hit_limit);
                for (edge, node) in reached {
                    if !visited.insert(node.id.clone()) {
                        continue;
                    }
                    let mut next_path = path.clone();
                    next_path.push(PathStep {
                        from: edge.from.clone(),
                        edge_type: edge.rel.clone(),
                        to: edge.to.clone(),
                    });
                    let contribution = seed.score * expansion.decay.powi(i32::from(hop));
                    for (doc_id, class_token) in entity_documents(graph, &node.id, filter).await? {
                        if doc_id == seed.doc {
                            continue;
                        }
                        // K-X3: resolve trust_class from the graph document node's
                        // props.provenance_class (stamped at index time).  Pre-stamp
                        // nodes carry None → effective DocumentIngested in the
                        // post-expansion retain pass.
                        let trust_class = class_token
                            .as_deref()
                            .and_then(TrustClass::from_payload_token);
                        let entry = hits.entry(doc_id.clone()).or_insert_with(|| RetrievedHit {
                            id: doc_id.clone(),
                            entity_id: node.id.clone(),
                            score: 0.0,
                            tags: Vec::new(),
                            trust_class,
                            explanation: HitExplanation::default(),
                            // Graph-expanded hits do not carry confidence (no direct
                            // entity→document ingest edge to score); the seed hit's
                            // confidence already reflects the anchor fact's trust.
                            confidence: None,
                        });
                        entry.score += contribution;
                        if entry.explanation.graph_path.is_none() {
                            entry.explanation.graph_path = Some(next_path.clone());
                            entry.explanation.seed_hit = Some(seed.doc.clone());
                        }
                    }
                    next.push((node.id, next_path));
                }
            }
            frontier = next;
            if frontier.is_empty() {
                break;
            }
        }
        Ok(())
    }
}

/// Turn fused candidates into explained hits keyed by document id.
///
/// Applies the K-X3 trust-dial gate: hits whose effective trust class is not
/// in `filter.provenance_classes` are silently excluded here (they never
/// reach the caller's ranked list).  Each passing hit carries its resolved
/// [`TrustClass`] in `trust_class` for explainability.
///
/// When a graph engine is attached and `query.confidence_weight` is nonzero,
/// each hit's RRF score is blended with its per-fact confidence (K-R6).  The
/// confidence is computed from the `described_by` edges of the hit's entity,
/// using all edges of that relation type as the corroboration pool (bounded by
/// [`tdw_core::MAX_CORROBORATION_CAP`]).
async fn assemble_hits(
    fused: &[Fused],
    run: &ChannelRun,
    expanded_tags: &[String],
    graph: Option<&dyn GraphEngine>,
    query: &KnowledgeQuery,
) -> Result<BTreeMap<String, RetrievedHit>> {
    let mut hits = BTreeMap::new();
    for candidate in fused {
        let doc_meta = run.meta.get(&candidate.id).cloned().unwrap_or_default();
        let trust_class = effective_trust_class(doc_meta.provenance_class_token.as_deref());
        if !trust_class_passes(trust_class, query.filter.provenance_classes.as_deref()) {
            continue;
        }
        let matched_tags: Vec<String> = doc_meta
            .tags
            .iter()
            .filter(|tag| expanded_tags.contains(tag))
            .cloned()
            .collect();

        // K-R6: compute confidence when graph is attached and weight is nonzero.
        let (confidence, final_score) = compute_hit_confidence(
            graph,
            query,
            &doc_meta.entity_id,
            &candidate.id,
            candidate.score,
        )
        .await?;

        hits.insert(
            candidate.id.clone(),
            RetrievedHit {
                id: candidate.id.clone(),
                entity_id: doc_meta.entity_id.clone(),
                score: final_score,
                tags: doc_meta.tags.clone(),
                trust_class: Some(trust_class),
                explanation: HitExplanation {
                    channels: candidate
                        .contributions
                        .iter()
                        .map(|&(channel_index, rank, raw_score)| ChannelEvidence {
                            channel: run.kinds[channel_index],
                            rank,
                            raw_score,
                        })
                        .collect(),
                    matched_tags,
                    graph_path: None,
                    seed_hit: None,
                },
                confidence,
            },
        );
    }
    Ok(hits)
}

/// Compute the confidence score for one hit and return `(score, blended_rrf_score)`.
///
/// When the graph is absent or the entity id is empty or the weight is zero,
/// returns `(None, rrf_score)` — confidence does not affect ranking.
///
/// The corroboration pool is the slice of `described_by` neighbors of the
/// entity (outgoing edges).  The **subject** edge is the one whose `to` target
/// matches `doc_node_id` — the document node for this specific hit.  Using any
/// other edge (e.g. the first neighbor) would score the wrong `(entity, doc)`
/// pair for multi-document entities.
async fn compute_hit_confidence(
    graph: Option<&dyn GraphEngine>,
    query: &KnowledgeQuery,
    entity_id: &str,
    doc_id: &str,
    rrf_score: f64,
) -> Result<(Option<ConfidenceScore>, f64)> {
    let weight = query.confidence_weight.0;
    let Some(graph) = graph else {
        return Ok((None, rrf_score));
    };
    if weight <= 0.0 || entity_id.is_empty() {
        return Ok((None, rrf_score));
    }

    // Fetch outgoing `described_by` edges of the entity — these are the
    // ingestion provenance edges used as the corroboration pool.
    let traversal = TraversalFilter {
        rels: Some(vec![DESCRIBED_BY.to_string()]),
        direction: Direction::Out,
        max_hops: 1,
        ..TraversalFilter::default()
    };
    let neighbors = graph.neighbors(entity_id, &traversal).await?;
    if neighbors.is_empty() {
        return Ok((None, rrf_score));
    }

    // The document node id is "document:<doc_id>" (the convention used by the
    // ingest pipeline).  Match the specific edge whose target is this hit's
    // document — a multi-document entity must not have its doc-x hit scored
    // with doc-a's confidence.
    let doc_node_id = format!("document:{doc_id}");
    // Match ONLY the edge whose target is this hit's document. There is no
    // fallback to an arbitrary neighbor: for a multi-document entity that would
    // score this hit with a *different* document's provenance — the wrong
    // (entity, doc) pair the doc-comment above forbids. If no `described_by`
    // edge targets this document, confidence is unknown and must not affect
    // ranking (return the raw RRF score, as the other no-confidence branches do).
    let subject_edge = neighbors.iter().map(|(e, _)| e).find(|e| e.to == doc_node_id);
    let Some(subject_edge) = subject_edge else {
        return Ok((None, rrf_score));
    };

    let pool: Vec<&tdw_core::GraphEdge> = neighbors.iter().map(|(e, _)| e).collect();
    let subject = EdgeConfidenceInput::from_edge(subject_edge);

    // K-R3 seam: no source_reliability yet → None (→ NEUTRAL).
    let score = compute_confidence(&subject, &pool, None);
    let blended = query.confidence_weight.blend(rrf_score, score.confidence);
    Ok((Some(score), blended))
}

#[cfg(test)]
mod confidence_subject_edge_tests {
    use super::*;
    use serde_json::json;
    use tdw_core::{GraphEdge, GraphNode, Provenance};
    use tdw_storage_graph::InMemoryGraphEngine;

    fn node(id: &str) -> GraphNode {
        GraphNode {
            id: id.to_string(),
            kind: EntityKind::Instrument,
            label: id.to_string(),
            aliases: Vec::new(),
            props: json!({}),
            valid_from: None,
            valid_to: None,
        }
    }

    // A multi-document entity that is `described_by` document:A but NOT
    // document:B. A hit for document:B must not borrow document:A's provenance:
    // with no subject edge for this hit's document, confidence is None and the
    // ranking score is the raw RRF score (the documented contract). The removed
    // `.or_else(neighbors.first())` fallback used to score B's hit with A's edge.
    #[tokio::test]
    async fn confidence_none_when_no_described_by_edge_targets_this_document() {
        let graph = InMemoryGraphEngine::default();
        graph
            .upsert_nodes(vec![node("entity:E"), node("document:A"), node("document:B")])
            .await
            .expect("nodes");
        graph
            .upsert_edges(vec![GraphEdge {
                from: "entity:E".to_string(),
                to: "document:A".to_string(),
                rel: DESCRIBED_BY.to_string(),
                props: json!({}),
                provenance: Provenance::Ingest {
                    source: "test".to_string(),
                },
                valid_from: None,
                valid_to: None,
            }])
            .await
            .expect("edge");

        let query = KnowledgeQuery::try_new("q", 10, QueryFilter::default(), None)
            .expect("query")
            .with_confidence_weight(ConfidenceRankingWeight(0.5));

        let graph_dyn: &dyn GraphEngine = &graph;
        let (score, blended) =
            compute_hit_confidence(Some(graph_dyn), &query, "entity:E", "B", 0.5)
                .await
                .expect("confidence");

        assert!(
            score.is_none(),
            "no described_by edge to this hit's document → confidence must be None, \
             never scored from a different document's edge"
        );
        assert!(
            (blended - 0.5).abs() < f64::EPSILON,
            "ranking score must be the raw rrf_score when no subject edge matches"
        );
    }

    // Sanity: when the subject edge for the hit's document IS present, confidence
    // is computed (guards against the fix over-suppressing the normal path).
    #[tokio::test]
    async fn confidence_present_when_described_by_edge_targets_this_document() {
        let graph = InMemoryGraphEngine::default();
        graph
            .upsert_nodes(vec![node("entity:E"), node("document:B")])
            .await
            .expect("nodes");
        graph
            .upsert_edges(vec![GraphEdge {
                from: "entity:E".to_string(),
                to: "document:B".to_string(),
                rel: DESCRIBED_BY.to_string(),
                props: json!({}),
                provenance: Provenance::Ingest {
                    source: "test".to_string(),
                },
                valid_from: None,
                valid_to: None,
            }])
            .await
            .expect("edge");

        let query = KnowledgeQuery::try_new("q", 10, QueryFilter::default(), None)
            .expect("query")
            .with_confidence_weight(ConfidenceRankingWeight(0.5));

        let graph_dyn: &dyn GraphEngine = &graph;
        let (score, _) = compute_hit_confidence(Some(graph_dyn), &query, "entity:E", "B", 0.5)
            .await
            .expect("confidence");

        assert!(
            score.is_some(),
            "a described_by edge to this hit's document must yield a confidence score"
        );
    }
}

/// Documents described by an entity (via `described_by` edges to
/// `document:<id>` nodes) that are VISIBLE under the query filter.
///
/// Returns `(doc_id, provenance_class_token)` pairs.  The class token is read
/// from the document node's `props.provenance_class` field, which is stamped at
/// index time by `write_durable_graph` (K-X3).  Absent tokens occur only on
/// pre-stamp nodes and default to [`TrustClass::DocumentIngested`] in callers.
///
/// Graph-derived documents must satisfy the same contract the payload filter
/// enforces on the vector/lexical channels — otherwise the tag channel and
/// graph expansion would be the leak around the `as_of` gate.
async fn entity_documents(
    graph: &dyn GraphEngine,
    entity_id: &str,
    filter: &QueryFilter,
) -> Result<Vec<(String, Option<String>)>> {
    let traversal = TraversalFilter {
        rels: Some(vec![DESCRIBED_BY.to_string()]),
        direction: Direction::Out,
        max_hops: 1,
        ..TraversalFilter::default()
    };
    Ok(graph
        .neighbors(entity_id, &traversal)
        .await?
        .into_iter()
        .filter(|(_, node)| document_visible(&node.props, filter))
        .filter_map(|(_, node)| {
            let doc_id = node
                .id
                .strip_prefix(DOCUMENT_PREFIX)
                .map(ToOwned::to_owned)?;
            let class_token = node
                .props
                .get("provenance_class")
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned);
            Some((doc_id, class_token))
        })
        .collect())
}

/// Mirror of the payload contract for document GRAPH nodes (`props.as_of`,
/// `props.plane`): on a temporal query a document without `as_of` is
/// excluded, and a missing `plane` fails a plane-scoped query — graph-derived
/// hits are never more permissive than payload-filtered ones.
fn document_visible(props: &serde_json::Value, filter: &QueryFilter) -> bool {
    if let Some(as_of) = &filter.as_of {
        let cutoff = date_to_timestamp(as_of);
        match props.get("as_of").and_then(serde_json::Value::as_str) {
            Some(doc_as_of) if doc_as_of <= cutoff.as_str() => {}
            _ => return false,
        }
    }
    if let Some(plane) = &filter.plane {
        match props.get("plane").and_then(serde_json::Value::as_str) {
            Some(doc_plane) if doc_plane == plane => {}
            _ => return false,
        }
    }
    true
}

/// Build the cross-channel payload filter from the query filter.
///
/// Contract payload keys: `tags` (array), `entity_kind`, `plane`, `as_of`
/// (normalized timestamp — undated documents are EXCLUDED by temporal
/// queries), and `provenance_class` (K-X3 trust-dial stamp).
///
/// ## Provenance-class pre-filter (K-X3, fix #4)
///
/// When a restrictive class set is requested AND it does **not** include
/// `DocumentIngested`, a `MatchAny` condition on `provenance_class` is added
/// to the payload filter.  This is filter-before-scoring: only stamped index
/// points with a matching token reach the scoring/fuse stage, which keeps
/// `top_k` semantics honest (the result set is drawn from the matching
/// population, not post-truncated from a larger unfiltered pool).
///
/// When `DocumentIngested` IS in the requested set we intentionally skip the
/// payload condition: old index points that pre-date the stamp have no
/// `provenance_class` field and would be wrongly excluded by a `MatchAny`
/// condition (the in-memory `PayloadFilter` treats a missing key as
/// non-matching).  Their effective class is `DocumentIngested` by the
/// backward-compat rule, so they must pass a filter that includes that class.
/// The post-assembly gate in [`assemble_hits`] handles them correctly.
///
/// Summary:
/// - classes not including `DocumentIngested` → `MatchAny` pushed → efficient
/// - classes including `DocumentIngested` → no `MatchAny` → post-gate only
/// - no filter (default) → no `MatchAny` → unfiltered (pre-K-X3 behaviour)
fn build_payload_filter(filter: &QueryFilter, expanded_tags: &[String]) -> PayloadFilter {
    let mut must = Vec::new();
    if !expanded_tags.is_empty() {
        must.push(PayloadCondition::MatchAny {
            key: "tags".to_string(),
            values: expanded_tags.to_vec(),
        });
    }
    if let Some(kinds) = &filter.entity_kinds {
        must.push(PayloadCondition::MatchAny {
            key: "entity_kind".to_string(),
            values: kinds
                .iter()
                .filter_map(|kind| {
                    serde_json::to_value(kind)
                        .ok()
                        .and_then(|value| value.as_str().map(ToOwned::to_owned))
                })
                .collect(),
        });
    }
    if let Some(plane) = &filter.plane {
        must.push(PayloadCondition::MatchString {
            key: "plane".to_string(),
            value: plane.clone(),
        });
    }
    if let Some(as_of) = &filter.as_of {
        must.push(PayloadCondition::RangeString {
            key: "as_of".to_string(),
            gte: None,
            lte: Some(date_to_timestamp(as_of)),
        });
    }
    // Provenance-class pre-filter: push only when DocumentIngested is NOT in
    // the requested set (see doc-comment above for the reasoning).
    if let Some(classes) = &filter.provenance_classes
        && !classes.is_empty()
        && !classes.contains(&TrustClass::DocumentIngested)
    {
        must.push(PayloadCondition::MatchAny {
            key: "provenance_class".to_string(),
            values: classes
                .iter()
                .map(|c| c.payload_token().to_string())
                .collect(),
        });
    }
    PayloadFilter { must }
}

/// Resolve the effective [`TrustClass`] for a hit given its raw payload token.
///
/// When the `provenance_class` field is absent (old-index point) we conservatively
/// default to [`TrustClass::DocumentIngested`] so a `DocumentIngested`-only
/// query against a pre-stamp index still returns ingested content rather than
/// silently excluding it.
fn effective_trust_class(provenance_class_token: Option<&str>) -> TrustClass {
    provenance_class_token
        .and_then(TrustClass::from_payload_token)
        .unwrap_or(TrustClass::DocumentIngested)
}

/// Whether a hit's trust class passes the requested filter.
///
/// `None` / empty class set = all classes pass (the pre-K-X3 default).
fn trust_class_passes(class: TrustClass, allowed: Option<&[TrustClass]>) -> bool {
    allowed.is_none_or(|classes| classes.is_empty() || classes.contains(&class))
}

fn record_meta(meta: &mut BTreeMap<String, DocMeta>, id: &str, payload: &serde_json::Value) {
    let entity_id = payload
        .get("entity_id")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string();
    let tags = payload
        .get("tags")
        .and_then(serde_json::Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(|value| value.as_str().map(ToOwned::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let provenance_class_token = payload
        .get("provenance_class")
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned);
    meta.entry(id.to_string()).or_insert(DocMeta {
        entity_id,
        tags,
        provenance_class_token,
    });
}
