//! Async Op dispatcher (P1 of the integration cycle).
//!
//! Routes `tdw_protocol::OpEnvelope`s through the secure service path:
//! - Sync, CPU-only policy guard (`enforce_request_path_with_backend`) runs
//!   first for auth + hook + mask.
//! - The real work then `.await`s the underlying async APIs directly. The
//!   facade's legacy busy-loop `block_on` is *not* on the hot path — it is
//!   bypassed entirely.
//!
//! Returns `Vec<EventMsg>` per envelope: a `Started` followed by a terminal
//! `Completed` or `Failed`.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;

use async_trait::async_trait;
use serde_json::{Value, json};
use tdw_app_server::Dispatcher;
#[cfg_attr(
    not(any(
        feature = "provider-akshare",
        feature = "provider-alpaca",
        feature = "provider-alpha-vantage",
        feature = "provider-ccdata",
        feature = "provider-coingecko",
        feature = "provider-databento",
        feature = "provider-fmp",
        feature = "provider-polygon",
        feature = "provider-sec",
        feature = "provider-tiingo",
    )),
    allow(unused_imports)
)]
use tdw_core::Fetcher;
use tdw_core::{Error, Result};
use tdw_domain::QuoteSnapshot;
use tdw_hooks::SystemHookHandlerBackend;
use tdw_protocol::{EventMsg, Op, OpEnvelope, TimeRange};
use tdw_provider_fileset::FilesetEquityHistoricalFetcher;
use tdw_runtime::CommandRunner;
use tdw_sandbox::{LocalUdfSandbox, SandboxRuntime, UdfRequest};
use tdw_tools::{RegisteredTool, ToolDefinition, ToolRegistry, ToolRouter};

use crate::{
    AppState, PolicyEnforcementConfig, SelectedYahooEquityHistoricalFetcher, ServiceEndpoint,
    enforce_request_path_with_backend, mask_json_response,
};

#[async_trait]
impl Dispatcher for AppState {
    async fn dispatch(&self, env: OpEnvelope) -> Vec<EventMsg> {
        dispatch_op(self, env).await
    }
}

pub async fn dispatch_op(state: &AppState, env: OpEnvelope) -> Vec<EventMsg> {
    let started = EventMsg::Started {
        op_id: env.op_id.clone(),
    };
    let terminal = match run_dispatch(state, &env).await {
        Ok(value) => EventMsg::Completed {
            op_id: env.op_id.clone(),
            summary: None,
            result: Some(value),
        },
        Err(err) => EventMsg::Failed {
            op_id: env.op_id.clone(),
            error: err.to_string(),
        },
    };
    vec![started, terminal]
}

async fn run_dispatch(state: &AppState, env: &OpEnvelope) -> Result<Value> {
    let policy = state
        .policy
        .as_ref()
        .ok_or_else(|| Error::Provider("daemon policy not configured".to_string()))?;
    match &env.op {
        Op::RunQuery { sql, .. } => dispatch_run_query(state, policy, sql).await,
        Op::IngestBatch {
            provider,
            endpoint,
            symbols,
            range,
        } => {
            dispatch_ingest(
                state,
                policy,
                env,
                provider,
                endpoint,
                symbols,
                range.as_ref(),
            )
            .await
        }
        Op::ToolCall {
            tool_name,
            arguments,
            ..
        } => dispatch_tool(policy, tool_name, arguments),
        Op::GetQuoteSnapshot { provider, symbol } => {
            dispatch_get_quote_snapshot(state, policy, provider, symbol).await
        }
        Op::StreamStart {
            provider,
            symbol,
            table,
        } => dispatch_stream_start(state, policy, provider, symbol, table.clone()).await,
        Op::StreamStop { stream_id } => dispatch_stream_stop(state, policy, stream_id).await,
        Op::ApprovalResponse { .. } => Ok(json!({ "acknowledged": "approval_response" })),
        Op::AppendUserMessage { .. } => Ok(json!({ "acknowledged": "append_user_message" })),
        Op::CompactContext { .. } => Ok(json!({ "acknowledged": "compact_context" })),
        Op::Cancel { .. } => Ok(json!({ "acknowledged": "cancel" })),
        Op::Shutdown => Ok(json!({ "shutdown": "requested" })),
    }
}

/// Fetch a fresh last-price quote snapshot for `symbol` from `provider`.
///
/// This is a **no-cache read path**: the provider's HTTP endpoint is called on
/// every dispatch and the result is returned directly — nothing is written to
/// any storage layer. This design is intentional: price-alert engine callers
/// must always see the freshest available price, so bypassing any cache is a
/// correctness requirement, not an optimisation trade-off.
///
/// Currently only the `fmp` provider is supported. Additional providers can be
/// wired behind `#[cfg(feature = "provider-*")]` arms following the same
/// pattern.
///
/// # Errors
///
/// Returns [`Error::Provider`] if policy enforcement fails, if `provider` is
/// not a recognised quote-snapshot provider for this build, or if the HTTP
/// fetch fails (network, non-2xx, parse error).
async fn dispatch_get_quote_snapshot(
    _state: &AppState,
    policy: &PolicyEnforcementConfig,
    provider: &str,
    symbol: &str,
) -> Result<Value> {
    let mut backend = SystemHookHandlerBackend::default();
    let evidence =
        enforce_request_path_with_backend(policy, ServiceEndpoint::IngestBatch, &mut backend)?;

    let snapshot = fetch_quote_snapshot(provider, symbol).await?;

    Ok(mask_json_response(
        json!({
            "evidence": evidence,
            "provider": provider,
            "symbol": symbol,
            "snapshot": snapshot,
        }),
        &policy.mask_rules,
    ))
}

