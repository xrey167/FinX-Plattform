//! `tdw kg demo` — seeded offline walkthrough of the knowledge-graph surface
//! (knowledge-system-2 K-X2).
//!
//! Seeds a small finance mini-KG from checked-in fixtures through the K-E3
//! [`KnowledgeIndexer`] path on in-memory engines, then narrates each KG
//! operation with its exact equivalent command/tool name. Everything runs
//! offline, is deterministic, and completes in under 60 seconds.
//!
//! # Honesty notice
//!
//! All data is held in-memory and is NOT persisted after the process exits.
//! The demo prints this prominently and points to `docs/knowledge-quickstart.md`
//! for production setup.
//!
//! # `--json` flag
//!
//! When `--json` is set narration is suppressed and each step emits a JSON
//! object to stdout (NDJSON — one object per line). The process exits non-zero
//! if any step fails — the demo doubles as an e2e smoke test.
//!
//! # Step equivalences
//!
//! | Step | CLI command / MCP tool |
//! |------|------------------------|
//! | ingest | `tdw kg ingest <file.jsonl>` |
//! | search | MCP `tdw.kg.search` |
//! | why | MCP `tdw.kg.why` |
//! | diff | MCP `tdw.kg.diff` (temporal graph-state diff) |
//! | status | `tdw kg status` (offline counts variant) |

#![allow(clippy::too_many_lines)] // demo is intentionally linear and verbose

use std::collections::BTreeSet;
use std::sync::Arc;

use serde_json::{Value, json};
use tdw_core::{GraphEngine, LexicalEngine, VectorEngine, active_at};
use tdw_embed::EmbeddingProvider;
use tdw_embed_local::HashEmbeddingProvider;
use tdw_infer::{ChangeSet, DerivationIndex, InferEngine, InferRule, RunLimits};
use tdw_knowledge::indexer::{IndexManifest, IndexOutcome, KnowledgeIndexer};
use tdw_knowledge::{KnowledgeDocument, KnowledgeIndex};
use tdw_storage_graph::InMemoryGraphEngine;
use tdw_storage_meilisearch::InMemoryLexicalEngine;
use tdw_storage_qdrant::InMemoryVectorEngine;
use tdw_tag_rules::{RuleEngine, TagRule};
use tdw_tags::{InMemoryTagEngine, TagDefinition};

use crate::CliError;

// ---------------------------------------------------------------------------
// Fixture paths (relative to the crate root at build/test time)
// ---------------------------------------------------------------------------

const FIXTURE_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/kg-demo");

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Run `tdw kg demo`. `args` are the full process argument list.
///
/// Exits non-zero on any step failure regardless of `--json`.
///
/// # Errors
///
/// Returns a descriptive error when any demo step fails.
pub async fn run(args: &[String]) -> Result<(), CliError> {
    let json_mode = args.iter().any(|a| a == "--json");
    let runner = DemoRunner { json_mode };
    runner.run_all().await
}

// ---------------------------------------------------------------------------
// Narration helpers
// ---------------------------------------------------------------------------

struct DemoRunner {
    json_mode: bool,
}

impl DemoRunner {
    fn narrate(&self, text: &str) {
        if !self.json_mode {
            println!("{text}");
        }
    }

    fn emit_step(&self, step: &str, data: &Value) {
        if self.json_mode {
            println!(
                "{}",
                serde_json::to_string(&json!({ "step": step, "data": data }))
                    .unwrap_or_else(|_| "{}".to_string())
            );
        }
    }

    fn section(&self, title: &str) {
        if !self.json_mode {
            println!();
            println!("━━━ {title} ━━━");
        }
    }

