#![forbid(unsafe_code)]
//! Daemon service API: the secure request path and composition root.
//!
//! This crate wires the daemon's ingress enforcement (`policy`), async op
//! dispatch (`dispatcher`), and the [`AppState`] composition root
//! (`app_state`). Every request passes a sync policy guard
//! (`enforce_request_path_with_backend`): OIDC claim validation via
//! `tdw-auth-oidc`, role authorization via `tdw-auth`, hook execution via
//! `tdw-hooks`, and response masking via `tdw-mask`.
//!
//! # Production auth scope: structural, not cryptographic
//!
//! In a `prod`/`production` profile the policy is built from the six
//! `TDW_OIDC_*` environment variables (see [`OidcPolicyError`] for the typed
//! failure causes and `docs/release/production-auth-oidc.md` for the operator
//! contract). That validation is **structural** — claim/JWKS consistency
//! (issuer, audience, `kid` ∈ JWKS, allowed algorithm, role shape) — and does
//! **not** verify JWT cryptographic signatures or fetch a remote JWKS. Non-prod
//! profiles synthesize a deterministic local-default policy so offline daemon
//! dispatches resolve; a fully-unset prod profile stays fail-closed by design.

#[cfg(feature = "agent-route")]
mod agent_bridge;
mod app_state;
mod dispatcher;
mod econometrics_compute;
mod event_sink;
pub mod fetch_policy;
#[cfg(feature = "functions")]
pub mod function_enqueue;
mod policy;
mod provider_resolve;
mod quant_compute;
#[cfg(feature = "rest-api-route")]
mod rest_handler;
mod stream_ingest;
mod technical_compute;
#[cfg(feature = "identity")]
pub mod user_events;

#[cfg(feature = "agent-route")]
pub use agent_bridge::AgentBridgeState;
pub use app_state::{AppState, OidcPolicyError};
#[cfg(feature = "rest-api-route")]
pub use dispatcher::{RestFetchError, rest_fetch_data};
pub use dispatcher::{dispatch_op, ingest_dispatch_pairs};
pub use policy::{
    IngressAuthContext, PolicyEnforcementConfig, PolicyEnforcementEvidence, SecureServiceRuntime,
    ServiceEndpoint, enforce_request_path_with_backend, mask_json_response,
    secure_endpoint_by_name, secure_endpoint_by_name_with_backend, secure_endpoint_response,
    secure_endpoint_response_with_backend, secure_udf_run, secure_udf_run_with_backend,
    service_hook_policy,
};
#[cfg(feature = "rest-api-route")]
pub use rest_handler::RestApiState;
pub use stream_ingest::{run_stream_ingest, run_ws_ingest};
pub use tdw_hooks::HookExecutionPolicy;

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};

use bytes::Bytes;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tdw_acp::{AcpRequest, AcpServerInfo, validate_request};
use tdw_actor::{ActorContext, OmcSpawn};
use tdw_agent::{
    Adaptivity, EntityMeta, EvalCase, EvalRunRequest, McpEntity, McpPrompt, McpTool, Origin,
    Registry, Source, Tier, WorkflowDefinition, WorkflowEdge, WorkflowNode, gotcha_seed,
    parse_slash_command_invocation, project_to_mcp, sample_agent_card, schema_bundle,
};
use tdw_agent_store::AgentStore;
use tdw_app_client::{AppClient, ClientInfo};
use tdw_app_server::{DaemonEndpoint, DaemonTransport, channel, validate_endpoint};
use tdw_auth::{AuthPolicy, Principal, authorize};
use tdw_auth_oidc::{JwksKey, JwtClaims, validate_claims};
use tdw_bus::EventBus;
use tdw_cdc::CdcStream;
use tdw_config::{ConfigLayer, ConfigLayerKind, default_layer_order, merge_layers};
use tdw_core::{
    BlobEngine, Error, LexicalDoc, LexicalEngine, OBBject, ProgressOrResult, ProgressStream,
    ProviderKind, ProviderRegistry, Result, ScoredDoc, ScoredPoint, TextQuery, VectorEngine,
    VectorPoint, VectorQuery,
};
use tdw_define::DefineEvent;
use tdw_domain::{EquityHistoricalData, ResearchNote};
use tdw_embed::EmbeddingProvider;
use tdw_embed_local::HashEmbeddingProvider;
use tdw_entity_resolver::{manual_merge_decision, resolve_symbol};
use tdw_eval_runner::{EvalRunner, StubLanguageModel};
use tdw_event::{EventEnvelope, event_schema_bundle};
use tdw_exec::try_run_headless;
use tdw_feature_store::FeatureStore;
use tdw_graph::DirectedGraph;
use tdw_hooks::{
    AdditionalContext, HandlerKind, HookEvent, HookRegistry, HookSpec, TransactionMode, event_hook,
};
use tdw_kg::{Entity, EntityKind, KnowledgeGraph, Relationship};
use tdw_knowledge::{KnowledgeDocument, KnowledgeIndex, summarize_syntax};
use tdw_llm::{ChatMessage, ChatRequest, LanguageModel, MessageRole};
use tdw_llm_anthropic::AnthropicMessagesModel;
use tdw_llm_openai_compat::OpenAiCompatibleModel;
use tdw_mask::{MaskMode, MaskRule, apply_masks, masking_hook};
use tdw_outbox::InMemoryOutbox;
use tdw_pipe::PipeDefinition;
use tdw_protocol::{
    ActorKind, ActorRef, EventMsg, Op, OpEnvelope, PermissionId, ReplayFrame, SessionId,
    schema_bundle as protocol_schema_bundle,
};
#[cfg(feature = "provider-adanos")]
use tdw_provider_adanos::{
    AdanosPolymarketHttpFetcher, AdanosSentimentHttpFetcher, AdanosTrendingHttpFetcher,
};
#[cfg(feature = "provider-akshare")]
use tdw_provider_akshare::AkShareHttpFetcher;
#[cfg(feature = "provider-alpaca")]
use tdw_provider_alpaca::AlpacaHttpStockBarsFetcher;
#[cfg(feature = "provider-alpha-vantage")]
use tdw_provider_alpha_vantage::AlphaVantageHttpFetcher;
#[cfg(feature = "provider-benzinga")]
use tdw_provider_benzinga::{BenzingaEarningsHttpFetcher, BenzingaNewsHttpFetcher};
#[cfg(feature = "provider-binance-http")]
use tdw_provider_binance::BinanceHttpTickerPriceFetcher;
#[cfg(feature = "provider-bls")]
use tdw_provider_bls::BlsHttpTimeSeriesFetcher;
#[cfg(feature = "provider-cboe")]
use tdw_provider_cboe::{
    CboeHttpIndexFetcher, CboeHttpIndexSnapshotFetcher, CboeHttpOptionsChainFetcher,
    CboeHttpOptionsFetcher,
};
#[cfg(feature = "provider-ccdata")]
use tdw_provider_ccdata::CCDataHttpFetcher;
#[cfg(feature = "provider-coingecko")]
use tdw_provider_coingecko::CoinGeckoHttpOhlcFetcher;
#[cfg(feature = "provider-databento")]
use tdw_provider_databento::{DatabentoHttpTimeseriesFetcher, DatabentoMetadataFetcher};
#[cfg(feature = "provider-deribit")]
use tdw_provider_deribit::{
    DeribitHttpFundingFetcher, DeribitHttpInstrumentsFetcher, DeribitHttpOrderBookFetcher,
};
#[cfg(feature = "provider-ecb")]
use tdw_provider_ecb::{EcbHttpDataFetcher, EcbHttpReferenceRatesFetcher};
#[cfg(feature = "provider-eia")]
use tdw_provider_eia::{
    EiaHttpNaturalGasFetcher, EiaHttpReportFetcher, EiaHttpSpotPriceFetcher, EiaReport,
};
#[cfg(feature = "provider-federal-reserve")]
use tdw_provider_federal_reserve::{FedFomcDocumentsHttpFetcher, FedMacroSeriesHttpFetcher};
use tdw_provider_fileset::FilesetEquityHistoricalFetcher;
#[cfg(feature = "provider-finnhub")]
use tdw_provider_finnhub::{FinnhubHttpProfileFetcher, FinnhubHttpQuoteSnapshotFetcher};
#[cfg(feature = "provider-finra")]
use tdw_provider_finra::{FinraOtcSummaryHttpFetcher, FinraShortInterestHttpFetcher};
#[cfg(feature = "provider-fmp")]
use tdw_provider_fmp::{
    FmpHttpHistoricalFetcher, FmpHttpIncomeFetcher, FmpHttpKeyMetricsFetcher,
    FmpHttpProfileFetcher, FmpHttpQuoteSnapshotFetcher, FmpHttpRatiosFetcher,
    FmpHttpStatementFetcher,
};
#[cfg(feature = "provider-fred")]
use tdw_provider_fred::{
    FredHttpMacroSeriesFetcher, FredHttpRateObservationFetcher, FredHttpSeriesObservationsFetcher,
    FredHttpSeriesSearchFetcher, FredHttpYieldCurveFetcher,
};
#[cfg(feature = "provider-geckoterminal")]
use tdw_provider_geckoterminal::GeckoTerminalHttpFetcher;
#[cfg(feature = "provider-glassnode")]
use tdw_provider_glassnode::GlassnodeHttpFetcher;
#[cfg(feature = "provider-government-us")]
use tdw_provider_government_us::{
    GovUsTreasuryAuctionsHttpFetcher, GovUsTreasuryPricesHttpFetcher,
};
#[cfg(feature = "provider-huggingface")]
use tdw_provider_huggingface::HuggingFaceHttpTextGenerationFetcher;
#[cfg(feature = "provider-nasdaq")]
use tdw_provider_nasdaq::{
    NasdaqCalendarKind, NasdaqHttpCalendarFetcher, NasdaqHttpDatasetFetcher,
};
#[cfg(feature = "provider-oecd")]
use tdw_provider_oecd::OecdHttpDataFetcher;
#[cfg(feature = "provider-polygon")]
use tdw_provider_polygon::PolygonHttpAggregatesFetcher;
#[cfg(feature = "provider-sec")]
use tdw_provider_sec::{
    SecCikMapHttpFetcher, SecEtfHoldingsHttpFetcher, SecFailsToDeliverHttpFetcher,
    SecFilingsHttpFetcher, SecForm13FHttpFetcher, SecXbrlHttpFetcher,
};
#[cfg(feature = "provider-seeking-alpha")]
use tdw_provider_seeking_alpha::{SeekingAlphaArticlesHttpFetcher, SeekingAlphaRatingsHttpFetcher};
#[cfg(feature = "provider-tiingo")]
use tdw_provider_tiingo::{TiingoHttpHistoricalFetcher, TiingoHttpNewsFetcher};
#[cfg(feature = "provider-tmx")]
use tdw_provider_tmx::{TmxHttpBatchQuoteFetcher, TmxHttpQuoteFetcher};
#[cfg(feature = "provider-tradier")]
use tdw_provider_tradier::{TradierHttpOptionsFetcher, TradierHttpQuoteFetcher};
#[cfg(feature = "provider-trading-economics")]
use tdw_provider_trading_economics::{
    TradingEconomicsHttpCalendarFetcher, TradingEconomicsHttpIndicatorFetcher,
};
#[cfg(feature = "provider-velodata")]
use tdw_provider_velodata::{
    VelodataHttpFundingFetcher, VelodataHttpLiquidationsFetcher, VelodataHttpOiFetcher,
};
use tdw_provider_ws_mock::MockEquityStreamer;
#[cfg(not(feature = "provider-yahoo-http"))]
use tdw_provider_yahoo::YahooEquityHistoricalFetcher;
#[cfg(feature = "provider-yahoo-http")]
use tdw_provider_yahoo::YahooHttpEquityHistoricalFetcher;
#[cfg(feature = "provider-yahoo-http")]
use tdw_provider_yahoo::{
    YahooHttpConsensusFetcher, YahooHttpDividendsFetcher, YahooHttpFuturesCurveFetcher,
    YahooHttpFuturesHistoricalFetcher, YahooHttpOptionsChainFetcher,
    YahooHttpPricePerformanceFetcher, YahooHttpProfileFetcher, YahooHttpQuoteFetcher,
    YahooHttpShareStatisticsFetcher,
};
use tdw_replay::ReplayEngine;
use tdw_rollout::RolloutRecord;
use tdw_runtime::CommandRunner;
use tdw_sandbox::{LocalUdfSandbox, SandboxRuntime, UdfRequest};
use tdw_snapshot::SnapshotStore;
use tdw_spatial::{BoundingBox, Point};
use tdw_stage::StageLocation;
use tdw_storage_meilisearch::InMemoryLexicalEngine;
use tdw_storage_qdrant::InMemoryVectorEngine;
use tdw_storage_s3::InMemoryS3BlobEngine;
use tdw_table_format::{TableFile, TableFormat, TableManifest};
use tdw_tag_rules::{RuleEngine, RulePredicate, TagRule};
use tdw_tags::{TagAssignment, TagDefinition, TagStore};
use tdw_tools::{ToolOrchestrator, ToolRegistry, echo_tool};
use tdw_tui::event_lines;
use tdw_udf::{UdfDefinition, UdfRuntime, evaluate};
use tdw_workflow_engine::WorkflowEngine;

