#![forbid(unsafe_code)]

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
        Ok(())
    }

    async fn search_text(&self, index: &str, query: TextQuery) -> Result<Vec<ScoredDoc>> {
        let indices = self
            .indices
            .lock()
            .map_err(|error| Error::Storage(error.to_string()))?;
        let docs = indices
            .get(index)
            .ok_or_else(|| Error::Storage(format!("unknown lexical index: {index}")))?;
        let needle = query.text.to_ascii_lowercase();
        let mut scored = docs
            .iter()
            .map(|doc| {
                let body = doc.body.to_ascii_lowercase();
                let score = body.matches(&needle).count() as f32;
                ScoredDoc {
                    id: doc.id.clone(),
                    score,
                    fields: doc.fields.clone(),
                }
            })
            .filter(|doc| doc.score > 0.0)
            .collect::<Vec<_>>();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_lexical_engine_contract() {
        fn assert_lexical<T: LexicalEngine>() {}

        assert_lexical::<InMemoryLexicalEngine>();
    }
}