/// Resolve `provider` to its quote-snapshot fetcher and execute a fresh read.
///
/// Only feature-enabled providers are compiled in. Offline (default) builds
/// return an `Err` for any provider, keeping the workspace test set network-free.
#[allow(unused_variables)] // `symbol` is used only inside the cfg-gated provider block
async fn fetch_quote_snapshot(provider: &str, symbol: &str) -> Result<QuoteSnapshot> {
    #[cfg(feature = "provider-fmp")]
    if provider == "fmp" {
        use crate::FmpHttpQuoteSnapshotFetcher;
        use tdw_runtime::CommandRunner;
        let runner = CommandRunner::new(crate::default_registry()?);
        let params = json!({ "symbol": symbol });
        let object = runner
            .run(&FmpHttpQuoteSnapshotFetcher::default(), params)
            .await?;
        return object.rows.into_iter().next().ok_or_else(|| {
            Error::Provider(format!("fmp quote snapshot returned no rows for {symbol}"))
        });
    }
    Err(Error::Provider(format!(
        "unsupported quote-snapshot provider: {provider}; available: {}",
        available_quote_snapshot_providers()
    )))
}

/// Comma-separated list of quote-snapshot providers available in this build.
fn available_quote_snapshot_providers() -> &'static str {
    #[cfg(feature = "provider-fmp")]
    {
        "fmp"
    }
    #[cfg(not(feature = "provider-fmp"))]
    {
        "(none — enable a provider-* feature)"
    }
}

/// Start a live streaming-ingest task and report its `stream_id`.
///
/// Stream ingest *is* ingest, so it reuses [`ServiceEndpoint::IngestBatch`] for
/// policy enforcement rather than introducing a new endpoint variant. Only the
/// `binance` provider is supported.
///
/// # Errors
///
/// Returns [`Error::Provider`] if policy enforcement fails, if `provider` is
/// not `binance`, or if the stream cannot be started (e.g. invalid symbol or a
/// stream with the same id is already running).
async fn dispatch_stream_start(
    state: &AppState,
    policy: &PolicyEnforcementConfig,
    provider: &str,
    symbol: &str,
    table: Option<String>,
) -> Result<Value> {
    let mut backend = SystemHookHandlerBackend::default();
    let evidence =
        enforce_request_path_with_backend(policy, ServiceEndpoint::IngestBatch, &mut backend)?;

    if provider != "binance" {
        return Err(Error::Provider(format!(
            "unsupported stream provider: {provider}"
        )));
    }

    let stream_id = state.start_binance_stream(symbol, table)?;

    Ok(mask_json_response(
        json!({
            "evidence": evidence,
            "stream_id": stream_id,
            "provider": provider,
            "status": "started",
        }),
        &policy.mask_rules,
    ))
}

/// Stop a running streaming-ingest task by `stream_id`.
///
/// Reuses [`ServiceEndpoint::IngestBatch`] for policy enforcement (stream ingest
/// is ingest). Reports whether a stream with that id was present.
///
/// # Errors
///
/// Returns [`Error::Provider`] if policy enforcement fails or if the internal
/// streams registry lock is poisoned.
async fn dispatch_stream_stop(
    state: &AppState,
    policy: &PolicyEnforcementConfig,
    stream_id: &str,
) -> Result<Value> {
    let mut backend = SystemHookHandlerBackend::default();
    let evidence =
        enforce_request_path_with_backend(policy, ServiceEndpoint::IngestBatch, &mut backend)?;

    let was_present = state.stop_stream(stream_id)?;

    Ok(mask_json_response(
        json!({
            "evidence": evidence,
            "stream_id": stream_id,
            "stopped": was_present,
        }),
        &policy.mask_rules,
    ))
}

async fn dispatch_run_query(
    state: &AppState,
    policy: &PolicyEnforcementConfig,
    sql: &str,
) -> Result<Value> {
    let mut backend = SystemHookHandlerBackend::default();
    let evidence =
        enforce_request_path_with_backend(policy, ServiceEndpoint::RunQuery, &mut backend)?;
    let rows = state.relational.fetch_json(sql, Value::Null).await?;
    Ok(mask_json_response(
        json!({
            "evidence": evidence,
            "rows": rows,
        }),
        &policy.mask_rules,
    ))
}

