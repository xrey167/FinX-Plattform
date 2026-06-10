#![forbid(unsafe_code)]

pub mod fallback;
pub mod router;
pub use fallback::FallbackModel;
pub use router::{CredentialProbe, ModelFactory, ModelRef, ModelRouter, parse_model_ref};

pub mod reasoning;
pub use reasoning::{
    ReasoningConfig, ReasoningLevel, ReasoningParams, clamp_temperature, parse_directive, resolve,
    to_params,
};

use serde::{Deserialize, Serialize};
use tdw_config::TdwConfig;
use thiserror::Error;

pub type Result<T> = std::result::Result<T, LlmError>;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum LlmError {
    #[error("chat request has no messages")]
    EmptyMessages,
    #[error("chat message content is empty")]
    EmptyMessageContent,
    #[error("chat request max_output_tokens must be greater than zero")]
    EmptyMaxOutputTokens,
    #[error("model id must not be empty")]
    EmptyModelId,
    #[error("model id contains unsupported control characters")]
    InvalidModelId,
    #[error("base url must start with http:// or https://")]
    InvalidBaseUrl,
    #[error("base url contains whitespace or control characters")]
    UnsafeBaseUrl,
    #[error("unsupported provider: {0}")]
    UnsupportedProvider(String),
    #[error("fallback chain must contain at least one model")]
    EmptyFallbackChain,
    #[error("model ref must be of the form <provider>/<model>")]
    InvalidModelRef,
    #[error("no credentials available for provider: {0}")]
    MissingCredentials(String),
    #[error("no eligible model: every candidate was unregistered or missing credentials")]
    NoEligibleModel,
}

/// Classification of an error's recoverability for retry decisions.
///
/// Callers consult this (via [`ClassifyError::retryability`]) to decide
/// whether a failed LLM request is worth re-attempting. Nothing in this
/// crate acts on the classification automatically; it is purely advisory
/// metadata so retry/backoff policy can live in the caller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Retryability {
    /// A transient failure (e.g. a transport hiccup or an overload /
    /// rate-limit response) that is reasonable to retry with backoff.
    Transient,
    /// A permanent failure (e.g. a malformed request or auth error) that
    /// will not succeed on retry without changing the input. This is the
    /// safe default for anything that cannot be confidently classified.
    Permanent,
    /// The request exceeded the model's context/length budget.
    ///
    /// Detection is heuristic and provider-wording-dependent: it relies on
    /// scanning a 4xx error body for context/length markers (see
    /// [`is_context_overflow_body`]). Unknown or ambiguous bodies are NOT
    /// classified here; they fall back to [`Retryability::Permanent`]. A
    /// caller may treat this as recoverable by, for example, trimming the
    /// prompt before retrying.
    ContextOverflow,
}

/// Recoverability classification for an LLM error type.
///
/// Implemented for [`LlmError`] in this crate and for the provider HTTP
/// error types in the `tdw-llm-anthropic` / `tdw-llm-openai-compat`
/// crates. The implementation is additive: it adds a query method and
/// changes no existing behavior.
pub trait ClassifyError {
    /// Classify this error's recoverability.
    fn retryability(&self) -> Retryability;
}

/// HTTP status codes that are treated as transient (worth retrying).
///
/// Covers request timeout (408), conflict (409), too-many-requests (429),
/// and the common 5xx server/gateway failures (500/502/503/504). Shared by
/// the provider HTTP error impls so the set is defined in exactly one place.
#[must_use]
pub const fn is_transient_http_status(status: u16) -> bool {
    matches!(status, 408 | 409 | 429 | 500 | 502 | 503 | 504)
}