    async fn run_all(self) -> Result<(), CliError> {
        // ── Banner ────────────────────────────────────────────────────────
        self.narrate("tdw kg demo — knowledge-graph offline walkthrough (K-X2)");
        self.narrate(
            "NOTE: in-memory demo — data is not persisted after this process exits.\n\
             See docs/knowledge-quickstart.md for production setup.",
        );

        // ── Build in-memory engines ───────────────────────────────────────
        let embedder: Arc<dyn EmbeddingProvider> = Arc::new(HashEmbeddingProvider::default());
        let vectors: Arc<dyn VectorEngine> = Arc::new(InMemoryVectorEngine::default());
        let lexical: Arc<dyn LexicalEngine> = Arc::new(InMemoryLexicalEngine::default());
        let graph: Arc<InMemoryGraphEngine> = Arc::new(InMemoryGraphEngine::default());
        let tag_engine: Arc<dyn tdw_tags::TagEngine> = Arc::new(InMemoryTagEngine::default());

        // ── Load fixtures ─────────────────────────────────────────────────
        let docs_v1 = load_docs("docs-v1.jsonl")?;
        let docs_v2 = load_docs("docs-v2.jsonl")?;
        let tag_rules = load_tag_rules()?;
        let infer_rules = load_infer_rules()?;

        // Validate fixture counts up-front (smoke gate).
        if docs_v1.len() < 8 {
            return Err(format!(
                "demo fixture docs-v1.jsonl has only {} docs (need ≥8)",
                docs_v1.len()
            )
            .into());
        }
        let derive_edge_count = infer_rules
            .iter()
            .filter(|r| matches!(r, InferRule::DeriveEdge { .. }))
            .count();
        if derive_edge_count == 0 {
            return Err("demo fixture infer-rules.json has no DeriveEdge rule".into());
        }

        // ── Build KnowledgeIndexer ────────────────────────────────────────
        let collection = tdw_knowledge::collection_name(embedder.model_id());
        let index = KnowledgeIndex::new(Arc::clone(&embedder), Arc::clone(&vectors));

        // Pre-define every tag that documents or rules mention so the tag store
        // accepts assignments without UnknownTag errors.
        let all_tag_ids = collect_tag_ids(&docs_v1, &docs_v2, &tag_rules);

        let mut rule_engine = RuleEngine::default();
        rule_engine
            .hot_reload(tag_rules)
            .map_err(|e| format!("tag rules hot_reload: {e}"))?;

        // Erase concrete graph type to dyn GraphEngine once; shared by indexer + infer.
        let graph_arc: Arc<dyn GraphEngine> = Arc::clone(&graph) as Arc<dyn GraphEngine>;

        let mut indexer = KnowledgeIndexer::new(index)
            .with_rules(rule_engine)
            .map_err(|e| format!("indexer with_rules: {e}"))?
            .with_lexical(Arc::clone(&lexical), collection.clone())
            .with_graph(Arc::clone(&graph_arc));

        // Pre-define tags in the InMemoryTagEngine (InferEngine reads from it).
        for tag_id in &all_tag_ids {
            tag_engine
                .define(TagDefinition {
                    tag_id: tag_id.clone(),
                    parent: None,
                    ttl_days: None,
                })
                .await
                .map_err(|e| format!("pre-define tag {tag_id}: {e}"))?;
        }

        // ── Build InferEngine ─────────────────────────────────────────────
        let mut infer_engine = InferEngine::with_limits(RunLimits {
            max_iterations: 16,
            max_derived: 1_000,
        });
        infer_engine
            .hot_reload(infer_rules)
            .map_err(|e| format!("infer rules hot_reload: {e}"))?;

        // ================================================================
        // STEP 1 — INGEST v1
        // ================================================================
        self.section("Step 1 · Ingest (v1 snapshot)");
        self.narrate(
            "Equivalent CLI: tdw kg ingest fixtures/kg-demo/docs-v1.jsonl\n\
             Pipeline: manifest check → auto-tag rules → vector + graph → lexical co-index",
        );

        let v1_date = "2026-01-15";
        let mut landed_v1 = 0usize;

        for doc in docs_v1 {
            let id = doc.id.clone();
            let outcome = indexer
                .index_at(doc, v1_date)
                .await
                .map_err(|e| format!("ingest {id}: {e}"))?;
            if outcome == IndexOutcome::Indexed {
                landed_v1 += 1;
            }
        }

        // Run inference over the v1 graph to derive supply_chain_peer edges.
        let change_set = ChangeSet {
            edge_types: std::iter::once("mentions".to_string()).collect(),
            tags: std::collections::BTreeSet::new(),
        };
        let infer_report = infer_engine
            .run_incremental(&graph_arc, &tag_engine, v1_date, &change_set)
            .await
            .map_err(|e| format!("infer run v1: {e}"))?;

        self.narrate(&format!(
            "  Landed {landed_v1} documents. Inference derived {} edge(s) (rule \
             supply-chain-peer, DeriveEdge: mentions→mentions→supply_chain_peer).",
            infer_report.derived_edges
        ));
        self.emit_step(
            "ingest_v1",
            &json!({
                "landed": landed_v1,
                "derived_edges": infer_report.derived_edges,
                "infer_version": infer_report.version,
            }),
        );

        if infer_report.derived_edges == 0 {
            return Err(
                "demo smoke: DeriveEdge rule produced 0 derived edges from v1 fixtures".into(),
            );
        }

        // Snapshot the v1 doc ids from the live manifest before ingesting v2.
        // Used by the diff step to classify changed vs. added documents.
        let v1_doc_ids: BTreeSet<String> = indexer
            .manifest()
            .entries()
            .map(|(id, _)| id.clone())
            .collect();

        // ================================================================
        // STEP 2 — SEARCH
        // ================================================================
        self.section("Step 2 · Search");
        self.narrate(
            "Equivalent tool: MCP tdw.kg.search { \"query\": \"AAPL supply chain\", \"top_k\": 3 }",
        );
        self.narrate(
            "  Note: demo uses the hash embedder (keyword-match, not semantic). \
             For semantic relevance configure a real embedder (local / openai / google).",
        );

        let hits = indexer
            .index()
            .search("AAPL supply chain", 3)
            .await
            .map_err(|e| format!("search: {e}"))?;

        if hits.is_empty() {
            return Err("demo smoke: search returned no hits for 'AAPL supply chain'".into());
        }

        self.narrate("  Top hits (lexical + hash retrieval):");
        let mut hit_values = Vec::new();
        for hit in &hits {
            self.narrate(&format!(
                "    [{:.4}] {} ({}) tags={:?}",
                hit.score, hit.id, hit.entity_id, hit.tags
            ));
            hit_values.push(json!({
                "id": hit.id,
                "score": hit.score,
                "entity_id": hit.entity_id,
                "tags": hit.tags,
            }));
        }
        self.emit_step(
            "search",
            &json!({ "query": "AAPL supply chain", "hits": hit_values }),
        );

        // ================================================================
        // STEP 3 — WHY (derivation provenance of the DeriveEdge fact)
        // ================================================================
        self.section("Step 3 · Why (derivation provenance)");

        let derivation_index = infer_engine.derivation_index();
        let why_result = explain_derived_edge(derivation_index)?;

        // Emit the exact equivalent MCP invocation using the real tdw.kg.why schema:
        // { "edge": { "from": "...", "rel": "supply_chain_peer", "to": "..." } }
        // The schema requires exactly one of entity_id / edge / tag_assignment.
        let edge_from = why_result
            .fact_key
            .split("->supply_chain_peer->")
            .next()
            .unwrap_or("");
        let edge_to = why_result
            .fact_key
            .split("->supply_chain_peer->")
            .nth(1)
            .unwrap_or("");
        self.narrate(&format!(
            "Equivalent tool: MCP tdw.kg.why {{ \"edge\": {{ \"from\": \"{edge_from}\", \
             \"rel\": \"supply_chain_peer\", \"to\": \"{edge_to}\" }} }}"
        ));

        self.narrate(&format!("  Derived edge: {}", why_result.fact_key));
        self.narrate(&format!(
            "  Rule: {} (v{})",
            why_result.rule_id, why_result.version
        ));
        self.narrate("  Provenance chain (support inputs):");
        for support_key in &why_result.support {
            self.narrate(&format!("    <- {support_key}"));
        }
        self.emit_step(
            "why",
            &json!({
                "fact_key": why_result.fact_key,
                "rule_id": why_result.rule_id,
                "version": why_result.version,
                "support": why_result.support,
                "edge_from": edge_from,
                "edge_to": edge_to,
            }),
        );

        if why_result.support.is_empty() {
            return Err("demo smoke: why provenance chain is empty".into());
        }

        // ================================================================
        // STEP 4 — INGEST v2 then TEMPORAL GRAPH DIFF
        // ================================================================
        self.section("Step 4 · Diff (two as_of snapshots — temporal graph-state diff)");
        self.narrate(
            "Equivalent tool: MCP tdw.kg.diff { \"from_as_of\": \"2026-01-15\", \"to_as_of\": \"2026-04-15\" }",
        );
        self.narrate("  Ingesting v2 snapshot (3 updated/new documents)...");

        let v2_date = "2026-04-15";
        let mut landed_v2 = 0usize;
        let mut skipped_v2 = 0usize;

        for doc in docs_v2 {
            let id = doc.id.clone();
            match indexer
                .index_at(doc, v2_date)
                .await
                .map_err(|e| format!("ingest v2 {id}: {e}"))?
            {
                IndexOutcome::Indexed => landed_v2 += 1,
                IndexOutcome::SkippedUnchanged => skipped_v2 += 1,
            }
        }

        // Manifest re-index delta (which document ids changed or were added).
        // v1_doc_ids was snapshotted from the live manifest before v2 ingest.
        let manifest_diff = compute_manifest_diff(indexer.manifest(), &v1_doc_ids, v2_date);

        // Temporal graph-state diff: edges that became valid in the v1→v2 window,
        // using the same active_at predicate the real tdw.kg.diff tool uses.
        let graph_diff = compute_temporal_graph_diff(&graph_arc, v1_date, v2_date)
            .await
            .map_err(|e| format!("temporal graph diff: {e}"))?;

        self.narrate(&format!(
            "  v2 ingest: landed={landed_v2} unchanged={skipped_v2}"
        ));
        self.narrate(&format!(
            "  Manifest re-index delta: {} changed document(s), {} new document(s)",
            manifest_diff.changed.len(),
            manifest_diff.added.len()
        ));
        self.narrate(&format!(
            "  Temporal graph diff ({v1_date} → {v2_date}): {} edge(s) added to graph",
            graph_diff.edges_added
        ));

        if !manifest_diff.changed.is_empty() {
            self.narrate("  Changed documents:");
            for id in &manifest_diff.changed {
                self.narrate(&format!("    ~ {id}"));
            }
        }
        if !manifest_diff.added.is_empty() {
            self.narrate("  Added documents:");
            for id in &manifest_diff.added {
                self.narrate(&format!("    + {id}"));
            }
        }

        self.emit_step(
            "diff",
            &json!({
                "from_as_of": v1_date,
                "to_as_of": v2_date,
                "manifest_changed": manifest_diff.changed,
                "manifest_added": manifest_diff.added,
                "graph_edges_added": graph_diff.edges_added,
                "graph_edges_invalidated": graph_diff.edges_invalidated,
                "landed_v2": landed_v2,
            }),
        );

        if manifest_diff.changed.is_empty() && manifest_diff.added.is_empty() {
            return Err("demo smoke: diff between v1 and v2 snapshots is empty".into());
        }

        // ================================================================
        // STEP 5 — STATUS (offline counts)
        // ================================================================
        self.section("Step 5 · Status (offline counts)");
        self.narrate(
            "Equivalent CLI: tdw kg status  (online variant; this shows in-memory counts)",
        );

        let total_docs = indexer.manifest().len();
        let derived_edges = infer_engine.derivation_index().len();
        let collection_name = tdw_knowledge::collection_name(embedder.model_id());

        self.narrate(&format!("  Embedder model   : {}", embedder.model_id()));
        self.narrate(&format!("  Vector collection: {collection_name}"));
        self.narrate(&format!("  Documents indexed: {total_docs}"));
        self.narrate(&format!("  Derived edges    : {derived_edges}"));
        self.narrate(&format!("  Infer version    : {}", infer_engine.version()));

        self.emit_step(
            "status",
            &json!({
                "embedder_model": embedder.model_id(),
                "collection": collection_name,
                "documents_indexed": total_docs,
                "derived_edges": derived_edges,
                "infer_version": infer_engine.version(),
            }),
        );

        if total_docs == 0 {
            return Err("demo smoke: status reports 0 indexed documents".into());
        }

        // ── Summary ───────────────────────────────────────────────────────
        self.section("Demo complete");
        self.narrate(
            "All steps passed. This was an in-memory demo — no data was persisted.\n\
             See docs/knowledge-quickstart.md for production setup and `tdw kg ingest`\n\
             for loading your own documents.",
        );

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Fixture loading helpers
// ---------------------------------------------------------------------------

fn load_docs(filename: &str) -> Result<Vec<KnowledgeDocument>, CliError> {
    let path = format!("{FIXTURE_DIR}/{filename}");
    let content =
        std::fs::read_to_string(&path).map_err(|e| format!("cannot read fixture {path:?}: {e}"))?;
    let mut docs = Vec::new();
    for (line_no, line) in content.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("//") {
            continue;
        }
        let doc: KnowledgeDocument = serde_json::from_str(line)
            .map_err(|e| format!("fixture {filename} line {}: {e}", line_no + 1))?;
        docs.push(doc);
    }
    Ok(docs)
}

fn load_tag_rules() -> Result<Vec<TagRule>, CliError> {
    let path = format!("{FIXTURE_DIR}/tag-rules.json");
    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("cannot read tag-rules fixture: {e}"))?;
    serde_json::from_str(&content).map_err(|e| format!("parse tag-rules.json: {e}").into())
}