#[cfg(feature = "provider-yahoo-http")]
type SelectedYahooEquityHistoricalFetcher = YahooHttpEquityHistoricalFetcher;
#[cfg(not(feature = "provider-yahoo-http"))]
type SelectedYahooEquityHistoricalFetcher = YahooEquityHistoricalFetcher;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderSummary {
    pub provider: String,
    pub endpoint: String,
    pub kind: ProviderKind,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResearchIndexEvidence {
    pub note_id: String,
    pub model_id: String,
    pub vector_hits: Vec<ScoredPoint>,
    pub lexical_hits: Vec<ScoredDoc>,
    pub blob_bytes: usize,
}

/// # Errors
///
/// Returns an error variant if the underlying operation fails.
#[allow(clippy::too_many_lines)] // flat provider registration list mirrors the dispatch table; splitting adds no clarity
pub fn default_registry() -> Result<ProviderRegistry> {
    let mut registry = ProviderRegistry::default();
    registry.register(FilesetEquityHistoricalFetcher::registry_entry())?;
    registry.register(SelectedYahooEquityHistoricalFetcher::registry_entry())?;
    // Keyless Yahoo expansion fetchers (gap-matrix item L2.4); only present when
    // the real HTTP implementation is enabled.
    #[cfg(feature = "provider-yahoo-http")]
    {
        registry.register(YahooHttpProfileFetcher::registry_entry())?;
        registry.register(YahooHttpQuoteFetcher::registry_entry())?;
        registry.register(YahooHttpPricePerformanceFetcher::registry_entry())?;
        registry.register(YahooHttpDividendsFetcher::registry_entry())?;
        registry.register(YahooHttpShareStatisticsFetcher::registry_entry())?;
        registry.register(YahooHttpConsensusFetcher::registry_entry())?;
        registry.register(YahooHttpOptionsChainFetcher::registry_entry())?;
        registry.register(YahooHttpFuturesHistoricalFetcher::registry_entry())?;
        registry.register(YahooHttpFuturesCurveFetcher::registry_entry())?;
    }
    registry.register(MockEquityStreamer::registry_entry())?;
    #[cfg(feature = "provider-adanos")]
    registry.register(AdanosSentimentHttpFetcher::registry_entry())?;
    #[cfg(feature = "provider-adanos")]
    registry.register(AdanosTrendingHttpFetcher::registry_entry())?;
    #[cfg(feature = "provider-adanos")]
    registry.register(AdanosPolymarketHttpFetcher::registry_entry())?;
    #[cfg(feature = "provider-akshare")]
    registry.register(AkShareHttpFetcher::registry_entry())?;
    #[cfg(feature = "provider-alpaca")]
    registry.register(AlpacaHttpStockBarsFetcher::registry_entry())?;
    #[cfg(feature = "provider-alpha-vantage")]
    registry.register(AlphaVantageHttpFetcher::registry_entry())?;
    #[cfg(feature = "provider-benzinga")]
    registry.register(BenzingaNewsHttpFetcher::registry_entry())?;
    #[cfg(feature = "provider-benzinga")]
    registry.register(BenzingaEarningsHttpFetcher::registry_entry())?;
    #[cfg(feature = "provider-bls")]
    registry.register(BlsHttpTimeSeriesFetcher::registry_entry())?;
    #[cfg(feature = "provider-cboe")]
    registry.register(CboeHttpIndexFetcher::registry_entry())?;
    #[cfg(feature = "provider-cboe")]
    registry.register(CboeHttpOptionsFetcher::registry_entry())?;
    // Catalog-facing CBOE fetchers emitting tdw_domain models (G004 part 2).
    #[cfg(feature = "provider-cboe")]
    registry.register(CboeHttpIndexSnapshotFetcher::registry_entry())?;
    #[cfg(feature = "provider-cboe")]
    registry.register(CboeHttpOptionsChainFetcher::registry_entry())?;
    #[cfg(feature = "provider-ccdata")]
    registry.register(CCDataHttpFetcher::registry_entry())?;
    #[cfg(feature = "provider-coingecko")]
    registry.register(CoinGeckoHttpOhlcFetcher::registry_entry())?;
    #[cfg(feature = "provider-databento")]
    registry.register(DatabentoHttpTimeseriesFetcher::registry_entry())?;
    #[cfg(feature = "provider-databento")]
    registry.register(DatabentoMetadataFetcher::registry_entry())?;
    #[cfg(feature = "provider-deribit")]
    registry.register(DeribitHttpInstrumentsFetcher::registry_entry())?;
    #[cfg(feature = "provider-deribit")]
    registry.register(DeribitHttpOrderBookFetcher::registry_entry())?;
    #[cfg(feature = "provider-deribit")]
    registry.register(DeribitHttpFundingFetcher::registry_entry())?;
    #[cfg(feature = "provider-ecb")]
    registry.register(EcbHttpDataFetcher::registry_entry())?;
    // Catalog-facing ECB reference-rates fetcher (G004 part 2).
    #[cfg(feature = "provider-ecb")]
    registry.register(EcbHttpReferenceRatesFetcher::registry_entry())?;
    #[cfg(feature = "provider-eia")]
    registry.register(EiaHttpSpotPriceFetcher::registry_entry())?;
    #[cfg(feature = "provider-eia")]
    registry.register(EiaHttpNaturalGasFetcher::registry_entry())?;
    // Catalog-facing EIA report fetcher (G004 part 2).
    #[cfg(feature = "provider-eia")]
    registry.register(EiaHttpReportFetcher::registry_entry())?;
    #[cfg(feature = "provider-finra")]
    registry.register(FinraOtcSummaryHttpFetcher::registry_entry())?;
    #[cfg(feature = "provider-finra")]
    registry.register(FinraShortInterestHttpFetcher::registry_entry())?;
    #[cfg(feature = "provider-finnhub")]
    registry.register(FinnhubHttpProfileFetcher::registry_entry())?;
    #[cfg(feature = "provider-finnhub")]
    registry.register(FinnhubHttpQuoteSnapshotFetcher::registry_entry())?;
    #[cfg(feature = "provider-fmp")]
    registry.register(FmpHttpHistoricalFetcher::registry_entry())?;
    #[cfg(feature = "provider-fmp")]
    registry.register(FmpHttpIncomeFetcher::registry_entry())?;
    #[cfg(feature = "provider-fmp")]
    registry.register(FmpHttpQuoteSnapshotFetcher::registry_entry())?;
    // FMP fundamentals breadth (G011): statement / ratios / key-metrics / profile.
    #[cfg(feature = "provider-fmp")]
    registry.register(FmpHttpStatementFetcher::registry_entry())?;
    #[cfg(feature = "provider-fmp")]
    registry.register(FmpHttpRatiosFetcher::registry_entry())?;
    #[cfg(feature = "provider-fmp")]
    registry.register(FmpHttpKeyMetricsFetcher::registry_entry())?;
    #[cfg(feature = "provider-fmp")]
    registry.register(FmpHttpProfileFetcher::registry_entry())?;
    #[cfg(feature = "provider-fred")]
    registry.register(FredHttpSeriesObservationsFetcher::registry_entry())?;
    #[cfg(feature = "provider-fred")]
    registry.register(FredHttpMacroSeriesFetcher::registry_entry())?;
    #[cfg(feature = "provider-fred")]
    registry.register(FredHttpRateObservationFetcher::registry_entry())?;
    #[cfg(feature = "provider-fred")]
    registry.register(FredHttpSeriesSearchFetcher::registry_entry())?;
    #[cfg(feature = "provider-fred")]
    registry.register(FredHttpYieldCurveFetcher::registry_entry())?;
    #[cfg(feature = "provider-federal-reserve")]
    registry.register(FedMacroSeriesHttpFetcher::registry_entry())?;
    #[cfg(feature = "provider-federal-reserve")]
    registry.register(FedFomcDocumentsHttpFetcher::registry_entry())?;
    #[cfg(feature = "provider-government-us")]
    registry.register(GovUsTreasuryAuctionsHttpFetcher::registry_entry())?;
    #[cfg(feature = "provider-government-us")]
    registry.register(GovUsTreasuryPricesHttpFetcher::registry_entry())?;
    register_extended_providers(&mut registry)?;
    Ok(registry)
}

/// Register the second half of the built-in provider catalog (geckoterminal onward).
///
/// `registry` is only mutated when at least one of the corresponding provider
/// features is enabled, so it is unused under the default (offline) feature set.
#[cfg_attr(
    not(any(
        feature = "provider-geckoterminal",
        feature = "provider-glassnode",
        feature = "provider-huggingface",
        feature = "provider-nasdaq",
        feature = "provider-oecd",
        feature = "provider-polygon",
        feature = "provider-sec",
        feature = "provider-seeking-alpha",
        feature = "provider-tiingo",
        feature = "provider-tmx",
        feature = "provider-tradier",
        feature = "provider-trading-economics",
        feature = "provider-velodata",
        feature = "provider-binance-http",
    )),
    allow(
        unused_variables,
        clippy::unnecessary_wraps,
        clippy::missing_const_for_fn,
        clippy::needless_pass_by_ref_mut
    )
)]
fn register_extended_providers(registry: &mut ProviderRegistry) -> Result<()> {
    #[cfg(feature = "provider-geckoterminal")]
    registry.register(GeckoTerminalHttpFetcher::registry_entry())?;
    #[cfg(feature = "provider-glassnode")]
    registry.register(GlassnodeHttpFetcher::registry_entry())?;
    #[cfg(feature = "provider-huggingface")]
    registry.register(HuggingFaceHttpTextGenerationFetcher::registry_entry())?;
    #[cfg(feature = "provider-nasdaq")]
    registry.register(NasdaqHttpDatasetFetcher::registry_entry())?;
    // Catalog-facing NASDAQ calendar fetcher (G004 part 2).
    #[cfg(feature = "provider-nasdaq")]
    registry.register(NasdaqHttpCalendarFetcher::registry_entry())?;
    #[cfg(feature = "provider-oecd")]
    registry.register(OecdHttpDataFetcher::registry_entry())?;
    #[cfg(feature = "provider-polygon")]
    registry.register(PolygonHttpAggregatesFetcher::registry_entry())?;
    #[cfg(feature = "provider-sec")]
    registry.register(SecFilingsHttpFetcher::registry_entry())?;
    #[cfg(feature = "provider-sec")]
    registry.register(SecXbrlHttpFetcher::registry_entry())?;
    #[cfg(feature = "provider-sec")]
    registry.register(SecCikMapHttpFetcher::registry_entry())?;
    #[cfg(feature = "provider-sec")]
    registry.register(SecForm13FHttpFetcher::registry_entry())?;
    #[cfg(feature = "provider-sec")]
    registry.register(SecFailsToDeliverHttpFetcher::registry_entry())?;
    #[cfg(feature = "provider-sec")]
    registry.register(SecEtfHoldingsHttpFetcher::registry_entry())?;
    #[cfg(feature = "provider-seeking-alpha")]
    registry.register(SeekingAlphaArticlesHttpFetcher::registry_entry())?;
    #[cfg(feature = "provider-seeking-alpha")]
    registry.register(SeekingAlphaRatingsHttpFetcher::registry_entry())?;
    #[cfg(feature = "provider-tiingo")]
    registry.register(TiingoHttpHistoricalFetcher::registry_entry())?;
    #[cfg(feature = "provider-tiingo")]
    registry.register(TiingoHttpNewsFetcher::registry_entry())?;
    #[cfg(feature = "provider-tmx")]
    registry.register(TmxHttpQuoteFetcher::registry_entry())?;
    #[cfg(feature = "provider-tmx")]
    registry.register(TmxHttpBatchQuoteFetcher::registry_entry())?;
    #[cfg(feature = "provider-tradier")]
    registry.register(TradierHttpQuoteFetcher::registry_entry())?;
    #[cfg(feature = "provider-tradier")]
    registry.register(TradierHttpOptionsFetcher::registry_entry())?;
    #[cfg(feature = "provider-trading-economics")]
    registry.register(TradingEconomicsHttpCalendarFetcher::registry_entry())?;
    #[cfg(feature = "provider-trading-economics")]
    registry.register(TradingEconomicsHttpIndicatorFetcher::registry_entry())?;
    #[cfg(feature = "provider-velodata")]
    registry.register(VelodataHttpFundingFetcher::registry_entry())?;
    #[cfg(feature = "provider-velodata")]
    registry.register(VelodataHttpLiquidationsFetcher::registry_entry())?;
    #[cfg(feature = "provider-velodata")]
    registry.register(VelodataHttpOiFetcher::registry_entry())?;
    #[cfg(feature = "provider-binance-http")]
    registry.register(BinanceHttpTickerPriceFetcher::registry_entry())?;
    Ok(())
}