/// Heuristically detect whether a 4xx error body describes a context /
/// length-budget overflow.
///
/// This is a conservative, provider-wording-dependent substring scan: it
/// reports `true` only when the body mentions `context` together with
/// either `length` or `token` (case-insensitive). Because provider error
/// wording varies, any body that does not clearly match is left
/// unclassified, and callers should fall back to
/// [`Retryability::Permanent`] for it. Keeping `Permanent` as the fallback
/// makes a false negative the safe direction.
#[must_use]
pub fn is_context_overflow_body(body: &str) -> bool {
    let body = body.to_ascii_lowercase();
    body.contains("context") && (body.contains("length") || body.contains("token"))
}

/// Classify an HTTP failure given its status code and (for 4xx) response
/// body, using the shared status set and context-overflow heuristic.
///
/// Shared by the provider HTTP error impls so the substring scan and the
/// transient-status set are never duplicated per provider:
/// - transient statuses (see [`is_transient_http_status`]) ->
///   [`Retryability::Transient`];
/// - a 4xx body matching [`is_context_overflow_body`] ->
///   [`Retryability::ContextOverflow`];
/// - anything else -> [`Retryability::Permanent`] (the safe default).
#[must_use]
pub fn classify_http_status(status: u16, body: &str) -> Retryability {
    if is_transient_http_status(status) {
        return Retryability::Transient;
    }
    if (400..500).contains(&status) && is_context_overflow_body(body) {
        return Retryability::ContextOverflow;
    }
    Retryability::Permanent
}

