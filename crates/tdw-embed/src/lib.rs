#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub type Result<T> = std::result::Result<T, EmbeddingError>;

#[derive(Debug, Error)]
pub enum EmbeddingError {
    #[error("embedding input is empty")]
    EmptyInput,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Embedding {
    pub model_id: String,
    pub vector: Vec<f32>,
}

pub trait EmbeddingProvider: Send + Sync {
    fn model_id(&self) -> &str;
    fn embed(&self, text: &str) -> Result<Embedding>;
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ConstantProvider;

    impl EmbeddingProvider for ConstantProvider {
        fn model_id(&self) -> &str {
            "constant"
        }

        fn embed(&self, text: &str) -> Result<Embedding> {
            if text.is_empty() {
                return Err(EmbeddingError::EmptyInput);
            }
            Ok(Embedding {
                model_id: self.model_id().to_string(),
                vector: vec![1.0],
            })
        }
    }

    #[test]
    fn provider_contract_returns_model_id_with_vector() {
        let provider = ConstantProvider;
        let embedding = provider
            .embed("research")
            .unwrap_or_else(|error| panic!("embedding should succeed: {error}"));

        assert_eq!(embedding.model_id, "constant");
        assert_eq!(embedding.vector, vec![1.0]);
    }
}
