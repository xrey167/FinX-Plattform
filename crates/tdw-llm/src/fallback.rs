//! Provider-fallback wrapper for [`LanguageModel`].
//!
//! [`FallbackModel`] delegates every completion to a primary model and,
//! on any error, transparently retries with a secondary model while
//! emitting a `tracing::warn!` so operators can observe the switch.

use crate::{ChatRequest, ChatResponse, LanguageModel, Result};

/// A [`LanguageModel`] that falls back to a secondary provider on primary error.
///
/// The model is generic over `P` (primary) and `S` (secondary), both of which
/// must implement [`LanguageModel`].  When `primary.complete` returns `Err`,
/// a warning is emitted via `tracing` and the same request is forwarded to
/// `secondary`.  The secondary's result — `Ok` or `Err` — is surfaced as-is.
///
/// # Example
///
/// ```rust
/// use tdw_llm::{FallbackModel, LanguageModel, ChatRequest, ChatMessage, MessageRole};
///
/// // Any two LanguageModel implementors can be composed.
/// // (Using the doc-only pattern; real usage wires concrete adapters.)
/// ```
pub struct FallbackModel<P, S> {
    primary: P,
    secondary: S,
}

impl<P, S> FallbackModel<P, S>
where
    P: LanguageModel,
    S: LanguageModel,
{
    /// Construct a new [`FallbackModel`] from a primary and a secondary model.
    #[must_use]
    pub fn new(primary: P, secondary: S) -> Self {
        Self { primary, secondary }
    }
}

impl<P, S> LanguageModel for FallbackModel<P, S>
where
    P: LanguageModel,
    S: LanguageModel,
{
    fn model_id(&self) -> &str {
        self.primary.model_id()
    }

    /// # Errors
    ///
    /// Returns the secondary model's error if both primary and secondary fail.
    fn complete(&self, request: ChatRequest) -> Result<ChatResponse> {
        match self.primary.complete(request.clone()) {
            Ok(response) => Ok(response),
            Err(primary_err) => {
                tracing::warn!(
                    primary = self.primary.model_id(),
                    secondary = self.secondary.model_id(),
                    error = %primary_err,
                    "primary model failed; engaging fallback",
                );
                self.secondary.complete(request)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ChatMessage, LlmError, MessageRole, Usage};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A stub model that returns a preset result and counts how many times
    /// `complete` was called.
    struct StubModel {
        id: &'static str,
        call_count: Arc<AtomicUsize>,
        /// When `Some(err)` the next call returns that error; when `None` it succeeds.
        fail: Option<LlmError>,
    }

    impl StubModel {
        fn succeeding(id: &'static str) -> (Self, Arc<AtomicUsize>) {
            let count = Arc::new(AtomicUsize::new(0));
            (
                Self {
                    id,
                    call_count: Arc::clone(&count),
                    fail: None,
                },
                count,
            )
        }

        fn failing(id: &'static str, err: LlmError) -> (Self, Arc<AtomicUsize>) {
            let count = Arc::new(AtomicUsize::new(0));
            (
                Self {
                    id,
                    call_count: Arc::clone(&count),
                    fail: Some(err),
                },
                count,
            )
        }
    }

    impl LanguageModel for StubModel {
        fn model_id(&self) -> &str {
            self.id
        }

        fn complete(&self, _request: ChatRequest) -> Result<ChatResponse> {
            self.call_count.fetch_add(1, Ordering::SeqCst);
            match &self.fail {
                None => Ok(ChatResponse {
                    model_id: self.id.to_string(),
                    message: ChatMessage {
                        role: MessageRole::Assistant,
                        content: format!("{} response", self.id),
                    },
                    usage: Usage {
                        input_tokens: 1,
                        output_tokens: 1,
                    },
                }),
                Some(err) => Err(match err {
                    LlmError::EmptyMessages => LlmError::EmptyMessages,
                    LlmError::EmptyMessageContent => LlmError::EmptyMessageContent,
                    LlmError::EmptyMaxOutputTokens => LlmError::EmptyMaxOutputTokens,
                    LlmError::EmptyModelId => LlmError::EmptyModelId,
                    LlmError::InvalidModelId => LlmError::InvalidModelId,
                    LlmError::InvalidBaseUrl => LlmError::InvalidBaseUrl,
                    LlmError::UnsafeBaseUrl => LlmError::UnsafeBaseUrl,
                    LlmError::UnsupportedProvider(p) => LlmError::UnsupportedProvider(p.clone()),
                }),
            }
        }
    }

    fn make_request() -> ChatRequest {
        ChatRequest {
            messages: vec![ChatMessage {
                role: MessageRole::User,
                content: "test prompt".to_string(),
            }],
            max_output_tokens: 32,
        }
    }

    /// (a) When the primary succeeds, the secondary is never invoked.
    #[test]
    fn primary_success_never_touches_secondary() {
        let (primary, primary_count) = StubModel::succeeding("primary");
        let (secondary, secondary_count) = StubModel::succeeding("secondary");
        let model = FallbackModel::new(primary, secondary);

        let response = model
            .complete(make_request())
            .expect("primary succeeds so fallback should return Ok");

        assert_eq!(
            primary_count.load(Ordering::SeqCst),
            1,
            "primary called once"
        );
        assert_eq!(
            secondary_count.load(Ordering::SeqCst),
            0,
            "secondary must not be called when primary succeeds"
        );
        assert_eq!(
            response.model_id, "primary",
            "response comes from the primary model"
        );
    }

    /// (b) When the primary fails, the secondary is invoked and its Ok result is returned.
    #[test]
    fn primary_failure_serves_from_secondary() {
        let (primary, primary_count) = StubModel::failing("primary", LlmError::EmptyMessages);
        let (secondary, secondary_count) = StubModel::succeeding("secondary");
        let model = FallbackModel::new(primary, secondary);

        let response = model
            .complete(make_request())
            .expect("secondary succeeds so fallback should return Ok");

        assert_eq!(
            primary_count.load(Ordering::SeqCst),
            1,
            "primary called once"
        );
        assert_eq!(
            secondary_count.load(Ordering::SeqCst),
            1,
            "secondary called once after primary fails"
        );
        assert_eq!(
            response.model_id, "secondary",
            "response comes from the secondary model"
        );
    }

    /// (c) When both primary and secondary fail, the secondary's error is surfaced.
    #[test]
    fn both_fail_surfaces_secondary_error() {
        let (primary, _) = StubModel::failing("primary", LlmError::EmptyMessages);
        let (secondary, secondary_count) = StubModel::failing(
            "secondary",
            LlmError::UnsupportedProvider("none".to_string()),
        );
        let model = FallbackModel::new(primary, secondary);

        let err = model
            .complete(make_request())
            .expect_err("both models fail so fallback should return Err");

        assert_eq!(
            secondary_count.load(Ordering::SeqCst),
            1,
            "secondary was attempted"
        );
        assert_eq!(
            err,
            LlmError::UnsupportedProvider("none".to_string()),
            "secondary error is surfaced"
        );
    }
}
