//! Credential-aware provider router for [`LanguageModel`].
//!
//! [`ModelRouter`] maps a `<provider>/<model>` reference to a concrete
//! [`LanguageModel`] by consulting an opt-in registry of provider entries.
//! Each entry pairs a *credential probe* (does the caller have credentials
//! for this provider?) with a *factory* (build the adapter for a resolved
//! model reference).
//!
//! The router is the leaf-crate-safe way to do provider routing: it never
//! names a concrete provider adapter and never performs network or
//! environment access itself. The caller injects both the probe and the
//! factory as closures, so all credential reads and adapter construction
//! live outside this crate. This keeps `tdw-llm` free of provider
//! dependencies (which would otherwise form a build cycle, since the
//! provider crates depend on `tdw-llm`).
//!
//! Resolution is synchronous and allocation-only: probes and factories are
//! expected to be cheap and non-blocking, performing no network I/O.
//!
//! # Example
//!
//! ```text
//! // Intended caller wiring (in a crate that already imports the provider
//! // adapters, e.g. tdw-service-api). The router itself stays provider-free.
//! use std::sync::Arc;
//! use tdw_llm::{ModelRouter, ModelRef};
//!
//! let router = ModelRouter::new()
//!     .with_provider(
//!         "anthropic",
//!         Arc::new(|_provider: &str| std::env::var("ANTHROPIC_API_KEY").is_ok()),
//!         Arc::new(|r: &ModelRef| Ok(Arc::new(AnthropicMessagesModel::new(&r.model)?) as _)),
//!     )
//!     .with_provider(
//!         "openai",
//!         Arc::new(|_provider: &str| std::env::var("OPENAI_API_KEY").is_ok()),
//!         Arc::new(|r: &ModelRef| {
//!             Ok(Arc::new(OpenAiCompatibleModel::new(&r.model, base_url)?) as _)
//!         }),
//!     );
//!
//! let model = router.resolve("anthropic/claude-3-5-sonnet")?;
//! ```

use crate::{LanguageModel, LlmError, Result, validate_model_id};
use std::collections::BTreeMap;
use std::sync::Arc;

/// A parsed `<provider>/<model>` reference.
///
/// `provider` is always lowercased for case-insensitive registry matching;
/// `model` preserves its original casing (it is forwarded to the factory).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelRef {
    /// Lowercased provider prefix (the segment before the first `/`).
    pub provider: String,
    /// The model identifier (everything after the first `/`).
    pub model: String,
}

/// Builds a concrete [`LanguageModel`] for a resolved [`ModelRef`].
///
/// The factory is injected by the caller and is responsible for constructing
/// the concrete provider adapter. It must not perform network I/O at
/// construction time — adapter constructors only validate inputs.
pub type ModelFactory = Arc<dyn Fn(&ModelRef) -> Result<Arc<dyn LanguageModel>> + Send + Sync>;

/// Reports whether credentials are available for a given provider name.
///
/// The probe is injected by the caller. It must be cheap, non-blocking, and
/// perform no network I/O; all environment/secret access lives in the caller,
/// keeping `tdw-llm` side-effect-free.
pub type CredentialProbe = Arc<dyn Fn(&str) -> bool + Send + Sync>;

/// A registered provider: a credential probe paired with a model factory.
struct ProviderEntry {
    probe: CredentialProbe,
    factory: ModelFactory,
}

/// Parse a `<provider>/<model>` reference.
///
/// Splits on the **first** `/` only, so the model segment may itself contain
/// slashes. Both segments are trimmed; the provider is lowercased. The model
/// segment is additionally validated via [`validate_model_id`].
///
/// # Errors
///
/// - [`LlmError::InvalidModelRef`] if there is no `/`, or the provider segment
///   is empty, or the model segment is empty after trimming.
/// - Whatever [`validate_model_id`] returns for a non-empty but invalid model
///   segment (e.g. [`LlmError::InvalidModelId`] for control characters).
pub fn parse_model_ref(name: &str) -> Result<ModelRef> {
    let (provider, model) = name.split_once('/').ok_or(LlmError::InvalidModelRef)?;
    let provider = provider.trim();
    let model = model.trim();
    if provider.is_empty() || model.is_empty() {
        return Err(LlmError::InvalidModelRef);
    }
    validate_model_id(model)?;
    Ok(ModelRef {
        provider: provider.to_lowercase(),
        model: model.to_string(),
    })
}