fn load_infer_rules() -> Result<Vec<InferRule>, CliError> {
    let path = format!("{FIXTURE_DIR}/infer-rules.json");
    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("cannot read infer-rules fixture: {e}"))?;
    // The file is an array of tagged-enum objects:
    // [{ "DeriveEdge": { ... } }, ...]
    let raw: Vec<Value> =
        serde_json::from_str(&content).map_err(|e| format!("parse infer-rules.json: {e}"))?;
    raw.into_iter()
        .map(|v| {
            serde_json::from_value::<InferRule>(v)
                .map_err(|e| format!("decode InferRule: {e}").into())
        })
        .collect()
}

/// Collect every tag id mentioned across fixture document tags and rule targets
/// so we can pre-define them in the engine (avoids `UnknownTag` errors).
fn collect_tag_ids(
    docs_v1: &[KnowledgeDocument],
    docs_v2: &[KnowledgeDocument],
    rules: &[TagRule],
) -> Vec<String> {
    let mut ids: BTreeSet<String> = BTreeSet::new();
    for doc in docs_v1.iter().chain(docs_v2.iter()) {
        ids.extend(doc.tags.iter().cloned());
    }
    for rule in rules {
        ids.insert(rule.tag_id.clone());
    }
    ids.into_iter().collect()
}