/// # Errors
///
/// Returns an error variant if the underlying operation fails.
pub fn list_providers() -> Result<Vec<ProviderSummary>> {
    Ok(default_registry()?
        .entries()
        .iter()
        .map(|entry| ProviderSummary {
            provider: entry.provider.to_string(),
            endpoint: entry.endpoint.to_string(),
            kind: entry.kind,
        })
        .collect())
}

/// # Errors
///
/// Returns an error variant if the underlying operation fails.
pub fn fetch_equity_historical(
    provider: &str,
    symbol: &str,
) -> Result<OBBject<EquityHistoricalData>> {
    let runner = CommandRunner::new(default_registry()?);
    let params = json!({ "symbol": symbol });

    match provider {
        "fileset" => block_on(runner.run(&FilesetEquityHistoricalFetcher, params)),
        "yahoo" => block_on(runner.run(&SelectedYahooEquityHistoricalFetcher::default(), params)),
        other => Err(Error::Registry(format!("unknown provider: {other}"))),
    }
}

/// Dispatch a raw `(provider, endpoint)` fetch against every compiled-in
/// [`Fetcher`](tdw_core::Fetcher) and return the serialized [`OBBject`].
///
/// This is the generic sibling of [`fetch_equity_historical`]: instead of the
/// two hardcoded equity-historical arms it covers EVERY fetcher that
/// [`default_registry`] (+ `register_extended_providers`) registers, behind the
/// same `#[cfg(feature = "provider-…")]` gates. Streamer-kind entries
/// (`mock-ws`, `binance/trades`) are intentionally excluded — only `Fetcher`
/// kinds are dispatchable here.
///
/// `params` is the provider-specific query object (the same shape
/// `transform_query` expects). The successful result is `serde_json::to_value`
/// of the provider's `OBBject<D>` (which is `Serialize`).
///
/// # Errors
///
/// Returns [`Error::Registry`] for an unknown `(provider, endpoint)` pair, or
/// the fetcher's own error (network, parse, …) on a failed run. A serialization
/// failure surfaces as [`Error::Provider`].
#[allow(clippy::too_many_lines)] // flat provider dispatch table; splitting adds no clarity
pub fn fetch_provider_json(provider: &str, endpoint: &str, params: Value) -> Result<Value> {
    let runner = CommandRunner::new(default_registry()?);

    // Each arm runs one registered fetcher and serializes its OBBject. The
    // string literals mirror the `const PROVIDER`/`const ENDPOINT` on each
    // fetcher's `impl Fetcher<…>`; the trailing comment names the concrete type
    // (and matches the `registry.register(<Type>::registry_entry())` line in
    // `default_registry`/`register_extended_providers`).
    macro_rules! dispatch {
        ($fetcher:expr) => {{
            let obbject = block_on(runner.run(&$fetcher, params))?;
            serde_json::to_value(&obbject).map_err(|error| Error::Provider(error.to_string()))
        }};
    }

    match (provider, endpoint) {
        // FilesetEquityHistoricalFetcher (always registered)
        ("fileset", "equity_historical") => dispatch!(FilesetEquityHistoricalFetcher),
        // SelectedYahooEquityHistoricalFetcher (always registered)
        ("yahoo", "equity_historical") => {
            dispatch!(SelectedYahooEquityHistoricalFetcher::default())
        }
        // AdanosSentimentHttpFetcher
        #[cfg(feature = "provider-adanos")]
        ("adanos", "sentiment") => dispatch!(AdanosSentimentHttpFetcher::default()),
        // AdanosTrendingHttpFetcher
        #[cfg(feature = "provider-adanos")]
        ("adanos", "trending") => dispatch!(AdanosTrendingHttpFetcher::default()),
        // AdanosPolymarketHttpFetcher
        #[cfg(feature = "provider-adanos")]
        ("adanos", "polymarket") => dispatch!(AdanosPolymarketHttpFetcher::default()),
        // AkShareHttpFetcher
        #[cfg(feature = "provider-akshare")]
        ("akshare", "hist") => dispatch!(AkShareHttpFetcher::default()),
        // AlpacaHttpStockBarsFetcher
        #[cfg(feature = "provider-alpaca")]
        ("alpaca", "stock_bars") => dispatch!(AlpacaHttpStockBarsFetcher::default()),
        // AlphaVantageHttpFetcher
        #[cfg(feature = "provider-alpha-vantage")]
        ("alpha_vantage", "market_data") => dispatch!(AlphaVantageHttpFetcher::default()),
        // BenzingaNewsHttpFetcher
        #[cfg(feature = "provider-benzinga")]
        ("benzinga", "news") => dispatch!(BenzingaNewsHttpFetcher::default()),
        // BenzingaEarningsHttpFetcher
        #[cfg(feature = "provider-benzinga")]
        ("benzinga", "earnings") => dispatch!(BenzingaEarningsHttpFetcher::default()),
        // BlsHttpTimeSeriesFetcher
        #[cfg(feature = "provider-bls")]
        ("bls", "timeseries_data") => dispatch!(BlsHttpTimeSeriesFetcher::default()),
        // CboeHttpIndexFetcher
        #[cfg(feature = "provider-cboe")]
        ("cboe", "index_quotes") => dispatch!(CboeHttpIndexFetcher::default()),
        // CboeHttpOptionsFetcher
        #[cfg(feature = "provider-cboe")]
        ("cboe", "options") => dispatch!(CboeHttpOptionsFetcher::default()),
        // CCDataHttpFetcher
        #[cfg(feature = "provider-ccdata")]
        ("ccdata", "crypto_ohlcv") => dispatch!(CCDataHttpFetcher::default()),
        // CoinGeckoHttpOhlcFetcher
        #[cfg(feature = "provider-coingecko")]
        ("coingecko", "ohlc") => dispatch!(CoinGeckoHttpOhlcFetcher::default()),
        // DatabentoHttpTimeseriesFetcher
        #[cfg(feature = "provider-databento")]
        ("databento", "timeseries") => dispatch!(DatabentoHttpTimeseriesFetcher::default()),
        // DatabentoMetadataFetcher
        #[cfg(feature = "provider-databento")]
        ("databento", "metadata") => dispatch!(DatabentoMetadataFetcher::default()),
        // DeribitHttpInstrumentsFetcher
        #[cfg(feature = "provider-deribit")]
        ("deribit", "instruments") => dispatch!(DeribitHttpInstrumentsFetcher::default()),
        // DeribitHttpOrderBookFetcher
        #[cfg(feature = "provider-deribit")]
        ("deribit", "order_book") => dispatch!(DeribitHttpOrderBookFetcher::default()),
        // DeribitHttpFundingFetcher
        #[cfg(feature = "provider-deribit")]
        ("deribit", "funding_rate") => dispatch!(DeribitHttpFundingFetcher::default()),
        // EcbHttpDataFetcher
        #[cfg(feature = "provider-ecb")]
        ("ecb", "data") => dispatch!(EcbHttpDataFetcher::default()),
        // EiaHttpSpotPriceFetcher
        #[cfg(feature = "provider-eia")]
        ("eia", "spot_price") => dispatch!(EiaHttpSpotPriceFetcher::default()),
        // EiaHttpNaturalGasFetcher
        #[cfg(feature = "provider-eia")]
        ("eia", "natural_gas") => dispatch!(EiaHttpNaturalGasFetcher::default()),
        // FinraOtcSummaryHttpFetcher
        #[cfg(feature = "provider-finra")]
        ("finra", "otc_summary") => dispatch!(FinraOtcSummaryHttpFetcher::default()),
        // FinraShortInterestHttpFetcher
        #[cfg(feature = "provider-finra")]
        ("finra", "short_interest") => dispatch!(FinraShortInterestHttpFetcher::default()),
        // FinnhubHttpProfileFetcher
        #[cfg(feature = "provider-finnhub")]
        ("finnhub", "company_profile") => dispatch!(FinnhubHttpProfileFetcher::default()),
        // FinnhubHttpQuoteSnapshotFetcher
        #[cfg(feature = "provider-finnhub")]
        ("finnhub", "quote_snapshot") => dispatch!(FinnhubHttpQuoteSnapshotFetcher::default()),
        // FmpHttpHistoricalFetcher
        #[cfg(feature = "provider-fmp")]
        ("fmp", "equity_historical") => dispatch!(FmpHttpHistoricalFetcher::default()),
        // FmpHttpIncomeFetcher
        #[cfg(feature = "provider-fmp")]
        ("fmp", "income_statement") => dispatch!(FmpHttpIncomeFetcher::default()),
        // FmpHttpQuoteSnapshotFetcher
        #[cfg(feature = "provider-fmp")]
        ("fmp", "quote_snapshot") => dispatch!(FmpHttpQuoteSnapshotFetcher::default()),
        // FredHttpSeriesObservationsFetcher
        #[cfg(feature = "provider-fred")]
        ("fred", "series_observations") => {
            dispatch!(FredHttpSeriesObservationsFetcher::default())
        }
        // GeckoTerminalHttpFetcher
        #[cfg(feature = "provider-geckoterminal")]
        ("geckoterminal", "pool") => dispatch!(GeckoTerminalHttpFetcher::default()),
        // GlassnodeHttpFetcher
        #[cfg(feature = "provider-glassnode")]
        ("glassnode", "metric") => dispatch!(GlassnodeHttpFetcher::default()),
        // HuggingFaceHttpTextGenerationFetcher
        #[cfg(feature = "provider-huggingface")]
        ("huggingface", "text_generation") => {
            dispatch!(HuggingFaceHttpTextGenerationFetcher::default())
        }
        // NasdaqHttpDatasetFetcher
        #[cfg(feature = "provider-nasdaq")]
        ("nasdaq", "datasets") => dispatch!(NasdaqHttpDatasetFetcher::default()),
        // OecdHttpDataFetcher
        #[cfg(feature = "provider-oecd")]
        ("oecd", "sdmx_data") => dispatch!(OecdHttpDataFetcher::default()),
        // PolygonHttpAggregatesFetcher
        #[cfg(feature = "provider-polygon")]
        ("polygon", "aggregates") => dispatch!(PolygonHttpAggregatesFetcher::default()),
        // SecFilingsHttpFetcher
        #[cfg(feature = "provider-sec")]
        ("sec", "filings") => dispatch!(SecFilingsHttpFetcher::default()),
        // SecXbrlHttpFetcher
        #[cfg(feature = "provider-sec")]
        ("sec", "xbrl_revenue") => dispatch!(SecXbrlHttpFetcher::default()),
        // SecCikMapHttpFetcher
        #[cfg(feature = "provider-sec")]
        ("sec", "cik_map") => dispatch!(SecCikMapHttpFetcher::default()),
        // SecForm13FHttpFetcher
        #[cfg(feature = "provider-sec")]
        ("sec", "form_13f") => dispatch!(SecForm13FHttpFetcher::default()),
        // SecFailsToDeliverHttpFetcher
        #[cfg(feature = "provider-sec")]
        ("sec", "fails_to_deliver") => dispatch!(SecFailsToDeliverHttpFetcher::default()),
        // SecEtfHoldingsHttpFetcher
        #[cfg(feature = "provider-sec")]
        ("sec", "etf_holdings") => dispatch!(SecEtfHoldingsHttpFetcher::default()),
        // FedMacroSeriesHttpFetcher
        #[cfg(feature = "provider-federal-reserve")]
        ("federal_reserve", "macro_series") => dispatch!(FedMacroSeriesHttpFetcher::default()),
        // FedFomcDocumentsHttpFetcher
        #[cfg(feature = "provider-federal-reserve")]
        ("federal_reserve", "fomc_documents") => {
            dispatch!(FedFomcDocumentsHttpFetcher::default())
        }
        // GovUsTreasuryAuctionsHttpFetcher
        #[cfg(feature = "provider-government-us")]
        ("government_us", "treasury_auctions") => {
            dispatch!(GovUsTreasuryAuctionsHttpFetcher::default())
        }
        // GovUsTreasuryPricesHttpFetcher
        #[cfg(feature = "provider-government-us")]
        ("government_us", "treasury_prices") => {
            dispatch!(GovUsTreasuryPricesHttpFetcher::default())
        }
        // SeekingAlphaArticlesHttpFetcher (PROVIDER_ID = "seeking-alpha")
        #[cfg(feature = "provider-seeking-alpha")]
        ("seeking-alpha", "articles") => dispatch!(SeekingAlphaArticlesHttpFetcher::default()),
        // SeekingAlphaRatingsHttpFetcher (PROVIDER_ID = "seeking-alpha")
        #[cfg(feature = "provider-seeking-alpha")]
        ("seeking-alpha", "ratings") => dispatch!(SeekingAlphaRatingsHttpFetcher::default()),
        // TiingoHttpHistoricalFetcher
        #[cfg(feature = "provider-tiingo")]
        ("tiingo", "historical") => dispatch!(TiingoHttpHistoricalFetcher::default()),
        // TiingoHttpNewsFetcher
        #[cfg(feature = "provider-tiingo")]
        ("tiingo", "news") => dispatch!(TiingoHttpNewsFetcher::default()),
        // TmxHttpQuoteFetcher
        #[cfg(feature = "provider-tmx")]
        ("tmx", "equity_quote") => dispatch!(TmxHttpQuoteFetcher::default()),
        // TmxHttpBatchQuoteFetcher
        #[cfg(feature = "provider-tmx")]
        ("tmx", "equity_batch_quote") => dispatch!(TmxHttpBatchQuoteFetcher::default()),
        // TradierHttpQuoteFetcher
        #[cfg(feature = "provider-tradier")]
        ("tradier", "quote") => dispatch!(TradierHttpQuoteFetcher::default()),
        // TradierHttpOptionsFetcher
        #[cfg(feature = "provider-tradier")]
        ("tradier", "options_chain") => dispatch!(TradierHttpOptionsFetcher::default()),
        // TradingEconomicsHttpCalendarFetcher
        #[cfg(feature = "provider-trading-economics")]
        ("trading_economics", "calendar") => {
            dispatch!(TradingEconomicsHttpCalendarFetcher::default())
        }
        // TradingEconomicsHttpIndicatorFetcher
        #[cfg(feature = "provider-trading-economics")]
        ("trading_economics", "indicator") => {
            dispatch!(TradingEconomicsHttpIndicatorFetcher::default())
        }
        // VelodataHttpFundingFetcher
        #[cfg(feature = "provider-velodata")]
        ("velodata", "funding_rates") => dispatch!(VelodataHttpFundingFetcher::default()),
        // VelodataHttpLiquidationsFetcher
        #[cfg(feature = "provider-velodata")]
        ("velodata", "liquidations_aggregated") => {
            dispatch!(VelodataHttpLiquidationsFetcher::default())
        }
        // VelodataHttpOiFetcher
        #[cfg(feature = "provider-velodata")]
        ("velodata", "oi_aggregated") => dispatch!(VelodataHttpOiFetcher::default()),
        // BinanceHttpTickerPriceFetcher
        #[cfg(feature = "provider-binance-http")]
        ("binance", "ticker_price") => dispatch!(BinanceHttpTickerPriceFetcher::default()),
        _ => Err(Error::Registry(format!(
            "no fetcher for {provider}/{endpoint}"
        ))),
    }
}

