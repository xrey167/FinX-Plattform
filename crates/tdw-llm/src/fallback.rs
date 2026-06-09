//! Opt-in retry-then-fallback wrapper for [`LanguageModel`].
//!
//! [`FallbackModel`] holds an ordered chain of models and serves a completion
//! from the first one that succeeds. Each model is attempted up to
//! `max_attempts_per_model` times; whether a failed attempt is retried (on the
//! same model) or whether the chain advances to the next model is decided by an
//! opt-in `should_retry` classifier.
//!
//! # Default behavior
//!
//! The default `should_retry` closure is a strict no-op (`|_| false`). With the
//! default, the chain tries each model exactly once, in order, advancing on any
//! `Err`, and returns the last error if every model fails. This keeps existing
//! single-model callers byte-for-byte unaffected.
//!
//! # Concurrency and blocking
//!
//! The wrapper holds no mutable state: the actually-answering model is conveyed
//! by the returned [`ChatResponse::model_id`]. No code path here sleeps or
//! blocks; backoff, if ever wanted, must be a separate opt-in.

use std::sync::Arc;

use crate::{ChatRequest, ChatResponse, LanguageModel, LlmError, Result};

/// Predicate deciding whether a failed attempt should be retried on the same
/// model before the chain advances to the next model.
type ShouldRetry = fn(&LlmError) -> bool;

/// A [`LanguageModel`] that retries each model then falls back across a chain.
///
/// Construct with [`FallbackModel::new`] (errors on an empty chain) and
/// optionally tune retries with [`FallbackModel::with_max_attempts`] and
/// classification with [`FallbackModel::with_should_retry`].
///
/// # Example
///
/// ```rust
/// use std::sync::Arc;
/// use tdw_llm::{FallbackModel, LanguageModel};
/// # use tdw_llm::{ChatRequest, ChatResponse, Result};
/// # struct Dummy(&'static str);
/// # impl LanguageModel for Dummy {
/// #     fn model_id(&self) -> &str { self.0 }
/// #     fn complete(&self, _r: ChatRequest) -> Result<ChatResponse> {
/// #         Err(tdw_llm::LlmError::EmptyMessages)
/// #     }
/// # }
/// let chain: Vec<Arc<dyn LanguageModel>> =
///     vec![Arc::new(Dummy("primary")), Arc::new(Dummy("secondary"))];
/// let fallback = FallbackModel::new(chain).expect("non-empty chain");
/// assert_eq!(fallback.model_id(), "primary");
/// ```
pub struct FallbackModel {
    chain: Vec<Arc<dyn LanguageModel>>,
    max_attempts_per_model: u32,
    should_retry: ShouldRetry,
}

impl FallbackModel {
    /// Construct a new [`FallbackModel`] from an ordered chain of models.
    ///
    /// The first model is the preferred one; later models are tried only when
    /// earlier ones exhaust their attempts. Defaults to a single attempt per
    /// model and a no-op `should_retry` (advance on any error).
    ///
    /// # Errors
    ///
    /// Returns [`LlmError::EmptyFallbackChain`] when `chain` is empty.
    pub fn new(chain: Vec<Arc<dyn LanguageModel>>) -> Result<Self> {
        if chain.is_empty() {
            return Err(LlmError::EmptyFallbackChain);
        }
        Ok(Self {
            chain,
            max_attempts_per_model: 1,
            should_retry: |_| false,
        })
    }

    /// Set the maximum number of attempts per model before advancing the chain.
    ///
    /// Values below one are clamped to one so every model is always tried at
    /// least once.
    #[must_use]
    pub fn with_max_attempts(mut self, max_attempts_per_model: u32) -> Self {
        self.max_attempts_per_model = max_attempts_per_model.max(1);
        self
    }

    /// Set the classifier deciding whether a failed attempt is retried on the
    /// same model (returns `true`) before the chain advances.
    ///
    /// The default is a strict no-op (`|_| false`), so by default each model is
    /// attempted exactly once and the chain advances on any error.
    #[must_use]
    pub fn with_should_retry(mut self, should_retry: ShouldRetry) -> Self {
        self.should_retry = should_retry;
        self
    }
}

impl LanguageModel for FallbackModel {
    fn model_id(&self) -> &str {
        // `new` guarantees a non-empty chain, so indexing the first is safe.
        self.chain[0].model_id()
    }