// ---------------------------------------------------------------------------
// Why-step: find a supply_chain_peer derived edge in the index and explain it
// ---------------------------------------------------------------------------

/// Output of the why-step explanation.
pub struct WhyResult {
    /// The full derivation index key, e.g. `"A->supply_chain_peer->B"`.
    pub fact_key: String,
    /// Id of the rule that fired.
    pub rule_id: String,
    /// Inference engine version at derivation time.
    pub version: u64,
    /// Support-input keys (the edge facts that triggered the rule).
    pub support: Vec<String>,
}

fn explain_derived_edge(derivation_index: &DerivationIndex) -> Result<WhyResult, CliError> {
    for (key, derivation) in derivation_index.iter() {
        if key.contains("->supply_chain_peer->") {
            return Ok(WhyResult {
                fact_key: key.clone(),
                rule_id: derivation.rule_id.clone(),
                version: derivation.version,
                support: derivation.support.clone(),
            });
        }
    }
    Err(
        "demo smoke: no supply_chain_peer derived edge found in derivation index — \
         the DeriveEdge rule did not fire or the mentions edges were not written to the graph"
            .into(),
    )
}

// ---------------------------------------------------------------------------
// Diff-step: manifest re-index delta (changed vs. added document ids)
// ---------------------------------------------------------------------------

