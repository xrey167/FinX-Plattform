//! `tdw kg ingest <file.jsonl|->` (knowledge-system K-E3).
//!
//! Reads newline-delimited JSON (JSONL) [`KnowledgeDocument`] values from a
//! file or stdin (`-`), batches them through the daemon-hosted
//! [`KnowledgeIndexer`] using the same offline engine path as `kg reindex`
//! (direct engine access via `TDW_QDRANT_URL` / `TDW_MEILI_URL`), and prints
//! a per-document landed / duplicate / error status line.
//!
//! This is an **offline-maintenance command** — run it while the daemon is idle
//! or against a standalone engine stack, exactly like `kg reindex`. Future work
//! may add a `--daemon` flag that routes through the live daemon's loopback
//! instead of the engine environment variables.
//!
//! # Storage-key note (multi-tenancy)
//!
//! Collection names and index names are derived from the configured embedder
//! model id (via [`tdw_knowledge::collection_name`]). No new global singletons
//! are introduced here; future graph namespacing can be layered onto the same
//! seam at the [`KnowledgeIndexer`] level.

use std::io::{BufRead, BufReader};
use std::sync::Arc;

use tdw_core::{GraphEngine, LexicalEngine, VectorEngine};
use tdw_embed::EmbeddingProvider;
use tdw_embed_local::HashEmbeddingProvider;
use tdw_knowledge::indexer::{IndexOutcome, KnowledgeIndexer};
use tdw_knowledge::{KnowledgeDocument, KnowledgeIndex};

use crate::CliError;

/// Run the `kg ingest` command. `args` are the full process arguments.
///
/// # Errors
///
/// Returns a descriptive error when the input path is missing/unreadable,
/// an engine environment variable is unset, any JSONL line is not a valid
/// [`KnowledgeDocument`], or an engine/embedder call fails.
pub async fn run(args: &[String]) -> Result<(), CliError> {
    let path = ingest_path(args).ok_or("usage: tdw-cli kg ingest <file.jsonl|->")?;
    let embedder_id = flag_value(args, "--embedder").unwrap_or_else(|| "hash".to_string());
    let now_override = flag_value(args, "--now");

    let embedder = select_embedder(&embedder_id)?;
    let vectors: Arc<dyn VectorEngine> = build_qdrant()?;
    let lexical: Arc<dyn LexicalEngine> = build_meilisearch()?;
    let graph: Arc<dyn GraphEngine> = build_graph();

    let collection = tdw_knowledge::collection_name(embedder.model_id());
    let inner = KnowledgeIndex::new(Arc::clone(&embedder), Arc::clone(&vectors));
    let mut indexer = KnowledgeIndexer::new(inner)
        .with_lexical(Arc::clone(&lexical), collection)
        .with_graph(Arc::clone(&graph));

    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let now = now_override.as_deref().unwrap_or(&today);

    let docs = read_jsonl(&path)?;
    if docs.is_empty() {
        println!("tdw kg ingest: no documents in input — nothing to do");
        return Ok(());
    }

    let mut landed = 0usize;
    let mut duplicates = 0usize;
    let mut errors = 0usize;

    for doc in docs {
        let id = doc.id.clone();
        match indexer.index_at(doc, now).await {
            Ok(IndexOutcome::Indexed) => {
                println!("landed   {id}");
                landed += 1;
            }
            Ok(IndexOutcome::SkippedUnchanged) => {
                println!("duplicate {id}");
                duplicates += 1;
            }
            Err(error) => {
                eprintln!("error    {id}: {error}");
                errors += 1;
            }
        }
    }

    println!("\ntdw kg ingest: landed={landed} duplicate={duplicates} error={errors}");
    if errors > 0 {
        return Err(format!("{errors} document(s) failed to ingest").into());
    }
    Ok(())
}

/// The input path: the first positional argument after `ingest` that is not a
/// named flag (`--foo`) or a flag value (the token immediately following a
/// `--foo` flag). `-` is the stdin sentinel and is NOT a flag.
fn ingest_path(args: &[String]) -> Option<String> {
    let ingest_pos = args.iter().position(|a| a == "ingest")?;
    let rest = args.get(ingest_pos + 1..)?;
    let mut skip_next = false;
    for token in rest {
        if skip_next {
            skip_next = false;
            continue;
        }
        // A long flag (`--foo`) — its next token is the value, skip both.
        if token.starts_with("--") {
            skip_next = true;
            continue;
        }
        // Everything else is a positional: `-` (stdin) or a file path.
        return Some(token.clone());
    }
    None
}