/// The `(provider, endpoint)` pairs [`fetch_provider_json`] can dispatch IN THIS
/// BUILD.
///
/// Gated by the exact same `#[cfg(feature = "provider-…")]` set as the match
/// arms above, so the MCP layer can advertise an honest tool description
/// reflecting only what is actually compiled in.
#[must_use]
#[allow(clippy::too_many_lines)] // flat provider target list mirrors dispatch table; splitting adds no clarity
pub fn provider_fetch_targets() -> Vec<(String, String)> {
    // `targets` is only mutated, and `target!` is only invoked, when at least
    // one provider feature is enabled; both are unused under the default
    // (offline) feature set.
    #[allow(unused_mut)]
    let mut targets: Vec<(String, String)> = vec![
        ("fileset".to_string(), "equity_historical".to_string()),
        ("yahoo".to_string(), "equity_historical".to_string()),
    ];
    #[allow(unused_macros)]
    macro_rules! target {
        ($provider:expr, $endpoint:expr) => {
            targets.push(($provider.to_string(), $endpoint.to_string()));
        };
    }
    #[cfg(feature = "provider-adanos")]
    {
        target!("adanos", "sentiment");
        target!("adanos", "trending");
        target!("adanos", "polymarket");
    }
    #[cfg(feature = "provider-akshare")]
    target!("akshare", "hist");
    #[cfg(feature = "provider-alpaca")]
    target!("alpaca", "stock_bars");
    #[cfg(feature = "provider-alpha-vantage")]
    target!("alpha_vantage", "market_data");
    #[cfg(feature = "provider-benzinga")]
    {
        target!("benzinga", "news");
        target!("benzinga", "earnings");
    }
    #[cfg(feature = "provider-bls")]
    target!("bls", "timeseries_data");
    #[cfg(feature = "provider-cboe")]
    {
        target!("cboe", "index_quotes");
        target!("cboe", "options");
    }
    #[cfg(feature = "provider-ccdata")]
    target!("ccdata", "crypto_ohlcv");
    #[cfg(feature = "provider-coingecko")]
    target!("coingecko", "ohlc");
    #[cfg(feature = "provider-databento")]
    {
        target!("databento", "timeseries");
        target!("databento", "metadata");
    }
    #[cfg(feature = "provider-deribit")]
    {
        target!("deribit", "instruments");
        target!("deribit", "order_book");
        target!("deribit", "funding_rate");
    }
    #[cfg(feature = "provider-ecb")]
    target!("ecb", "data");
    #[cfg(feature = "provider-eia")]
    {
        target!("eia", "spot_price");
        target!("eia", "natural_gas");
    }
    #[cfg(feature = "provider-finra")]
    {
        target!("finra", "otc_summary");
        target!("finra", "short_interest");
    }
    #[cfg(feature = "provider-finnhub")]
    {
        target!("finnhub", "company_profile");
        target!("finnhub", "quote_snapshot");
    }
    #[cfg(feature = "provider-fmp")]
    {
        target!("fmp", "equity_historical");
        target!("fmp", "income_statement");
        target!("fmp", "quote_snapshot");
    }
    #[cfg(feature = "provider-fred")]
    target!("fred", "series_observations");
    #[cfg(feature = "provider-geckoterminal")]
    target!("geckoterminal", "pool");
    #[cfg(feature = "provider-glassnode")]
    target!("glassnode", "metric");
    #[cfg(feature = "provider-huggingface")]
    target!("huggingface", "text_generation");
    #[cfg(feature = "provider-nasdaq")]
    target!("nasdaq", "datasets");
    #[cfg(feature = "provider-oecd")]
    target!("oecd", "sdmx_data");
    #[cfg(feature = "provider-polygon")]
    target!("polygon", "aggregates");
    #[cfg(feature = "provider-sec")]
    {
        target!("sec", "filings");
        target!("sec", "xbrl_revenue");
        target!("sec", "cik_map");
        target!("sec", "form_13f");
        target!("sec", "fails_to_deliver");
        target!("sec", "etf_holdings");
    }
    #[cfg(feature = "provider-federal-reserve")]
    {
        target!("federal_reserve", "macro_series");
        target!("federal_reserve", "fomc_documents");
    }
    #[cfg(feature = "provider-government-us")]
    {
        target!("government_us", "treasury_auctions");
        target!("government_us", "treasury_prices");
    }
    #[cfg(feature = "provider-seeking-alpha")]
    {
        target!("seeking-alpha", "articles");
        target!("seeking-alpha", "ratings");
    }
    #[cfg(feature = "provider-tiingo")]
    {
        target!("tiingo", "historical");
        target!("tiingo", "news");
    }
    #[cfg(feature = "provider-tmx")]
    {
        target!("tmx", "equity_quote");
        target!("tmx", "equity_batch_quote");
    }
    #[cfg(feature = "provider-tradier")]
    {
        target!("tradier", "quote");
        target!("tradier", "options_chain");
    }
    #[cfg(feature = "provider-trading-economics")]
    {
        target!("trading_economics", "calendar");
        target!("trading_economics", "indicator");
    }
    #[cfg(feature = "provider-velodata")]
    {
        target!("velodata", "funding_rates");
        target!("velodata", "liquidations_aggregated");
        target!("velodata", "oi_aggregated");
    }
    #[cfg(feature = "provider-binance-http")]
    target!("binance", "ticker_price");
    targets
}

/// # Errors
///
/// Returns an error variant if the underlying operation fails.
pub fn mcp_progress_sample(symbol: &str) -> Result<Vec<String>> {
    let runner = CommandRunner::new(default_registry()?);
    let mut stream = block_on(
        runner.run_streaming(&FilesetEquityHistoricalFetcher, json!({ "symbol": symbol })),
    )?;
    let mut events = Vec::new();

    while let Some(event) = poll_stream_next(&mut stream)? {
        match event {
            ProgressOrResult::Progress {
                stage, fraction, ..
            } => events.push(format!("progress:{stage}:{fraction:.1}")),
            ProgressOrResult::Partial(_) => events.push("partial".to_string()),
            ProgressOrResult::Done(object) => {
                events.push(format!("done:{}:{}", object.provider, object.rows.len()));
            }
            ProgressOrResult::Error(error) => events.push(format!("error:{error}")),
        }
    }

    Ok(events)
}

/// # Errors
///
/// Returns an error variant if the underlying operation fails.
pub fn protocol_config_sample() -> Result<Value> {
    let config = merge_layers(&[
        ConfigLayer::from_toml(
            ConfigLayerKind::ProjectConfig,
            "project",
            r#"
profile = "service"

[model]
provider = "openai-compatible"
model = "unset"
"#,
        )
        .map_err(|error| Error::Provider(error.to_string()))?,
        ConfigLayer::new(
            ConfigLayerKind::CliFlags,
            "cli",
            json!({
                "protocol": { "max_event_bytes": 4096 },
                "permissions": { "last_match_wins": true }
            }),
        ),
    ])
    .map_err(|error| Error::Provider(error.to_string()))?;
    let session_id = SessionId::new("session-service-sample")
        .map_err(|error| Error::Provider(error.to_string()))?;
    let actor = ActorRef {
        actor_id: "system:tdw-service-api".to_string(),
        kind: ActorKind::Service,
        tenant_id: Some("default".to_string()),
    };
    let op = OpEnvelope::new(
        session_id,
        1,
        actor,
        Op::ApprovalResponse {
            permission_id: PermissionId::new("approval-service-sample")
                .map_err(|error| Error::Provider(error.to_string()))?,
            decision: tdw_protocol::ApprovalDecision::AllowOnce,
            reason: Some("sample".to_string()),
        },
    );
    let event = EventMsg::Started { op_id: op.op_id };

    Ok(json!({
        "profile": config.profile,
        "layer_order": default_layer_order().iter().map(|layer| layer.source).collect::<Vec<_>>(),
        "max_event_bytes": config.protocol.max_event_bytes,
        "protocol_schemas": protocol_schema_bundle().keys().copied().collect::<Vec<_>>(),
        "op_sequence": op.sequence,
        "event": event,
    }))
}