async fn dispatch_ingest(
    state: &AppState,
    policy: &PolicyEnforcementConfig,
    env: &OpEnvelope,
    provider: &str,
    endpoint: &str,
    symbols: &[String],
    range: Option<&TimeRange>,
) -> Result<Value> {
    let mut backend = SystemHookHandlerBackend::default();
    let evidence =
        enforce_request_path_with_backend(policy, ServiceEndpoint::IngestBatch, &mut backend)?;

    if symbols.is_empty() {
        return Err(Error::Provider(
            "ingest requires at least one symbol".to_string(),
        ));
    }

    // Registry-driven dispatch: resolve (provider, endpoint) against the build's
    // ingest dispatch table — every feature-enabled fetcher with a bronze landing
    // table is dispatchable. An unknown pair yields a structured error listing the
    // providers/endpoints this build can actually ingest, rather than a flat
    // "unsupported" string.
    let table = ingest_dispatch_table();
    let Some(binding) = table.get(&(provider, endpoint)) else {
        return Err(Error::Provider(format!(
            "unsupported ingest provider/endpoint: {provider}/{endpoint}; available: {}",
            available_ingest_pairs(&table)
        )));
    };
    let bronze_table = binding.table;

    let runner = CommandRunner::new((*state.registry).clone());
    let mut per_symbol = Vec::with_capacity(symbols.len());
    let mut total_rows = 0usize;

    for symbol in symbols {
        let mut params = json!({ "symbol": symbol });
        if let Some(range) = range {
            params["range"] = json!({ "start": range.start, "end": range.end });
        }
        // Per-(op, symbol) dedup token: stable across retries of the same op
        // (same session_id + sequence) yet distinct per symbol, so a multi-symbol
        // op does not dedup later symbols' blocks against the first.
        let token = tdw_storage_clickhouse::ingest_dedup_token(
            env.session_id.as_str(),
            env.sequence,
            &format!("{bronze_table}:{symbol}"),
        );
        let rows = (binding.run)(state, &runner, params, bronze_table, token.clone()).await?;
        total_rows += rows;
        per_symbol.push(json!({ "symbol": symbol, "rows": rows, "dedup_token": token }));
    }

    Ok(mask_json_response(
        json!({
            "evidence": evidence,
            "provider": provider,
            "endpoint": endpoint,
            "table": bronze_table,
            "rows": total_rows,
            "symbols": per_symbol,
        }),
        &policy.mask_rules,
    ))
}

/// One entry in the registry-driven ingest dispatch table.
///
/// `table` is the bronze landing table for the fetcher's data model; `run` is a
/// type-erased closure that fetches one symbol through the concrete fetcher and
/// persists it under the supplied dedup token, returning the row count. Erasing
/// the fetcher's `(Query, DataModel)` types behind a closure lets a single,
/// data-driven loop dispatch over heterogeneous providers without a per-provider
/// `match` arm on the hot path.
struct IngestBinding {
    table: &'static str,
    run: IngestRunner,
}

type IngestRunner = Box<
    dyn for<'a> Fn(
            &'a AppState,
            &'a CommandRunner,
            Value,
            &'static str,
            String,
        ) -> Pin<Box<dyn Future<Output = Result<usize>> + Send + 'a>>
        + Send
        + Sync,
>;

