#![forbid(unsafe_code)]
#![deny(clippy::pedantic, clippy::nursery)]
use tdw_embed::{Embedding, EmbeddingError, EmbeddingProvider, Result};

#[cfg(feature = "model")]
mod model;
#[cfg(feature = "model")]
pub use model::LocalModelEmbeddingProvider;

#[derive(Clone, Debug)]
pub struct HashEmbeddingProvider {
    model_id: String,
    dimensions: usize,
}

impl Default for HashEmbeddingProvider {
    fn default() -> Self {
        Self {
            model_id: "local-hash-8".to_string(),
            dimensions: 8,
        }
    }
}

impl HashEmbeddingProvider {
    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub fn new(model_id: impl Into<String>, dimensions: usize) -> Result<Self> {
        let model_id = model_id.into();
        if model_id.trim().is_empty() {
            return Err(EmbeddingError::EmptyModelId);
        }
        if dimensions == 0 {
            return Err(EmbeddingError::InvalidDimensions);
        }
        Ok(Self {
            model_id,
            dimensions,
        })
    }
}

#[async_trait::async_trait]
impl EmbeddingProvider for HashEmbeddingProvider {
    fn model_id(&self) -> &str {
        &self.model_id
    }

    async fn embed(&self, text: &str) -> Result<Embedding> {
        if text.trim().is_empty() {
            return Err(EmbeddingError::EmptyInput);
        }

        let mut vector = vec![0.0; self.dimensions];
        for (index, byte) in text.bytes().enumerate() {
            let slot = index % self.dimensions;
            vector[slot] += f32::from(byte) / 255.0;
        }
        Ok(Embedding {
            model_id: self.model_id.clone(),
            vector,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn hash_embedding_is_deterministic() {
        let provider = HashEmbeddingProvider::default();

        let first = provider
            .embed("macro research")
            .await
            .unwrap_or_else(|error| panic!("embedding should succeed: {error}"));
        let second = provider
            .embed("macro research")
            .await
            .unwrap_or_else(|error| panic!("embedding should succeed: {error}"));

        assert_eq!(first, second);
        assert_eq!(first.vector.len(), 8);
        assert!(tdw_embed::validate_embedding(&first).is_ok());
        assert!(HashEmbeddingProvider::new("", 8).is_err());
        assert!(HashEmbeddingProvider::new("bad", 0).is_err());
    }
}