/// An opt-in, network-free, credential-aware provider router.
///
/// A freshly constructed router has no providers; callers register them via
/// [`ModelRouter::with_provider`]. Nothing is constructed or probed until a
/// caller registers a provider and calls [`ModelRouter::resolve`] or
/// [`ModelRouter::resolve_chain`].
#[derive(Default)]
pub struct ModelRouter {
    providers: BTreeMap<String, ProviderEntry>,
}

impl ModelRouter {
    /// Construct an empty router (no providers, no network).
    #[must_use]
    pub const fn new() -> Self {
        Self {
            providers: BTreeMap::new(),
        }
    }

    /// Register a provider under `prefix` (lowercased) with its credential
    /// probe and model factory.
    ///
    /// Registration is opt-in and chainable. Re-registering the same prefix
    /// replaces the previous entry — last registration wins.
    #[must_use]
    pub fn with_provider(
        mut self,
        prefix: impl Into<String>,
        probe: CredentialProbe,
        factory: ModelFactory,
    ) -> Self {
        let prefix = prefix.into().to_lowercase();
        self.providers
            .insert(prefix, ProviderEntry { probe, factory });
        self
    }

    /// Return the registered provider prefixes in deterministic (sorted) order.
    #[must_use]
    pub fn registered_providers(&self) -> Vec<&str> {
        self.providers.keys().map(String::as_str).collect()
    }

    /// Resolve a `<provider>/<model>` reference to a concrete model.
    ///
    /// # Errors
    ///
    /// - Whatever [`parse_model_ref`] returns for a malformed reference.
    /// - [`LlmError::UnsupportedProvider`] if the provider is not registered.
    /// - [`LlmError::MissingCredentials`] if the provider's probe returns
    ///   `false` (the factory is not invoked in this case).
    /// - Whatever the factory returns if adapter construction fails.
    pub fn resolve(&self, name: &str) -> Result<Arc<dyn LanguageModel>> {
        let model_ref = parse_model_ref(name)?;
        let entry = self
            .providers
            .get(&model_ref.provider)
            .ok_or_else(|| LlmError::UnsupportedProvider(model_ref.provider.clone()))?;
        if !(entry.probe)(&model_ref.provider) {
            return Err(LlmError::MissingCredentials(model_ref.provider.clone()));
        }
        (entry.factory)(&model_ref)
    }

    /// Resolve a list of references into an ordered, credential-aware chain.
    ///
    /// Candidates are kept in input order. A candidate is **silently dropped**
    /// (not errored) when its provider is unregistered, when its probe reports
    /// missing credentials, or when its reference fails to parse. The surviving
    /// adapters are built via their factories and returned in order, ready to
    /// feed into [`crate::FallbackModel`].
    ///
    /// # Errors
    ///
    /// - [`LlmError::NoEligibleModel`] if no candidate survives filtering.
    /// - Whatever a surviving candidate's factory returns if construction fails.
    pub fn resolve_chain(&self, names: &[&str]) -> Result<Vec<Arc<dyn LanguageModel>>> {
        let mut chain: Vec<Arc<dyn LanguageModel>> = Vec::new();
        for name in names {
            let Ok(model_ref) = parse_model_ref(name) else {
                continue;
            };
            let Some(entry) = self.providers.get(&model_ref.provider) else {
                continue;
            };
            if !(entry.probe)(&model_ref.provider) {
                continue;
            }
            chain.push((entry.factory)(&model_ref)?);
        }
        if chain.is_empty() {
            return Err(LlmError::NoEligibleModel);
        }
        Ok(chain)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ChatMessage, ChatRequest, ChatResponse, MessageRole, Usage};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A stub model that reports a chosen model id.
    struct StubModel {
        id: String,
    }

    impl LanguageModel for StubModel {
        fn model_id(&self) -> &str {
            &self.id
        }