/// Build an [`IngestBinding`] for a concrete fetcher writing into `table`.
///
/// The fetcher is constructed per call via `Default` (fetchers are unit/cheap
/// structs); the binding fetches one symbol's batch and persists it.
fn binding<F, Q, D>(table: &'static str) -> IngestBinding
where
    F: tdw_core::Fetcher<Q, D> + Default,
    Q: tdw_core::QueryParams,
    D: tdw_core::DataModel,
{
    IngestBinding {
        table,
        run: Box::new(
            move |state: &AppState,
                  runner: &CommandRunner,
                  params: Value,
                  table: &'static str,
                  token: String| {
                Box::pin(async move {
                    let object = runner.run(&F::default(), params).await?;
                    persist_batch(state, table, &token, &object).await
                })
            },
        ),
    }
}

/// The registry-driven ingest dispatch table for this build.
///
/// Each `(provider, endpoint)` key mirrors a feature-enabled `Fetcher`
/// registered in [`crate::default_registry`]; the value binds it to its bronze
/// landing table. Only fetchers with a canonical bronze table are listed —
/// `EquityHistoricalData` → `raw.equity_historical`, `MarketDataBar` →
/// `raw.market_data_bar` — so each landing write stays JSONEachRow-coherent with
/// its destination schema. The offline-default build registers exactly the two
/// fixture equity fetchers, keeping ingest network-free without any features.
fn ingest_dispatch_table() -> BTreeMap<(&'static str, &'static str), IngestBinding> {
    let mut table: BTreeMap<(&'static str, &'static str), IngestBinding> = BTreeMap::new();
    // Offline fixture fetchers — always available.
    table.insert(
        ("fileset", "equity_historical"),
        binding::<FilesetEquityHistoricalFetcher, _, _>("raw.equity_historical"),
    );
    table.insert(
        ("yahoo", "equity_historical"),
        binding::<SelectedYahooEquityHistoricalFetcher, _, _>("raw.equity_historical"),
    );
    // Feature-enabled `MarketDataBar` (canonical OHLC bar) fetchers land in the
    // shared bronze bar table. Each arm mirrors a `provider-*` feature wired into
    // `default_registry`.
    #[cfg(feature = "provider-akshare")]
    table.insert(
        ("akshare", crate::AkShareHttpFetcher::ENDPOINT),
        binding::<crate::AkShareHttpFetcher, _, _>("raw.market_data_bar"),
    );
    #[cfg(feature = "provider-alpaca")]
    table.insert(
        ("alpaca", crate::AlpacaHttpStockBarsFetcher::ENDPOINT),
        binding::<crate::AlpacaHttpStockBarsFetcher, _, _>("raw.market_data_bar"),
    );
    #[cfg(feature = "provider-alpha-vantage")]
    table.insert(
        ("alpha_vantage", crate::AlphaVantageHttpFetcher::ENDPOINT),
        binding::<crate::AlphaVantageHttpFetcher, _, _>("raw.market_data_bar"),
    );
    #[cfg(feature = "provider-ccdata")]
    table.insert(
        ("ccdata", crate::CCDataHttpFetcher::ENDPOINT),
        binding::<crate::CCDataHttpFetcher, _, _>("raw.market_data_bar"),
    );
    #[cfg(feature = "provider-coingecko")]
    table.insert(
        ("coingecko", crate::CoinGeckoHttpOhlcFetcher::ENDPOINT),
        binding::<crate::CoinGeckoHttpOhlcFetcher, _, _>("raw.market_data_bar"),
    );
    #[cfg(feature = "provider-databento")]
    table.insert(
        ("databento", crate::DatabentoHttpTimeseriesFetcher::ENDPOINT),
        binding::<crate::DatabentoHttpTimeseriesFetcher, _, _>("raw.market_data_bar"),
    );
    #[cfg(feature = "provider-fmp")]
    table.insert(
        ("fmp", crate::FmpHttpHistoricalFetcher::ENDPOINT),
        binding::<crate::FmpHttpHistoricalFetcher, _, _>("raw.market_data_bar"),
    );
    #[cfg(feature = "provider-polygon")]
    table.insert(
        ("polygon", crate::PolygonHttpAggregatesFetcher::ENDPOINT),
        binding::<crate::PolygonHttpAggregatesFetcher, _, _>("raw.market_data_bar"),
    );
    #[cfg(feature = "provider-sec")]
    table.insert(
        ("sec", crate::SecXbrlHttpFetcher::ENDPOINT),
        binding::<crate::SecXbrlHttpFetcher, _, _>("raw.market_data_bar"),
    );
    #[cfg(feature = "provider-tiingo")]
    table.insert(
        ("tiingo", crate::TiingoHttpHistoricalFetcher::ENDPOINT),
        binding::<crate::TiingoHttpHistoricalFetcher, _, _>("raw.market_data_bar"),
    );
    table
}

/// Comma-separated, sorted `provider/endpoint` list for the unsupported-pair
/// error. `BTreeMap` iteration is already sorted, so the output is stable.
fn available_ingest_pairs(table: &BTreeMap<(&'static str, &'static str), IngestBinding>) -> String {
    table
        .keys()
        .map(|(provider, endpoint)| format!("{provider}/{endpoint}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Persist a fetched batch as an idempotent `INSERT … FORMAT JSONEachRow` and
/// return the row count. The caller supplies the deduplication token (so a
/// client retry of the same op is dropped by `ClickHouse` rather than
/// double-written, and double-counted through dependent materialized views).
/// Routes through `state.olap` so the offline recording engine captures the
/// statement in unit tests and the real `ClickHouseHttpEngine` issues it over
/// HTTP in integration.
async fn persist_batch<T: tdw_core::DataModel>(
    state: &AppState,
    table: &str,
    token: &str,
    object: &tdw_core::OBBject<T>,
) -> Result<usize> {
    let insert = tdw_storage_clickhouse::build_insert_jsoneachrow(table, object, token)?;
    state.olap.execute(&insert).await?;
    Ok(object.rows.len())
}

fn dispatch_tool(
    policy: &PolicyEnforcementConfig,
    tool_name: &str,
    arguments: &Value,
) -> Result<Value> {
    let mut backend = SystemHookHandlerBackend::default();
    let evidence =
        enforce_request_path_with_backend(policy, ServiceEndpoint::ToolCall, &mut backend)?;

    // Route through the tool registry: an unknown tool yields a structured error
    // listing the tools this build exposes, rather than a flat "unsupported"
    // string. Policy enforcement above runs *before* this resolution, so an
    // unauthorized caller never reaches a tool — known or unknown.
    let registry = service_tool_registry();
    let router = ToolRouter::new(registry.clone());
    if router.route(tool_name).is_err() {
        return Err(Error::Provider(format!(
            "unsupported tool: {tool_name}; available: {}",
            available_tool_names(&registry)
        )));
    }

    match tool_name {
        "udf.run" => {
            let request: UdfRequest = serde_json::from_value(arguments.clone())
                .map_err(|err| Error::Provider(format!("invalid udf.run arguments: {err}")))?;
            let sandbox = LocalUdfSandbox;
            let response = sandbox
                .run(request)
                .map_err(|err| Error::Provider(format!("sandbox denied request: {err}")))?;
            Ok(mask_json_response(
                json!({
                    "evidence": evidence,
                    "runtime": response.runtime,
                    "output": response.output,
                }),
                &policy.mask_rules,
            ))
        }
        // The registry above is the single source of truth for *which* tools
        // exist; a registered name without a dispatch arm here is a build bug.
        other => Err(Error::Provider(format!(
            "tool registered but not dispatchable: {other}"
        ))),
    }
}

/// The service tool registry for this build.
///
/// Built per dispatch (cheap: a `BTreeMap` of small definitions). The handler
/// closure is a placeholder — `udf.run` is executed via the sandbox in
/// [`dispatch_tool`] because it needs the WASM runtime and structured error
/// mapping — but registering the tool here makes the registry the authoritative
/// listing for routing and the unknown-tool error.
fn service_tool_registry() -> ToolRegistry {
    let mut registry = ToolRegistry::default();
    registry
        .register(udf_run_tool())
        .expect("udf.run tool definition is valid and registered exactly once");
    registry
}

/// Definition for the `udf.run` tool. The handler is unused on the hot path
/// (see [`dispatch_tool`]); it exists only to satisfy [`RegisteredTool`].
fn udf_run_tool() -> RegisteredTool {
    RegisteredTool::new(
        ToolDefinition {
            name: "udf.run".to_string(),
            description: "Execute a sandboxed user-defined function.".to_string(),
            input_schema: json!({ "type": "object" }),
            output_schema: json!({ "type": "object" }),
            permission_pattern: "udf.run".to_string(),
        },
        udf_run_placeholder_handler,
    )
}

/// Placeholder handler for the `udf.run` registry entry. Never invoked:
/// [`dispatch_tool`] executes `udf.run` via the sandbox (which needs the WASM
/// runtime and structured error mapping). The registry only needs a handler to
/// construct a [`RegisteredTool`]; the echo behaviour here is inert.
fn udf_run_placeholder_handler(input: Value) -> tdw_tools::Result<Value> {
    Ok(input)
}

/// Comma-separated, sorted tool names for the unknown-tool error.
fn available_tool_names(registry: &ToolRegistry) -> String {
    registry
        .definitions()
        .into_iter()
        .map(|definition| definition.name)
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tdw_auth_oidc::{JwksKey, JwtClaims};
    use tdw_protocol::{ActorKind, ActorRef, SessionId};

    use crate::IngressAuthContext;

    fn analyst_policy() -> PolicyEnforcementConfig {
        PolicyEnforcementConfig {
            auth: IngressAuthContext {
                claims: JwtClaims {
                    sub: "alice".to_string(),
                    iss: "https://issuer".to_string(),
                    aud: "tdw".to_string(),
                    kid: "k1".to_string(),
                    roles: vec!["analyst".to_string()],
                },
                jwks: vec![JwksKey {
                    kid: "k1".to_string(),
                    alg: "RS256".to_string(),
                }],
                issuer: "https://issuer".to_string(),
                audience: "tdw".to_string(),
            },
            hooks: Vec::new(),
            hook_execution: tdw_hooks::HookExecutionPolicy::default(),
            mask_rules: Vec::new(),
        }
    }

    /// Policy whose principal also holds the `udf_runner` role required by the
    /// `tdw.udf.run` (ToolCall) endpoint — analyst alone is denied there.
    fn udf_runner_policy() -> PolicyEnforcementConfig {
        let mut policy = analyst_policy();
        policy.auth.claims.roles = vec!["analyst".to_string(), "udf_runner".to_string()];
        policy
    }

    fn make_envelope(op: Op) -> OpEnvelope {
        OpEnvelope::new(
            SessionId::new("session-test").expect("session id"),
            1,
            ActorRef {
                actor_id: "user:test".to_string(),
                kind: ActorKind::User,
                tenant_id: Some("default".to_string()),
            },
            op,
        )
    }

    #[tokio::test]
    async fn run_query_dispatches_through_relational_engine() {
        let state = AppState::in_memory_for_tests()
            .await
            .with_policy(analyst_policy());
        let env = make_envelope(Op::RunQuery {
            sql: "select 1".to_string(),
            plan_id: None,
            cost_hint: None,
        });
        let op_id = env.op_id.clone();
        let events = dispatch_op(&state, env).await;

        assert_eq!(events.len(), 2);
        match &events[0] {
            EventMsg::Started { op_id: started_id } => assert_eq!(started_id, &op_id),
            other => panic!("expected Started, got {other:?}"),
        }
        match &events[1] {
            EventMsg::Completed {
                op_id: completed_id,
                result: Some(value),
                ..
            } => {
                assert_eq!(completed_id, &op_id);
                assert_eq!(value["rows"][0]["engine"], "postgres-recording");
                assert_eq!(value["rows"][0]["sql"], "select 1");
                assert_eq!(value["evidence"]["endpoint"], "tdw.query.run");
                assert_eq!(value["evidence"]["principal"], "alice");
            }
            other => panic!("expected Completed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn dispatch_without_policy_fails_deny_by_default() {
        let mut state = AppState::in_memory_for_tests().await;
        state.policy = None;
        let env = make_envelope(Op::RunQuery {
            sql: "select 1".to_string(),
            plan_id: None,
            cost_hint: None,
        });
        let op_id = env.op_id.clone();
        let events = dispatch_op(&state, env).await;

        assert_eq!(events.len(), 2);
        match &events[1] {
            EventMsg::Failed {
                op_id: failed_id,
                error,
            } => {
                assert_eq!(failed_id, &op_id);
                assert!(error.contains("policy not configured"), "got: {error}");
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn run_query_uses_policy_built_from_config() {
        let state = AppState::in_memory_for_tests().await;
        let env = make_envelope(Op::RunQuery {
            sql: "select 1".to_string(),
            plan_id: None,
            cost_hint: None,
        });
        let events = dispatch_op(&state, env).await;

        match &events[1] {
            EventMsg::Completed {
                result: Some(value),
                ..
            } => {
                assert_eq!(value["evidence"]["principal"], "local:default");
                assert_eq!(value["evidence"]["endpoint"], "tdw.query.run");
            }
            other => panic!("expected Completed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn run_query_with_wrong_role_is_denied_by_authorize() {
        let mut policy = analyst_policy();
        policy.auth.claims.roles = vec!["guest".to_string()];
        let state = AppState::in_memory_for_tests().await.with_policy(policy);
        let env = make_envelope(Op::RunQuery {
            sql: "select 1".to_string(),
            plan_id: None,
            cost_hint: None,
        });
        let events = dispatch_op(&state, env).await;

        match &events[1] {
            EventMsg::Failed { error, .. } => {
                assert!(
                    error.contains("authorization denied"),
                    "expected authorization denial, got: {error}"
                );
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn shutdown_op_returns_completed_with_shutdown_marker() {
        let state = AppState::in_memory_for_tests()
            .await
            .with_policy(analyst_policy());
        let env = make_envelope(Op::Shutdown);
        let events = dispatch_op(&state, env).await;

        match &events[1] {
            EventMsg::Completed {
                result: Some(value),
                ..
            } => {
                assert_eq!(value["shutdown"], "requested");
            }
            other => panic!("expected Completed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn ingest_batch_persists_and_reports_dedup_token() {
        let state = AppState::in_memory_for_tests()
            .await
            .with_policy(analyst_policy());
        let env = make_envelope(Op::IngestBatch {
            provider: "yahoo".to_string(),
            endpoint: "equity_historical".to_string(),
            symbols: vec!["AAPL".to_string(), "MSFT".to_string()],
            range: None,
        });
        let events = dispatch_op(&state, env).await;

        assert_eq!(events.len(), 2);
        match &events[1] {
            EventMsg::Completed {
                result: Some(value),
                ..
            } => {
                assert_eq!(value["provider"], "yahoo");
                assert_eq!(value["endpoint"], "equity_historical");
                assert_eq!(value["table"], "raw.equity_historical");
                assert!(
                    value["rows"].as_u64().expect("rows count") >= 2,
                    "expected at least one persisted row per symbol, got {value}"
                );
                // Per-symbol results, each with a token keyed by
                // (session_id, sequence, table:symbol) — stable on retry, distinct
                // per symbol so the second symbol is not deduped against the first.
                let per = value["symbols"].as_array().expect("symbols array");
                assert_eq!(per.len(), 2);
                assert_eq!(per[0]["symbol"], "AAPL");
                assert_eq!(
                    per[0]["dedup_token"],
                    "session-test:1:raw.equity_historical:AAPL"
                );
                assert_eq!(per[1]["symbol"], "MSFT");
                assert_eq!(
                    per[1]["dedup_token"],
                    "session-test:1:raw.equity_historical:MSFT"
                );
                assert_ne!(per[0]["dedup_token"], per[1]["dedup_token"]);
            }
            other => panic!("expected Completed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn ingest_batch_unsupported_provider_fails() {
        let state = AppState::in_memory_for_tests()
            .await
            .with_policy(analyst_policy());
        let env = make_envelope(Op::IngestBatch {
            provider: "nope".to_string(),
            endpoint: "equity_historical".to_string(),
            symbols: vec!["AAPL".to_string()],
            range: None,
        });
        let events = dispatch_op(&state, env).await;

        match &events[1] {
            EventMsg::Failed { error, .. } => {
                assert!(
                    error.contains("unsupported ingest provider/endpoint: nope/equity_historical"),
                    "got: {error}"
                );
                // Registry-driven: the structured error advertises the pairs this
                // (offline-default) build can actually ingest.
                assert!(
                    error.contains("available:"),
                    "error should list available providers, got: {error}"
                );
                assert!(
                    error.contains("fileset/equity_historical")
                        && error.contains("yahoo/equity_historical"),
                    "error should list both offline fixture providers, got: {error}"
                );
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn ingest_dispatch_table_lists_offline_fixture_providers() {
        // The two fixture equity providers are always dispatchable (regardless of
        // enabled provider features), both routed through the generic
        // (type-erased) ingest binding rather than a hardcoded provider match, and
        // both land in the equity bronze table.
        let table = ingest_dispatch_table();
        assert_eq!(
            table[&("fileset", "equity_historical")].table,
            "raw.equity_historical"
        );
        assert_eq!(
            table[&("yahoo", "equity_historical")].table,
            "raw.equity_historical"
        );
        let available = available_ingest_pairs(&table);
        assert!(
            available.contains("fileset/equity_historical")
                && available.contains("yahoo/equity_historical"),
            "available pairs should list both fixture providers, got: {available}"
        );
    }

    /// The offline-default build (no `provider-*` features) dispatches over
    /// *exactly* the two fixture equity providers, keeping ingest network-free.
    #[cfg(not(any(
        feature = "provider-akshare",
        feature = "provider-alpaca",
        feature = "provider-alpha-vantage",
        feature = "provider-ccdata",
        feature = "provider-coingecko",
        feature = "provider-databento",
        feature = "provider-fmp",
        feature = "provider-polygon",
        feature = "provider-sec",
        feature = "provider-tiingo",
    )))]
    #[tokio::test]
    async fn ingest_dispatch_table_offline_default_is_exactly_two_fixtures() {
        let table = ingest_dispatch_table();
        assert_eq!(
            available_ingest_pairs(&table),
            "fileset/equity_historical, yahoo/equity_historical"
        );
    }

    #[tokio::test]
    async fn ingest_batch_dispatches_fileset_fixture_provider() {
        // Second fixture provider (the first, `yahoo`, is covered by
        // `ingest_batch_persists_and_reports_dedup_token`): proves the
        // registry-driven path dispatches more than one provider.
        let state = AppState::in_memory_for_tests()
            .await
            .with_policy(analyst_policy());
        let env = make_envelope(Op::IngestBatch {
            provider: "fileset".to_string(),
            endpoint: "equity_historical".to_string(),
            symbols: vec!["AAPL".to_string()],
            range: None,
        });
        let events = dispatch_op(&state, env).await;

        match &events[1] {
            EventMsg::Completed {
                result: Some(value),
                ..
            } => {
                assert_eq!(value["provider"], "fileset");
                assert_eq!(value["table"], "raw.equity_historical");
                assert!(value["rows"].as_u64().expect("rows count") >= 1);
            }
            other => panic!("expected Completed, got {other:?}"),
        }
    }

    #[test]
    fn tool_registry_routes_known_and_rejects_unknown() {
        let registry = service_tool_registry();
        let router = ToolRouter::new(registry.clone());
        // Known tool routes.
        assert!(router.route("udf.run").is_ok());
        // Unknown tool is rejected by the router.
        assert!(router.route("does.not.exist").is_err());
        // The available-names helper lists the build's tools for the error.
        assert_eq!(available_tool_names(&registry), "udf.run");
    }

    #[tokio::test]
    async fn tool_call_unknown_tool_lists_available_names() {
        let state = AppState::in_memory_for_tests()
            .await
            .with_policy(udf_runner_policy());
        let env = make_envelope(Op::ToolCall {
            call_id: tdw_protocol::ToolCallId::new("tc-1").expect("tool call id"),
            tool_name: "bogus.tool".to_string(),
            arguments: json!({}),
            permission_id: None,
        });
        let events = dispatch_op(&state, env).await;

        match &events[1] {
            EventMsg::Failed { error, .. } => {
                assert!(
                    error.contains("unsupported tool: bogus.tool"),
                    "got: {error}"
                );
                assert!(error.contains("available: udf.run"), "got: {error}");
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn tool_call_udf_run_executes_through_registry() {
        let state = AppState::in_memory_for_tests()
            .await
            .with_policy(udf_runner_policy());
        let env = make_envelope(Op::ToolCall {
            call_id: tdw_protocol::ToolCallId::new("tc-2").expect("tool call id"),
            tool_name: "udf.run".to_string(),
            arguments: json!({
                "name": "upper",
                "runtime": "Wasm",
                "source": "upper(input)",
                "input": "aapl",
                "allow_network": false,
                "allow_filesystem": false,
            }),
            permission_id: None,
        });
        let events = dispatch_op(&state, env).await;

        match &events[1] {
            EventMsg::Completed {
                result: Some(value),
                ..
            } => {
                assert!(value.get("runtime").is_some(), "got: {value}");
                assert!(value.get("output").is_some(), "got: {value}");
            }
            other => panic!("expected Completed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn stream_start_dispatches_and_reports_stream_id() {
        let state = AppState::in_memory_for_tests()
            .await
            .with_policy(analyst_policy());
        let env = make_envelope(Op::StreamStart {
            provider: "binance".to_string(),
            symbol: "BTCUSDT".to_string(),
            table: None,
        });
        let events = dispatch_op(&state, env).await;

        assert_eq!(events.len(), 2);
        match &events[1] {
            EventMsg::Completed {
                result: Some(value),
                ..
            } => {
                assert_eq!(value["provider"], "binance");
                assert_eq!(value["status"], "started");
                assert_eq!(value["stream_id"], "binance:trades:BTCUSDT");
            }
            other => panic!("expected Completed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn stream_start_unsupported_provider_fails() {
        let state = AppState::in_memory_for_tests()
            .await
            .with_policy(analyst_policy());
        let env = make_envelope(Op::StreamStart {
            provider: "kraken".to_string(),
            symbol: "BTCUSDT".to_string(),
            table: None,
        });
        let events = dispatch_op(&state, env).await;

        match &events[1] {
            EventMsg::Failed { error, .. } => {
                assert!(
                    error.contains("unsupported stream provider"),
                    "got: {error}"
                );
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn stream_stop_reports_present_after_start() {
        let state = AppState::in_memory_for_tests()
            .await
            .with_policy(analyst_policy());

        // Start a stream so the registry has an entry to stop. We assert on the
        // dispatch result rather than row counts: in offline mode the streamer
        // emits one tick then ends, so the spawned task may already be finished
        // by the time we stop it — `stop_stream` still reports it was present.
        let stream_id = state
            .start_binance_stream("BTCUSDT", None)
            .unwrap_or_else(|error| panic!("start should succeed: {error}"));
        assert_eq!(stream_id, "binance:trades:BTCUSDT");

        let env = make_envelope(Op::StreamStop {
            stream_id: stream_id.clone(),
        });
        let events = dispatch_op(&state, env).await;

        match &events[1] {
            EventMsg::Completed {
                result: Some(value),
                ..
            } => {
                assert_eq!(value["stream_id"], stream_id);
                assert_eq!(value["stopped"], true);
            }
            other => panic!("expected Completed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn get_quote_snapshot_unsupported_provider_fails_with_descriptive_error() {
        // Without any `provider-*` feature enabled the quote-snapshot dispatch
        // table is empty, so any provider name must produce a structured error
        // listing the (empty) available set rather than a panic.
        let state = AppState::in_memory_for_tests()
            .await
            .with_policy(analyst_policy());
        let env = make_envelope(Op::GetQuoteSnapshot {
            provider: "nope".to_string(),
            symbol: "AAPL".to_string(),
        });
        let events = dispatch_op(&state, env).await;

        assert_eq!(events.len(), 2);
        match &events[1] {
            EventMsg::Failed { error, .. } => {
                assert!(
                    error.contains("unsupported quote-snapshot provider: nope"),
                    "got: {error}"
                );
                assert!(error.contains("available:"), "got: {error}");
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn stream_stop_unknown_id_reports_not_present() {
        let state = AppState::in_memory_for_tests()
            .await
            .with_policy(analyst_policy());
        let env = make_envelope(Op::StreamStop {
            stream_id: "binance:trades:NOPE".to_string(),
        });
        let events = dispatch_op(&state, env).await;

        match &events[1] {
            EventMsg::Completed {
                result: Some(value),
                ..
            } => {
                assert_eq!(value["stopped"], false);
            }
            other => panic!("expected Completed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn ingest_batch_without_symbols_fails() {
        let state = AppState::in_memory_for_tests()
            .await
            .with_policy(analyst_policy());
        let env = make_envelope(Op::IngestBatch {
            provider: "yahoo".to_string(),
            endpoint: "equity_historical".to_string(),
            symbols: Vec::new(),
            range: None,
        });
        let events = dispatch_op(&state, env).await;

        match &events[1] {
            EventMsg::Failed { error, .. } => {
                assert!(
                    error.contains("ingest requires at least one symbol"),
                    "got: {error}"
                );
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    /// Per-request `WasmLimits` carried in the `udf.run` op payload are parsed
    /// into the `UdfRequest` and plumbed to the WASM runtime. Over-ceiling values
    /// are clamped (never raise the built-in maximum) rather than rejected, so the
    /// op still completes through the wasm runtime. (The sandbox covers the clamp
    /// arithmetic itself; this asserts the dispatcher plumbs the field end-to-end.)
    #[cfg(feature = "udf-wasm")]
    #[tokio::test]
    async fn tool_call_udf_run_plumbs_and_clamps_wasm_limits() {
        let state = AppState::in_memory_for_tests()
            .await
            .with_policy(udf_runner_policy());
        let env = make_envelope(Op::ToolCall {
            call_id: tdw_protocol::ToolCallId::new("tc-3").expect("tool call id"),
            tool_name: "udf.run".to_string(),
            arguments: json!({
                "name": "upper",
                "runtime": "Wasm",
                // Non-base64-wasm source routes to the deterministic fixture path,
                // which still resolves (and clamps) the per-request limits.
                "source": "plain udf source",
                "input": "aapl",
                "allow_network": false,
                "allow_filesystem": false,
                // Over-ceiling fuel + memory: must be clamped, not rejected.
                "wasm_limits": { "fuel": u64::MAX, "max_memory_bytes": usize::MAX },
            }),
            permission_id: None,
        });
        let events = dispatch_op(&state, env).await;

        match &events[1] {
            EventMsg::Completed {
                result: Some(value),
                ..
            } => {
                assert_eq!(value["runtime"], "Wasm");
                assert_eq!(value["output"], "AAPL");
            }
            other => panic!("expected Completed (limits clamped, not rejected), got {other:?}"),
        }
    }
}