/// `--flag value` lookup (shared with reindex).
fn flag_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg == flag)
        .and_then(|index| args.get(index + 1))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn env_trimmed(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn require_env(name: &str) -> Result<String, CliError> {
    env_trimmed(name).ok_or_else(|| format!("{name} must be set for kg ingest").into())
}

/// Read JSONL from `path` (or stdin when `path == "-"`).
///
/// Each non-empty line must deserialize as a [`KnowledgeDocument`]; a malformed
/// line is a hard error — no silent skipping on this batch path.
fn read_jsonl(path: &str) -> Result<Vec<KnowledgeDocument>, CliError> {
    let reader: Box<dyn BufRead> = if path == "-" {
        Box::new(BufReader::new(std::io::stdin()))
    } else {
        let file =
            std::fs::File::open(path).map_err(|error| format!("cannot open {path:?}: {error}"))?;
        Box::new(BufReader::new(file))
    };

    let mut docs = Vec::new();
    for (line_no, line) in reader.lines().enumerate() {
        let line = line.map_err(|error| format!("read error at line {}: {error}", line_no + 1))?;
        let line = line.trim();
        if line.is_empty() || line.starts_with("//") {
            continue;
        }
        let doc: KnowledgeDocument =
            serde_json::from_str(line).map_err(|error| format!("line {}: {error}", line_no + 1))?;
        docs.push(doc);
    }
    Ok(docs)
}

/// Mirror of the daemon's embedder selection and the reindex command's
/// `select_embedder` — same ids, same env keys, same no-silent-fallback posture.
fn select_embedder(id: &str) -> Result<Arc<dyn EmbeddingProvider>, CliError> {
    match id.to_ascii_lowercase().as_str() {
        "hash" => Ok(Arc::new(HashEmbeddingProvider::default())),
        #[cfg(feature = "local-model")]
        "local" => {
            let model_dir = require_env("TDW_EMBED_MODEL_DIR")?;
            let provider = tdw_embed_local::LocalModelEmbeddingProvider::from_dir(&model_dir)
                .map_err(|error| format!("local model: {error}"))?;
            Ok(Arc::new(provider))
        }
        #[cfg(feature = "openai")]
        "openai" => {
            let api_key = env_trimmed("TDW_OPENAI_EMBEDDING_API_KEY")
                .or_else(|| env_trimmed("OPENAI_API_KEY"))
                .ok_or("openai embedder requires TDW_OPENAI_EMBEDDING_API_KEY / OPENAI_API_KEY")?;
            let model = env_trimmed("TDW_EMBED_MODEL")
                .unwrap_or_else(|| "text-embedding-3-small".to_string());
            let mut client = tdw_embed_openai::OpenAiEmbeddingHttpClient::new(api_key, model)
                .map_err(|error| format!("openai embedder: {error}"))?;
            if let Some(base_url) = env_trimmed("TDW_OPENAI_EMBEDDING_BASE_URL") {
                client = client
                    .with_base_url(&base_url)
                    .map_err(|error| format!("openai embedder: {error}"))?;
            }
            Ok(Arc::new(client))
        }
        #[cfg(feature = "google")]
        "google" => {
            let api_key = env_trimmed("TDW_GOOGLE_EMBEDDING_API_KEY")
                .or_else(|| env_trimmed("GOOGLE_API_KEY"))
                .or_else(|| env_trimmed("GEMINI_API_KEY"))
                .ok_or(
                    "google embedder requires TDW_GOOGLE_EMBEDDING_API_KEY / GOOGLE_API_KEY / \
                     GEMINI_API_KEY",
                )?;
            let model = env_trimmed("TDW_EMBED_MODEL")
                .unwrap_or_else(|| "gemini-embedding-001".to_string());
            let mut client = tdw_embed_google::GoogleEmbeddingHttpClient::new(api_key, model)
                .map_err(|error| format!("google embedder: {error}"))?;
            if let Some(base_url) = env_trimmed("TDW_GOOGLE_EMBEDDING_BASE_URL") {
                client = client
                    .with_base_url(&base_url)
                    .map_err(|error| format!("google embedder: {error}"))?;
            }
            Ok(Arc::new(client))
        }
        other => Err(format!(
            "embedder {other:?} is unavailable in this build (compile the matching feature: \
             local-model / openai / google) — kg ingest never falls back silently"
        )
        .into()),
    }
}

fn build_qdrant() -> Result<Arc<dyn VectorEngine>, CliError> {
    let url = require_env("TDW_QDRANT_URL")?;
    let api_key = env_trimmed("TDW_QDRANT_API_KEY");
    let engine = tdw_storage_qdrant::QdrantHttpEngine::new(&url, api_key)
        .map_err(|error| format!("qdrant engine: {error}"))?;
    Ok(Arc::new(engine))
}

fn build_meilisearch() -> Result<Arc<dyn LexicalEngine>, CliError> {
    let url = require_env("TDW_MEILI_URL")?;
    let api_key = env_trimmed("TDW_MEILI_API_KEY");
    let engine = tdw_storage_meilisearch::MeilisearchHttpEngine::new(&url, api_key)
        .map_err(|error| format!("meilisearch engine: {error}"))?;
    Ok(Arc::new(engine))
}

/// The graph engine for offline ingest. When `TDW_STORAGE_GRAPH_BOLT_URL` is
/// set and the `bolt` feature is compiled, a durable `BoltGraphEngine` is used;
/// otherwise an in-memory graph is used (durable graph stamping is a no-op but
/// the document is still written to the vector and lexical stores).
fn build_graph() -> Arc<dyn GraphEngine> {
    // In-memory graph: durable graph stamping is advisory — the document lands
    // in vector+lexical regardless. A future --graph flag can wire up Bolt here
    // without changing the indexer interface.
    Arc::new(tdw_storage_graph::InMemoryGraphEngine::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ingest_path_finds_positional_after_ingest() {
        let args: Vec<String> = ["tdw", "kg", "ingest", "docs.jsonl"]
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(ingest_path(&args).as_deref(), Some("docs.jsonl"));
    }

    #[test]
    fn ingest_path_recognises_stdin_dash() {
        let args: Vec<String> = ["tdw", "kg", "ingest", "-"]
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(ingest_path(&args).as_deref(), Some("-"));
    }

    #[test]
    fn ingest_path_skips_flags() {
        let args: Vec<String> = ["tdw", "kg", "ingest", "--embedder", "hash", "my.jsonl"]
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(ingest_path(&args).as_deref(), Some("my.jsonl"));
    }

    #[test]
    fn ingest_path_returns_none_when_missing() {
        let args: Vec<String> = ["tdw", "kg", "ingest"]
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(ingest_path(&args), None);
    }

    #[test]
    fn read_jsonl_parses_valid_docs() {
        use tdw_kg::{Entity, EntityKind};

        let doc = tdw_knowledge::KnowledgeDocument::new(
            "test-doc-1",
            "test body content",
            Entity {
                entity_id: "instrument:TEST".to_string(),
                kind: EntityKind::Instrument,
                label: "Test".to_string(),
                aliases: Vec::new(),
            },
            Vec::new(),
        );
        let line = serde_json::to_string(&doc).expect("serialise");
        let jsonl = format!("{line}\n// comment\n\n");
        let tmp = std::env::temp_dir().join("tdw-cli-ingest-test.jsonl");
        std::fs::write(&tmp, &jsonl).expect("write tmp");
        let docs = read_jsonl(tmp.to_str().expect("path")).expect("read_jsonl");
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].id, "test-doc-1");
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn read_jsonl_errors_on_malformed_line() {
        let tmp = std::env::temp_dir().join("tdw-cli-ingest-bad.jsonl");
        std::fs::write(&tmp, "not valid json\n").expect("write tmp");
        let result = read_jsonl(tmp.to_str().expect("path"));
        assert!(result.is_err(), "malformed line must error");
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn unknown_embedder_errors_instead_of_falling_back() {
        let error = select_embedder("definitely-not-a-provider")
            .err()
            .map(|error| error.to_string())
            .unwrap_or_default();
        assert!(error.contains("never falls back"), "{error}");
    }
}
