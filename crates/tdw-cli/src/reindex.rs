//! `tdw kg reindex --embedder <id>` (knowledge-system B6).
//!
//! Rebuilds the target embedder's vector collection from the durable
//! documents in the lexical store, so switching the configured embedder
//! never loses data and can never mix dimensions (collections are
//! namespaced per embedder model id). This is an OFFLINE maintenance
//! command: it talks to the engines directly via the same environment
//! variables the daemon uses (`TDW_QDRANT_URL`/`TDW_QDRANT_API_KEY`,
//! `TDW_MEILI_URL`/`TDW_MEILI_API_KEY`) — run it while the daemon is idle.

use std::sync::Arc;

use tdw_core::{LexicalEngine, VectorEngine};
use tdw_embed::EmbeddingProvider;
use tdw_embed_local::HashEmbeddingProvider;

use crate::CliError;

/// The lexical index holding the durable documents (`TDW_KNOWLEDGE_TEXT_INDEX`
/// overrides).
const DEFAULT_TEXT_INDEX: &str = "tdw_knowledge_text";

/// Run the reindex command. `args` are the full process arguments.
///
/// # Errors
///
/// Returns a descriptive error when the embedder id is missing/unknown, an
/// engine environment variable is unset, or any engine/embedder call fails —
/// there is no silent fallback anywhere on this path.
pub async fn run(args: &[String]) -> Result<(), CliError> {
    let embedder_id = flag_value(args, "--embedder")
        .ok_or("usage: tdw-cli kg reindex --embedder <hash|local|openai|google>")?;
    let embedder = select_embedder(&embedder_id)?;
    let vectors: Arc<dyn VectorEngine> = build_qdrant()?;
    let lexical: Arc<dyn LexicalEngine> = build_meilisearch()?;
    let text_index =
        env_trimmed("TDW_KNOWLEDGE_TEXT_INDEX").unwrap_or_else(|| DEFAULT_TEXT_INDEX.to_string());

    let collection = tdw_knowledge::collection_name(embedder.model_id());
    let count =
        tdw_knowledge::reindex::reindex_collection(&embedder, &vectors, &lexical, &text_index)
            .await
            .map_err(|error| format!("reindex failed: {error}"))?;
    println!(
        "reindexed {count} documents from lexical index {text_index:?} into collection \
         {collection:?} (embedder {})",
        embedder.model_id()
    );
    Ok(())
}

/// `--flag value` lookup.
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
    env_trimmed(name).ok_or_else(|| format!("{name} must be set for kg reindex").into())
}

/// Mirror of the daemon's embedder selection (`tdw-backend::select_embedder`)
/// for the offline command path: same ids, same env keys, same
/// no-silent-fallback posture.
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
             local-model / openai / google) — kg reindex never falls back silently"
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flag_value_parses_and_rejects_empty() {
        let args: Vec<String> = ["tdw", "kg", "reindex", "--embedder", "hash"]
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(flag_value(&args, "--embedder").as_deref(), Some("hash"));
        assert_eq!(flag_value(&args, "--missing"), None);
        let empty: Vec<String> = ["--embedder", "  "]
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(flag_value(&empty, "--embedder"), None);
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