/// # Errors
///
/// Returns an error variant if the underlying operation fails.
pub async fn index_research_note(note: ResearchNote) -> Result<ResearchIndexEvidence> {
    let embedder = HashEmbeddingProvider::default();
    let embedding = embedder
        .embed(&format!("{} {}", note.title, note.body))
        .await
        .map_err(|error| Error::Provider(error.to_string()))?;
    let vector = InMemoryVectorEngine::default();
    let lexical = InMemoryLexicalEngine::default();
    let blob = InMemoryS3BlobEngine::default();
    let blob_key = format!("research-note/{}.json", note.id);
    let payload = json!({
        "id": note.id,
        "title": note.title,
        "tags": note.tags,
    });

    vector
        .upsert(
            "research_note__local_hash",
            vec![VectorPoint {
                id: note.id.clone(),
                vector: embedding.vector.clone(),
                payload: payload.clone(),
            }],
        )
        .await?;
    lexical
        .index(
            "research_note",
            vec![LexicalDoc {
                id: note.id.clone(),
                body: note.body.clone(),
                fields: payload,
            }],
        )
        .await?;
    let serialized =
        serde_json::to_vec(&note).map_err(|error| Error::Storage(error.to_string()))?;
    blob.put_object(&blob_key, Bytes::from(serialized), "application/json")
        .await?;

    let vector_hits = vector
        .search_knn(
            "research_note__local_hash",
            VectorQuery {
                vector: embedding.vector,
                top_k: 1,
                filter: tdw_core::PayloadFilter::default(),
            },
        )
        .await?;
    let lexical_hits = lexical
        .search_text(
            "research_note",
            TextQuery {
                text: "Fixture".to_string(),
                top_k: 1,
                filter: tdw_core::PayloadFilter::default(),
            },
        )
        .await?;
    let blob_bytes = blob.get_object(&blob_key).await?.len();

    Ok(ResearchIndexEvidence {
        note_id: note.id,
        model_id: embedding.model_id,
        vector_hits,
        lexical_hits,
        blob_bytes,
    })
}

/// # Errors
///
/// Returns an error variant if the underlying operation fails.
pub fn endpoint_response(provider: &str, symbol: &str) -> Result<Value> {
    let object = fetch_equity_historical(provider, symbol)?;
    Ok(json!({
        "provider": object.provider,
        "endpoint": object.endpoint,
        "rows": object.rows,
    }))
}

#[must_use]
pub fn agent_schema_names() -> Vec<String> {
    schema_bundle()
        .keys()
        .map(|name| (*name).to_string())
        .collect()
}

#[must_use]
pub fn mcp_agent_tools() -> Vec<String> {
    vec![
        "agent.card.get".to_string(),
        "agent.skill.parse".to_string(),
        "agent.command.parse".to_string(),
        "agent.eval.run".to_string(),
        "agent.workflow.compile".to_string(),
        "agent.gotcha.list".to_string(),
    ]
}

#[must_use]
pub fn mcp_extensibility_tools() -> Vec<String> {
    vec![
        "tdw.query.plan".to_string(),
        "tdw.query.run".to_string(),
        "tdw.ingest.run".to_string(),
        "tdw.agent.run".to_string(),
        "tdw.kg.search".to_string(),
        "tdw.udf.run".to_string(),
    ]
}

/// # Errors
///
/// Returns an error variant if the underlying operation fails.
pub fn extensibility_sample() -> Result<Value> {
    let mut registry = ToolRegistry::default();
    registry
        .register(echo_tool())
        .map_err(|error| Error::Provider(error.to_string()))?;
    let mut permissions = tdw_hooks::PermissionRules::default();
    permissions.push(tdw_hooks::PermissionRule::new(
        tdw_hooks::PermissionEffect::Allow,
        "tdw.echo",
        "tdw.echo",
    ));
    let orchestrator = ToolOrchestrator::new(registry, permissions);
    let tool = orchestrator
        .run(
            tdw_protocol::ToolCallId::new("tool-call-1")
                .map_err(|error| Error::Provider(error.to_string()))?,
            "tdw.echo",
            json!({"symbol": "AAPL"}),
        )
        .map_err(|error| Error::Provider(error.to_string()))?;
    let sandbox = LocalUdfSandbox;
    let udf = sandbox
        .run(UdfRequest {
            name: "upper".to_string(),
            runtime: UdfRuntime::Wasm,
            source: "upper(input)".to_string(),
            input: "aapl".to_string(),
            allow_network: false,
            allow_filesystem: false,
            wasm_limits: None,
        })
        .map_err(|error| Error::Provider(error.to_string()))?;
    let acp = AcpServerInfo::default();
    validate_request(&AcpRequest::Initialize {
        client_name: "tdw-mcp".to_string(),
    })
    .map_err(|error| Error::Provider(error.to_string()))?;

    Ok(json!({
        "tool": tool,
        "sandbox_runtime": sandbox.runtime_name(),
        "udf_output": udf.output,
        "mcp_tools": mcp_extensibility_tools(),
        "acp": acp,
        "acp_validated": true,
    }))
}

/// # Errors
///
/// Returns an error variant if the underlying operation fails.
pub fn agent_tool_sample() -> Result<Value> {
    let mut store = AgentStore::new();
    let card = sample_agent_card();
    store.upsert_agent(card.clone());
    for gotcha in gotcha_seed() {
        store.upsert_gotcha(gotcha);
    }

    let workflow = WorkflowDefinition {
        meta: EntityMeta::new(
            "research-flow",
            "research-flow",
            "0.1.0",
            Origin {
                tier: Tier::Domain,
                source: Source::Internal,
            },
            Adaptivity::Configured,
            false,
        ),
        nodes: vec![
            WorkflowNode {
                node_id: "retrieve".to_string(),
                task: "retrieve context".to_string(),
                skill_id: None,
            },
            WorkflowNode {
                node_id: "draft".to_string(),
                task: "draft note".to_string(),
                skill_id: Some("research.note".to_string()),
            },
        ],
        edges: vec![WorkflowEdge {
            from: "retrieve".to_string(),
            to: "draft".to_string(),
        }],
    };
    let plan =
        WorkflowEngine::compile(&workflow).map_err(|error| Error::Provider(error.to_string()))?;
    store.upsert_workflow(workflow);

    // Inject the deterministic offline stub so this sample never reaches the network.
    let eval_runner = EvalRunner::new(Arc::new(StubLanguageModel));
    let eval = eval_runner.run(
        EvalRunRequest {
            run_id: "eval-1".to_string(),
            agent_id: card.meta.id.clone(),
            dataset_id: "golden-market-notes".to_string(),
            cases: vec![EvalCase {
                case_id: "case-1".to_string(),
                prompt: "Summarize AAPL".to_string(),
                expected_refs: card.content_refs.clone(),
            }],
        },
        &mut store,
    );
    let command = parse_slash_command_invocation("/research symbol=AAPL horizon=1d")
        .map_err(|error| Error::Provider(error.to_string()))?;

    Ok(json!({
        "agent_id": card.meta.id,
        "schemas": agent_schema_names(),
        "tools": mcp_agent_tools(),
        "workflow_order": plan.ordered_node_ids,
        "eval_status": eval.status,
        "slash_command": command,
        "storage_mappings": store.storage_mappings(),
    }))
}

/// Project every MCP-exposable `tool` resource in `registry` onto its MCP wire form.
///
/// Iterates the registry, runs [`project_to_mcp`] on each resource, and collects the
/// [`McpEntity::Tool`] variants. Resources that fail to project (a `serde_json::Error`)
/// or that are not tools are skipped silently — this is a best-effort surface for
/// `tools/list`, not a validation pass.
#[must_use]
pub fn registry_mcp_tools(registry: &Registry) -> Vec<McpTool> {
    registry
        .iter()
        .filter_map(|resource| match project_to_mcp(resource) {
            Ok(Some(McpEntity::Tool(tool))) => Some(tool),
            Ok(Some(McpEntity::Prompt(_)) | None) | Err(_) => None,
        })
        .collect()
}

/// Project every MCP-exposable `prompt` resource in `registry` onto its MCP wire form.
///
/// Iterates the registry, runs [`project_to_mcp`] on each resource, and collects the
/// [`McpEntity::Prompt`] variants. Resources that fail to project or that are not prompts
/// are skipped silently.
#[must_use]
pub fn registry_mcp_prompts(registry: &Registry) -> Vec<McpPrompt> {
    registry
        .iter()
        .filter_map(|resource| match project_to_mcp(resource) {
            Ok(Some(McpEntity::Prompt(prompt))) => Some(prompt),
            Ok(Some(McpEntity::Tool(_)) | None) | Err(_) => None,
        })
        .collect()
}

#[must_use]
pub fn event_schema_names() -> Vec<String> {
    event_schema_bundle()
        .keys()
        .map(|name| (*name).to_string())
        .collect()
}

/// # Errors
///
/// Returns an error variant if the underlying operation fails.
pub fn event_spine_sample(entrypoint: &str) -> Result<Value> {
    let context = ActorContext::service(entrypoint);
    let task = OmcSpawn::capture(&context, "ingress");
    let envelope = EventEnvelope::new(
        "ingress.received",
        task.actor.clone(),
        task.origin.clone(),
        task.trace.clone(),
        "2026-05-21T00:00:00Z",
        json!({ "entrypoint": entrypoint, "task": task.task_name }),
    );

    let mut bus = EventBus::new(16);
    let sequence = bus.publish(envelope.clone());
    let mut hooks = HookRegistry::default();
    hooks.register(event_hook!("audit", 10, TransactionMode::PostCommit));
    hooks.register(
        HookSpec::new("policy_context", 15, TransactionMode::InTransaction)
            .for_event(HookEvent::PreToolCall)
            .with_handler(HandlerKind::Prompt {
                prompt_path: "crates/tdw-hooks/src/tool_prompt.txt".to_string(),
            })
            .with_context(AdditionalContext {
                uri: "tdw://context/hook-policy".to_string(),
                body: "policy context".to_string(),
                priority: 10,
            }),
    );
    hooks.register(event_hook!("cdc", 20, TransactionMode::InTransaction));
    let hook_outcomes = hooks
        .execute_runtime(&envelope)
        .map_err(|error| Error::Provider(error.to_string()))?;

    let mut outbox = InMemoryOutbox::default();
    outbox.append(envelope.clone());
    let pending = outbox.pending_after(0);
    let cdc = CdcStream::from_outbox(&pending);
    let replay = ReplayEngine::dry_run(&cdc.records);

    Ok(json!({
        "entrypoint": entrypoint,
        "actor_id": envelope.actor.actor_id,
        "trace_id": envelope.trace.trace_id,
        "bus_sequence": sequence,
        "bus_lag": bus.lag_since(0),
        "hook_order": hook_outcomes.iter().map(|hook| hook.name.clone()).collect::<Vec<_>>(),
        "hook_contexts": hook_outcomes.iter().flat_map(|hook| hook.additional_contexts.iter().map(|context| context.uri.clone())).collect::<Vec<_>>(),
        "hook_can_stop": hook_outcomes.iter().any(|hook| hook.should_stop),
        "outbox_pending": pending.len(),
        "cdc_offsets": cdc.records.iter().map(|record| record.offset).collect::<Vec<_>>(),
        "replay_dry_run": replay.dry_run,
        "schemas": event_schema_names(),
    }))
}

/// # Errors
///
/// Returns an error variant if the underlying operation fails.
pub fn parity_layer_sample() -> Result<Value> {
    let mut snapshots = SnapshotStore::default();
    snapshots.commit(
        "raw.market_data_bar",
        "2026-05-21T00:00:00Z",
        vec!["bar-1".into()],
    );
    let snapshot = snapshots.commit(
        "raw.market_data_bar",
        "2026-05-21T01:00:00Z",
        vec!["bar-1".into(), "bar-2".into()],
    );
    let time_travel_rows = snapshots
        .as_of_version("raw.market_data_bar", 1)
        .map(|snapshot| snapshot.row_ids.len())
        .ok_or_else(|| Error::Provider("missing time-travel snapshot version 1".to_string()))?;

    let mut event_bus = EventBus::new(8);
    let stream_offset = event_bus.publish(EventEnvelope::new(
        "stream.market_data_bar",
        tdw_event::sample_actor_context("parity").0,
        tdw_event::sample_actor_context("parity").1,
        tdw_event::sample_actor_context("parity").2,
        "2026-05-21T00:00:00Z",
        json!({ "snapshot_version": snapshot.version }),
    ));

    parity_layer_evidence(snapshot.version, time_travel_rows, stream_offset)
}