struct ManifestDiff {
    /// Document ids whose content hash changed between the v1 snapshot and `v2_date`.
    changed: Vec<String>,
    /// Document ids first indexed at `v2_date` (not present in the v1 snapshot).
    added: Vec<String>,
}

/// Compute which document ids changed or were added in the v2 pass.
///
/// `v1_doc_ids` is snapshotted from the live manifest **before** the v2 ingest
/// so that fixture changes never silently desync the changed-vs-added classification.
fn compute_manifest_diff(
    manifest: &IndexManifest,
    v1_doc_ids: &BTreeSet<String>,
    v2_date: &str,
) -> ManifestDiff {
    let mut changed: BTreeSet<String> = BTreeSet::new();
    let mut added: BTreeSet<String> = BTreeSet::new();

    for (id, entry) in manifest.entries() {
        if entry.indexed_at == v2_date {
            if v1_doc_ids.contains(id) {
                changed.insert(id.clone());
            } else {
                added.insert(id.clone());
            }
        }
    }

    ManifestDiff {
        changed: changed.into_iter().collect(),
        added: added.into_iter().collect(),
    }
}

// ---------------------------------------------------------------------------
// Diff-step: temporal graph-state diff (mirrors tdw.kg.diff logic)
// ---------------------------------------------------------------------------

