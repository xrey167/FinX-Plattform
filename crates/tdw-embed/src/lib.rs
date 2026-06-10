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

    /// Embed many texts, preserving input order (knowledge-system B2).
    ///
    /// The default implementation loops over [`EmbeddingProvider::embed`], so
    /// every provider gets batching for free; providers with a native batch
    /// endpoint (`OpenAI`, Google) override this with one round-trip. The
    /// contract every implementation must hold — enforced by
    /// [`validate_batch`] — is: the result has exactly one embedding per
    /// input, in input order, and each embedding passes
    /// [`validate_embedding`].
    ///
    /// # Errors
    ///
    /// Returns an error variant if any input is empty, any underlying embed
    /// fails, or the batch contract is violated.
    async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Embedding>> {
        let mut embeddings = Vec::with_capacity(texts.len());
        for text in texts {
            embeddings.push(self.embed(text).await?);
        }
        validate_batch(texts, &embeddings)?;
        Ok(embeddings)
    }
}

/// The batch contract: exactly one valid embedding per input.
///
/// # Errors
///
/// Returns [`EmbeddingError::Provider`] on a length mismatch, or the first
/// per-embedding validation failure.
pub fn validate_batch(texts: &[String], embeddings: &[Embedding]) -> Result<()> {
    if embeddings.len() != texts.len() {
        return Err(EmbeddingError::Provider(format!(
            "batch length mismatch: {} inputs, {} embeddings",
            texts.len(),
            embeddings.len()
        )));
    }
    for embedding in embeddings {
        validate_embedding(embedding)?;
    }
    Ok(())
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
    async fn default_embed_batch_loops_in_order_and_validates() {
        let provider = ConstantProvider;
        let texts = vec!["alpha".to_string(), "beta".to_string()];
        let embeddings = provider
            .embed_batch(&texts)
            .await
            .unwrap_or_else(|error| panic!("batch should embed: {error}"));
        assert_eq!(embeddings.len(), 2);
        assert!(embeddings.iter().all(|e| e.model_id == "constant"));

        // An empty input inside the batch fails through the per-text path.
        assert!(
            provider
                .embed_batch(&["ok".to_string(), String::new()])
                .await
                .is_err()
        );
        // The shared contract validator rejects length mismatches and bad vectors.
        let good = Embedding {
            model_id: "constant".to_string(),
            vector: vec![1.0],
        };
        assert!(
            validate_batch(&texts, std::slice::from_ref(&good)).is_err(),
            "mismatch"
        );
        assert!(
            validate_batch(
                &texts,
                &[
                    good,
                    Embedding {
                        model_id: "constant".to_string(),
                        vector: vec![f32::NAN],
                    },
                ],
            )
            .is_err(),
            "non-finite member"
        );
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