    /// Serve the request from the first model that succeeds.
    ///
    /// Each model is attempted up to `max_attempts_per_model` times; a failed
    /// attempt is retried on the same model only while `should_retry` returns
    /// `true`, otherwise the chain advances. The actually-answering model is
    /// reported by the returned [`ChatResponse::model_id`].
    ///
    /// # Errors
    ///
    /// Returns the error from the final attempt of the last model when every
    /// model in the chain fails.
    fn complete(&self, request: ChatRequest) -> Result<ChatResponse> {
        // `new` guarantees a non-empty chain, so this is always overwritten.
        let mut last_error = LlmError::EmptyFallbackChain;

        for model in &self.chain {
            for attempt in 1..=self.max_attempts_per_model {
                match model.complete(request.clone()) {
                    Ok(response) => return Ok(response),
                    Err(error) => {
                        let exhausted = attempt >= self.max_attempts_per_model;
                        let retry = !exhausted && (self.should_retry)(&error);
                        if !retry {
                            tracing::warn!(
                                model = model.model_id(),
                                attempt,
                                error = %error,
                                "model attempt failed; advancing fallback chain",
                            );
                        }
                        last_error = error;
                        if !retry {
                            break;
                        }
                    }
                }
            }
        }

        Err(last_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ChatMessage, MessageRole, Usage};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A stub model that returns a preset result and counts `complete` calls.
    struct StubModel {
        id: &'static str,
        call_count: Arc<AtomicUsize>,
        /// `Some(err)` makes every call fail with that error; `None` succeeds.
        fail: Option<LlmError>,
    }

    impl StubModel {
        fn succeeding(id: &'static str) -> (Arc<Self>, Arc<AtomicUsize>) {
            let count = Arc::new(AtomicUsize::new(0));
            (
                Arc::new(Self {
                    id,
                    call_count: Arc::clone(&count),
                    fail: None,
                }),
                count,
            )
        }

        fn failing(id: &'static str, err: LlmError) -> (Arc<Self>, Arc<AtomicUsize>) {
            let count = Arc::new(AtomicUsize::new(0));
            (
                Arc::new(Self {
                    id,
                    call_count: Arc::clone(&count),
                    fail: Some(err),
                }),
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
            self.fail.as_ref().map_or_else(
                || {
                    Ok(ChatResponse {
                        model_id: self.id.to_string(),
                        message: ChatMessage {
                            role: MessageRole::Assistant,
                            content: format!("{} response", self.id),
                        },
                        usage: Usage {
                            input_tokens: 1,
                            output_tokens: 1,
                        },
                    })
                },
                |err| Err(clone_error(err)),
            )
        }
    }

    /// Reproduce an [`LlmError`] without requiring `Clone` on the public type.
    fn clone_error(err: &LlmError) -> LlmError {
        match err {
            LlmError::EmptyMessages => LlmError::EmptyMessages,
            LlmError::EmptyMessageContent => LlmError::EmptyMessageContent,
            LlmError::EmptyMaxOutputTokens => LlmError::EmptyMaxOutputTokens,
            LlmError::EmptyModelId => LlmError::EmptyModelId,
            LlmError::InvalidModelId => LlmError::InvalidModelId,
            LlmError::InvalidBaseUrl => LlmError::InvalidBaseUrl,
            LlmError::UnsafeBaseUrl => LlmError::UnsafeBaseUrl,
            LlmError::UnsupportedProvider(p) => LlmError::UnsupportedProvider(p.clone()),
            LlmError::EmptyFallbackChain => LlmError::EmptyFallbackChain,
            LlmError::InvalidModelRef => LlmError::InvalidModelRef,
            LlmError::MissingCredentials(p) => LlmError::MissingCredentials(p.clone()),
            LlmError::NoEligibleModel => LlmError::NoEligibleModel,
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

    #[test]
    fn new_rejects_empty_chain() {
        let chain: Vec<Arc<dyn LanguageModel>> = Vec::new();
        // `FallbackModel` is intentionally not `Debug` (it holds trait objects
        // and a fn pointer), so match the error rather than calling `unwrap_err`.
        match FallbackModel::new(chain) {
            Err(error) => assert_eq!(error, LlmError::EmptyFallbackChain),
            Ok(_) => panic!("empty chain must be rejected"),
        }
    }

    #[test]
    fn model_id_reports_first_model() {
        let (first, _) = StubModel::succeeding("first");
        let (second, _) = StubModel::succeeding("second");
        let chain: Vec<Arc<dyn LanguageModel>> = vec![first, second];
        let model = FallbackModel::new(chain).expect("non-empty chain");
        assert_eq!(model.model_id(), "first");
    }

    /// Default behavior: a chain of two where the first errs and the second
    /// succeeds returns the second's response, and the returned `model_id`
    /// reflects the model that actually answered.
    #[test]
    fn falls_back_to_second_model_on_first_error() {
        let (first, first_count) = StubModel::failing("first", LlmError::EmptyMessages);
        let (second, second_count) = StubModel::succeeding("second");
        let chain: Vec<Arc<dyn LanguageModel>> = vec![first, second];
        let model = FallbackModel::new(chain).expect("non-empty chain");

        let response = model
            .complete(make_request())
            .expect("second model succeeds so fallback returns Ok");

        assert_eq!(first_count.load(Ordering::SeqCst), 1, "first tried once");
        assert_eq!(second_count.load(Ordering::SeqCst), 1, "second tried once");
        assert_eq!(
            response.model_id, "second",
            "response carries the answering model's id",
        );
    }

    #[test]
    fn first_success_never_touches_second() {
        let (first, first_count) = StubModel::succeeding("first");
        let (second, second_count) = StubModel::succeeding("second");
        let chain: Vec<Arc<dyn LanguageModel>> = vec![first, second];
        let model = FallbackModel::new(chain).expect("non-empty chain");

        let response = model.complete(make_request()).expect("first succeeds");

        assert_eq!(first_count.load(Ordering::SeqCst), 1, "first tried once");
        assert_eq!(
            second_count.load(Ordering::SeqCst),
            0,
            "second untouched when first succeeds",
        );
        assert_eq!(response.model_id, "first");
    }

    /// An all-failing chain returns the last model's error.
    #[test]
    fn all_failing_chain_returns_last_error() {
        let (first, _) = StubModel::failing("first", LlmError::EmptyMessages);
        let (second, second_count) =
            StubModel::failing("second", LlmError::UnsupportedProvider("none".to_string()));
        let chain: Vec<Arc<dyn LanguageModel>> = vec![first, second];
        let model = FallbackModel::new(chain).expect("non-empty chain");

        let err = model
            .complete(make_request())
            .expect_err("every model fails so fallback returns Err");

        assert_eq!(second_count.load(Ordering::SeqCst), 1, "second attempted");
        assert_eq!(
            err,
            LlmError::UnsupportedProvider("none".to_string()),
            "last model's error is surfaced",
        );
    }

    /// Default `should_retry` is a no-op: a failing model is tried exactly once
    /// even when `max_attempts_per_model` is greater than one.
    #[test]
    fn default_should_retry_does_not_retry() {
        let (first, first_count) = StubModel::failing("first", LlmError::EmptyMessages);
        let (second, _) = StubModel::succeeding("second");
        let chain: Vec<Arc<dyn LanguageModel>> = vec![first, second];
        let model = FallbackModel::new(chain)
            .expect("non-empty chain")
            .with_max_attempts(5);

        let response = model.complete(make_request()).expect("second succeeds");

        assert_eq!(
            first_count.load(Ordering::SeqCst),
            1,
            "default no-op should_retry means the first model is tried exactly once",
        );
        assert_eq!(response.model_id, "second");
    }

    /// Opt-in `should_retry` retries the same model up to `max_attempts`.
    #[test]
    fn opt_in_should_retry_retries_same_model() {
        let (first, first_count) = StubModel::failing("first", LlmError::EmptyMessages);
        let (second, _) = StubModel::succeeding("second");
        let chain: Vec<Arc<dyn LanguageModel>> = vec![first, second];
        let model = FallbackModel::new(chain)
            .expect("non-empty chain")
            .with_max_attempts(3)
            .with_should_retry(|error| matches!(error, LlmError::EmptyMessages));

        let response = model.complete(make_request()).expect("second succeeds");

        assert_eq!(
            first_count.load(Ordering::SeqCst),
            3,
            "first model retried up to max_attempts before advancing",
        );
        assert_eq!(response.model_id, "second");
    }

    #[test]
    fn with_max_attempts_clamps_to_one() {
        let (first, first_count) = StubModel::failing("first", LlmError::EmptyMessages);
        let chain: Vec<Arc<dyn LanguageModel>> = vec![first];
        let model = FallbackModel::new(chain)
            .expect("non-empty chain")
            .with_max_attempts(0)
            .with_should_retry(|_| true);

        let err = model
            .complete(make_request())
            .expect_err("only model fails");

        assert_eq!(
            first_count.load(Ordering::SeqCst),
            1,
            "zero max_attempts is clamped to one",
        );
        assert_eq!(err, LlmError::EmptyMessages);
    }
}