impl ClassifyError for LlmError {
    /// All current [`LlmError`] variants are local validation failures, so
    /// they are permanent: retrying without changing the input cannot help.
    fn retryability(&self) -> Retryability {
        Retryability::Permanent
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageRole {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: MessageRole,
    pub content: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatRequest {
    pub messages: Vec<ChatMessage>,
    pub max_output_tokens: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatResponse {
    pub model_id: String,
    pub message: ChatMessage,
    pub usage: Usage,
}

pub trait LanguageModel: Send + Sync {
    fn model_id(&self) -> &str;
    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    fn complete(&self, request: ChatRequest) -> Result<ChatResponse>;
}

/// Token-level streaming extension over [`LanguageModel`] (story G016).
///
/// `complete_streaming` mirrors the synchronous [`LanguageModel::complete`]
/// shape — it blocks until the full answer is produced and returns the same
/// terminal [`ChatResponse`] (full text + [`Usage`]) — but additionally
/// invokes `on_chunk` for each incremental text fragment as it arrives, so a
/// downstream consumer (the `OpenBB` Workspace agent bridge, the TUI, or an MCP
/// progress channel) can render output token-by-token.
///
/// # Object safety
///
/// The callback is taken as `&mut dyn FnMut(&str)` rather than a generic
/// `impl FnMut(&str)`, and the method has no other type parameters, so the
/// trait stays object-safe: a model can live behind `Box<dyn
/// StreamingLanguageModel>` / `Arc<dyn StreamingLanguageModel>` exactly as it
/// does behind `dyn LanguageModel`. The trait is **not** blanket-implemented,
/// so a concrete adapter is free to provide a native streaming implementation
/// (e.g. one consuming a provider SSE stream) while every other model inherits
/// the protocol-conformant default below.
///
/// The default implementation calls [`LanguageModel::complete`] and emits the
/// entire answer as a single `on_chunk` call. This is protocol-conformant: a
/// well-behaved chunk handler that concatenates every fragment reconstructs
/// the final [`ChatResponse::message`] content verbatim, whether the model
/// streamed one chunk or many.
pub trait StreamingLanguageModel: LanguageModel {
    /// Complete `request`, invoking `on_chunk` for each incremental text
    /// fragment, and return the terminal [`ChatResponse`] carrying the full
    /// text and [`Usage`].
    ///
    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    fn complete_streaming(
        &self,
        request: &ChatRequest,
        on_chunk: &mut dyn FnMut(&str),
    ) -> Result<ChatResponse> {
        let response = self.complete(request.clone())?;
        on_chunk(&response.message.content);
        Ok(response)
    }
}

/// Forwarding impl so a boxed [`LanguageModel`] also satisfies the streaming
/// extension (via the default single-chunk fallback), keeping
/// `Arc<dyn LanguageModel>` usable wherever streaming is offered.
impl StreamingLanguageModel for std::sync::Arc<dyn LanguageModel> {}

/// Split `text` into at most `max_chunks` contiguous, non-empty pieces whose
/// concatenation is exactly `text`.
///
/// Used by the deterministic offline models to fabricate a multi-chunk stream
/// without a network round-trip, so downstream chunk-handling logic is
/// exercisable in offline tests. The split happens on UTF-8 character
/// boundaries at roughly even offsets; an empty input yields no chunks and
/// `max_chunks == 0` is treated as `1`. The invariant `chunks.concat() == text`
/// always holds, matching the streaming protocol's reconstruction contract.
#[must_use]
pub fn split_into_chunks(text: &str, max_chunks: usize) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }
    let max_chunks = max_chunks.max(1);
    let boundaries: Vec<usize> = text
        .char_indices()
        .map(|(index, _)| index)
        .chain(std::iter::once(text.len()))
        .collect();
    // Number of characters; boundaries has char_count + 1 entries.
    let char_count = boundaries.len() - 1;
    let chunk_count = max_chunks.min(char_count);
    let mut chunks = Vec::with_capacity(chunk_count);
    for chunk_index in 0..chunk_count {
        let start_char = chunk_index * char_count / chunk_count;
        let end_char = (chunk_index + 1) * char_count / chunk_count;
        let start = boundaries[start_char];
        let end = boundaries[end_char];
        if start < end {
            chunks.push(text[start..end].to_string());
        }
    }
    chunks
}

/// Blanket forwarding impl so a boxed [`LanguageModel`] (e.g. an element of a
/// `Vec<Arc<dyn LanguageModel>>` produced by [`router::ModelRouter::resolve_chain`])
/// can itself be used wherever a `LanguageModel` is required, such as in a
/// [`FallbackModel`] chain.
impl LanguageModel for std::sync::Arc<dyn LanguageModel> {
    fn model_id(&self) -> &str {
        (**self).model_id()
    }

    fn complete(&self, request: ChatRequest) -> Result<ChatResponse> {
        (**self).complete(request)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelSelection {
    pub provider: String,
    pub model: String,
    pub base_url: Option<String>,
}

impl ModelSelection {
    #[must_use]
    pub fn from_config(config: &TdwConfig) -> Self {
        Self {
            provider: config.model.provider.clone(),
            model: config.model.model.clone(),
            base_url: config.model.base_url.clone(),
        }
    }
}

/// # Errors
///
/// Returns an error variant if the underlying operation fails.
pub fn last_user_message(request: &ChatRequest) -> Result<&str> {
    validate_chat_request(request)?;
    request
        .messages
        .iter()
        .rev()
        .find(|message| message.role == MessageRole::User)
        .map(|message| message.content.as_str())
        .ok_or(LlmError::EmptyMessages)
}

/// # Errors
///
/// Returns an error variant if the underlying operation fails.
pub fn validate_chat_request(request: &ChatRequest) -> Result<()> {
    if request.messages.is_empty() {
        return Err(LlmError::EmptyMessages);
    }
    if request.max_output_tokens == 0 {
        return Err(LlmError::EmptyMaxOutputTokens);
    }
    if request
        .messages
        .iter()
        .any(|message| message.content.trim().is_empty())
    {
        return Err(LlmError::EmptyMessageContent);
    }
    Ok(())
}

/// # Errors
///
/// Returns an error variant if the underlying operation fails.
pub fn validate_model_id(model_id: &str) -> Result<()> {
    let model_id = model_id.trim();
    if model_id.is_empty() {
        return Err(LlmError::EmptyModelId);
    }
    if model_id.chars().any(char::is_control) {
        return Err(LlmError::InvalidModelId);
    }
    Ok(())
}

/// # Errors
///
/// Returns an error variant if the underlying operation fails.
pub fn validate_base_url(base_url: &str) -> Result<()> {
    if base_url
        .chars()
        .any(|character| character.is_whitespace() || character.is_control())
    {
        return Err(LlmError::UnsafeBaseUrl);
    }
    if !(base_url.starts_with("http://") || base_url.starts_with("https://")) {
        return Err(LlmError::InvalidBaseUrl);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct EchoModel;

    impl LanguageModel for EchoModel {
        fn model_id(&self) -> &'static str {
            "echo"
        }

        fn complete(&self, request: ChatRequest) -> Result<ChatResponse> {
            let content = last_user_message(&request)?.to_string();
            Ok(ChatResponse {
                model_id: self.model_id().to_string(),
                message: ChatMessage {
                    role: MessageRole::Assistant,
                    content,
                },
                usage: Usage {
                    input_tokens: 1,
                    output_tokens: 1,
                },
            })
        }
    }

    impl StreamingLanguageModel for EchoModel {}

    #[test]
    fn default_streaming_emits_whole_answer_as_single_chunk() {
        let model = EchoModel;
        let request = ChatRequest {
            messages: vec![ChatMessage {
                role: MessageRole::User,
                content: "hello world".to_string(),
            }],
            max_output_tokens: 32,
        };
        let mut chunks = Vec::new();
        let response = model
            .complete_streaming(&request, &mut |chunk| chunks.push(chunk.to_string()))
            .unwrap_or_else(|error| panic!("streaming completes: {error}"));

        // Exactly one chunk, equal to the full answer (the protocol-conformant
        // fallback), and the terminal response matches the non-streaming path.
        assert_eq!(chunks, vec!["hello world".to_string()]);
        assert_eq!(chunks.concat(), response.message.content);
        assert_eq!(
            response,
            model
                .complete(request)
                .unwrap_or_else(|error| panic!("complete: {error}"))
        );
    }

    #[test]
    fn split_into_chunks_preserves_text_and_bounds_count() {
        // Multi-chunk split: concatenation is verbatim, count is bounded.
        let chunks = split_into_chunks("Hello, world!", 3);
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks.concat(), "Hello, world!");

        // Requesting more chunks than characters caps at the character count;
        // every chunk stays non-empty.
        let many = split_into_chunks("abc", 10);
        assert_eq!(many, vec!["a", "b", "c"]);
        assert!(many.iter().all(|chunk| !chunk.is_empty()));

        // Empty input yields no chunks; max_chunks == 0 is treated as 1.
        assert!(split_into_chunks("", 4).is_empty());
        assert_eq!(split_into_chunks("xyz", 0), vec!["xyz".to_string()]);

        // UTF-8 boundaries are respected (no panic, verbatim reconstruction).
        let unicode = split_into_chunks("café déjà", 4);
        assert_eq!(unicode.concat(), "café déjà");
    }

    #[test]
    fn default_streaming_works_behind_dyn_language_model() {
        let model: std::sync::Arc<dyn LanguageModel> = std::sync::Arc::new(EchoModel);
        let request = ChatRequest {
            messages: vec![ChatMessage {
                role: MessageRole::User,
                content: "boxed".to_string(),
            }],
            max_output_tokens: 8,
        };
        let mut chunks = Vec::new();
        let response = model
            .complete_streaming(&request, &mut |chunk| chunks.push(chunk.to_string()))
            .unwrap_or_else(|error| panic!("streaming completes: {error}"));
        assert_eq!(chunks, vec!["boxed".to_string()]);
        assert_eq!(response.message.content, "boxed");
    }

    #[test]
    fn model_trait_completes_chat_request() {
        let model = EchoModel;
        let response = model
            .complete(ChatRequest {
                messages: vec![ChatMessage {
                    role: MessageRole::User,
                    content: "hello".to_string(),
                }],
                max_output_tokens: 32,
            })
            .unwrap_or_else(|error| panic!("model completes: {error}"));

        assert_eq!(response.model_id, "echo");
        assert_eq!(response.message.content, "hello");
        assert_eq!(
            last_user_message(&ChatRequest {
                messages: vec![
                    ChatMessage {
                        role: MessageRole::System,
                        content: "policy".to_string(),
                    },
                    ChatMessage {
                        role: MessageRole::User,
                        content: "latest".to_string(),
                    },
                ],
                max_output_tokens: 8,
            }),
            Ok("latest")
        );
        assert!(
            model
                .complete(ChatRequest {
                    messages: vec![ChatMessage {
                        role: MessageRole::User,
                        content: "hello".to_string(),
                    }],
                    max_output_tokens: 0,
                })
                .is_err()
        );
        assert!(validate_model_id("bad\nmodel").is_err());
        assert!(validate_base_url("https://api.example.com/v1").is_ok());
        assert!(validate_base_url("https://api.example.com/v1 token").is_err());
    }

    #[test]
    fn validation_reports_each_remaining_error_variant() {
        assert_eq!(
            validate_chat_request(&ChatRequest {
                messages: vec![],
                max_output_tokens: 8,
            }),
            Err(LlmError::EmptyMessages)
        );
        assert_eq!(
            validate_chat_request(&ChatRequest {
                messages: vec![ChatMessage {
                    role: MessageRole::User,
                    content: "  ".to_string(),
                }],
                max_output_tokens: 8,
            }),
            Err(LlmError::EmptyMessageContent)
        );

        assert_eq!(validate_model_id("   "), Err(LlmError::EmptyModelId));

        // A non-http(s) scheme without whitespace is InvalidBaseUrl, distinct from
        // the whitespace/control UnsafeBaseUrl path.
        assert_eq!(
            validate_base_url("ftp://api.example.com"),
            Err(LlmError::InvalidBaseUrl)
        );

        // No user turn present -> EmptyMessages from last_user_message.
        assert_eq!(
            last_user_message(&ChatRequest {
                messages: vec![ChatMessage {
                    role: MessageRole::System,
                    content: "policy".to_string(),
                }],
                max_output_tokens: 8,
            }),
            Err(LlmError::EmptyMessages)
        );
    }

    #[test]
    fn llm_error_variants_are_permanent() {
        assert_eq!(
            LlmError::EmptyMessages.retryability(),
            Retryability::Permanent
        );
        assert_eq!(
            LlmError::UnsupportedProvider("bogus".to_string()).retryability(),
            Retryability::Permanent
        );
    }

    #[test]
    fn transient_status_set_matches_expected_codes() {
        for status in [408, 409, 429, 500, 502, 503, 504] {
            assert!(
                is_transient_http_status(status),
                "{status} should be transient"
            );
        }
        for status in [400, 401, 403, 404, 422, 200] {
            assert!(
                !is_transient_http_status(status),
                "{status} should not be transient"
            );
        }
    }

    #[test]
    fn classify_http_status_maps_429_to_transient() {
        assert_eq!(classify_http_status(429, ""), Retryability::Transient);
    }

    #[test]
    fn classify_http_status_maps_400_to_permanent() {
        assert_eq!(
            classify_http_status(400, "bad request"),
            Retryability::Permanent
        );
    }

    #[test]
    fn classify_http_status_detects_context_overflow_body() {
        // Synthetic provider-style body mixing case and wording.
        let body = r#"{"error":{"message":"This model's maximum CONTEXT length is 8192 tokens"}}"#;
        assert!(is_context_overflow_body(body));
        assert_eq!(
            classify_http_status(400, body),
            Retryability::ContextOverflow
        );
    }

    #[test]
    fn ambiguous_4xx_body_falls_back_to_permanent() {
        assert!(!is_context_overflow_body("unauthorized"));
        assert_eq!(
            classify_http_status(400, "unauthorized"),
            Retryability::Permanent
        );
    }
}
