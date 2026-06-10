#![forbid(unsafe_code)]

#[cfg(feature = "meilisearch")]
pub mod http_engine;

#[cfg(feature = "meilisearch")]
pub use http_engine::MeilisearchHttpEngine;

use std::collections::BTreeMap;
use std::sync::Mutex;

use async_trait::async_trait;
use tdw_core::{Error, LexicalDoc, LexicalEngine, Result, ScoredDoc, TextQuery};

#[derive(Debug, Default)]
pub struct InMemoryLexicalEngine {
    indices: Mutex<BTreeMap<String, Vec<LexicalDoc>>>,
}

#[async_trait]
impl LexicalEngine for InMemoryLexicalEngine {
    async fn index(&self, index: &str, docs: Vec<LexicalDoc>) -> Result<()> {
        validate_index(index)?;
        for doc in &docs {
            validate_doc(doc)?;
        }
        let mut indices = self
            .indices
            .lock()
            .map_err(|error| Error::Storage(error.to_string()))?;
        let index_docs = indices.entry(index.to_string()).or_default();
        for doc in docs {
            if let Some(existing) = index_docs.iter_mut().find(|existing| existing.id == doc.id) {
                *existing = doc;
            } else {
                index_docs.push(doc);
            }
        }
        drop(indices);
        Ok(())
    }

    async fn search_text(&self, index: &str, query: TextQuery) -> Result<Vec<ScoredDoc>> {
        validate_index(index)?;
        validate_query(&query)?;
        let needle = query.text.to_ascii_lowercase();
        let mut scored = {
            let indices = self
                .indices
                .lock()
                .map_err(|error| Error::Storage(error.to_string()))?;
            let docs = indices
                .get(index)
                .ok_or_else(|| Error::Storage(format!("unknown lexical index: {index}")))?;
            let collected = docs
                .iter()
                .filter(|doc| query.filter.matches(&doc.fields))
                .map(|doc| {
                    let body = doc.body.to_ascii_lowercase();
                    // Match count -> f32 relevance score; match counts are tiny.
                    #[allow(clippy::cast_precision_loss)]
                    let score = body.matches(&needle).count() as f32;
                    ScoredDoc {
                        id: doc.id.clone(),
                        score,
                        fields: doc.fields.clone(),
                    }
                })
                .filter(|doc| doc.score > 0.0)
                .collect::<Vec<_>>();
            drop(indices);
            collected
        };
        scored.sort_by(|left, right| {
            right
                .score
                .partial_cmp(&left.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        scored.truncate(query.top_k);
        Ok(scored)
    }
}

fn validate_index(index: &str) -> Result<()> {
    if index.trim().is_empty() {
        return Err(Error::Storage(
            "lexical index must not be empty".to_string(),
        ));
    }
    Ok(())
}

fn validate_doc(doc: &LexicalDoc) -> Result<()> {
    if doc.id.trim().is_empty() {
        return Err(Error::Storage(
            "lexical document id must not be empty".to_string(),
        ));
    }
    Ok(())
}

fn validate_query(query: &TextQuery) -> Result<()> {
    if query.text.trim().is_empty() {
        return Err(Error::Storage(
            "lexical query text must not be empty".to_string(),
        ));
    }
    if query.top_k == 0 {
        return Err(Error::Storage(
            "lexical query top_k must be greater than zero".to_string(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_lexical_engine_contract() {
        fn assert_lexical<T: LexicalEngine>() {}

        assert_lexical::<InMemoryLexicalEngine>();
    }

    #[tokio::test]
    async fn indexes_searches_and_rejects_empty_queries() {
        let engine = InMemoryLexicalEngine::default();
        engine
            .index(
                "research",
                vec![LexicalDoc {
                    id: "note-1".to_string(),
                    body: "AAPL volatility note".to_string(),
                    fields: serde_json::json!({ "symbol": "AAPL" }),
                }],
            )
            .await
            .unwrap_or_else(|error| panic!("document should index: {error}"));

        let docs = engine
            .search_text(
                "research",
                TextQuery {
                    text: "volatility".to_string(),
                    top_k: 5,
                    filter: tdw_core::PayloadFilter::default(),
                },
            )
            .await
            .unwrap_or_else(|error| panic!("search should succeed: {error}"));

        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].id, "note-1");
        assert!(
            engine
                .search_text(
                    "research",
                    TextQuery {
                        text: String::new(),
                        top_k: 1,
                        filter: tdw_core::PayloadFilter::default(),
                    },
                )
                .await
                .is_err()
        );
        assert!(
            engine
                .search_text(
                    "research",
                    TextQuery {
                        text: "volatility".to_string(),
                        top_k: 0,
                        filter: tdw_core::PayloadFilter::default(),
                    },
                )
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn fields_filter_excludes_matching_docs() {
        use tdw_core::{PayloadCondition, PayloadFilter};
        let engine = InMemoryLexicalEngine::default();
        engine
            .index(
                "research",
                vec![
                    LexicalDoc {
                        id: "aapl-note".to_string(),
                        body: "volatility note".to_string(),
                        fields: serde_json::json!({ "symbol": "AAPL" }),
                    },
                    LexicalDoc {
                        id: "msft-note".to_string(),
                        body: "volatility note".to_string(),
                        fields: serde_json::json!({ "symbol": "MSFT" }),
                    },
                ],
            )
            .await
            .unwrap_or_else(|error| panic!("documents should index: {error}"));

        let docs = engine
            .search_text(
                "research",
                TextQuery {
                    text: "volatility".to_string(),
                    top_k: 5,
                    filter: PayloadFilter {
                        must: vec![PayloadCondition::MatchString {
                            key: "symbol".to_string(),
                            value: "AAPL".to_string(),
                        }],
                    },
                },
            )
            .await
            .unwrap_or_else(|error| panic!("filtered search should succeed: {error}"));
        assert_eq!(
            docs.iter().map(|doc| doc.id.as_str()).collect::<Vec<_>>(),
            vec!["aapl-note"]
        );
    }
}
