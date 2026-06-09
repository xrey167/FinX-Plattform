#![forbid(unsafe_code)]

#![deny(clippy::pedantic, clippy::nursery)]
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub type Result<T> = std::result::Result<T, EmbeddingError>;

#[derive(Debug, Error)]
pub enum EmbeddingError {
    #[error("embedding model id is empty")]
    EmptyModelId,
    #[error("embedding input is empty")]
    EmptyInput,
    #[error("embedding dimensions must be greater than zero")]
    InvalidDimensions,
    #[error("embedding vector is empty")]
    EmptyVector,
    #[error("embedding vector contains a non-finite value")]
    NonFiniteVector,
    #[error("embedding provider error: {0}")]
    Provider(String),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Embedding {
    pub model_id: String,
    pub vector: Vec<f32>,
}

#[async_trait::async_trait]
pub trait EmbeddingProvider: Send + Sync {
    fn model_id(&self) -> &str;
    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    async fn embed(&self, text: &str) -> Result<Embedding>;
}

/// # Errors
///
/// Returns an error variant if the underlying operation fails.
pub fn validate_embedding(embedding: &Embedding) -> Result<()> {
    if embedding.model_id.trim().is_empty() {
        return Err(EmbeddingError::EmptyModelId);
    }
    if embedding.vector.is_empty() {
        return Err(EmbeddingError::EmptyVector);
    }
    if embedding.vector.iter().any(|value| !value.is_finite()) {
        return Err(EmbeddingError::NonFiniteVector);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ConstantProvider;

    #[async_trait::async_trait]
    impl EmbeddingProvider for ConstantProvider {
        fn model_id(&self) -> &'static str {
            "constant"
        }

        async fn embed(&self, text: &str) -> Result<Embedding> {
            if text.is_empty() {
                return Err(EmbeddingError::EmptyInput);
            }
            Ok(Embedding {
                model_id: self.model_id().to_string(),
                vector: vec![1.0],
            })
        }
    }

    #[tokio::test]
    async fn provider_contract_returns_model_id_with_vector() {
        let provider = ConstantProvider;
        let embedding = provider
            .embed("research")
            .await
            .unwrap_or_else(|error| panic!("embedding should succeed: {error}"));

        assert_eq!(embedding.model_id, "constant");
        assert_eq!(embedding.vector, vec![1.0]);
        assert!(validate_embedding(&embedding).is_ok());
        assert!(
            validate_embedding(&Embedding {
                model_id: String::new(),
                vector: vec![1.0],
            })
            .is_err()
        );
        assert!(
            validate_embedding(&Embedding {
                model_id: "bad".to_string(),
                vector: vec![f32::NAN],
            })
            .is_err()
        );
    }
}