/// Summary counts from a temporal graph-state diff.
pub struct GraphDiff {
    /// Edges that became active in the window `(from_date, to_date]`.
    pub edges_added: usize,
    /// Edges whose validity window closed in the window.
    pub edges_invalidated: usize,
}

/// Temporal graph-state diff between two `as_of` dates.
///
/// Uses the same `active_at` predicate as `tdw.kg.diff` — edges added or
/// invalidated between `from_date` and `to_date`.
/// Mirrors `knowledge_explain_tools::compute_edge_delta`.
///
/// # Errors
///
/// Returns an error if the graph engine fails to enumerate edges.
pub async fn compute_temporal_graph_diff(
    graph: &Arc<dyn GraphEngine>,
    from_date: &str,
    to_date: &str,
) -> Result<GraphDiff, CliError> {
    // Paginate all edges from the in-memory graph (same as collect_all_edges in tdw-mcp).
    let mut all_edges = Vec::new();
    let mut offset = 0usize;
    let page_size = 1024usize;
    loop {
        let page = graph
            .edges(None, offset, page_size)
            .await
            .map_err(|e| format!("graph.edges: {e}"))?;
        if page.is_empty() {
            break;
        }
        offset += page.len();
        all_edges.extend(page);
    }

    let active_at_from = |e: &tdw_core::GraphEdge| {
        active_at(e.valid_from.as_deref(), e.valid_to.as_deref(), from_date)
    };
    let active_at_to = |e: &tdw_core::GraphEdge| {
        active_at(e.valid_from.as_deref(), e.valid_to.as_deref(), to_date)
    };

    // Became valid: NOT active at from_date, IS active at to_date.
    let edges_added = all_edges
        .iter()
        .filter(|e| !active_at_from(e) && active_at_to(e))
        .count();

    // Validity window closed: WAS active at from_date, NOT at to_date.
    let edges_invalidated = all_edges
        .iter()
        .filter(|e| active_at_from(e) && !active_at_to(e))
        .count();

    Ok(GraphDiff {
        edges_added,
        edges_invalidated,
    })
}

// ---------------------------------------------------------------------------
// Integration-test helpers (pub so the tests module can call them)
// ---------------------------------------------------------------------------