/// Assemble the parity-layer evidence payload from the snapshot version,
/// time-travel row count, and stream offset.
///
/// # Errors
///
/// Returns an error variant if the underlying operation fails.
fn parity_layer_evidence(
    snapshot_version: u64,
    time_travel_rows: usize,
    stream_offset: u64,
) -> Result<Value> {
    let mut graph = DirectedGraph::default();
    graph.add_edge("account", "position");
    graph.add_edge("position", "instrument");
    let bbox = BoundingBox {
        min: Point {
            lat: 40.0,
            lon: -75.0,
        },
        max: Point {
            lat: 41.0,
            lon: -73.0,
        },
    };
    let stage = StageLocation {
        name: "market-stage".to_string(),
        uri: "s3://bucket/market".to_string(),
    };
    let mut pipe = PipeDefinition {
        name: "market-pipe".to_string(),
        stage,
        target_table: "raw.market_data_bar".to_string(),
        last_offset: 0,
    };
    let copy_plan = pipe
        .copy_plan(vec!["ohlcv.parquet".to_string()])
        .map_err(|error| Error::Storage(error.to_string()))?;
    pipe.advance(stream_offset);
    let manifest_file = TableFile::from_reader(
        "s3://bucket/market/ohlcv.parquet",
        std::io::Cursor::new(b"demo-content"),
    )
    .map_err(|error| Error::Storage(error.to_string()))?;
    let manifest = TableManifest {
        format: TableFormat::Iceberg,
        table: "raw.market_data_bar".to_string(),
        version: snapshot_version,
        files: vec![manifest_file],
    };
    let udf_output = evaluate(
        &UdfDefinition {
            name: "upper".to_string(),
            runtime: UdfRuntime::Wasm,
            source: "upper(input)".to_string(),
            allow_network: false,
            allow_filesystem: false,
        },
        "aapl",
    )
    .map_err(|error| Error::Provider(error.to_string()))?;
    let (claims, auth_policy, define, masked_account) = parity_auth_mask_evidence()?;

    Ok(json!({
        "snapshot_version": snapshot_version,
        "time_travel_rows": time_travel_rows,
        "stream_offset": stream_offset,
        "live_query_event_type": "stream.market_data_bar",
        "graph_path": graph.traverse("account"),
        "spatial_contains": bbox.contains(Point { lat: 40.7, lon: -74.0 }),
        "copy_checksum": copy_plan.checksum,
        "pipe_offset": pipe.last_offset,
        "table_manifest_ok": manifest.verify_checksums(|_| Ok(std::io::Cursor::new(b"demo-content"))).is_ok(),
        "udf_output": udf_output,
        "jwt_valid": validate_claims(
            &claims,
            &[JwksKey { kid: "k1".to_string(), alg: "RS256".to_string() }],
            "https://issuer",
            "tdw"
        ),
        "authorized": authorize(
            &Principal { subject: "alice".to_string(), roles: claims.roles },
            &auth_policy
        ),
        "define_hook": define.compile_hook().name,
        "define_key": define.idempotency_key(),
        "mask_hook": masking_hook().name,
        "masked_account": masked_account,
    }))
}

/// Build the JWT claims, auth policy, define event, and masked account fields
/// used by the parity-layer evidence payload.
///
/// # Errors
///
/// Returns an error variant if the underlying operation fails.
fn parity_auth_mask_evidence() -> Result<(JwtClaims, AuthPolicy, DefineEvent, String)> {
    let claims = JwtClaims {
        sub: "alice".to_string(),
        iss: "https://issuer".to_string(),
        aud: "tdw".to_string(),
        kid: "k1".to_string(),
        roles: vec!["analyst".to_string()],
    };
    let auth_policy = AuthPolicy {
        table: "analytics.gold_daily_returns".to_string(),
        required_role: "analyst".to_string(),
        row_filter: Some("tenant_id = current_tenant()".to_string()),
    };
    let define = DefineEvent {
        event_name: "market_data_changed".to_string(),
        on_table: "raw.market_data_bar".to_string(),
        hook_name: "emit.market_data_changed".to_string(),
        transaction_mode: TransactionMode::PostCommit,
    };
    let mut row = BTreeMap::new();
    row.insert("account_id".to_string(), "ACC123456".to_string());
    let masked = apply_masks(
        &row,
        &[MaskRule {
            field: "account_id".to_string(),
            mode: MaskMode::Last4,
        }],
    );
    let masked_account = masked
        .get("account_id")
        .cloned()
        .ok_or_else(|| Error::Provider("masked account_id missing".to_string()))?;

    Ok((claims, auth_policy, define, masked_account))
}

#[must_use]
pub fn mcp_tag_tools() -> Vec<String> {
    vec![
        "kg.entity.get".to_string(),
        "kg.neighbors".to_string(),
        "tag.define".to_string(),
        "tag.assign".to_string(),
        "tag.rule.reload".to_string(),
        "tag.live.subscribe".to_string(),
    ]
}

/// # Errors
///
/// Returns an error variant if the underlying operation fails.
pub fn kg_tag_sample() -> Result<Value> {
    let instrument = Entity {
        entity_id: "instrument:AAPL".to_string(),
        kind: EntityKind::Instrument,
        label: "Apple".to_string(),
        aliases: vec!["AAPL".to_string()],
    };
    let dataset = Entity {
        entity_id: "dataset:ohlcv".to_string(),
        kind: EntityKind::Dataset,
        label: "OHLCV".to_string(),
        aliases: Vec::new(),
    };
    let mut kg = KnowledgeGraph::default();
    kg.upsert_entity(instrument.clone());
    kg.upsert_entity(dataset.clone());
    kg.add_relationship(Relationship {
        from: instrument.entity_id.clone(),
        to: dataset.entity_id,
        rel_type: "has_prices".to_string(),
        provenance: "dbt:bronze_ohlcv".to_string(),
    });
    let merge = manual_merge_decision("instrument:AAPL", "instrument:APPLE", true);
    if merge.approved {
        kg.manual_merge(&merge.source, &merge.target, "architect");
    }

    let resolved = resolve_symbol("aapl", std::slice::from_ref(&instrument));
    let mut tags = TagStore::default();
    tags.define(TagDefinition {
        tag_id: "asset:equity".to_string(),
        parent: None,
        ttl_days: None,
    })
    .map_err(|error| Error::Provider(error.to_string()))?;
    tags.define(TagDefinition {
        tag_id: "style:momentum".to_string(),
        parent: Some("asset:equity".to_string()),
        ttl_days: Some(30),
    })
    .map_err(|error| Error::Provider(error.to_string()))?;
    tags.assign(TagAssignment {
        entity_id: instrument.entity_id.clone(),
        tag_id: "asset:equity".to_string(),
        assigned_at: "2026-05-21".to_string(),
        expires_at: None,
        provenance: "manual:seed".to_string(),
    })
    .map_err(|error| Error::Provider(error.to_string()))?;

    let mut rules = RuleEngine::default();
    rules
        .hot_reload(vec![TagRule {
            rule_id: "momentum-label".to_string(),
            tag_id: "style:momentum".to_string(),
            predicate: RulePredicate::LabelContains {
                label: "AAPL".to_string(),
            },
        }])
        .map_err(|error| Error::Provider(error.to_string()))?;
    let rule_assignments = rules
        .apply(
            &instrument.entity_id,
            "AAPL momentum",
            "2026-05-21",
            &mut tags,
        )
        .map_err(|error| Error::Provider(error.to_string()))?;

    let mut features = BTreeMap::new();
    features.insert("return_1d".to_string(), 0.012);
    let mut feature_store = FeatureStore::default();
    let feature_snapshot =
        feature_store.materialize(&instrument.entity_id, "2026-05-21", features, &tags);
    let mut live_bus = EventBus::new(8);
    let live_event = tdw_event::sample_event("tag-live").child_event(
        "tag.assignment.changed",
        json!({ "entity_id": instrument.entity_id.clone(), "tags": feature_snapshot.tags }),
    );
    let live_offset = live_bus.publish(live_event);

    Ok(json!({
        "entity": instrument.entity_id,
        "neighbors": kg.neighbors("instrument:AAPL").iter().map(|entity| entity.entity_id.clone()).collect::<Vec<_>>(),
        "resolved": resolved.iter().map(|candidate| candidate.entity_id.clone()).collect::<Vec<_>>(),
        "manual_merge_audited": merge.audited,
        "active_tags": tags.active_tags("instrument:AAPL", "2026-05-21"),
        "rule_version": rules.version(),
        "rule_assignments": rule_assignments.len(),
        "taxonomy_stats": tags.taxonomy_stats(),
        "hybrid_search_filter": "tag:asset:equity",
        "dbt_model": "meta_tag_assignments",
        "agent_tag_interests": ["asset:equity", "style:momentum"],
        "mcp_tools": mcp_tag_tools(),
        "live_offset": live_offset,
        "feature_tags": feature_snapshot.tags,
        "feature_count": feature_snapshot.features.len(),
    }))
}

/// # Errors
///
/// Returns an error variant if the underlying operation fails.
pub fn llm_knowledge_sample() -> Result<Value> {
    let anthropic = AnthropicMessagesModel::new("claude-fixture")
        .map_err(|error| Error::Provider(error.to_string()))?;
    let openai_compat = OpenAiCompatibleModel::new(
        "openai-compatible-fixture",
        Some("http://localhost:11434".to_string()),
    )
    .map_err(|error| Error::Provider(error.to_string()))?;
    let response = anthropic
        .complete(ChatRequest {
            messages: vec![ChatMessage {
                role: MessageRole::User,
                content: "Summarize AAPL momentum".to_string(),
            }],
            max_output_tokens: 128,
        })
        .map_err(|error| Error::Provider(error.to_string()))?;

    let mut index = KnowledgeIndex::default();
    block_on(index.index_document_at(
        KnowledgeDocument {
            id: "doc-1".to_string(),
            body: "AAPL equity momentum research".to_string(),
            entity: Entity {
                entity_id: "instrument:AAPL".to_string(),
                kind: EntityKind::Instrument,
                label: "Apple".to_string(),
                aliases: vec!["AAPL".to_string()],
            },
            tags: vec!["asset:equity".to_string()],
            source: None,
            plane: None,
            as_of: None,
            mentions: Vec::new(),
        },
        "2026-05-22",
    ))
    .map_err(|error| Error::Provider(error.to_string()))?;
    let hits = block_on(index.search("AAPL momentum", 1))
        .map_err(|error| Error::Provider(error.to_string()))?;
    let syntax = summarize_syntax("create table raw.market_data_bar (symbol text);");

    Ok(json!({
        "anthropic_model": response.model_id,
        "anthropic_message": response.message.content,
        "openai_base_url": openai_compat.base_url(),
        "knowledge_hits": hits,
        "active_tags": index.active_tags("instrument:AAPL", "2026-05-22"),
        "syntax_symbols": syntax.symbols,
    }))
}