        fn complete(&self, _request: ChatRequest) -> Result<ChatResponse> {
            Ok(ChatResponse {
                model_id: self.id.clone(),
                message: ChatMessage {
                    role: MessageRole::Assistant,
                    content: format!("{} response", self.id),
                },
                usage: Usage {
                    input_tokens: 1,
                    output_tokens: 1,
                },
            })
        }
    }

    /// Extract the error from a `resolve_chain` result whose `Ok` payload
    /// (a `Vec` of `dyn LanguageModel`) is neither `Debug` nor `PartialEq`.
    fn chain_err(result: Result<Vec<Arc<dyn LanguageModel>>>) -> LlmError {
        match result {
            Ok(_) => panic!("expected an error, got Ok"),
            Err(error) => error,
        }
    }

    /// Extract the error from a `resolve` result whose `Ok` payload
    /// (a `dyn LanguageModel`) is neither `Debug` nor `PartialEq`.
    fn resolve_err(result: Result<Arc<dyn LanguageModel>>) -> LlmError {
        match result {
            Ok(_) => panic!("expected an error, got Ok"),
            Err(error) => error,
        }
    }

    fn always() -> CredentialProbe {
        Arc::new(|_provider: &str| true)
    }

    fn never() -> CredentialProbe {
        Arc::new(|_provider: &str| false)
    }

    /// Factory that builds a `StubModel` whose id is the resolved model name.
    fn stub_factory() -> ModelFactory {
        Arc::new(|r: &ModelRef| {
            Ok(Arc::new(StubModel {
                id: r.model.clone(),
            }) as Arc<dyn LanguageModel>)
        })
    }

    // ---- parse_model_ref -------------------------------------------------

    #[test]
    fn parse_basic_ref() {
        assert_eq!(
            parse_model_ref("anthropic/claude-3-5-sonnet"),
            Ok(ModelRef {
                provider: "anthropic".to_string(),
                model: "claude-3-5-sonnet".to_string(),
            })
        );
    }

    #[test]
    fn parse_lowercases_provider() {
        let parsed = parse_model_ref("OpenAI/gpt-4o").expect("valid ref");
        assert_eq!(parsed.provider, "openai");
        assert_eq!(parsed.model, "gpt-4o");
    }

    #[test]
    fn parse_splits_on_first_slash() {
        let parsed = parse_model_ref("openai/foo/bar").expect("valid ref");
        assert_eq!(parsed.provider, "openai");
        assert_eq!(parsed.model, "foo/bar");
    }

    #[test]
    fn parse_rejects_malformed_refs() {
        assert_eq!(parse_model_ref("noslash"), Err(LlmError::InvalidModelRef));
        assert_eq!(parse_model_ref("/model"), Err(LlmError::InvalidModelRef));
        assert_eq!(parse_model_ref("prov/"), Err(LlmError::InvalidModelRef));
        // control char in the model segment surfaces validate_model_id's error
        assert_eq!(
            parse_model_ref("prov/bad\nmodel"),
            Err(LlmError::InvalidModelId)
        );
    }

    // ---- resolve ---------------------------------------------------------

    #[test]
    fn resolve_returns_model_when_creds_present() {
        let router = ModelRouter::new().with_provider("p", always(), stub_factory());
        let model = router.resolve("p/m").expect("resolves with creds");
        assert_eq!(model.model_id(), "m");
    }

