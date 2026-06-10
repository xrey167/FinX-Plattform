//! Real local sentence-embedding provider (knowledge-system B6).
//!
//! Loads a `BERT`-family model (for example `bge-small-en-v1.5` or
//! `multilingual-e5-small`) from a local directory and runs it fully offline
//! via pure-Rust `candle` — no network, no auto-download. Construction reads
//! `config.json`, `tokenizer.json`, and `model.safetensors` from the directory;
//! [`LocalModelEmbeddingProvider::embed`] tokenizes, runs the forward pass,
//! mean-pools over the attention mask, and `L2`-normalizes the result.

use std::path::Path;

use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config};
use tdw_embed::{Embedding, EmbeddingError, EmbeddingProvider, Result};
use tokenizers::Tokenizer;

/// A local `BERT`-family sentence-embedding provider backed by `candle`.
///
/// Holds the tokenizer, the loaded model, and the resolved maximum sequence
/// length. `embed` truncates input to the model's max position embeddings,
/// runs a forward pass on the CPU, mean-pools over the attention mask, and
/// `L2`-normalizes. The struct is `Send + Sync` (both `Tokenizer` and
/// `BertModel` are `Send + Sync`), satisfying the [`EmbeddingProvider`] bound.
pub struct LocalModelEmbeddingProvider {
    model_id: String,
    tokenizer: Tokenizer,
    model: BertModel,
    device: Device,
    max_len: usize,
}

impl std::fmt::Debug for LocalModelEmbeddingProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalModelEmbeddingProvider")
            .field("model_id", &self.model_id)
            .field("max_len", &self.max_len)
            .finish_non_exhaustive()
    }
}

impl LocalModelEmbeddingProvider {
    /// Loads a `BERT`-family embedding model from a local directory.
    ///
    /// Reads `config.json` (the `BERT` config; `hidden_size` is the embedding
    /// dimension), `tokenizer.json`, and `model.safetensors` from `model_dir`.
    /// Weights load as `F32` on the CPU device. The default `model_id` is the
    /// directory's file name; override it with
    /// [`LocalModelEmbeddingProvider::with_model_id`].
    ///
    /// # Errors
    ///
    /// Returns [`EmbeddingError::Provider`] if the directory is missing, a
    /// required file (`config.json`, `tokenizer.json`, `model.safetensors`) is
    /// absent or malformed, or model construction fails.
    pub fn from_dir(model_dir: impl AsRef<Path>) -> Result<Self> {
        let dir = model_dir.as_ref();
        if !dir.is_dir() {
            return Err(EmbeddingError::Provider(format!(
                "model directory does not exist: {}",
                dir.display()
            )));
        }

        let model_id = dir
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("local-model")
            .to_string();

        let config_path = dir.join("config.json");
        let tokenizer_path = dir.join("tokenizer.json");
        let weights_path = dir.join("model.safetensors");

        let config_bytes = std::fs::read(&config_path).map_err(|error| {
            EmbeddingError::Provider(format!("failed to read {}: {error}", config_path.display()))
        })?;
        let config: Config = serde_json::from_slice(&config_bytes).map_err(|error| {
            EmbeddingError::Provider(format!(
                "failed to parse {}: {error}",
                config_path.display()
            ))
        })?;

        let tokenizer = Tokenizer::from_file(&tokenizer_path).map_err(|error| {
            EmbeddingError::Provider(format!(
                "failed to load {}: {error}",
                tokenizer_path.display()
            ))
        })?;

        if !weights_path.is_file() {
            return Err(EmbeddingError::Provider(format!(
                "missing weights file: {}",
                weights_path.display()
            )));
        }

        let device = Device::Cpu;
        // SAFETY note: `from_mmaped_safetensors` is `unsafe` because it mmaps the
        // file; this crate forbids `unsafe`, so use the owned-buffer loader.
        let weights = candle_core::safetensors::load(&weights_path, &device).map_err(|error| {
            EmbeddingError::Provider(format!(
                "failed to read weights {}: {error}",
                weights_path.display()
            ))
        })?;
        let var_builder = VarBuilder::from_tensors(weights, DType::F32, &device);

        let model = BertModel::load(var_builder, &config).map_err(|error| {
            EmbeddingError::Provider(format!("failed to build bert model: {error}"))
        })?;

        let max_len = config.max_position_embeddings;

        Ok(Self {
            model_id,
            tokenizer,
            model,
            device,
            max_len,
        })
    }