/// # Errors
///
/// Returns an error variant if the underlying operation fails.
pub fn client_event_sample() -> Result<Value> {
    let session_id = SessionId::new("session-client-sample")
        .map_err(|error| Error::Provider(error.to_string()))?;
    let op = Op::RunQuery {
        sql: "select 1".to_string(),
        plan_id: None,
        cost_hint: None,
    };
    validate_request(&AcpRequest::SubmitOp {
        session_id: session_id.clone(),
        op: op.clone(),
    })
    .map_err(|error| Error::Provider(error.to_string()))?;
    let envelope = OpEnvelope::new(
        session_id.clone(),
        1,
        ActorRef {
            actor_id: "user:cli".to_string(),
            kind: ActorKind::User,
            tenant_id: Some("default".to_string()),
        },
        op,
    );
    let endpoint = DaemonEndpoint {
        transport: DaemonTransport::Uds,
        address: "~/.tdw/daemon.sock".to_string(),
    };
    validate_endpoint(&endpoint).map_err(|error| Error::Provider(error.to_string()))?;
    let (handle, mut daemon_events, mut daemon_loop) = channel();
    let client = AppClient::try_new(
        ClientInfo {
            name: "tdw-cli".to_string(),
            endpoint,
        },
        handle,
    )
    .map_err(|error| Error::Provider(error.to_string()))?;
    let daemon_envelope = OpEnvelope::new(
        session_id.clone(),
        2,
        ActorRef {
            actor_id: "user:cli".to_string(),
            kind: ActorKind::User,
            tenant_id: Some("default".to_string()),
        },
        Op::Shutdown,
    );
    client
        .submit(daemon_envelope)
        .map_err(|_| Error::Provider("daemon submission failed".to_string()))?;
    let daemon_event = block_on(daemon_loop.run_once())
        .ok_or_else(|| Error::Provider("daemon did not emit an event".to_string()))?;
    let daemon_observed_event = block_on(daemon_events.recv())
        .ok_or_else(|| Error::Provider("daemon event channel was empty".to_string()))?;

    let run = try_run_headless(envelope).map_err(|error| Error::Provider(error.to_string()))?;
    let lines = event_lines(&run.events);
    let rollout_records = run
        .events
        .iter()
        .cloned()
        .enumerate()
        .map(|(index, event)| RolloutRecord {
            recorded_at: "2026-05-22T00:00:00Z".to_string(),
            frame: ReplayFrame {
                session_id: session_id.clone(),
                sequence: (index + 1) as u64,
                event,
            },
        })
        .collect::<Vec<_>>();
    let replay = ReplayEngine::from_rollout(&rollout_records);

    Ok(json!({
        "events": run.events,
        "tui_lines": lines.iter().map(|line| line.spans[0].content.to_string()).collect::<Vec<_>>(),
        "client_name": client.info().name.clone(),
        "daemon_event": daemon_event,
        "daemon_observed_event": daemon_observed_event,
        "replay": replay,
    }))
}

struct NoopWake;

impl Wake for NoopWake {
    fn wake(self: Arc<Self>) {}
}

fn block_on<F>(future: F) -> F::Output
where
    F: Future + Send,
    F::Output: Send,
{
    // Live HTTP fetchers (reqwest) need a reactor and timers; the
    // deterministic in-memory futures don't care either way. The previous
    // noop-waker busy-poll panicked with "there is no reactor running" the
    // moment a live provider feature was enabled.
    match tokio::runtime::Handle::try_current() {
        // Already inside a runtime (e.g. tdw-worker's #[tokio::main] calling
        // the sync helpers): a nested runtime/block_on panics, and
        // block_in_place is multi-thread-only — so drive the future on the
        // existing runtime from a scoped thread, which is not an async
        // context.
        Ok(handle) => std::thread::scope(|scope| {
            scope
                .spawn(|| handle.block_on(future))
                .join()
                .expect("tdw-service-api block_on: scoped thread panicked")
        }),
        Err(_) => tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tdw-service-api block_on: build current-thread runtime")
            .block_on(future),
    }
}

