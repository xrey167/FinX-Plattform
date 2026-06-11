//! The knowledge runtime the MCP server hosts (knowledge-system B8).
//!
//! [`KnowledgeRuntime`] bundles the hybrid [`Retriever`] with direct handles
//! to the graph and tag engines (for entity/traverse/path/tag tools that
//! bypass retrieval) plus the version triple every search response reports,
//! so agents can attribute result drift to the exact rule-set/infer/embedder
//! versions that produced it. B9 adds the gated write side (proposals) to
//! this same seam.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tdw_core::{GraphEngine, LexicalEngine, VectorEngine};
use tdw_embed::EmbeddingProvider;
use tdw_retrieve::Retriever;
use tdw_tags::TagEngine;

/// The versions stamped onto every `tdw.kg.search` response.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnowledgeVersions {
    /// The embedder's `model_id`.
    pub embedder_model: String,
    /// The tag-rule set version, when auto-tagging is wired.
    #[serde(default)]
    pub rules_version: Option<u64>,
    /// The inference rule-set version, when `tdw-infer` is wired.
    #[serde(default)]
    pub infer_version: Option<u64>,
}

/// Everything the MCP knowledge tools need, bundled behind one optional
/// runtime on the server.
///
/// Channels are optional exactly like the [`Retriever`]'s — a vector-only
/// runtime serves `tdw.kg.search` and reports the graph/tag tools as
/// unavailable (tool errors, never protocol errors).
pub struct KnowledgeRuntime {
    retriever: Retriever,
    graph: Option<Arc<dyn GraphEngine>>,
    tags: Option<Arc<dyn TagEngine>>,
    versions: KnowledgeVersions,
}

impl KnowledgeRuntime {
    /// A vector-only runtime over an embedder + vector engine.
    #[must_use]
    pub fn new(embedder: Arc<dyn EmbeddingProvider>, vectors: Arc<dyn VectorEngine>) -> Self {
        let versions = KnowledgeVersions {
            embedder_model: embedder.model_id().to_string(),
            rules_version: None,
            infer_version: None,
        };
        let collection = crate::collection_name(embedder.model_id());
        Self {
            retriever: Retriever::new(embedder, vectors, collection),
            graph: None,
            tags: None,
            versions,
        }
    }

    /// Attach the lexical channel (hybrid search).
    #[must_use]
    pub fn with_lexical(
        mut self,
        engine: Arc<dyn LexicalEngine>,
        index_name: impl Into<String>,
    ) -> Self {
        self.retriever = self.retriever.with_lexical(engine, index_name);
        self
    }

    /// Attach the tag engine — enables `tdw.tags.query`, subsumption
    /// expansion, and the retriever's tag channel.
    #[must_use]
    pub fn with_tags(mut self, engine: Arc<dyn TagEngine>) -> Self {
        self.retriever = self.retriever.with_tags(engine.clone());
        self.tags = Some(engine);
        self
    }

    /// Attach the graph engine — enables `tdw.kg.entity` / `tdw.kg.traverse`
    /// / `tdw.kg.path` and the retriever's graph expansion.
    #[must_use]
    pub fn with_graph(mut self, engine: Arc<dyn GraphEngine>) -> Self {
        self.retriever = self.retriever.with_graph(engine.clone());
        self.graph = Some(engine);
        self
    }

    /// Stamp the rule/infer versions reported by `tdw.kg.search`.
    #[must_use]
    pub const fn with_versions(
        mut self,
        rules_version: Option<u64>,
        infer_version: Option<u64>,
    ) -> Self {
        self.versions.rules_version = rules_version;
        self.versions.infer_version = infer_version;
        self
    }

    /// The hybrid retriever.
    #[must_use]
    pub const fn retriever(&self) -> &Retriever {
        &self.retriever
    }

    /// The graph engine, when attached.
    #[must_use]
    pub const fn graph(&self) -> Option<&Arc<dyn GraphEngine>> {
        self.graph.as_ref()
    }

    /// The tag engine, when attached.
    #[must_use]
    pub const fn tags(&self) -> Option<&Arc<dyn TagEngine>> {
        self.tags.as_ref()
    }

    /// The version triple stamped onto search responses.
    #[must_use]
    pub const fn versions(&self) -> &KnowledgeVersions {
        &self.versions
    }
}

impl std::fmt::Debug for KnowledgeRuntime {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("KnowledgeRuntime")
            .field("versions", &self.versions)
            .field("graph", &self.graph.is_some())
            .field("tags", &self.tags.is_some())
            .finish_non_exhaustive()
    }
}