    /// Overrides the reported `model_id` (defaults to the directory name).
    #[must_use]
    pub fn with_model_id(mut self, model_id: impl Into<String>) -> Self {
        self.model_id = model_id.into();
        self
    }

    /// Runs tokenization, the forward pass, mean pooling, and `L2` normalization.
    ///
    /// All of the work here is synchronous CPU compute; the async wrapper in
    /// [`EmbeddingProvider::embed`] uses `block_in_place` so it does not stall
    /// the tokio runtime.
    fn embed_blocking(&self, text: &str) -> Result<Vec<f32>> {
        let mut encoding = self
            .tokenizer
            .encode(text, true)
            .map_err(|error| EmbeddingError::Provider(format!("tokenize failed: {error}")))?;
        encoding.truncate(self.max_len, 0, tokenizers::TruncationDirection::Right);

        let ids = encoding.get_ids();
        let mask = encoding.get_attention_mask();
        if ids.is_empty() {
            return Err(EmbeddingError::EmptyInput);
        }

        let token_ids = Tensor::new(ids, &self.device)
            .and_then(|tensor| tensor.unsqueeze(0))
            .map_err(|error| map_candle(&error))?;
        let attention_mask = Tensor::new(mask, &self.device)
            .and_then(|tensor| tensor.unsqueeze(0))
            .map_err(|error| map_candle(&error))?;
        let token_type_ids = token_ids.zeros_like().map_err(|error| map_candle(&error))?;

        let hidden = self
            .model
            .forward(&token_ids, &token_type_ids, Some(&attention_mask))
            .map_err(|error| map_candle(&error))?;

        let pooled = mean_pool(&hidden, &attention_mask)?;
        let normalized = l2_normalize(&pooled)?;

        normalized
            .squeeze(0)
            .and_then(|tensor| tensor.to_vec1::<f32>())
            .map_err(|error| map_candle(&error))
    }
}

/// Mean-pools `hidden` (shape `[1, seq, hidden]`) over the attention `mask`.
fn mean_pool(hidden: &Tensor, mask: &Tensor) -> Result<Tensor> {
    // mask: [1, seq] -> [1, seq, 1] broadcast over the hidden dimension.
    let mask = mask
        .to_dtype(DType::F32)
        .and_then(|tensor| tensor.unsqueeze(2))
        .map_err(|error| map_candle(&error))?;
    let masked = hidden
        .broadcast_mul(&mask)
        .map_err(|error| map_candle(&error))?;
    let summed = masked.sum(1).map_err(|error| map_candle(&error))?;
    let counts = mask
        .sum(1)
        .and_then(|tensor| tensor.clamp(1e-9_f64, f64::INFINITY))
        .map_err(|error| map_candle(&error))?;
    summed
        .broadcast_div(&counts)
        .map_err(|error| map_candle(&error))
}

/// `L2`-normalizes the pooled embedding (shape `[1, hidden]`).
fn l2_normalize(pooled: &Tensor) -> Result<Tensor> {
    let norm = pooled
        .sqr()
        .and_then(|tensor| tensor.sum_keepdim(1))
        .and_then(|tensor| tensor.sqrt())
        .and_then(|tensor| tensor.clamp(1e-12_f64, f64::INFINITY))
        .map_err(|error| map_candle(&error))?;
    pooled
        .broadcast_div(&norm)
        .map_err(|error| map_candle(&error))
}

/// Maps a `candle` error into the existing [`EmbeddingError::Provider`] variant.
fn map_candle(error: &candle_core::Error) -> EmbeddingError {
    EmbeddingError::Provider(format!("candle inference failed: {error}"))
}

#[async_trait::async_trait]
impl EmbeddingProvider for LocalModelEmbeddingProvider {
    fn model_id(&self) -> &str {
        &self.model_id
    }

    async fn embed(&self, text: &str) -> Result<Embedding> {
        if text.trim().is_empty() {
            return Err(EmbeddingError::EmptyInput);
        }
        // `BertModel::forward` is synchronous CPU work; `block_in_place` moves it
        // off the async poll path so it does not starve the tokio runtime.
        let vector = tokio::task::block_in_place(|| self.embed_blocking(text))?;
        Ok(Embedding {
            model_id: self.model_id.clone(),
            vector,
        })
    }
}