    #[test]
    fn resolve_missing_creds_skips_factory() {
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_in_factory = Arc::clone(&calls);
        let factory: ModelFactory = Arc::new(move |r: &ModelRef| {
            calls_in_factory.fetch_add(1, Ordering::SeqCst);
            Ok(Arc::new(StubModel {
                id: r.model.clone(),
            }) as Arc<dyn LanguageModel>)
        });
        let router = ModelRouter::new().with_provider("p", never(), factory);

        assert_eq!(
            resolve_err(router.resolve("p/m")),
            LlmError::MissingCredentials("p".to_string())
        );
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "factory must not run when creds are absent"
        );
    }

    #[test]
    fn resolve_unregistered_provider_errors() {
        let router = ModelRouter::new().with_provider("p", always(), stub_factory());
        assert_eq!(
            resolve_err(router.resolve("unknown/m")),
            LlmError::UnsupportedProvider("unknown".to_string())
        );
    }

    #[test]
    fn resolve_propagates_factory_error() {
        let factory: ModelFactory = Arc::new(|_r: &ModelRef| Err(LlmError::EmptyModelId));
        let router = ModelRouter::new().with_provider("p", always(), factory);
        assert_eq!(resolve_err(router.resolve("p/m")), LlmError::EmptyModelId);
    }

    // ---- resolve_chain ---------------------------------------------------

    #[test]
    fn chain_drops_absent_cred_candidate() {
        let router = ModelRouter::new()
            .with_provider("a", never(), stub_factory())
            .with_provider("b", always(), stub_factory());

        let chain = router
            .resolve_chain(&["a/m1", "b/m2"])
            .expect("at least one eligible candidate");
        assert_eq!(chain.len(), 1, "absent-cred provider dropped, not errored");
        assert_eq!(chain[0].model_id(), "m2");
    }

    #[test]
    fn chain_preserves_order_and_builds_fallback() {
        let router = ModelRouter::new()
            .with_provider("a", always(), stub_factory())
            .with_provider("b", always(), stub_factory());

        let chain = router
            .resolve_chain(&["a/m1", "b/m2"])
            .expect("both eligible");
        assert_eq!(chain.len(), 2);
        assert_eq!(chain[0].model_id(), "m1");
        assert_eq!(chain[1].model_id(), "m2");

        // Feed the chain into a FallbackModel (primary + secondary).
        let mut iter = chain.into_iter();
        let primary = iter.next().expect("primary present");
        let secondary = iter.next().expect("secondary present");
        let fallback = crate::FallbackModel::new(primary, secondary);
        assert_eq!(fallback.model_id(), "m1");
    }

    #[test]
    fn chain_all_ineligible_errors() {
        let router = ModelRouter::new()
            .with_provider("a", never(), stub_factory())
            .with_provider("b", never(), stub_factory());

        // all absent creds — Ok variant holds non-Debug trait objects, so
        // assert on the error directly rather than via assert_eq! on the Result.
        assert_eq!(
            chain_err(router.resolve_chain(&["a/m1", "b/m2"])),
            LlmError::NoEligibleModel
        );
        // all unregistered
        assert_eq!(
            chain_err(router.resolve_chain(&["x/m1", "y/m2"])),
            LlmError::NoEligibleModel
        );
    }

    #[test]
    fn chain_skips_unparseable_ref() {
        let router = ModelRouter::new().with_provider("b", always(), stub_factory());
        let chain = router
            .resolve_chain(&["noslash", "b/m2"])
            .expect("one valid candidate remains");
        assert_eq!(chain.len(), 1);
        assert_eq!(chain[0].model_id(), "m2");
    }

    // ---- builder / opt-in ------------------------------------------------

    #[test]
    fn new_and_default_are_empty() {
        assert!(ModelRouter::new().registered_providers().is_empty());
        assert!(ModelRouter::default().registered_providers().is_empty());
    }

    #[test]
    fn re_registering_prefix_last_wins() {
        let first: ModelFactory = Arc::new(|_r: &ModelRef| {
            Ok(Arc::new(StubModel {
                id: "first".to_string(),
            }) as Arc<dyn LanguageModel>)
        });
        let second: ModelFactory = Arc::new(|_r: &ModelRef| {
            Ok(Arc::new(StubModel {
                id: "second".to_string(),
            }) as Arc<dyn LanguageModel>)
        });
        let router = ModelRouter::new()
            .with_provider("p", always(), first)
            .with_provider("p", always(), second);

        let model = router.resolve("p/m").expect("resolves");
        assert_eq!(model.model_id(), "second", "last registration wins");
    }

    #[test]
    fn registered_providers_sorted() {
        let router = ModelRouter::new()
            .with_provider("zeta", always(), stub_factory())
            .with_provider("alpha", always(), stub_factory())
            .with_provider("mid", always(), stub_factory());
        assert_eq!(router.registered_providers(), vec!["alpha", "mid", "zeta"]);
    }
}