fn poll_stream_next<T: tdw_core::DataModel>(
    stream: &mut ProgressStream<T>,
) -> Result<Option<ProgressOrResult<T>>> {
    let waker = Waker::from(Arc::new(NoopWake));
    let mut context = Context::from_waker(&waker);
    match stream.as_mut().poll_next(&mut context) {
        Poll::Ready(Some(item)) => item.map(Some),
        Poll::Ready(None) | Poll::Pending => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tdw_hooks::{HookExecutionPolicy, HookHandlerBackend};

    fn policy_config(
        roles: Vec<&str>,
        hooks: Vec<HookSpec>,
        mask_rules: Vec<MaskRule>,
    ) -> PolicyEnforcementConfig {
        policy_config_with_hook_policy(roles, hooks, mask_rules, HookExecutionPolicy::default())
    }

    fn policy_config_with_hook_policy(
        roles: Vec<&str>,
        hooks: Vec<HookSpec>,
        mask_rules: Vec<MaskRule>,
        hook_execution: HookExecutionPolicy,
    ) -> PolicyEnforcementConfig {
        PolicyEnforcementConfig {
            auth: IngressAuthContext {
                claims: JwtClaims {
                    sub: "alice".to_string(),
                    iss: "https://issuer".to_string(),
                    aud: "tdw".to_string(),
                    kid: "k1".to_string(),
                    roles: roles.into_iter().map(str::to_string).collect(),
                },
                jwks: vec![JwksKey {
                    kid: "k1".to_string(),
                    alg: "RS256".to_string(),
                }],
                issuer: "https://issuer".to_string(),
                audience: "tdw".to_string(),
            },
            hooks,
            hook_execution,
            mask_rules,
        }
    }

    #[derive(Default)]
    struct RecordingHookBackend {
        calls: Vec<String>,
    }

    impl HookHandlerBackend for RecordingHookBackend {
        fn run_command(
            &mut self,
            command: &str,
            args: &[String],
            payload: Value,
        ) -> std::result::Result<Value, tdw_hooks::HookError> {
            self.calls.push(format!("command:{command}"));
            Ok(json!({ "command": command, "args": args, "payload": payload }))
        }

        fn call_http(
            &mut self,
            url: &str,
            payload: Value,
        ) -> std::result::Result<Value, tdw_hooks::HookError> {
            self.calls.push(format!("http:{url}"));
            Ok(json!({ "url": url, "payload": payload }))
        }

        fn call_mcp(
            &mut self,
            server: &str,
            tool: &str,
            payload: Value,
        ) -> std::result::Result<Value, tdw_hooks::HookError> {
            self.calls.push(format!("mcp:{server}.{tool}"));
            Ok(json!({ "server": server, "tool": tool, "payload": payload }))
        }

        fn load_prompt(
            &mut self,
            prompt_path: &str,
            payload: Value,
        ) -> std::result::Result<Value, tdw_hooks::HookError> {
            self.calls.push(format!("prompt:{prompt_path}"));
            Ok(json!({ "prompt_path": prompt_path, "payload": payload }))
        }

        fn run_agent(
            &mut self,
            agent_id: &str,
            skill_id: &str,
            payload: Value,
        ) -> std::result::Result<Value, tdw_hooks::HookError> {
            self.calls.push(format!("agent:{agent_id}.{skill_id}"));
            Ok(json!({ "agent_id": agent_id, "skill_id": skill_id, "payload": payload }))
        }
    }

    fn uppercase_udf_request(allow_network: bool) -> UdfRequest {
        UdfRequest {
            name: "upper".to_string(),
            runtime: UdfRuntime::Wasm,
            source: "upper(input)".to_string(),
            input: "aapl".to_string(),
            allow_network,
            allow_filesystem: false,
            wasm_limits: None,
        }
    }

    #[test]
    fn lists_fetchers_and_streamer() {
        let providers =
            list_providers().unwrap_or_else(|error| panic!("providers should list: {error}"));

        assert!(providers.iter().any(|provider| {
            provider.provider == "fileset" && provider.kind == ProviderKind::Fetcher
        }));
        assert!(
            providers
                .iter()
                .any(|provider| provider.provider == "mock-ws"
                    && provider.kind == ProviderKind::Streamer)
        );
    }

    #[cfg(feature = "provider-yahoo-http")]
    #[test]
    fn yahoo_http_feature_selects_http_fetcher_for_execution_paths() {
        let selected = std::any::type_name::<SelectedYahooEquityHistoricalFetcher>();

        assert!(
            selected.ends_with("YahooHttpEquityHistoricalFetcher"),
            "selected yahoo fetcher was {selected}"
        );
    }

    /// The default (no-feature) build must register exactly the three offline
    /// providers (fileset, yahoo, mock-ws). Enabling provider features adds live
    /// HTTP fetchers on top, except `provider-yahoo-http`, which swaps Yahoo's
    /// offline fixture for the live implementation under the same key. This
    /// exact offline count is asserted only when no provider feature is active.
    #[cfg(not(any(
        feature = "provider-adanos",
        feature = "provider-akshare",
        feature = "provider-alpaca",
        feature = "provider-alpha-vantage",
        feature = "provider-benzinga",
        feature = "provider-bls",
        feature = "provider-cboe",
        feature = "provider-ccdata",
        feature = "provider-coingecko",
        feature = "provider-databento",
        feature = "provider-deribit",
        feature = "provider-ecb",
        feature = "provider-eia",
        feature = "provider-federal-reserve",
        feature = "provider-finra",
        feature = "provider-finnhub",
        feature = "provider-fmp",
        feature = "provider-fred",
        feature = "provider-government-us",
        feature = "provider-geckoterminal",
        feature = "provider-glassnode",
        feature = "provider-huggingface",
        feature = "provider-nasdaq",
        feature = "provider-oecd",
        feature = "provider-polygon",
        feature = "provider-sec",
        feature = "provider-seeking-alpha",
        feature = "provider-tiingo",
        feature = "provider-tmx",
        feature = "provider-tradier",
        feature = "provider-trading-economics",
        feature = "provider-velodata",
        feature = "provider-binance-http",
        feature = "provider-yahoo-http",
    )))]
    #[test]
    fn default_registry_is_offline_only() {
        let providers =
            list_providers().unwrap_or_else(|error| panic!("providers should list: {error}"));
        assert_eq!(
            providers.len(),
            3,
            "default build must register exactly the 3 offline providers"
        );
    }

    #[test]
    fn fetch_endpoint_uses_command_runner() {
        let object = fetch_equity_historical("fileset", "aapl")
            .unwrap_or_else(|error| panic!("fetch should succeed: {error}"));

        assert_eq!(object.provider, "fileset");
        assert_eq!(object.rows[0].symbol, "AAPL");
    }

    #[test]
    fn fetch_provider_json_dispatches_fileset_and_errors_on_unknown() {
        let value =
            fetch_provider_json("fileset", "equity_historical", json!({ "symbol": "AAPL" }))
                .unwrap_or_else(|error| panic!("fileset dispatch should succeed: {error}"));

        assert_eq!(value["provider"], "fileset");
        assert_eq!(value["endpoint"], "equity_historical");
        assert!(
            value["rows"]
                .as_array()
                .is_some_and(|rows| !rows.is_empty()),
            "expected non-empty rows, got {value}"
        );
        assert_eq!(value["rows"][0]["symbol"], "AAPL");

        let unknown =
            fetch_provider_json("nope", "missing", json!({})).expect_err("unknown pair must error");
        assert!(
            unknown.to_string().contains("no fetcher for nope/missing"),
            "unexpected error: {unknown}"
        );
    }

    #[test]
    fn provider_fetch_targets_includes_fileset_equity_historical() {
        let targets = provider_fetch_targets();
        assert!(
            targets.contains(&("fileset".to_string(), "equity_historical".to_string())),
            "fileset/equity_historical must be a dispatch target, got {targets:?}"
        );
    }

    #[test]
    fn provider_fetch_targets_never_drift_from_dispatch_arms() {
        // Drift guard: every advertised target must reach a dispatch arm in
        // fetch_provider_json. A `null` params value fails every fetcher's
        // transform_query BEFORE any I/O, so the probe is network-free in all
        // feature configurations — the one error that must never appear is
        // the unknown-pair Registry error.
        for (provider, endpoint) in provider_fetch_targets() {
            match fetch_provider_json(&provider, &endpoint, Value::Null) {
                Ok(_) => {}
                Err(error) => {
                    let message = error.to_string();
                    assert!(
                        !message.contains("no fetcher for"),
                        "{provider}/{endpoint} is advertised by provider_fetch_targets                          but has no dispatch arm: {message}"
                    );
                }
            }
        }
    }

    #[test]
    fn secure_endpoint_validates_auth_hooks_and_masks_response() {
        let config = policy_config_with_hook_policy(
            vec!["analyst"],
            vec![
                HookSpec::new("audit_request", 1, TransactionMode::InTransaction).with_handler(
                    HandlerKind::Mcp {
                        server: "local".to_string(),
                        tool: "audit".to_string(),
                    },
                ),
            ],
            vec![MaskRule {
                field: "provider".to_string(),
                mode: MaskMode::Redact,
            }],
            service_hook_policy(["hook.mcp.local.audit"], false),
        );
        let mut backend = RecordingHookBackend::default();

        let response =
            secure_endpoint_response_with_backend(&config, "fileset", "aapl", &mut backend)
                .unwrap_or_else(|error| panic!("secure endpoint should succeed: {error}"));

        assert_eq!(response["policy"]["endpoint"], "equity_historical");
        assert_eq!(response["policy"]["principal"], "alice");
        assert_eq!(response["policy"]["hooks"][0], "audit_request");
        assert_eq!(response["response"]["provider"], "***");
        assert_eq!(response["response"]["rows"][0]["symbol"], "AAPL");
        assert_eq!(backend.calls, vec!["mcp:local.audit"]);
    }

    #[test]
    fn secure_endpoint_is_deny_by_default_and_role_gated() {
        let guest = policy_config(Vec::new(), Vec::new(), Vec::new());

        let denied = secure_endpoint_response(&guest, "fileset", "aapl")
            .expect_err("missing analyst role must deny");
        assert!(denied.to_string().contains("authorization denied"));

        let unknown = secure_endpoint_by_name(
            &policy_config(vec!["analyst"], Vec::new(), Vec::new()),
            "unregistered",
            "fileset",
            "aapl",
        )
        .expect_err("unknown endpoint must deny");
        assert!(unknown.to_string().contains("denied by default"));
    }

    #[test]
    fn secure_endpoint_rejects_bad_ingress_claims_and_hook_veto() {
        let mut bad_claims = policy_config(vec!["analyst"], Vec::new(), Vec::new());
        bad_claims.auth.claims.aud = "other".to_string();
        let rejected = secure_endpoint_response(&bad_claims, "fileset", "aapl")
            .expect_err("bad audience must deny");
        assert!(rejected.to_string().contains("ingress jwt rejected"));

        let veto = policy_config_with_hook_policy(
            vec!["analyst"],
            vec![
                HookSpec::new("deny_request", 1, TransactionMode::InTransaction)
                    .with_handler(HandlerKind::Mcp {
                        server: "local".to_string(),
                        tool: "deny".to_string(),
                    })
                    .should_stop(),
            ],
            Vec::new(),
            service_hook_policy(["hook.mcp.local.deny"], true),
        );
        let mut backend = RecordingHookBackend::default();
        let stopped = secure_endpoint_response_with_backend(&veto, "fileset", "aapl", &mut backend)
            .expect_err("stopping hook must veto");
        assert!(stopped.to_string().contains("hook vetoed request"));
        assert_eq!(backend.calls, vec!["mcp:local.deny"]);
    }

    #[test]
    fn secure_udf_path_enforces_role_and_sandbox_capabilities() {
        let allowed = policy_config(vec!["udf_runner"], Vec::new(), Vec::new());
        let response = secure_udf_run(&allowed, uppercase_udf_request(false))
            .unwrap_or_else(|error| panic!("allowed udf should run: {error}"));
        assert_eq!(response["policy"]["endpoint"], "udf.run");
        assert_eq!(response["response"]["output"], "AAPL");

        let unauthorized = secure_udf_run(
            &policy_config(vec!["analyst"], Vec::new(), Vec::new()),
            uppercase_udf_request(false),
        )
        .expect_err("missing udf_runner role must deny");
        assert!(unauthorized.to_string().contains("authorization denied"));

        let denied = secure_udf_run(&allowed, uppercase_udf_request(true))
            .expect_err("network capability must deny");
        assert!(denied.to_string().contains("sandbox denied capability"));
    }

    #[test]
    fn secure_endpoint_hook_policy_denies_before_backend_execution() {
        let config = policy_config(
            vec!["analyst"],
            vec![
                HookSpec::new("audit_request", 1, TransactionMode::InTransaction).with_handler(
                    HandlerKind::Mcp {
                        server: "local".to_string(),
                        tool: "audit".to_string(),
                    },
                ),
            ],
            Vec::new(),
        );
        let mut backend = RecordingHookBackend::default();

        let denied =
            secure_endpoint_response_with_backend(&config, "fileset", "aapl", &mut backend)
                .expect_err("unallowed hook action must block before backend execution");

        // HK2/CFG2: the default posture is Ask, so an unmatched hook action
        // surfaces as requires-approval rather than a hard deny — but it must
        // still block the backend from executing without an explicit Allow.
        assert!(
            denied
                .to_string()
                .contains("hook permission requires approval")
        );
        assert!(backend.calls.is_empty());
    }

    #[test]
    fn secure_service_runtime_reuses_bound_hook_backend() {
        let config = policy_config_with_hook_policy(
            vec!["udf_runner"],
            vec![
                HookSpec::new("udf_audit", 1, TransactionMode::InTransaction).with_handler(
                    HandlerKind::Mcp {
                        server: "local".to_string(),
                        tool: "udf_audit".to_string(),
                    },
                ),
            ],
            Vec::new(),
            service_hook_policy(["hook.mcp.local.udf_audit"], false),
        );
        let mut runtime = SecureServiceRuntime::new(config, RecordingHookBackend::default());

        let response = runtime
            .udf_run(uppercase_udf_request(false))
            .unwrap_or_else(|error| panic!("runtime-bound udf should succeed: {error}"));

        assert_eq!(response["policy"]["endpoint"], "udf.run");
        assert_eq!(response["policy"]["hooks"][0], "udf_audit");
        assert_eq!(runtime.hook_backend().calls, vec!["mcp:local.udf_audit"]);
    }

    #[tokio::test]
    async fn research_note_indexes_across_retrieval_stores() {
        let evidence = index_research_note(ResearchNote {
            id: "note-1".to_string(),
            title: "Macro note".to_string(),
            body: "Fixture note for deterministic tests.".to_string(),
            tags: vec!["macro".to_string()],
        })
        .await
        .unwrap_or_else(|error| panic!("indexing should succeed: {error}"));

        assert_eq!(evidence.note_id, "note-1");
        assert_eq!(evidence.vector_hits.len(), 1);
        assert_eq!(evidence.lexical_hits.len(), 1);
        assert!(evidence.blob_bytes > 0);
    }

    #[test]
    fn mcp_progress_sample_emits_progress_and_done() {
        let events = mcp_progress_sample("aapl")
            .unwrap_or_else(|error| panic!("progress sample should succeed: {error}"));

        assert_eq!(events[0], "progress:fetch:0.0");
        assert_eq!(events[2], "done:fileset:2");
    }

    #[test]
    fn protocol_config_sample_wires_service_contracts() {
        let evidence = protocol_config_sample()
            .unwrap_or_else(|error| panic!("protocol/config sample should succeed: {error}"));

        assert_eq!(evidence["profile"], "service");
        assert_eq!(evidence["max_event_bytes"], 4096);
        assert_eq!(evidence["op_sequence"], 1);
        assert_eq!(evidence["event"]["type"], "started");
        assert!(
            evidence["protocol_schemas"]
                .as_array()
                .is_some_and(|schemas| schemas.len() >= 4)
        );
    }

    #[test]
    fn registry_mcp_bridge_collects_tools_and_prompts() {
        use tdw_agent::{
            Prompt, PromptArgument, RegistryEntity, Tool, ToolEffect, ToolImplementation,
        };

        let tool = Tool {
            meta: EntityMeta::new(
                "search",
                "search",
                "0.1.0",
                Origin {
                    tier: Tier::Domain,
                    source: Source::Internal,
                },
                Adaptivity::None,
                false,
            )
            .with_title("Search"),
            input_schema: json!({"type": "object"}),
            output_schema: None,
            effect: ToolEffect::ReadOnly,
            idempotent: true,
            open_world: false,
            implementation: ToolImplementation::Unbound,
        };
        let prompt = Prompt {
            meta: EntityMeta::new(
                "research.prompt",
                "research.prompt",
                "0.1.0",
                Origin {
                    tier: Tier::Domain,
                    source: Source::Internal,
                },
                Adaptivity::None,
                false,
            )
            .with_title("Research Prompt"),
            template: "Summarize {{ symbol }}".to_string(),
            arguments: vec![PromptArgument {
                name: "symbol".to_string(),
                description: Some("Ticker".to_string()),
                required: true,
                default: None,
            }],
        };

        let registry = Registry::from_resources([
            tool.to_resource()
                .unwrap_or_else(|error| panic!("tool resource: {error}")),
            prompt
                .to_resource()
                .unwrap_or_else(|error| panic!("prompt resource: {error}")),
        ])
        .unwrap_or_else(|error| panic!("registry should build: {error}"));

        let tools = registry_mcp_tools(&registry);
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].base.name, "search");

        let prompts = registry_mcp_prompts(&registry);
        assert_eq!(prompts.len(), 1);
        assert_eq!(prompts[0].base.name, "research.prompt");
    }

    #[test]
    fn agent_tool_sample_wires_schema_store_eval_and_workflow() {
        let evidence = agent_tool_sample()
            .unwrap_or_else(|error| panic!("agent tool sample should succeed: {error}"));

        assert_eq!(evidence["agent_id"], "market-researcher");
        assert_eq!(evidence["eval_status"], "success");
        assert_eq!(evidence["workflow_order"][0], "retrieve");
        assert!(
            evidence["tools"]
                .as_array()
                .is_some_and(|tools| tools.len() >= 5)
        );
    }

    #[test]
    fn extensibility_sample_wires_tools_sandbox_mcp_and_acp() {
        let evidence = extensibility_sample()
            .unwrap_or_else(|error| panic!("extensibility sample should succeed: {error}"));

        assert_eq!(evidence["tool"]["permission"], "Allow");
        assert_eq!(evidence["tool"]["output"]["symbol"], "AAPL");
        assert_eq!(evidence["sandbox_runtime"], "local-tdw-udf");
        assert_eq!(evidence["udf_output"], "AAPL");
        assert_eq!(evidence["mcp_tools"][5], "tdw.udf.run");
        assert_eq!(evidence["acp"]["supports_streaming"], true);
    }

    #[test]
    fn event_spine_sample_wires_actor_bus_hooks_outbox_cdc_and_replay() {
        let evidence = event_spine_sample("mcp")
            .unwrap_or_else(|error| panic!("event sample should succeed: {error}"));

        assert_eq!(evidence["entrypoint"], "mcp");
        assert_eq!(evidence["hook_order"][0], "audit");
        assert_eq!(evidence["hook_contexts"][0], "tdw://context/hook-policy");
        assert_eq!(evidence["hook_can_stop"], false);
        assert_eq!(evidence["outbox_pending"], 1);
        assert_eq!(evidence["replay_dry_run"], true);
    }

    #[test]
    fn parity_layer_sample_wires_layer_c_features() {
        let evidence = parity_layer_sample()
            .unwrap_or_else(|error| panic!("parity sample should succeed: {error}"));

        assert_eq!(evidence["snapshot_version"], 2);
        assert_eq!(evidence["time_travel_rows"], 1);
        assert_eq!(evidence["udf_output"], "AAPL");
        assert_eq!(evidence["table_manifest_ok"], true);
        assert_eq!(evidence["jwt_valid"], true);
        assert_eq!(evidence["authorized"], true);
        assert_eq!(evidence["masked_account"], "***3456");
    }

    #[test]
    fn kg_tag_sample_wires_kg_tags_rules_mcp_and_features() {
        let evidence =
            kg_tag_sample().unwrap_or_else(|error| panic!("kg sample should succeed: {error}"));

        assert_eq!(evidence["entity"], "instrument:AAPL");
        assert_eq!(evidence["neighbors"][0], "dataset:ohlcv");
        assert_eq!(evidence["manual_merge_audited"], true);
        assert_eq!(evidence["rule_assignments"], 1);
        assert_eq!(evidence["hybrid_search_filter"], "tag:asset:equity");
        assert_eq!(evidence["dbt_model"], "meta_tag_assignments");
        assert_eq!(evidence["feature_count"], 1);
    }

    #[test]
    fn llm_knowledge_sample_wires_model_and_retrieval_contracts() {
        let evidence = llm_knowledge_sample()
            .unwrap_or_else(|error| panic!("llm knowledge sample should succeed: {error}"));

        assert_eq!(evidence["anthropic_model"], "claude-fixture");
        assert_eq!(evidence["openai_base_url"], "http://localhost:11434");
        assert_eq!(evidence["knowledge_hits"][0]["id"], "doc-1");
        assert_eq!(evidence["active_tags"][0], "asset:equity");
        assert_eq!(evidence["syntax_symbols"][0]["kind"], "table");
    }

    #[test]
    fn client_event_sample_wires_exec_tui_and_replay() {
        let evidence = client_event_sample()
            .unwrap_or_else(|error| panic!("client event sample should succeed: {error}"));

        assert_eq!(evidence["events"][0]["type"], "started");
        assert_eq!(evidence["tui_lines"][0], "started");
        assert_eq!(evidence["replay"]["event_types"][1], "completed");
    }
}