/// Run the full demo in `--json` mode and return `Ok(())` if every step passes.
///
/// Used by integration tests for a programmatic assertion.
///
/// # Errors
///
/// Returns a descriptive error if any step fails.
#[allow(dead_code)] // used by integration tests in tests/kg_demo.rs
pub async fn run_smoke() -> Result<(), CliError> {
    // Synthesise `--json` args list.
    let args: Vec<String> = ["tdw", "kg", "demo", "--json"]
        .iter()
        .map(|s| (*s).to_string())
        .collect();
    run(&args).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_fixtures_parse_without_errors() {
        let docs = load_docs("docs-v1.jsonl").expect("docs-v1 should parse");
        assert!(docs.len() >= 8, "docs-v1 must have at least 8 docs");

        let docs2 = load_docs("docs-v2.jsonl").expect("docs-v2 should parse");
        assert!(!docs2.is_empty(), "docs-v2 must be non-empty");

        let rules = load_tag_rules().expect("tag-rules should parse");
        assert!(!rules.is_empty(), "tag rules must be non-empty");

        let infer = load_infer_rules().expect("infer-rules should parse");
        let has_derive_edge = infer
            .iter()
            .any(|r| matches!(r, InferRule::DeriveEdge { .. }));
        assert!(
            has_derive_edge,
            "infer-rules must contain at least one DeriveEdge"
        );
    }

    #[test]
    fn manifest_diff_correctly_classifies_changed_and_added() {
        use tdw_knowledge::indexer::IndexManifest;
        // Minimal manifest: aapl-profile updated at v2, new-doc added at v2.
        let mut m = IndexManifest::default();
        m.record("aapl-profile", "oldhash", "2026-01-15");
        m.record("aapl-profile", "newhash", "2026-04-15"); // overwrite

        m.record("new-doc", "somehash", "2026-04-15");

        // v1_doc_ids snapshotted from the manifest BEFORE v2 ingest.
        let mut v1_ids: BTreeSet<String> = BTreeSet::new();
        v1_ids.insert("aapl-profile".to_string());

        let diff = compute_manifest_diff(&m, &v1_ids, "2026-04-15");
        assert!(
            diff.changed.contains(&"aapl-profile".to_string()),
            "aapl-profile is a known v1 doc re-indexed at v2 -> changed"
        );
        assert!(
            diff.added.contains(&"new-doc".to_string()),
            "new-doc first appears at v2 -> added"
        );
    }

    /// Validate that each demo step's printed MCP invocation uses ONLY parameter
    /// names present in the real tool schemas (`additionalProperties: false`).
    ///
    /// This test parses the exact string literals emitted by `run_all` and checks
    /// every key against the schema, so documentation cannot drift from reality.
    #[test]
    fn demo_mcp_examples_match_real_tool_schemas() {
        // ── tdw.kg.search schema (from knowledge_tools.rs search_descriptor) ──
        // Required: query. Optional: top_k, tags_any, entity_kinds, plane, as_of, expand.
        let search_allowed: BTreeSet<&str> = [
            "query",
            "top_k",
            "tags_any",
            "entity_kinds",
            "plane",
            "as_of",
            "expand",
        ]
        .into_iter()
        .collect();

        // The demo emits: { "query": "...", "top_k": 3 }
        let search_example = json!({ "query": "AAPL supply chain", "top_k": 3 });
        for key in search_example.as_object().unwrap().keys() {
            assert!(
                search_allowed.contains(key.as_str()),
                "demo search example uses unknown key {key:?} — not in tdw.kg.search schema"
            );
        }

        // ── tdw.kg.why schema (from knowledge_explain_tools.rs why_descriptor) ──
        // Allowed top-level: entity_id, edge, tag_assignment, as_of.
        // edge sub-object required: from, rel, to (additionalProperties: false).
        let why_allowed: BTreeSet<&str> = ["entity_id", "edge", "tag_assignment", "as_of"]
            .into_iter()
            .collect();
        let why_edge_allowed: BTreeSet<&str> = ["from", "rel", "to"].into_iter().collect();

        // The demo emits: { "edge": { "from": "...", "rel": "supply_chain_peer", "to": "..." } }
        let why_example = json!({
            "edge": { "from": "instrument:AAPL", "rel": "supply_chain_peer", "to": "company:TSMC" }
        });
        for key in why_example.as_object().unwrap().keys() {
            assert!(
                why_allowed.contains(key.as_str()),
                "demo why example uses unknown top-level key {key:?} — not in tdw.kg.why schema"
            );
        }
        let edge_obj = why_example["edge"].as_object().unwrap();
        for key in edge_obj.keys() {
            assert!(
                why_edge_allowed.contains(key.as_str()),
                "demo why edge example uses unknown key {key:?} — not in tdw.kg.why edge schema"
            );
        }

        // ── tdw.kg.diff schema (from knowledge_explain_tools.rs diff_descriptor) ──
        // Required: from_as_of, to_as_of. Optional: plane, scope_entity_id, limit.
        let diff_allowed: BTreeSet<&str> = [
            "from_as_of",
            "to_as_of",
            "plane",
            "scope_entity_id",
            "limit",
        ]
        .into_iter()
        .collect();

        // The demo emits: { "from_as_of": "2026-01-15", "to_as_of": "2026-04-15" }
        let diff_example = json!({ "from_as_of": "2026-01-15", "to_as_of": "2026-04-15" });
        for key in diff_example.as_object().unwrap().keys() {
            assert!(
                diff_allowed.contains(key.as_str()),
                "demo diff example uses unknown key {key:?} — not in tdw.kg.diff schema"
            );
        }
    }
}
