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
#[cfg(feature = "alerts")]
use tdw_alerts::{AlertDirection, NewAlert, PriceAlert};
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
use tdw_domain::{QuoteSnapshot, ResultEnvelope, ResultExtra, Warning};
#[cfg(feature = "identity")]
use tdw_event::{EventEnvelope, sample_actor_context};
use tdw_hooks::SystemHookHandlerBackend;
#[cfg(feature = "identity")]
use tdw_identity::{IdentityError, NewUser};
use tdw_protocol::{EventMsg, Op, OpEnvelope, TimeRange};
use tdw_provider_fileset::FilesetEquityHistoricalFetcher;
use tdw_runtime::CommandRunner;
use tdw_sandbox::{LocalUdfSandbox, SandboxRuntime, UdfRequest};
use tdw_tools::{RegisteredTool, ToolDefinition, ToolRegistry, ToolRouter};
#[cfg(feature = "alerts")]
use uuid::Uuid;

use crate::provider_resolve::{is_logical_endpoint, resolve_logical_endpoint};
use crate::technical_compute;
use crate::{
    AppState, PolicyEnforcementConfig, PolicyEnforcementEvidence,
    SelectedYahooEquityHistoricalFetcher, ServiceEndpoint, enforce_request_path_with_backend,
    mask_json_response,
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
        } => dispatch_tool(state, policy, tool_name, arguments).await,
        Op::GetQuoteSnapshot { provider, symbol } => {
            dispatch_get_quote_snapshot(state, policy, provider, symbol).await
        }
        Op::FetchData { route, params } => dispatch_fetch_data(state, policy, route, params).await,
        Op::StreamStart {
            provider,
            symbol,
            table,
        } => dispatch_stream_start(state, policy, provider, symbol, table.clone()),
        Op::StreamStop { stream_id } => dispatch_stream_stop(state, policy, stream_id),
        #[cfg(feature = "alerts")]
        Op::CreateAlert {
            symbol,
            target_price,
            condition,
        } => {
            dispatch_create_alert(state, policy, symbol, target_price, condition).await
        }
        #[cfg(feature = "alerts")]
        Op::ListAlerts {} => dispatch_list_alerts(state, policy).await,
        #[cfg(feature = "alerts")]
        Op::DeleteAlert { id } => dispatch_delete_alert(state, policy, id).await,
        #[cfg(feature = "alerts")]
        Op::SetAlertActive { id, active } => {
            dispatch_set_alert_active(state, policy, id, *active).await
        }
        // When the `alerts` feature is disabled these variants are unreachable
        // from the daemon (the ACP validation layer still parses them), so we
        // return a descriptive error rather than silently dropping the request.
        #[cfg(not(feature = "alerts"))]
        Op::CreateAlert { .. }
        | Op::ListAlerts {}
        | Op::DeleteAlert { .. }
        | Op::SetAlertActive { .. } => Err(Error::Provider(
            "alert ops require the `alerts` feature; rebuild tdw-service-api with --features alerts"
                .to_string(),
        )),
        #[cfg(feature = "identity")]
        Op::RegisterUser {
            id,
            email,
            password,
            display_name,
            now_ms,
        } => {
            dispatch_register_user(
                state,
                policy,
                id.clone(),
                email.clone(),
                password.clone(),
                display_name.clone(),
                *now_ms,
            )
            .await
        }
        // When the `identity` feature is disabled this variant is unreachable
        // from the daemon (the ACP validation layer still parses it), so we
        // return a descriptive error rather than silently dropping the request.
        #[cfg(not(feature = "identity"))]
        Op::RegisterUser { .. } => Err(Error::Provider(
            "user registration requires the `identity` feature; rebuild tdw-service-api with \
             --features identity"
                .to_string(),
        )),
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
#[allow(unused_variables)]
// `symbol` is used only inside the cfg-gated provider block
// Awaits the HTTP fetch inside the `#[cfg(feature = "provider-*")]` arms; only the offline (no-provider) build has no await, so `async` is part of the contract callers `.await`.
#[allow(clippy::unused_async)]
async fn fetch_quote_snapshot(provider: &str, symbol: &str) -> Result<QuoteSnapshot> {
    #[cfg(feature = "provider-finnhub")]
    if provider == "finnhub" {
        use crate::FinnhubHttpQuoteSnapshotFetcher;
        use tdw_runtime::CommandRunner;
        let runner = CommandRunner::new(crate::default_registry()?);
        let params = json!({ "symbol": symbol });
        let object = runner
            .run(&FinnhubHttpQuoteSnapshotFetcher::default(), params)
            .await?;
        return object.rows.into_iter().next().ok_or_else(|| {
            Error::Provider(format!(
                "finnhub quote snapshot returned no rows for {symbol}"
            ))
        });
    }
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
const fn available_quote_snapshot_providers() -> &'static str {
    #[cfg(all(feature = "provider-finnhub", feature = "provider-fmp"))]
    {
        "finnhub, fmp"
    }
    #[cfg(all(feature = "provider-finnhub", not(feature = "provider-fmp")))]
    {
        "finnhub"
    }
    #[cfg(all(not(feature = "provider-finnhub"), feature = "provider-fmp"))]
    {
        "fmp"
    }
    #[cfg(not(any(feature = "provider-finnhub", feature = "provider-fmp")))]
    {
        "(none — enable a provider-* feature)"
    }
}

/// Fetch a catalog route's data and return the records directly — no persist.
///
/// This is the `Op::FetchData` handler and the runtime home of WS0's catalog
/// resolution + provider fallback. Flow (mirroring `GetQuoteSnapshot`, a
/// no-cache read):
///
/// 1. **Policy guard first** — `enforce_request_path_with_backend` runs before
///    any resolution, identical to every other op.
/// 2. **Catalog resolution** — `route` is looked up in
///    [`tdw_endpoint_catalog::catalog`]; an unknown route or one absent from
///    the fetch dispatch table yields a structured error.
/// 3. **Candidate selection + fallback** — with no explicit `provider`, the
///    registered candidates are tried in declaration order; a *retryable*
///    provider-side failure ([`Error::Provider`]/`Storage`/`Registry`) advances
///    to the next candidate and records a `provider_fallback` warning. A
///    *validation* failure ([`Error::InvalidQuery`]) fails fast — no fallback.
///    An explicit `provider` selects exactly one candidate and **never** falls
///    back.
/// 4. The standardized records are returned inside a [`ResultEnvelope`] in the
///    terminal event; nothing is written to any storage layer.
///
/// # Errors
///
/// Returns [`Error::Provider`] if policy enforcement fails, if `route` is
/// unknown or not a `Fetch` route, if the route has no registered candidate in
/// this build, if an explicit provider is not a candidate / not registered, or
/// if every tried candidate fails (the last error is surfaced).
async fn dispatch_fetch_data(
    state: &AppState,
    policy: &PolicyEnforcementConfig,
    route: &str,
    params: &Value,
) -> Result<Value> {
    let mut backend = SystemHookHandlerBackend::default();
    let evidence =
        enforce_request_path_with_backend(policy, ServiceEndpoint::IngestBatch, &mut backend)?;

    let Some(entry) = tdw_endpoint_catalog::lookup(route) else {
        return Err(Error::Provider(format!(
            "unknown catalog route: {route}; known: {}",
            known_catalog_routes()
        )));
    };
    // Compute routes (e.g. `technical/*`) derive their result from a caller-
    // supplied OHLCV series rather than a provider fetch; they share the
    // `Op::FetchData` op but take a parallel execution path.
    if entry.kind == tdw_endpoint_catalog::EndpointKind::Compute {
        return dispatch_compute(state, policy, route, params, &evidence).await;
    }

    let requested_provider = params
        .get("provider")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();

    let runner = CommandRunner::new((*state.registry).clone());
    let table = fetch_dispatch_table();

    let outcome = resolve_and_fetch(&entry, requested_provider, params, &table, &runner).await?;

    let extra = ResultExtra::default()
        .with_route(route)
        .with_argument("provider", outcome.provider);
    let mut envelope = ResultEnvelope::new(route, outcome.records)
        .with_provider(outcome.provider)
        .with_extra(extra);
    envelope.warnings = outcome.warnings;
    let body = serde_json::to_value(&envelope)
        .map_err(|e| Error::Provider(format!("result envelope serialize: {e}")))?;
    Ok(mask_json_response(
        json!({ "evidence": evidence, "result": body }),
        &policy.mask_rules,
    ))
}

/// Classified failure of [`rest_fetch_data`], so the REST transport can map it
/// to the right HTTP status without re-inspecting the opaque error string.
///
/// `route` resolution / shape problems and `InvalidQuery` validation are caller
/// errors (HTTP `400`); a provider-side failure after every candidate is tried
/// is upstream (HTTP `502`). The unknown-route message carries the known-routes
/// list so clients can discover valid routes.
#[cfg(feature = "rest-api-route")]
#[derive(Debug)]
pub enum RestFetchError {
    /// The route is not in the catalog or is not a fetch route. The message
    /// includes the known-routes list. Maps to HTTP `400`.
    UnknownRoute(String),
    /// A query-parameter validation error ([`Error::InvalidQuery`]). Maps to
    /// HTTP `400`.
    InvalidParams(String),
    /// Policy enforcement or a provider-side failure (every candidate failed).
    /// Maps to HTTP `502`.
    Provider(String),
}

/// REST seam for the catalog fetch path.
///
/// Resolves `route` and fetches through the SAME policy-guarded path
/// `Op::FetchData` uses, returning the `ResultEnvelope`-shaped body (no
/// `{evidence, result}` wrapper — the REST surface returns the envelope
/// directly) or a classified [`RestFetchError`].
///
/// This is the public entry point the `tdw-app-server` REST route family calls
/// through the `RestApiHandler` trait (implemented in [`crate::rest_handler`]).
/// It reuses the exact same policy guard, catalog resolution, dispatch table,
/// and provider-fallback logic as [`dispatch_fetch_data`]; only the error
/// classification and the response framing differ.
///
/// # Errors
///
/// Returns [`RestFetchError`] classified for HTTP status mapping (see the enum).
#[cfg(feature = "rest-api-route")]
pub async fn rest_fetch_data(
    state: &AppState,
    route: &str,
    params: Value,
) -> std::result::Result<Value, RestFetchError> {
    let policy = state
        .policy
        .as_ref()
        .ok_or_else(|| RestFetchError::Provider("daemon policy not configured".to_string()))?;

    let mut backend = SystemHookHandlerBackend::default();
    let evidence =
        enforce_request_path_with_backend(policy, ServiceEndpoint::IngestBatch, &mut backend)
            .map_err(|error| RestFetchError::Provider(error.to_string()))?;
    // The policy guard succeeded; REST returns the envelope directly, so the
    // evidence block is intentionally not surfaced to the client (mirroring the
    // OBBject-style response shape) — bind it to make that explicit.
    let _ = evidence;

    let Some(entry) = tdw_endpoint_catalog::lookup(route) else {
        return Err(RestFetchError::UnknownRoute(format!(
            "unknown catalog route: {route}; known: {}",
            known_catalog_routes()
        )));
    };
    if entry.kind != tdw_endpoint_catalog::EndpointKind::Fetch {
        return Err(RestFetchError::UnknownRoute(format!(
            "catalog route {route} is not a fetch route; known: {}",
            known_catalog_routes()
        )));
    }

    let requested_provider = params
        .get("provider")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();

    let runner = CommandRunner::new((*state.registry).clone());
    let table = fetch_dispatch_table();

    let outcome = resolve_and_fetch(&entry, requested_provider, &params, &table, &runner)
        .await
        .map_err(|error| match error {
            Error::InvalidQuery(message) => RestFetchError::InvalidParams(message),
            other => RestFetchError::Provider(other.to_string()),
        })?;

    let extra = ResultExtra::default()
        .with_route(route)
        .with_argument("provider", outcome.provider);
    let mut envelope = ResultEnvelope::new(route, outcome.records)
        .with_provider(outcome.provider)
        .with_extra(extra);
    envelope.warnings = outcome.warnings;
    serde_json::to_value(&envelope)
        .map_err(|e| RestFetchError::Provider(format!("result envelope serialize: {e}")))
}

/// Execute a `Compute` catalog route (the `technical/*` indicators).
///
/// The OHLCV series the indicator runs over comes from one of two mutually
/// exclusive sources in `params`:
///
/// 1. **Inline** — a `data` array of OHLCV records, computed directly.
/// 2. **Nested fetch** — a `source` object `{ "route": …, "params": … }` whose
///    `route` is a `Fetch` catalog route. The nested fetch runs through the
///    *same* policy-guarded fetch path (`resolve_and_fetch`), so the price
///    series is sourced under the normal provider-fallback policy, then piped
///    into the indicator. This enables e.g. `equity/price/historical` →
///    `technical/rsi` in one call.
///
/// The policy guard has already run in [`dispatch_fetch_data`]; `evidence` is
/// threaded through so the compute response carries the same evidence block as a
/// fetch response. Nothing is persisted (read path).
///
/// # Errors
///
/// Returns [`Error::InvalidQuery`] when neither `data` nor `source` is supplied,
/// when the bar records are malformed, or when the indicator params are invalid;
/// returns [`Error::Provider`] when a nested `source` route is unknown or not a
/// `Fetch` route, or when the indicator has no registered compute implementation.
async fn dispatch_compute(
    state: &AppState,
    policy: &PolicyEnforcementConfig,
    route: &str,
    params: &Value,
    evidence: &PolicyEnforcementEvidence,
) -> Result<Value> {
    let mut source_provider: Option<&'static str> = None;
    let mut warnings: Vec<Warning> = Vec::new();

    let bars = if let Some(source) = params.get("source").filter(|v| !v.is_null()) {
        let source_route = source.get("route").and_then(Value::as_str).ok_or_else(|| {
            Error::InvalidQuery(
                "technical compute: `source.route` must be a catalog route string".to_string(),
            )
        })?;
        let source_params = source.get("params").cloned().unwrap_or_else(|| json!({}));

        let Some(source_entry) = tdw_endpoint_catalog::lookup(source_route) else {
            return Err(Error::Provider(format!(
                "technical compute: unknown source route {source_route}"
            )));
        };
        if source_entry.kind != tdw_endpoint_catalog::EndpointKind::Fetch {
            return Err(Error::Provider(format!(
                "technical compute: source route {source_route} is not a fetch route"
            )));
        }

        let requested_provider = source_params
            .get("provider")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        let runner = CommandRunner::new((*state.registry).clone());
        let table = fetch_dispatch_table();
        let outcome = resolve_and_fetch(
            &source_entry,
            &requested_provider,
            &source_params,
            &table,
            &runner,
        )
        .await?;
        source_provider = Some(outcome.provider);
        warnings = outcome.warnings;
        let records = Value::Array(outcome.records);
        technical_compute::parse_bars(&records)?
    } else if let Some(data) = params.get("data").filter(|v| !v.is_null()) {
        technical_compute::parse_bars(data)?
    } else {
        return Err(Error::InvalidQuery(
            "technical compute: provide either an inline `data` OHLCV array or a nested `source` \
             { route, params } object"
                .to_string(),
        ));
    };

    let rows = technical_compute::run_compute(route, &bars, params)?;

    let mut extra = ResultExtra::default().with_route(route);
    if let Some(provider) = source_provider {
        extra = extra.with_argument("source_provider", provider);
    }
    let mut envelope = ResultEnvelope::new(route, rows).with_extra(extra);
    envelope.warnings = warnings;
    let body = serde_json::to_value(&envelope)
        .map_err(|e| Error::Provider(format!("compute envelope serialize: {e}")))?;
    Ok(mask_json_response(
        json!({ "evidence": evidence, "result": body }),
        &policy.mask_rules,
    ))
}

/// Successful outcome of [`resolve_and_fetch`]: which provider served the
/// request, the standardized records, and any fallback warnings accumulated.
#[derive(Debug)]
struct FetchOutcome {
    provider: &'static str,
    records: Vec<Value>,
    warnings: Vec<Warning>,
}

/// Resolve a catalog route to a provider and fetch its records, applying the
/// runtime fallback policy. Pure of policy/evidence/masking so it can be unit
/// tested with an injected fetch `table` and `runner`.
///
/// With no explicit provider, registered candidates are tried in declaration
/// order: a retryable provider-side error advances to the next candidate (and
/// records a `provider_fallback` warning naming the failed and next provider);
/// a validation error ([`Error::InvalidQuery`]) fails fast. An explicit provider
/// resolves to exactly one candidate and never falls back.
async fn resolve_and_fetch(
    entry: &tdw_endpoint_catalog::CatalogEntry,
    requested_provider: &str,
    params: &Value,
    table: &BTreeMap<(&'static str, &'static str), FetchBinding>,
    runner: &CommandRunner,
) -> Result<FetchOutcome> {
    let route = entry.route;
    let is_registered =
        |c: &tdw_endpoint_catalog::ProviderCandidate| table.contains_key(&(c.provider, c.endpoint));

    let attempt_order: Vec<tdw_endpoint_catalog::ProviderCandidate> =
        if requested_provider.is_empty() {
            let registered: Vec<_> = entry
                .candidates
                .iter()
                .copied()
                .filter(is_registered)
                .collect();
            if registered.is_empty() {
                return Err(Error::Provider(format!(
                    "no registered provider for catalog route {route}; candidates: {} \
                     (enable a provider-* feature)",
                    catalog_candidate_providers(entry.candidates)
                )));
            }
            registered
        } else {
            let Some(candidate) = entry
                .candidates
                .iter()
                .copied()
                .find(|c| c.provider == requested_provider)
            else {
                return Err(Error::Provider(format!(
                    "provider {requested_provider} does not serve catalog route {route}; \
                     candidates: {}",
                    catalog_candidate_providers(entry.candidates)
                )));
            };
            if !is_registered(&candidate) {
                return Err(Error::Provider(format!(
                    "provider {requested_provider} serves catalog route {route} but is not \
                     registered in this build"
                )));
            }
            vec![candidate]
        };

    let mut warnings: Vec<Warning> = Vec::new();
    let mut last_error: Option<Error> = None;

    for (index, candidate) in attempt_order.iter().enumerate() {
        let binding = table
            .get(&(candidate.provider, candidate.endpoint))
            .ok_or_else(|| {
                Error::Provider(format!(
                    "catalog candidate {}/{} vanished from the fetch dispatch table",
                    candidate.provider, candidate.endpoint
                ))
            })?;
        match (binding.run)(runner, params.clone()).await {
            Ok(records) => {
                return Ok(FetchOutcome {
                    provider: candidate.provider,
                    records,
                    warnings,
                });
            }
            Err(error) => {
                // Validation errors fail fast — never fall back.
                if !is_retryable_provider_error(&error) {
                    return Err(error);
                }
                let next = attempt_order
                    .get(index + 1)
                    .map_or("(none)", |c| c.provider);
                warnings.push(Warning::new(
                    "provider_fallback",
                    format!(
                        "provider {} failed ({error}); chosen provider {next}",
                        candidate.provider
                    ),
                ));
                last_error = Some(error);
            }
        }
    }

    Err(last_error.unwrap_or_else(|| {
        Error::Provider(format!("no candidate could serve catalog route {route}"))
    }))
}

/// Whether `error` is a retryable provider-side failure (fallback-eligible) as
/// opposed to a validation error (fail-fast).
///
/// `InvalidQuery` is a caller/validation error: the same bad params would fail
/// against every provider, so falling back is pointless and would mask the
/// real cause. `Provider`, `Storage`, and `Registry` errors are provider-side
/// and may succeed on a different candidate, so they are retryable.
const fn is_retryable_provider_error(error: &Error) -> bool {
    !matches!(error, Error::InvalidQuery(_))
}

/// Comma-separated, sorted list of known catalog routes for error messages.
fn known_catalog_routes() -> String {
    let mut routes: Vec<&'static str> = tdw_endpoint_catalog::catalog()
        .iter()
        .map(|e| e.route)
        .collect();
    routes.sort_unstable();
    routes.join(", ")
}

/// Comma-separated candidate provider names (in preference order).
fn catalog_candidate_providers(candidates: &[tdw_endpoint_catalog::ProviderCandidate]) -> String {
    candidates
        .iter()
        .map(|c| c.provider)
        .collect::<Vec<_>>()
        .join(", ")
}

/// One entry in the no-persist fetch dispatch table.
///
/// Mirrors [`IngestBinding`] but `run` returns the standardized records as JSON
/// rather than persisting them — the `Op::FetchData` read path writes nothing.
struct FetchBinding {
    run: FetchRunner,
}

type FetchRunner = Box<
    dyn for<'a> Fn(
            &'a CommandRunner,
            Value,
        ) -> Pin<Box<dyn Future<Output = Result<Vec<Value>>> + Send + 'a>>
        + Send
        + Sync,
>;

/// Build a [`FetchBinding`] for a concrete fetcher, returning its records as
/// JSON values without persistence.
fn fetch_binding<F, Q, D>() -> FetchBinding
where
    F: tdw_core::Fetcher<Q, D> + Default,
    Q: tdw_core::QueryParams,
    D: tdw_core::DataModel,
{
    FetchBinding {
        run: Box::new(move |runner: &CommandRunner, params: Value| {
            Box::pin(async move {
                let object = runner.run(&F::default(), params).await?;
                let mut records = Vec::with_capacity(object.rows.len());
                for row in &object.rows {
                    records
                        .push(serde_json::to_value(row).map_err(|e| {
                            Error::Provider(format!("fetch record serialize: {e}"))
                        })?);
                }
                Ok(records)
            })
        }),
    }
}

/// Build a [`FetchBinding`] for a FRED catalog-backed fetcher (macro or rate)
/// that resolves a fixed `OpenBB` `command` to its series. The command is
/// injected into the caller's params before the shared fetcher runs, so one
/// fetcher type serves every command in its cluster while the dispatch key
/// stays per-command.
#[cfg(feature = "provider-fred")]
fn fred_command_fetch_binding<F, D>(command: &'static str) -> FetchBinding
where
    F: tdw_core::Fetcher<tdw_provider_fred::FredCatalogQuery, D> + Default,
    D: tdw_core::DataModel,
{
    FetchBinding {
        run: Box::new(move |runner: &CommandRunner, mut params: Value| {
            if let Value::Object(map) = &mut params {
                map.insert("command".to_string(), Value::String(command.to_string()));
            }
            Box::pin(async move {
                let object = runner.run(&F::default(), params).await?;
                let mut records = Vec::with_capacity(object.rows.len());
                for row in &object.rows {
                    records
                        .push(serde_json::to_value(row).map_err(|e| {
                            Error::Provider(format!("fetch record serialize: {e}"))
                        })?);
                }
                Ok(records)
            })
        }),
    }
}

/// Resolve a FRED `OpenBB` `command` to the `'static` `(provider, endpoint)`
/// dispatch key declared for it in the endpoint catalog.
///
/// The catalog candidate's `endpoint` is the canonical `'static` key
/// (`<route with '/'→'_'>`); resolving through the catalog (rather than leaking
/// a freshly-derived string) keeps the dispatch key, the catalog candidate, and
/// the conformance test pinned to one allocation-free source of truth. Returns
/// `None` only if a FRED command is missing its catalog route — a bug the
/// conformance test catches.
#[cfg(feature = "provider-fred")]
fn fred_catalog_key(command: &str) -> Option<(&'static str, &'static str)> {
    let route = tdw_endpoint_catalog::lookup(command)?;
    route
        .candidates
        .iter()
        .find(|candidate| candidate.provider == "fred")
        .map(|candidate| (candidate.provider, candidate.endpoint))
}

/// Register every FRED-backed catalog fetch binding into `table`, keyed by the
/// catalog candidate endpoint. Macro/rate commands share one fetcher each (the
/// command is injected per binding); the aggregate yield-curve and the metadata
/// search are their own fetchers.
#[cfg(feature = "provider-fred")]
fn insert_fred_fetch_bindings(table: &mut BTreeMap<(&'static str, &'static str), FetchBinding>) {
    for endpoint in tdw_provider_fred::ENDPOINTS {
        let Some(key) = fred_catalog_key(endpoint.command) else {
            continue;
        };
        let binding = match endpoint.model {
            tdw_provider_fred::FredModel::Macro => fred_command_fetch_binding::<
                tdw_provider_fred::FredHttpMacroSeriesFetcher,
                tdw_domain::MacroSeries,
            >(endpoint.command),
            tdw_provider_fred::FredModel::Rate => fred_command_fetch_binding::<
                tdw_provider_fred::FredHttpRateObservationFetcher,
                tdw_domain::RateObservation,
            >(endpoint.command),
        };
        table.insert(key, binding);
    }
    table.insert(
        ("fred", "fixedincome_government_yield_curve"),
        fetch_binding::<tdw_provider_fred::FredHttpYieldCurveFetcher, _, _>(),
    );
    table.insert(
        ("fred", "fred_search"),
        fetch_binding::<tdw_provider_fred::FredHttpSeriesSearchFetcher, _, _>(),
    );
}

/// Register the ECB catalog fetch binding (G004 part 2): the euro FX
/// reference-rates snapshot, keyed by the fetcher's `ENDPOINT` const.
#[cfg(feature = "provider-ecb")]
fn insert_ecb_fetch_bindings(table: &mut BTreeMap<(&'static str, &'static str), FetchBinding>) {
    use crate::EcbHttpReferenceRatesFetcher;
    table.insert(
        ("ecb", EcbHttpReferenceRatesFetcher::ENDPOINT),
        fetch_binding::<EcbHttpReferenceRatesFetcher, _, _>(),
    );
}

/// Register the CBOE catalog fetch bindings (G004 part 2): the delayed index
/// snapshot and the delayed options chain, each keyed by its fetcher's
/// `ENDPOINT` const.
#[cfg(feature = "provider-cboe")]
fn insert_cboe_fetch_bindings(table: &mut BTreeMap<(&'static str, &'static str), FetchBinding>) {
    use crate::{CboeHttpIndexSnapshotFetcher, CboeHttpOptionsChainFetcher};
    table.insert(
        ("cboe", CboeHttpIndexSnapshotFetcher::ENDPOINT),
        fetch_binding::<CboeHttpIndexSnapshotFetcher, _, _>(),
    );
    table.insert(
        ("cboe", CboeHttpOptionsChainFetcher::ENDPOINT),
        fetch_binding::<CboeHttpOptionsChainFetcher, _, _>(),
    );
}

/// Build a [`FetchBinding`] for the EIA report fetcher that injects a fixed
/// `report` discriminator into the caller's params before the shared fetcher
/// runs, so one fetcher type serves both report routes while the dispatch key
/// stays per-report. Mirrors [`fred_command_fetch_binding`].
#[cfg(feature = "provider-eia")]
fn eia_report_fetch_binding(report: &'static str) -> FetchBinding {
    use crate::EiaHttpReportFetcher;
    FetchBinding {
        run: Box::new(move |runner: &CommandRunner, mut params: Value| {
            if let Value::Object(map) = &mut params {
                map.insert("report".to_string(), Value::String(report.to_string()));
            }
            Box::pin(async move {
                let object = runner.run(&EiaHttpReportFetcher::default(), params).await?;
                let mut records = Vec::with_capacity(object.rows.len());
                for row in &object.rows {
                    records
                        .push(serde_json::to_value(row).map_err(|e| {
                            Error::Provider(format!("fetch record serialize: {e}"))
                        })?);
                }
                Ok(records)
            })
        }),
    }
}

/// Register the EIA report fetch bindings (G004 part 2), keyed by each report
/// route's catalog endpoint and bound to the matching `report` discriminator.
#[cfg(feature = "provider-eia")]
fn insert_eia_fetch_bindings(table: &mut BTreeMap<(&'static str, &'static str), FetchBinding>) {
    use crate::EiaReport;
    table.insert(
        ("eia", "commodity_petroleum_status_report"),
        eia_report_fetch_binding(EiaReport::PetroleumStatusReport.id()),
    );
    table.insert(
        ("eia", "commodity_short_term_energy_outlook"),
        eia_report_fetch_binding(EiaReport::ShortTermEnergyOutlook.id()),
    );
}

/// Build a [`FetchBinding`] for the NASDAQ calendar fetcher that injects a fixed
/// `calendar` discriminator into the caller's params, so one fetcher type serves
/// all three calendars while the dispatch key stays per-calendar.
#[cfg(feature = "provider-nasdaq")]
fn nasdaq_calendar_fetch_binding(calendar: &'static str) -> FetchBinding {
    use crate::NasdaqHttpCalendarFetcher;
    FetchBinding {
        run: Box::new(move |runner: &CommandRunner, mut params: Value| {
            if let Value::Object(map) = &mut params {
                map.insert("calendar".to_string(), Value::String(calendar.to_string()));
            }
            Box::pin(async move {
                let object = runner
                    .run(&NasdaqHttpCalendarFetcher::default(), params)
                    .await?;
                let mut records = Vec::with_capacity(object.rows.len());
                for row in &object.rows {
                    records
                        .push(serde_json::to_value(row).map_err(|e| {
                            Error::Provider(format!("fetch record serialize: {e}"))
                        })?);
                }
                Ok(records)
            })
        }),
    }
}

/// Register the NASDAQ calendar fetch bindings (G004 part 2), keyed by each
/// calendar route's catalog endpoint and bound to the matching `calendar`
/// discriminator.
#[cfg(feature = "provider-nasdaq")]
fn insert_nasdaq_fetch_bindings(table: &mut BTreeMap<(&'static str, &'static str), FetchBinding>) {
    use crate::NasdaqCalendarKind;
    table.insert(
        ("nasdaq", "equity_calendar_dividends"),
        nasdaq_calendar_fetch_binding(NasdaqCalendarKind::Dividends.as_query_str()),
    );
    table.insert(
        ("nasdaq", "equity_calendar_earnings"),
        nasdaq_calendar_fetch_binding(NasdaqCalendarKind::Earnings.as_query_str()),
    );
    table.insert(
        ("nasdaq", "equity_calendar_ipo"),
        nasdaq_calendar_fetch_binding(NasdaqCalendarKind::Ipo.as_query_str()),
    );
}

/// Register the keyless Yahoo expansion fetch bindings (gap-matrix item L2.4),
/// keyed by each fetcher's `ENDPOINT` const — the same key its catalog candidate
/// declares. Mirrors [`insert_yahoo_ingest_bindings`] so the fetch and ingest
/// paths stay in lockstep; a conformance test keeps these keys and the catalog
/// candidates in sync.
#[cfg(feature = "provider-yahoo-http")]
fn insert_yahoo_fetch_bindings(table: &mut BTreeMap<(&'static str, &'static str), FetchBinding>) {
    use crate::{
        YahooHttpConsensusFetcher, YahooHttpDividendsFetcher, YahooHttpFuturesCurveFetcher,
        YahooHttpFuturesHistoricalFetcher, YahooHttpOptionsChainFetcher,
        YahooHttpPricePerformanceFetcher, YahooHttpProfileFetcher, YahooHttpQuoteFetcher,
        YahooHttpShareStatisticsFetcher,
    };
    table.insert(
        ("yahoo", YahooHttpProfileFetcher::ENDPOINT),
        fetch_binding::<YahooHttpProfileFetcher, _, _>(),
    );
    table.insert(
        ("yahoo", YahooHttpQuoteFetcher::ENDPOINT),
        fetch_binding::<YahooHttpQuoteFetcher, _, _>(),
    );
    table.insert(
        ("yahoo", YahooHttpPricePerformanceFetcher::ENDPOINT),
        fetch_binding::<YahooHttpPricePerformanceFetcher, _, _>(),
    );
    table.insert(
        ("yahoo", YahooHttpDividendsFetcher::ENDPOINT),
        fetch_binding::<YahooHttpDividendsFetcher, _, _>(),
    );
    table.insert(
        ("yahoo", YahooHttpShareStatisticsFetcher::ENDPOINT),
        fetch_binding::<YahooHttpShareStatisticsFetcher, _, _>(),
    );
    table.insert(
        ("yahoo", YahooHttpConsensusFetcher::ENDPOINT),
        fetch_binding::<YahooHttpConsensusFetcher, _, _>(),
    );
    table.insert(
        ("yahoo", YahooHttpOptionsChainFetcher::ENDPOINT),
        fetch_binding::<YahooHttpOptionsChainFetcher, _, _>(),
    );
    table.insert(
        ("yahoo", YahooHttpFuturesHistoricalFetcher::ENDPOINT),
        fetch_binding::<YahooHttpFuturesHistoricalFetcher, _, _>(),
    );
    table.insert(
        ("yahoo", YahooHttpFuturesCurveFetcher::ENDPOINT),
        fetch_binding::<YahooHttpFuturesCurveFetcher, _, _>(),
    );
}

/// The no-persist fetch dispatch table for this build.
///
/// Keyed identically to [`ingest_dispatch_table`] — every feature-enabled
/// fetcher plus the two always-on offline fixtures — but bound to a
/// records-returning closure (no bronze write). `Op::FetchData` resolves a
/// catalog candidate against these keys.
fn fetch_dispatch_table() -> BTreeMap<(&'static str, &'static str), FetchBinding> {
    let mut table: BTreeMap<(&'static str, &'static str), FetchBinding> = BTreeMap::new();
    table.insert(
        ("fileset", "equity_historical"),
        fetch_binding::<FilesetEquityHistoricalFetcher, _, _>(),
    );
    table.insert(
        ("yahoo", "equity_historical"),
        fetch_binding::<SelectedYahooEquityHistoricalFetcher, _, _>(),
    );
    #[cfg(feature = "provider-yahoo-http")]
    insert_yahoo_fetch_bindings(&mut table);
    #[cfg(feature = "provider-akshare")]
    table.insert(
        ("akshare", crate::AkShareHttpFetcher::ENDPOINT),
        fetch_binding::<crate::AkShareHttpFetcher, _, _>(),
    );
    #[cfg(feature = "provider-alpaca")]
    table.insert(
        ("alpaca", crate::AlpacaHttpStockBarsFetcher::ENDPOINT),
        fetch_binding::<crate::AlpacaHttpStockBarsFetcher, _, _>(),
    );
    #[cfg(feature = "provider-alpha-vantage")]
    table.insert(
        ("alpha_vantage", crate::AlphaVantageHttpFetcher::ENDPOINT),
        fetch_binding::<crate::AlphaVantageHttpFetcher, _, _>(),
    );
    #[cfg(feature = "provider-ccdata")]
    table.insert(
        ("ccdata", crate::CCDataHttpFetcher::ENDPOINT),
        fetch_binding::<crate::CCDataHttpFetcher, _, _>(),
    );
    #[cfg(feature = "provider-coingecko")]
    table.insert(
        ("coingecko", crate::CoinGeckoHttpOhlcFetcher::ENDPOINT),
        fetch_binding::<crate::CoinGeckoHttpOhlcFetcher, _, _>(),
    );
    #[cfg(feature = "provider-databento")]
    table.insert(
        ("databento", crate::DatabentoHttpTimeseriesFetcher::ENDPOINT),
        fetch_binding::<crate::DatabentoHttpTimeseriesFetcher, _, _>(),
    );
    #[cfg(feature = "provider-fmp")]
    table.insert(
        ("fmp", crate::FmpHttpHistoricalFetcher::ENDPOINT),
        fetch_binding::<crate::FmpHttpHistoricalFetcher, _, _>(),
    );
    #[cfg(feature = "provider-polygon")]
    table.insert(
        ("polygon", crate::PolygonHttpAggregatesFetcher::ENDPOINT),
        fetch_binding::<crate::PolygonHttpAggregatesFetcher, _, _>(),
    );
    #[cfg(feature = "provider-tiingo")]
    table.insert(
        ("tiingo", crate::TiingoHttpHistoricalFetcher::ENDPOINT),
        fetch_binding::<crate::TiingoHttpHistoricalFetcher, _, _>(),
    );
    #[cfg(feature = "provider-fred")]
    insert_fred_fetch_bindings(&mut table);
    #[cfg(feature = "provider-sec")]
    insert_sec_government_fetch_bindings(&mut table);
    #[cfg(feature = "provider-government-us")]
    insert_government_us_fetch_bindings(&mut table);
    #[cfg(feature = "provider-federal-reserve")]
    insert_federal_reserve_fetch_bindings(&mut table);
    // G004 part 2: ECB / CBOE / EIA / NASDAQ catalog projection.
    #[cfg(feature = "provider-ecb")]
    insert_ecb_fetch_bindings(&mut table);
    #[cfg(feature = "provider-cboe")]
    insert_cboe_fetch_bindings(&mut table);
    #[cfg(feature = "provider-eia")]
    insert_eia_fetch_bindings(&mut table);
    #[cfg(feature = "provider-nasdaq")]
    insert_nasdaq_fetch_bindings(&mut table);
    table
}

/// Register the keyless-government-wave SEC catalog fetch bindings (cik_map,
/// form_13f, fails_to_deliver, etf_holdings) into `table`. Each is its own
/// endpoint keyed by the fetcher's `ENDPOINT` const, mirroring the SEC
/// candidates declared in the endpoint catalog.
#[cfg(feature = "provider-sec")]
fn insert_sec_government_fetch_bindings(
    table: &mut BTreeMap<(&'static str, &'static str), FetchBinding>,
) {
    table.insert(
        ("sec", crate::SecCikMapHttpFetcher::ENDPOINT),
        fetch_binding::<crate::SecCikMapHttpFetcher, _, _>(),
    );
    table.insert(
        ("sec", crate::SecForm13FHttpFetcher::ENDPOINT),
        fetch_binding::<crate::SecForm13FHttpFetcher, _, _>(),
    );
    table.insert(
        ("sec", crate::SecFailsToDeliverHttpFetcher::ENDPOINT),
        fetch_binding::<crate::SecFailsToDeliverHttpFetcher, _, _>(),
    );
    table.insert(
        ("sec", crate::SecEtfHoldingsHttpFetcher::ENDPOINT),
        fetch_binding::<crate::SecEtfHoldingsHttpFetcher, _, _>(),
    );
}

/// Build a [`FetchBinding`] for a US Treasury FiscalData catalog-backed fetcher
/// that resolves a fixed `OpenBB` `command` to its dataset. The command is
/// injected into the caller's params before the shared fetcher runs. Mirrors
/// [`fred_command_fetch_binding`].
#[cfg(feature = "provider-government-us")]
fn gov_us_command_fetch_binding<F, D>(command: &'static str) -> FetchBinding
where
    F: tdw_core::Fetcher<tdw_provider_government_us::GovUsCatalogQuery, D> + Default,
    D: tdw_core::DataModel,
{
    FetchBinding {
        run: Box::new(move |runner: &CommandRunner, mut params: Value| {
            if let Value::Object(map) = &mut params {
                map.insert("command".to_string(), Value::String(command.to_string()));
            }
            Box::pin(async move {
                let object = runner.run(&F::default(), params).await?;
                let mut records = Vec::with_capacity(object.rows.len());
                for row in &object.rows {
                    records
                        .push(serde_json::to_value(row).map_err(|e| {
                            Error::Provider(format!("fetch record serialize: {e}"))
                        })?);
                }
                Ok(records)
            })
        }),
    }
}

/// Register the US Treasury FiscalData catalog fetch bindings into `table`,
/// keyed by the fetcher's short `ENDPOINT` (which equals the catalog candidate
/// endpoint). Each binding injects its route's `command`.
#[cfg(feature = "provider-government-us")]
fn insert_government_us_fetch_bindings(
    table: &mut BTreeMap<(&'static str, &'static str), FetchBinding>,
) {
    table.insert(
        ("government_us", "treasury_auctions"),
        gov_us_command_fetch_binding::<
            crate::GovUsTreasuryAuctionsHttpFetcher,
            tdw_domain::TreasuryAuction,
        >("fixedincome/government/treasury_auctions"),
    );
    table.insert(
        ("government_us", "treasury_prices"),
        gov_us_command_fetch_binding::<
            crate::GovUsTreasuryPricesHttpFetcher,
            tdw_domain::TreasuryPrice,
        >("fixedincome/government/treasury_prices"),
    );
}

/// Build a [`FetchBinding`] for a Federal Reserve catalog-backed fetcher that
/// resolves a fixed `OpenBB` `command` to its series/document set. The command
/// is injected into the caller's params before the shared fetcher runs, so one
/// fetcher type serves every command in its cluster (e.g. the macro fetcher
/// serves both `economy/money_measures` and `dealer_stats`) while the dispatch
/// key stays per-route. Mirrors [`fred_command_fetch_binding`].
#[cfg(feature = "provider-federal-reserve")]
fn fed_command_fetch_binding<F, D>(command: &'static str) -> FetchBinding
where
    F: tdw_core::Fetcher<tdw_provider_federal_reserve::FedCatalogQuery, D> + Default,
    D: tdw_core::DataModel,
{
    FetchBinding {
        run: Box::new(move |runner: &CommandRunner, mut params: Value| {
            if let Value::Object(map) = &mut params {
                map.insert("command".to_string(), Value::String(command.to_string()));
            }
            Box::pin(async move {
                let object = runner.run(&F::default(), params).await?;
                let mut records = Vec::with_capacity(object.rows.len());
                for row in &object.rows {
                    records
                        .push(serde_json::to_value(row).map_err(|e| {
                            Error::Provider(format!("fetch record serialize: {e}"))
                        })?);
                }
                Ok(records)
            })
        }),
    }
}

/// Register the Federal Reserve catalog fetch bindings into `table`, keyed by
/// the route-derived endpoint key (`<route with '/'→'_'>`), matching the
/// catalog candidates. The macro-series fetcher serves both
/// `economy/money_measures` and `fixedincome/government/dealer_stats` (each
/// binding injects its own `command`); the FOMC fetcher is its own command.
#[cfg(feature = "provider-federal-reserve")]
fn insert_federal_reserve_fetch_bindings(
    table: &mut BTreeMap<(&'static str, &'static str), FetchBinding>,
) {
    table.insert(
        ("federal_reserve", "economy_money_measures"),
        fed_command_fetch_binding::<crate::FedMacroSeriesHttpFetcher, tdw_domain::MacroSeries>(
            "economy/money_measures",
        ),
    );
    table.insert(
        ("federal_reserve", "fixedincome_government_dealer_stats"),
        fed_command_fetch_binding::<crate::FedMacroSeriesHttpFetcher, tdw_domain::MacroSeries>(
            "fixedincome/government/dealer_stats",
        ),
    );
    table.insert(
        ("federal_reserve", "regulators_fed_fomc_documents"),
        fed_command_fetch_binding::<crate::FedFomcDocumentsHttpFetcher, tdw_domain::FomcDocument>(
            "regulators/fed/fomc_documents",
        ),
    );
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
fn dispatch_stream_start(
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
fn dispatch_stream_stop(
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

/// Create a price alert owned by the authenticated principal.
///
/// Owner identity is derived exclusively from the verified JWT subject in
/// `policy` — it is never read from request fields, closing the id-only gap
/// in the source spec. `target_price` arrives as a decimal string from the
/// protocol layer (preserving `Op: Eq`); it is parsed here and rejected if
/// non-finite or non-positive. The alert id is minted as a `UUIDv7` string,
/// matching the `OpId::generated()` convention used elsewhere in the daemon.
/// When the `alerts` feature is absent this arm is unreachable (the match arm
/// in `run_dispatch` is `#[cfg(feature = "alerts")]`-gated at the call site),
/// so this function is only compiled with that feature.
///
/// # Errors
///
/// Returns [`Error::Provider`] if policy enforcement fails, if `target_price`
/// cannot be parsed as a finite positive `f64`, if `condition` is not a valid
/// [`AlertDirection`], or if the store write fails.
#[cfg(feature = "alerts")]
async fn dispatch_create_alert(
    state: &AppState,
    policy: &PolicyEnforcementConfig,
    symbol: &str,
    target_price: &str,
    condition: &str,
) -> Result<Value> {
    let mut backend = SystemHookHandlerBackend::default();
    let evidence =
        enforce_request_path_with_backend(policy, ServiceEndpoint::AlertManage, &mut backend)?;

    // Server-derived owner — never from the request payload.
    let owner_id = evidence.principal.clone();

    let price: f64 = target_price.parse().map_err(|_| {
        Error::Provider(format!(
            "target_price must be a decimal number, got: {target_price:?}"
        ))
    })?;
    if !price.is_finite() || price <= 0.0 {
        return Err(Error::Provider(format!(
            "target_price must be a finite positive number, got: {target_price:?}"
        )));
    }

    let direction: AlertDirection = condition.parse().map_err(|_| {
        Error::Provider(format!(
            "condition must be \"Above\" or \"Below\", got: {condition:?}"
        ))
    })?;

    let now_ms = now_ms();
    let id = Uuid::now_v7().to_string();
    let alert = PriceAlert::new(
        NewAlert {
            owner_id,
            symbol: symbol.to_string(),
            target_price: price,
            condition: direction,
            expires_at_ms: None,
        },
        id,
        now_ms,
    );

    state
        .alert_store
        .insert(&alert)
        .await
        .map_err(|e| Error::Provider(format!("alert store insert: {e}")))?;

    Ok(mask_json_response(
        json!({
            "evidence": evidence,
            "alert": serde_json::to_value(&alert)
                .map_err(|e| Error::Provider(format!("alert serialize: {e}")))?,
        }),
        &policy.mask_rules,
    ))
}

/// List all price alerts owned by the authenticated principal, newest first.
///
/// Owner scoping is enforced server-side: the query passes only the verified
/// JWT subject to `list_by_owner`, so a caller can never see another user's
/// alerts regardless of what any request field might carry.
///
/// # Errors
///
/// Returns [`Error::Provider`] if policy enforcement fails or if the store
/// read fails.
#[cfg(feature = "alerts")]
async fn dispatch_list_alerts(state: &AppState, policy: &PolicyEnforcementConfig) -> Result<Value> {
    let mut backend = SystemHookHandlerBackend::default();
    let evidence =
        enforce_request_path_with_backend(policy, ServiceEndpoint::AlertManage, &mut backend)?;

    let owner_id = evidence.principal.clone();

    let alerts = state
        .alert_store
        .list_by_owner(&owner_id)
        .await
        .map_err(|e| Error::Provider(format!("alert store list: {e}")))?;

    Ok(mask_json_response(
        json!({
            "evidence": evidence,
            "alerts": serde_json::to_value(&alerts)
                .map_err(|e| Error::Provider(format!("alerts serialize: {e}")))?,
            "count": alerts.len(),
        }),
        &policy.mask_rules,
    ))
}

/// Delete a price alert, with server-enforced owner check.
///
/// The store is queried by owner first: `list_by_owner` is used to check
/// ownership before calling `delete_by_id`. If no alert with `id` belonging
/// to this principal exists, the error does **not** leak whether an alert with
/// that id exists for another owner — both cases return the same
/// "not found or not owned" message (deliberate deviation from the source's
/// id-only delete gap).
///
/// # Errors
///
/// Returns [`Error::Provider`] if policy enforcement fails, if the caller does
/// not own the alert, or if the store write fails.
#[cfg(feature = "alerts")]
async fn dispatch_delete_alert(
    state: &AppState,
    policy: &PolicyEnforcementConfig,
    id: &str,
) -> Result<Value> {
    let mut backend = SystemHookHandlerBackend::default();
    let evidence =
        enforce_request_path_with_backend(policy, ServiceEndpoint::AlertManage, &mut backend)?;

    let owner_id = evidence.principal.clone();

    // Owner check: fetch this principal's alerts and confirm the id is among
    // them. Using list_by_owner (not a direct id lookup) ensures the existence
    // of an alert with that id belonging to *another* owner is not leaked.
    let owned = state
        .alert_store
        .list_by_owner(&owner_id)
        .await
        .map_err(|e| Error::Provider(format!("alert store list: {e}")))?;

    if !owned.iter().any(|a| a.id == id) {
        return Err(Error::Provider(
            "alert not found or not owned by the authenticated principal".to_string(),
        ));
    }

    state
        .alert_store
        .delete_by_id(id)
        .await
        .map_err(|e| Error::Provider(format!("alert store delete: {e}")))?;

    Ok(mask_json_response(
        json!({
            "evidence": evidence,
            "id": id,
            "deleted": true,
        }),
        &policy.mask_rules,
    ))
}

/// Toggle the `active` flag on a price alert, with server-enforced owner check.
///
/// Same ownership enforcement as [`dispatch_delete_alert`]: the alert must
/// belong to the authenticated principal; a mismatched or non-existent id
/// returns a non-leaking error.
///
/// # Errors
///
/// Returns [`Error::Provider`] if policy enforcement fails, if the caller does
/// not own the alert, or if the store write fails.
#[cfg(feature = "alerts")]
async fn dispatch_set_alert_active(
    state: &AppState,
    policy: &PolicyEnforcementConfig,
    id: &str,
    active: bool,
) -> Result<Value> {
    let mut backend = SystemHookHandlerBackend::default();
    let evidence =
        enforce_request_path_with_backend(policy, ServiceEndpoint::AlertManage, &mut backend)?;

    let owner_id = evidence.principal.clone();

    let owned = state
        .alert_store
        .list_by_owner(&owner_id)
        .await
        .map_err(|e| Error::Provider(format!("alert store list: {e}")))?;

    if !owned.iter().any(|a| a.id == id) {
        return Err(Error::Provider(
            "alert not found or not owned by the authenticated principal".to_string(),
        ));
    }

    state
        .alert_store
        .set_active(id, active)
        .await
        .map_err(|e| Error::Provider(format!("alert store set_active: {e}")))?;

    Ok(mask_json_response(
        json!({
            "evidence": evidence,
            "id": id,
            "active": active,
        }),
        &policy.mask_rules,
    ))
}

/// Register a new first-party user and emit a `user.created` event.
///
/// Mirrors the alert-create handler: a sync policy guard runs first
/// ([`ServiceEndpoint::UserRegister`]), then the async store call. The password
/// is validated and hashed inside the `tdw-identity` store; the returned
/// [`tdw_identity::User`] **never** carries the hash, and this handler never
/// logs or returns the password or hash. On success a [`UserCreatedPayload`]
/// is emitted onto the daemon outbox under [`USER_CREATED_EVENT_TYPE`] via the
/// same `tdw-event` envelope path the rest of the service uses, then the
/// created user is returned (response-masked).
///
/// [`UserCreatedPayload`]: crate::user_events::UserCreatedPayload
/// [`USER_CREATED_EVENT_TYPE`]: crate::user_events::USER_CREATED_EVENT_TYPE
///
/// # Errors
///
/// Returns [`Error::Provider`] if policy enforcement fails, if registration is
/// rejected by the store (invalid email, weak password, duplicate email), or if
/// the store write otherwise fails. Identity errors are mapped to
/// [`Error::Provider`] the same way alert-store errors are.
#[cfg(feature = "identity")]
#[allow(clippy::too_many_arguments)]
async fn dispatch_register_user(
    state: &AppState,
    policy: &PolicyEnforcementConfig,
    id: String,
    email: String,
    password: String,
    display_name: String,
    now_ms: i64,
) -> Result<Value> {
    use crate::user_events::{USER_CREATED_EVENT_TYPE, UserCreatedPayload};

    let mut backend = SystemHookHandlerBackend::default();
    let evidence =
        enforce_request_path_with_backend(policy, ServiceEndpoint::UserRegister, &mut backend)?;

    let user = state
        .user_store
        .register(
            NewUser {
                email,
                password,
                display_name,
            },
            id,
            now_ms,
        )
        .await
        .map_err(|error| match error {
            IdentityError::EmailTaken => Error::Provider("email already registered".to_string()),
            IdentityError::WeakPassword(reason) => {
                Error::Provider(format!("weak password: {reason}"))
            }
            IdentityError::InvalidEmail(reason) => {
                Error::Provider(format!("invalid email: {reason}"))
            }
            // Map any remaining identity error generically; the message never
            // includes the password or hash (none of these variants carry it).
            other => Error::Provider(format!("user store register: {other}")),
        })?;

    // Emit the `user.created` domain event onto the daemon outbox — the same
    // relay path the EventSink uses for EventMsgs. The payload carries only the
    // non-secret projection (id / email / created_at); never the password hash.
    let payload = UserCreatedPayload {
        user_id: user.id.clone(),
        email: user.email.clone(),
        created_at_ms: user.created_at_ms,
    };
    let payload_value = serde_json::to_value(&payload)
        .map_err(|e| Error::Provider(format!("user.created payload serialize: {e}")))?;
    // Clone before payload_value is moved into the EventEnvelope below.
    #[cfg(feature = "functions")]
    let enqueue_payload = payload_value.clone();
    let (actor, origin, trace) = sample_actor_context("tdw-service-api");
    let envelope: EventEnvelope<Value> = EventEnvelope::new(
        USER_CREATED_EVENT_TYPE,
        actor,
        origin,
        trace,
        "2026-05-28T00:00:00Z",
        payload_value,
    );
    {
        let mut outbox = state
            .outbox
            .lock()
            .map_err(|e| Error::Provider(format!("outbox lock poisoned: {e}")))?;
        outbox.append(envelope);
    }

    // Enqueue subscribed application-function jobs (e.g. welcome mailer) for
    // the `user.created` event.  Non-fatal: a failure here is warn-and-continue
    // so the registration result is never rolled back by a queue hiccup.
    #[cfg(feature = "functions")]
    if let Some(enqueuer) = state.function_enqueuer.as_ref() {
        let run_id_seed = format!("user.created:{}", payload.user_id);
        match enqueuer.enqueue_for_event(
            USER_CREATED_EVENT_TYPE,
            enqueue_payload,
            &run_id_seed,
            now_ms,
        ) {
            Ok(enqueued) => {
                tracing::debug!(
                    user_id = %payload.user_id,
                    enqueued,
                    "enqueued user.created function jobs"
                );
            }
            Err(error) => {
                tracing::warn!(
                    user_id = %payload.user_id,
                    %error,
                    "failed to enqueue user.created function jobs — continuing"
                );
            }
        }
    }

    // The returned user never carries the password hash (the `User` type has no
    // such field), so the masked response is safe to surface to the caller.
    Ok(mask_json_response(
        json!({
            "evidence": evidence,
            "user": serde_json::to_value(&user)
                .map_err(|e| Error::Provider(format!("user serialize: {e}")))?,
            "event_type": USER_CREATED_EVENT_TYPE,
        }),
        &policy.mask_rules,
    ))
}

/// Current Unix-epoch millisecond timestamp.
///
/// Used to supply `now_ms` when constructing a [`PriceAlert`] from a
/// [`NewAlert`] spec. Saturates to `i64::MAX` rather than panicking on
/// platforms where `SystemTime` could overflow, though that scenario is not
/// reachable on any target this crate is built for.
#[cfg(feature = "alerts")]
fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
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

    // Logical-endpoint resolution layer (L1.5): when `endpoint` is a logical
    // path (slash form, e.g. `equity/price/historical`), map it — honouring any
    // explicit `provider` argument, else first available registered candidate —
    // to a concrete (provider, endpoint) pair before the dispatch-table lookup.
    // Concrete endpoints (no slash) pass through unchanged, so the direct path is
    // fully backwards-compatible. This runs *after* the policy guard above, so
    // enforcement ordering is unchanged.
    let (provider, endpoint) = if is_logical_endpoint(endpoint) {
        let resolved =
            resolve_logical_endpoint(endpoint, provider, |p, e| table.contains_key(&(p, e)))?;
        (resolved.provider, resolved.endpoint)
    } else {
        (provider, endpoint)
    };

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

/// Build an [`IngestBinding`] for a FRED catalog-backed fetcher (macro or rate)
/// that resolves a fixed `OpenBB` `command` to its series, injecting the command
/// into the caller's params before the shared fetcher fetches one batch and
/// persists it into `table`.
#[cfg(feature = "provider-fred")]
fn fred_command_ingest_binding<F, D>(command: &'static str, table: &'static str) -> IngestBinding
where
    F: tdw_core::Fetcher<tdw_provider_fred::FredCatalogQuery, D> + Default,
    D: tdw_core::DataModel,
{
    IngestBinding {
        table,
        run: Box::new(
            move |state: &AppState,
                  runner: &CommandRunner,
                  mut params: Value,
                  table: &'static str,
                  token: String| {
                if let Value::Object(map) = &mut params {
                    map.insert("command".to_string(), Value::String(command.to_string()));
                }
                Box::pin(async move {
                    let object = runner.run(&F::default(), params).await?;
                    persist_batch(state, table, &token, &object).await
                })
            },
        ),
    }
}

/// Register every FRED-backed catalog ingest binding into `table`, keyed by the
/// catalog candidate endpoint and bound to its bronze landing table. Mirrors
/// [`insert_fred_fetch_bindings`] so the fetch and ingest paths stay in lockstep.
#[cfg(feature = "provider-fred")]
fn insert_fred_ingest_bindings(table: &mut BTreeMap<(&'static str, &'static str), IngestBinding>) {
    for endpoint in tdw_provider_fred::ENDPOINTS {
        let Some(key) = fred_catalog_key(endpoint.command) else {
            continue;
        };
        let binding = match endpoint.model {
            tdw_provider_fred::FredModel::Macro => {
                fred_command_ingest_binding::<
                    tdw_provider_fred::FredHttpMacroSeriesFetcher,
                    tdw_domain::MacroSeries,
                >(endpoint.command, "raw.macro_series")
            }
            tdw_provider_fred::FredModel::Rate => {
                fred_command_ingest_binding::<
                    tdw_provider_fred::FredHttpRateObservationFetcher,
                    tdw_domain::RateObservation,
                >(endpoint.command, "raw.rate_observation")
            }
        };
        table.insert(key, binding);
    }
    table.insert(
        ("fred", "fixedincome_government_yield_curve"),
        binding::<tdw_provider_fred::FredHttpYieldCurveFetcher, _, _>("raw.yield_curve_point"),
    );
    table.insert(
        ("fred", "fred_search"),
        binding::<tdw_provider_fred::FredHttpSeriesSearchFetcher, _, _>("raw.series_search_result"),
    );
}

/// Register the ECB catalog ingest binding (G004 part 2), keyed identically to
/// [`insert_ecb_fetch_bindings`] and bound to the shared `raw.macro_series`
/// bronze table (the reference rates normalize to `MacroSeries`).
#[cfg(feature = "provider-ecb")]
fn insert_ecb_ingest_bindings(table: &mut BTreeMap<(&'static str, &'static str), IngestBinding>) {
    use crate::EcbHttpReferenceRatesFetcher;
    table.insert(
        ("ecb", EcbHttpReferenceRatesFetcher::ENDPOINT),
        binding::<EcbHttpReferenceRatesFetcher, _, _>("raw.macro_series"),
    );
}

/// Register the CBOE catalog ingest bindings (G004 part 2), keyed identically to
/// [`insert_cboe_fetch_bindings`]; the index snapshot lands in `raw.price_quote`
/// and the options chain in `raw.option_contract`.
#[cfg(feature = "provider-cboe")]
fn insert_cboe_ingest_bindings(table: &mut BTreeMap<(&'static str, &'static str), IngestBinding>) {
    use crate::{CboeHttpIndexSnapshotFetcher, CboeHttpOptionsChainFetcher};
    table.insert(
        ("cboe", CboeHttpIndexSnapshotFetcher::ENDPOINT),
        binding::<CboeHttpIndexSnapshotFetcher, _, _>("raw.price_quote"),
    );
    table.insert(
        ("cboe", CboeHttpOptionsChainFetcher::ENDPOINT),
        binding::<CboeHttpOptionsChainFetcher, _, _>("raw.option_contract"),
    );
}

/// Build an [`IngestBinding`] for the EIA report fetcher that injects a fixed
/// `report` discriminator before fetching one batch and persisting it into
/// `table`. Mirrors [`fred_command_ingest_binding`].
#[cfg(feature = "provider-eia")]
fn eia_report_ingest_binding(report: &'static str, table: &'static str) -> IngestBinding {
    use crate::EiaHttpReportFetcher;
    IngestBinding {
        table,
        run: Box::new(
            move |state: &AppState,
                  runner: &CommandRunner,
                  mut params: Value,
                  table: &'static str,
                  token: String| {
                if let Value::Object(map) = &mut params {
                    map.insert("report".to_string(), Value::String(report.to_string()));
                }
                Box::pin(async move {
                    let object = runner.run(&EiaHttpReportFetcher::default(), params).await?;
                    persist_batch(state, table, &token, &object).await
                })
            },
        ),
    }
}

/// Register the EIA report ingest bindings (G004 part 2), keyed identically to
/// [`insert_eia_fetch_bindings`] and bound to the `raw.commodity_report_row`
/// bronze table.
#[cfg(feature = "provider-eia")]
fn insert_eia_ingest_bindings(table: &mut BTreeMap<(&'static str, &'static str), IngestBinding>) {
    use crate::EiaReport;
    table.insert(
        ("eia", "commodity_petroleum_status_report"),
        eia_report_ingest_binding(
            EiaReport::PetroleumStatusReport.id(),
            "raw.commodity_report_row",
        ),
    );
    table.insert(
        ("eia", "commodity_short_term_energy_outlook"),
        eia_report_ingest_binding(
            EiaReport::ShortTermEnergyOutlook.id(),
            "raw.commodity_report_row",
        ),
    );
}

/// Build an [`IngestBinding`] for the NASDAQ calendar fetcher that injects a
/// fixed `calendar` discriminator before fetching and persisting into `table`.
#[cfg(feature = "provider-nasdaq")]
fn nasdaq_calendar_ingest_binding(calendar: &'static str, table: &'static str) -> IngestBinding {
    use crate::NasdaqHttpCalendarFetcher;
    IngestBinding {
        table,
        run: Box::new(
            move |state: &AppState,
                  runner: &CommandRunner,
                  mut params: Value,
                  table: &'static str,
                  token: String| {
                if let Value::Object(map) = &mut params {
                    map.insert("calendar".to_string(), Value::String(calendar.to_string()));
                }
                Box::pin(async move {
                    let object = runner
                        .run(&NasdaqHttpCalendarFetcher::default(), params)
                        .await?;
                    persist_batch(state, table, &token, &object).await
                })
            },
        ),
    }
}

/// Register the NASDAQ calendar ingest bindings (G004 part 2), keyed identically
/// to [`insert_nasdaq_fetch_bindings`] and bound to the `raw.calendar_event`
/// bronze table.
#[cfg(feature = "provider-nasdaq")]
fn insert_nasdaq_ingest_bindings(
    table: &mut BTreeMap<(&'static str, &'static str), IngestBinding>,
) {
    use crate::NasdaqCalendarKind;
    table.insert(
        ("nasdaq", "equity_calendar_dividends"),
        nasdaq_calendar_ingest_binding(
            NasdaqCalendarKind::Dividends.as_query_str(),
            "raw.calendar_event",
        ),
    );
    table.insert(
        ("nasdaq", "equity_calendar_earnings"),
        nasdaq_calendar_ingest_binding(
            NasdaqCalendarKind::Earnings.as_query_str(),
            "raw.calendar_event",
        ),
    );
    table.insert(
        ("nasdaq", "equity_calendar_ipo"),
        nasdaq_calendar_ingest_binding(
            NasdaqCalendarKind::Ipo.as_query_str(),
            "raw.calendar_event",
        ),
    );
}

/// Register the keyless Yahoo expansion ingest bindings (gap-matrix item L2.4),
/// keyed identically to [`insert_yahoo_fetch_bindings`] and bound to each route's
/// bronze landing table. The bronze table for each row shape matches the catalog
/// route's `bronze_table`, so the `JSONEachRow` landing write stays
/// schema-coherent.
#[cfg(feature = "provider-yahoo-http")]
fn insert_yahoo_ingest_bindings(table: &mut BTreeMap<(&'static str, &'static str), IngestBinding>) {
    use crate::{
        YahooHttpConsensusFetcher, YahooHttpDividendsFetcher, YahooHttpFuturesCurveFetcher,
        YahooHttpFuturesHistoricalFetcher, YahooHttpOptionsChainFetcher,
        YahooHttpPricePerformanceFetcher, YahooHttpProfileFetcher, YahooHttpQuoteFetcher,
        YahooHttpShareStatisticsFetcher,
    };
    table.insert(
        ("yahoo", YahooHttpProfileFetcher::ENDPOINT),
        binding::<YahooHttpProfileFetcher, _, _>("raw.company_profile"),
    );
    table.insert(
        ("yahoo", YahooHttpQuoteFetcher::ENDPOINT),
        binding::<YahooHttpQuoteFetcher, _, _>("raw.price_quote"),
    );
    table.insert(
        ("yahoo", YahooHttpPricePerformanceFetcher::ENDPOINT),
        binding::<YahooHttpPricePerformanceFetcher, _, _>("raw.price_performance"),
    );
    table.insert(
        ("yahoo", YahooHttpDividendsFetcher::ENDPOINT),
        binding::<YahooHttpDividendsFetcher, _, _>("raw.corporate_action"),
    );
    table.insert(
        ("yahoo", YahooHttpShareStatisticsFetcher::ENDPOINT),
        binding::<YahooHttpShareStatisticsFetcher, _, _>("raw.ownership_record"),
    );
    table.insert(
        ("yahoo", YahooHttpConsensusFetcher::ENDPOINT),
        binding::<YahooHttpConsensusFetcher, _, _>("raw.estimate"),
    );
    table.insert(
        ("yahoo", YahooHttpOptionsChainFetcher::ENDPOINT),
        binding::<YahooHttpOptionsChainFetcher, _, _>("raw.option_contract"),
    );
    table.insert(
        ("yahoo", YahooHttpFuturesHistoricalFetcher::ENDPOINT),
        binding::<YahooHttpFuturesHistoricalFetcher, _, _>("raw.equity_historical"),
    );
    table.insert(
        ("yahoo", YahooHttpFuturesCurveFetcher::ENDPOINT),
        binding::<YahooHttpFuturesCurveFetcher, _, _>("raw.futures_curve_point"),
    );
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
    #[cfg(feature = "provider-yahoo-http")]
    insert_yahoo_ingest_bindings(&mut table);
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
    #[cfg(feature = "provider-fred")]
    insert_fred_ingest_bindings(&mut table);
    #[cfg(feature = "provider-sec")]
    insert_sec_government_ingest_bindings(&mut table);
    #[cfg(feature = "provider-government-us")]
    insert_government_us_ingest_bindings(&mut table);
    #[cfg(feature = "provider-federal-reserve")]
    insert_federal_reserve_ingest_bindings(&mut table);
    // G004 part 2: ECB / CBOE / EIA / NASDAQ catalog projection.
    #[cfg(feature = "provider-ecb")]
    insert_ecb_ingest_bindings(&mut table);
    #[cfg(feature = "provider-cboe")]
    insert_cboe_ingest_bindings(&mut table);
    #[cfg(feature = "provider-eia")]
    insert_eia_ingest_bindings(&mut table);
    #[cfg(feature = "provider-nasdaq")]
    insert_nasdaq_ingest_bindings(&mut table);
    table
}

/// Register the keyless-government-wave SEC catalog ingest bindings, mirroring
/// [`insert_sec_government_fetch_bindings`] so the fetch and ingest paths stay in
/// lockstep. Each binds the fetcher to its bronze landing table.
#[cfg(feature = "provider-sec")]
fn insert_sec_government_ingest_bindings(
    table: &mut BTreeMap<(&'static str, &'static str), IngestBinding>,
) {
    table.insert(
        ("sec", crate::SecCikMapHttpFetcher::ENDPOINT),
        binding::<crate::SecCikMapHttpFetcher, _, _>("raw.symbol_mapping"),
    );
    table.insert(
        ("sec", crate::SecForm13FHttpFetcher::ENDPOINT),
        binding::<crate::SecForm13FHttpFetcher, _, _>("raw.ownership_record"),
    );
    table.insert(
        ("sec", crate::SecFailsToDeliverHttpFetcher::ENDPOINT),
        binding::<crate::SecFailsToDeliverHttpFetcher, _, _>("raw.ownership_record"),
    );
    table.insert(
        ("sec", crate::SecEtfHoldingsHttpFetcher::ENDPOINT),
        binding::<crate::SecEtfHoldingsHttpFetcher, _, _>("raw.etf_holding"),
    );
}

/// Build an [`IngestBinding`] for a US Treasury catalog-backed fetcher that
/// injects a fixed `command` before fetching one batch and persisting it.
#[cfg(feature = "provider-government-us")]
fn gov_us_command_ingest_binding<F, D>(command: &'static str, table: &'static str) -> IngestBinding
where
    F: tdw_core::Fetcher<tdw_provider_government_us::GovUsCatalogQuery, D> + Default,
    D: tdw_core::DataModel,
{
    IngestBinding {
        table,
        run: Box::new(
            move |state: &AppState,
                  runner: &CommandRunner,
                  mut params: Value,
                  table: &'static str,
                  token: String| {
                if let Value::Object(map) = &mut params {
                    map.insert("command".to_string(), Value::String(command.to_string()));
                }
                Box::pin(async move {
                    let object = runner.run(&F::default(), params).await?;
                    persist_batch(state, table, &token, &object).await
                })
            },
        ),
    }
}

/// Register the US Treasury ingest bindings, mirroring the fetch path.
#[cfg(feature = "provider-government-us")]
fn insert_government_us_ingest_bindings(
    table: &mut BTreeMap<(&'static str, &'static str), IngestBinding>,
) {
    table.insert(
        ("government_us", "treasury_auctions"),
        gov_us_command_ingest_binding::<
            crate::GovUsTreasuryAuctionsHttpFetcher,
            tdw_domain::TreasuryAuction,
        >(
            "fixedincome/government/treasury_auctions",
            "raw.treasury_auction",
        ),
    );
    table.insert(
        ("government_us", "treasury_prices"),
        gov_us_command_ingest_binding::<
            crate::GovUsTreasuryPricesHttpFetcher,
            tdw_domain::TreasuryPrice,
        >(
            "fixedincome/government/treasury_prices",
            "raw.treasury_price",
        ),
    );
}

/// Build an [`IngestBinding`] for a Federal Reserve catalog-backed fetcher that
/// injects a fixed `command` before fetching one batch and persisting it.
#[cfg(feature = "provider-federal-reserve")]
fn fed_command_ingest_binding<F, D>(command: &'static str, table: &'static str) -> IngestBinding
where
    F: tdw_core::Fetcher<tdw_provider_federal_reserve::FedCatalogQuery, D> + Default,
    D: tdw_core::DataModel,
{
    IngestBinding {
        table,
        run: Box::new(
            move |state: &AppState,
                  runner: &CommandRunner,
                  mut params: Value,
                  table: &'static str,
                  token: String| {
                if let Value::Object(map) = &mut params {
                    map.insert("command".to_string(), Value::String(command.to_string()));
                }
                Box::pin(async move {
                    let object = runner.run(&F::default(), params).await?;
                    persist_batch(state, table, &token, &object).await
                })
            },
        ),
    }
}

/// Register the Federal Reserve ingest bindings, mirroring the fetch path.
#[cfg(feature = "provider-federal-reserve")]
fn insert_federal_reserve_ingest_bindings(
    table: &mut BTreeMap<(&'static str, &'static str), IngestBinding>,
) {
    table.insert(
        ("federal_reserve", "economy_money_measures"),
        fed_command_ingest_binding::<crate::FedMacroSeriesHttpFetcher, tdw_domain::MacroSeries>(
            "economy/money_measures",
            "raw.macro_series",
        ),
    );
    table.insert(
        ("federal_reserve", "fixedincome_government_dealer_stats"),
        fed_command_ingest_binding::<crate::FedMacroSeriesHttpFetcher, tdw_domain::MacroSeries>(
            "fixedincome/government/dealer_stats",
            "raw.macro_series",
        ),
    );
    table.insert(
        ("federal_reserve", "regulators_fed_fomc_documents"),
        fed_command_ingest_binding::<crate::FedFomcDocumentsHttpFetcher, tdw_domain::FomcDocument>(
            "regulators/fed/fomc_documents",
            "raw.fomc_document",
        ),
    );
}

/// The `(provider, endpoint)` keys registered in this build's ingest dispatch
/// table, sorted.
///
/// Exposed so conformance tooling (e.g. the feature-gated
/// `catalog_candidates_all_dispatchable_under_full_providers` test) can verify
/// every catalog candidate is dispatchable under the build's features without
/// reaching into the private binding closures.
#[must_use]
pub fn ingest_dispatch_pairs() -> Vec<(&'static str, &'static str)> {
    ingest_dispatch_table().into_keys().collect()
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

async fn dispatch_tool(
    state: &AppState,
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

    // The `technical.*` tools are the catalog's `technical/*` Compute routes
    // exposed as tools: the tool name maps to the route by swapping `.` for `/`,
    // and execution reuses the same compute path as `Op::FetchData`. Policy
    // enforcement already ran above; thread its evidence through unchanged.
    if let Some(indicator) = tool_name.strip_prefix("technical.") {
        let route = format!("technical/{indicator}");
        return dispatch_compute(state, policy, &route, arguments, &evidence).await;
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
    register_technical_tools(&mut registry);
    registry
}

/// Register one [`RegisteredTool`] per `technical/*` catalog Compute route.
///
/// The tool name is the route with `/` swapped for `.` (`technical/sma` →
/// `technical.sma`), so `Op::ToolCall` and the MCP tool list expose every
/// indicator for free with the route's own param/model JSON schemas attached.
/// Driving this from `tdw_endpoint_catalog::catalog()` keeps the tool set in
/// lockstep with the catalog (a conformance test pins the two together).
fn register_technical_tools(registry: &mut ToolRegistry) {
    for entry in tdw_endpoint_catalog::catalog() {
        if entry.kind != tdw_endpoint_catalog::EndpointKind::Compute {
            continue;
        }
        let Some(indicator) = entry.route.strip_prefix("technical/") else {
            continue;
        };
        let name = format!("technical.{indicator}");
        let input_schema = serde_json::to_value((entry.params_schema)())
            .unwrap_or_else(|_| json!({ "type": "object" }));
        let output_schema =
            serde_json::to_value((entry.model)()).unwrap_or_else(|_| json!({ "type": "object" }));
        let definition = ToolDefinition {
            name: name.clone(),
            description: entry.doc.to_string(),
            input_schema,
            output_schema,
            permission_pattern: name,
        };
        registry
            .register(RegisteredTool::new(
                definition,
                technical_tool_placeholder_handler,
            ))
            .expect("technical tool definitions are valid and registered exactly once");
    }
}

/// Placeholder handler for a `technical.*` registry entry. Never invoked:
/// [`dispatch_tool`] executes technical tools via [`dispatch_compute`] (which
/// needs `AppState` for the nested-fetch path). The registry only needs a
/// handler to construct a [`RegisteredTool`]; the echo behaviour is inert.
#[allow(clippy::unnecessary_wraps)]
const fn technical_tool_placeholder_handler(input: Value) -> tdw_tools::Result<Value> {
    Ok(input)
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
// Signature is fixed by the `ToolHandler = fn(Value) -> tdw_tools::Result<Value>` alias that `RegisteredTool::new` requires; the `Result` cannot be unwrapped away.
#[allow(clippy::unnecessary_wraps)]
const fn udf_run_placeholder_handler(input: Value) -> tdw_tools::Result<Value> {
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
    use std::sync::Arc;
    use tdw_auth_oidc::{JwksKey, JwtClaims};
    use tdw_core::ProviderRegistry;
    use tdw_protocol::{ActorKind, ActorRef, SessionId};
    use tdw_provider_yahoo::YahooEquityHistoricalFetcher;

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
    /// `tdw.udf.run` (`ToolCall`) endpoint — analyst alone is denied there.
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

    async fn offline_fixture_ingest_state() -> AppState {
        let mut registry = ProviderRegistry::default();
        registry
            .register(FilesetEquityHistoricalFetcher::registry_entry())
            .expect("fileset fixture registers");
        registry
            .register(YahooEquityHistoricalFetcher::registry_entry())
            .expect("yahoo fixture registers");

        let mut state = AppState::in_memory_for_tests()
            .await
            .with_policy(analyst_policy());
        state.registry = Arc::new(registry);
        state
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
        // Uses `fileset` (never feature-swapped) so the test stays hermetic:
        // under `all-http-providers` the yahoo binding is the live HTTP
        // fetcher, and dispatching it would hit the network from a unit test.
        let state = offline_fixture_ingest_state().await;
        let env = make_envelope(Op::IngestBatch {
            provider: "fileset".to_string(),
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
                assert_eq!(value["provider"], "fileset");
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
        feature = "provider-cboe",
        feature = "provider-ccdata",
        feature = "provider-coingecko",
        feature = "provider-databento",
        feature = "provider-ecb",
        feature = "provider-eia",
        feature = "provider-finnhub",
        feature = "provider-fmp",
        feature = "provider-nasdaq",
        feature = "provider-polygon",
        feature = "provider-sec",
        feature = "provider-tiingo",
        feature = "provider-yahoo-http",
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

    #[tokio::test]
    async fn ingest_logical_endpoint_default_resolves_first_available_provider() {
        // L1.5: a logical endpoint with no explicit provider resolves to the
        // first registered candidate. In the offline build that is `fileset`
        // (it leads `yahoo` in the candidate order), and the dispatch result
        // reports the *resolved* concrete provider/endpoint.
        let state = AppState::in_memory_for_tests()
            .await
            .with_policy(analyst_policy());
        let env = make_envelope(Op::IngestBatch {
            provider: String::new(),
            endpoint: "equity/price/historical".to_string(),
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
                assert_eq!(value["endpoint"], "equity_historical");
                assert_eq!(value["table"], "raw.equity_historical");
                assert!(value["rows"].as_u64().expect("rows count") >= 1);
            }
            other => panic!("expected Completed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn ingest_logical_endpoint_explicit_provider_wins() {
        // L1.5: an explicit `provider` selects that candidate even when it is not
        // first in the candidate order (`yahoo` follows `fileset`).
        let state = offline_fixture_ingest_state().await;
        let env = make_envelope(Op::IngestBatch {
            provider: "yahoo".to_string(),
            endpoint: "equity/price/historical".to_string(),
            symbols: vec!["AAPL".to_string()],
            range: None,
        });
        let events = dispatch_op(&state, env).await;

        // The claim under test is ROUTING: the explicit `yahoo` must win over
        // the first candidate (`fileset`). Under `all-http-providers` the
        // yahoo binding is the live HTTP fetcher, so the dispatch may fail at
        // the network layer in an offline CI sandbox — a yahoo-attributed
        // failure still proves yahoo was selected. The offline default build
        // always takes the Completed arm.
        match &events[1] {
            EventMsg::Completed {
                result: Some(value),
                ..
            } => {
                assert_eq!(value["provider"], "yahoo");
                assert_eq!(value["endpoint"], "equity_historical");
            }
            EventMsg::Failed { error, .. } => {
                assert!(
                    error.contains("yahoo"),
                    "explicit provider must still be yahoo in the failure, got: {error}"
                );
            }
            other => panic!("expected Completed or a yahoo-attributed Failed, got {other:?}"),
        }
    }

    /// Offline-only: with `provider-fmp` enabled, `fmp` IS registered, so this
    /// unavailable-provider assertion only holds in the default (no-feature)
    /// build. Mirrors `ingest_dispatch_table_offline_default_is_exactly_two_fixtures`.
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
    async fn ingest_logical_endpoint_unavailable_provider_fails_with_candidates() {
        // L1.5: an explicit provider that is a candidate but not registered in
        // this build fails with a structured error listing the registered
        // candidates (`fmp` is feature-gated; offline build has fileset+yahoo).
        let state = AppState::in_memory_for_tests()
            .await
            .with_policy(analyst_policy());
        let env = make_envelope(Op::IngestBatch {
            provider: "fmp".to_string(),
            endpoint: "equity/price/historical".to_string(),
            symbols: vec!["AAPL".to_string()],
            range: None,
        });
        let events = dispatch_op(&state, env).await;

        match &events[1] {
            EventMsg::Failed { error, .. } => {
                assert!(
                    error.contains("not registered in this build"),
                    "got: {error}"
                );
                assert!(
                    error.contains("fileset") && error.contains("yahoo"),
                    "error should list registered candidates, got: {error}"
                );
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    /// Offline-only: the crypto candidates (`coingecko`/`ccdata`) become
    /// registered under their provider features, so the no-available-provider
    /// assertion only holds in the default build.
    #[cfg(not(any(feature = "provider-coingecko", feature = "provider-ccdata")))]
    #[tokio::test]
    async fn ingest_logical_endpoint_no_available_provider_fails() {
        // L1.5: `crypto/price/historical` candidates are all feature-gated, so an
        // offline build cannot resolve any and reports a structured error naming
        // the candidates.
        let state = AppState::in_memory_for_tests()
            .await
            .with_policy(analyst_policy());
        let env = make_envelope(Op::IngestBatch {
            provider: String::new(),
            endpoint: "crypto/price/historical".to_string(),
            symbols: vec!["BTC".to_string()],
            range: None,
        });
        let events = dispatch_op(&state, env).await;

        match &events[1] {
            EventMsg::Failed { error, .. } => {
                assert!(
                    error.contains("no registered provider for logical endpoint"),
                    "got: {error}"
                );
                assert!(
                    error.contains("coingecko") && error.contains("ccdata"),
                    "got: {error}"
                );
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn ingest_concrete_endpoint_bypasses_logical_resolution() {
        // L1.5 backwards-compat: a concrete endpoint (no slash) is dispatched
        // directly, exactly as before the resolution layer existed. Uses
        // `fileset` (never feature-swapped) so the test stays hermetic under
        // `all-http-providers`, where the yahoo binding is the live fetcher.
        let state = offline_fixture_ingest_state().await;
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
                assert_eq!(value["endpoint"], "equity_historical");
            }
            other => panic!("expected Completed, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // WS0 catalog: FetchData dispatch + runtime fallback conformance
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn fetch_data_offline_default_resolves_equity_fixture_network_free() {
        // The offline default build resolves `equity/price/historical` to the
        // first registered candidate (`fileset`) and returns its records in the
        // terminal event without touching the network or any storage layer.
        let state = AppState::in_memory_for_tests()
            .await
            .with_policy(analyst_policy());
        let env = make_envelope(Op::FetchData {
            route: "equity/price/historical".to_string(),
            params: json!({ "symbol": "AAPL" }),
        });
        let events = dispatch_op(&state, env).await;

        match &events[1] {
            EventMsg::Completed {
                result: Some(value),
                ..
            } => {
                assert_eq!(value["result"]["provider"], "fileset");
                assert_eq!(value["result"]["extra"]["route"], "equity/price/historical");
                let rows = value["result"]["results"]
                    .as_array()
                    .expect("results array");
                assert!(!rows.is_empty(), "fixture must return rows, got {value}");
                // No-persist read path: no fallback warning on a clean first hit.
                assert!(
                    value["result"]["warnings"]
                        .as_array()
                        .expect("warnings array")
                        .is_empty(),
                    "no fallback expected on first-candidate success, got {value}"
                );
            }
            other => panic!("expected Completed, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // L4.1 technical compute: inline-data and nested-fetch conformance
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn fetch_data_compute_inline_rsi_is_date_aligned() {
        // A `technical/rsi` Compute route over an inline OHLCV `data` array runs
        // the indicator network-free and returns one row per input bar.
        let state = AppState::in_memory_for_tests()
            .await
            .with_policy(analyst_policy());
        let mut data = Vec::new();
        // Strictly rising closes ⇒ RSI saturates at 100 once defined.
        for i in 0..10 {
            let close = 10.0 + f64::from(i);
            data.push(json!({
                "date": format!("2026-01-{:02}", i + 1),
                "open": close, "high": close + 0.5, "low": close - 0.5,
                "close": close, "volume": 1000
            }));
        }
        let env = make_envelope(Op::FetchData {
            route: "technical/rsi".to_string(),
            params: json!({ "data": data, "length": 3 }),
        });
        let events = dispatch_op(&state, env).await;
        match &events[1] {
            EventMsg::Completed {
                result: Some(value),
                ..
            } => {
                assert_eq!(value["result"]["extra"]["route"], "technical/rsi");
                let rows = value["result"]["results"].as_array().expect("results");
                assert_eq!(rows.len(), 10, "one row per input bar, got {value}");
                assert!(rows[0].is_null(), "leading None before the window fills");
                let last = rows[9].as_f64().expect("rsi value");
                assert!(
                    (last - 100.0).abs() < 1e-6,
                    "all-gains RSI is 100, got {last}"
                );
            }
            other => panic!("expected Completed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn fetch_data_compute_nested_fetch_fileset_to_sma_offline() {
        // A nested `source` first fetches `equity/price/historical` through the
        // offline fileset fixture, then pipes the bars into `technical/sma`. The
        // whole chain stays network-free and the response names the source
        // provider.
        let state = offline_fixture_ingest_state().await;
        let env = make_envelope(Op::FetchData {
            route: "technical/sma".to_string(),
            params: json!({
                "length": 3,
                "source": {
                    "route": "equity/price/historical",
                    "params": { "symbol": "AAPL" }
                }
            }),
        });
        let events = dispatch_op(&state, env).await;
        match &events[1] {
            EventMsg::Completed {
                result: Some(value),
                ..
            } => {
                assert_eq!(value["result"]["extra"]["route"], "technical/sma");
                assert_eq!(
                    value["result"]["extra"]["arguments"]["source_provider"], "fileset",
                    "nested fetch resolved through the offline fileset fixture, got {value}"
                );
                let rows = value["result"]["results"].as_array().expect("results");
                assert!(
                    !rows.is_empty(),
                    "fixture-sourced SMA returns rows, got {value}"
                );
                // The first two positions are leading-None for an SMA(3).
                assert!(rows[0].is_null());
            }
            other => panic!("expected Completed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn fetch_data_compute_without_data_or_source_is_invalid() {
        let state = AppState::in_memory_for_tests()
            .await
            .with_policy(analyst_policy());
        let env = make_envelope(Op::FetchData {
            route: "technical/sma".to_string(),
            params: json!({ "length": 3 }),
        });
        let events = dispatch_op(&state, env).await;
        match &events[1] {
            EventMsg::Failed { error, .. } => {
                assert!(error.contains("inline `data`"), "got: {error}");
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn tool_call_technical_sma_inline_data_runs_compute() {
        // The `technical.sma` ToolCall maps to the `technical/sma` Compute route
        // and runs the same inline-data path as `Op::FetchData`. The `ToolCall`
        // endpoint requires the `udf_runner` role.
        let state = AppState::in_memory_for_tests()
            .await
            .with_policy(udf_runner_policy());
        let data = json!([
            { "date": "2026-01-01", "open": 10.0, "high": 10.5, "low": 9.8, "close": 10.0, "volume": 1000 },
            { "date": "2026-01-02", "open": 10.0, "high": 11.2, "low": 9.9, "close": 11.0, "volume": 1100 },
            { "date": "2026-01-03", "open": 11.0, "high": 12.4, "low": 10.8, "close": 12.0, "volume": 1200 }
        ]);
        let env = make_envelope(Op::ToolCall {
            call_id: tdw_protocol::ToolCallId::new("tc-tech-1").expect("tool call id"),
            tool_name: "technical.sma".to_string(),
            arguments: json!({ "data": data, "length": 2 }),
            permission_id: None,
        });
        let events = dispatch_op(&state, env).await;
        match &events[1] {
            EventMsg::Completed {
                result: Some(value),
                ..
            } => {
                let rows = value["result"]["results"].as_array().expect("results");
                assert_eq!(rows.len(), 3);
                assert!((rows[1].as_f64().expect("sma") - 10.5).abs() < 1e-9);
            }
            other => panic!("expected Completed, got {other:?}"),
        }
    }

    /// Every `technical/*` catalog Compute route is registered both as a compute
    /// implementation and as a `technical.*` tool, and vice versa — no orphan on
    /// either side.
    #[test]
    fn technical_compute_routes_tools_and_catalog_agree() {
        use std::collections::BTreeSet;

        let catalog_routes: BTreeSet<String> = tdw_endpoint_catalog::catalog()
            .iter()
            .filter(|e| e.kind == tdw_endpoint_catalog::EndpointKind::Compute)
            .map(|e| e.route.to_string())
            .collect();

        let registry = service_tool_registry();
        let tool_routes: BTreeSet<String> = registry
            .definitions()
            .into_iter()
            .filter_map(|d| {
                d.name
                    .strip_prefix("technical.")
                    .map(|ind| format!("technical/{ind}"))
            })
            .collect();
        assert_eq!(
            catalog_routes, tool_routes,
            "technical tool set must equal the catalog Compute route set"
        );

        // And each route has a runnable compute implementation.
        let bars: Vec<tdw_domain::MarketDataBar> = Vec::new();
        for route in &catalog_routes {
            let result = technical_compute::run_compute(route, &bars, &json!({}));
            assert!(
                result.is_ok(),
                "compute route {route} has no registered implementation"
            );
        }
    }

    #[tokio::test]
    async fn fetch_data_unknown_route_fails_with_known_list() {
        let state = AppState::in_memory_for_tests()
            .await
            .with_policy(analyst_policy());
        let env = make_envelope(Op::FetchData {
            route: "equity/does/not/exist".to_string(),
            params: json!({ "symbol": "AAPL" }),
        });
        let events = dispatch_op(&state, env).await;
        match &events[1] {
            EventMsg::Failed { error, .. } => {
                assert!(error.contains("unknown catalog route"), "got: {error}");
                assert!(
                    error.contains("equity/price/historical"),
                    "should list known routes, got: {error}"
                );
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    /// Offline-only: `provider-fmp` registers `fmp`, so the not-registered
    /// assertion only holds in the default (no-feature) build. Mirrors
    /// `ingest_logical_endpoint_unavailable_provider_fails_with_candidates`.
    #[cfg(not(feature = "provider-fmp"))]
    #[tokio::test]
    async fn fetch_data_explicit_unregistered_provider_does_not_fall_back() {
        // Explicit `provider=fmp` is a candidate but unregistered in the offline
        // build: it fails fast with a not-registered error and never falls back
        // to fileset/yahoo.
        let state = AppState::in_memory_for_tests()
            .await
            .with_policy(analyst_policy());
        let env = make_envelope(Op::FetchData {
            route: "equity/price/historical".to_string(),
            params: json!({ "symbol": "AAPL", "provider": "fmp" }),
        });
        let events = dispatch_op(&state, env).await;
        match &events[1] {
            EventMsg::Failed { error, .. } => {
                assert!(
                    error.contains("not registered in this build"),
                    "got: {error}"
                );
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    /// Catalog ⊆ dispatch table: every `Fetch` candidate the fetch dispatch
    /// table registers must be a declared catalog candidate (no orphan bindings),
    /// and the always-on fixtures must be present in both. Under the offline
    /// default build only the fixtures are registered.
    #[test]
    fn catalog_fetch_candidates_are_consistent_with_dispatch_table() {
        use std::collections::BTreeSet;

        let catalog_pairs: BTreeSet<(&str, &str)> = tdw_endpoint_catalog::catalog()
            .iter()
            .flat_map(|e| e.candidates.iter().map(|c| (c.provider, c.endpoint)))
            .collect();

        let fetch_table = fetch_dispatch_table();
        for (provider, endpoint) in fetch_table.keys() {
            assert!(
                catalog_pairs.contains(&(*provider, *endpoint)),
                "fetch dispatch table key {provider}/{endpoint} is not a catalog candidate"
            );
        }

        // The two offline fixtures are catalog candidates AND registered.
        assert!(catalog_pairs.contains(&("fileset", "equity_historical")));
        assert!(catalog_pairs.contains(&("yahoo", "equity_historical")));
        assert!(fetch_table.contains_key(&("fileset", "equity_historical")));
        assert!(fetch_table.contains_key(&("yahoo", "equity_historical")));

        // And the same fixtures are present in the ingest (persist) dispatch
        // table, so a catalog candidate is reachable on both paths.
        let ingest_table = ingest_dispatch_table();
        assert!(ingest_table.contains_key(&("fileset", "equity_historical")));
        assert!(ingest_table.contains_key(&("yahoo", "equity_historical")));
    }

    /// Dispatch table ⊇ catalog: under `all-http-providers`, EVERY `Fetch`
    /// candidate declared in the endpoint catalog must have an ingest binding —
    /// a candidate without one is unreachable dead weight. This is the
    /// full-table conformance check that `xtask catalog-check` deliberately
    /// does not perform (an xtask dep on this crate with `all-http-providers`
    /// would feature-unify every HTTP provider crate into default workspace
    /// builds and lints). CI runs it via
    /// `cargo test -p tdw-service-api --features all-http-providers`.
    #[cfg(feature = "all-http-providers")]
    #[test]
    fn catalog_candidates_all_dispatchable_under_full_providers() {
        use std::collections::BTreeSet;

        let ingest_pairs: BTreeSet<(&str, &str)> = ingest_dispatch_pairs().into_iter().collect();
        for entry in tdw_endpoint_catalog::catalog() {
            if entry.kind != tdw_endpoint_catalog::EndpointKind::Fetch {
                continue;
            }
            for candidate in entry.candidates {
                assert!(
                    ingest_pairs.contains(&(candidate.provider, candidate.endpoint)),
                    "catalog route {} candidate {}/{} has no ingest dispatch binding \
                     under all-http-providers",
                    entry.route,
                    candidate.provider,
                    candidate.endpoint
                );
            }
        }
    }

    /// Catalog ↔ FRED `ENDPOINTS` sync (gap-matrix **L2.3**): every standardized
    /// FRED command has a catalog route whose sole `fred` candidate endpoint is
    /// the route's `'/'→'_'` form, and every FRED-backed catalog route maps back
    /// to an `ENDPOINTS` command — except the two aggregate/discovery routes
    /// (`yield_curve`, `fred_search`) that have no single backing series. Keeps
    /// the hand-written catalog rows and the provider's series table from
    /// drifting without `tdw-endpoint-catalog` depending on the provider.
    #[cfg(feature = "provider-fred")]
    #[test]
    fn fred_catalog_routes_match_provider_endpoints() {
        use std::collections::BTreeSet;

        // The aggregate yield-curve and the metadata search route have no single
        // backing series, so they are FRED-backed but absent from ENDPOINTS.
        const EXTRA_FRED_ROUTES: &[&str] =
            &["fixedincome/government/yield_curve", "economy/fred_search"];

        // Forward: every ENDPOINTS command -> a catalog route with the derived
        // fred candidate endpoint.
        for endpoint in tdw_provider_fred::ENDPOINTS {
            let entry = tdw_endpoint_catalog::lookup(endpoint.command).unwrap_or_else(|| {
                panic!("FRED command {} has no catalog route", endpoint.command)
            });
            let expected = tdw_endpoint_catalog::endpoint_key_for_route(endpoint.command);
            let fred = entry
                .candidates
                .iter()
                .find(|c| c.provider == "fred")
                .unwrap_or_else(|| {
                    panic!("catalog route {} has no fred candidate", endpoint.command)
                });
            assert_eq!(
                fred.endpoint, expected,
                "catalog route {} fred endpoint key drifted",
                endpoint.command
            );
        }

        // Reverse: every FRED-backed catalog route is an ENDPOINTS command,
        // excluding the aggregate yield-curve and the metadata search route.
        let commands: BTreeSet<&str> = tdw_provider_fred::ENDPOINTS
            .iter()
            .map(|e| e.command)
            .collect();
        for entry in tdw_endpoint_catalog::catalog() {
            let is_fred = entry.candidates.iter().any(|c| c.provider == "fred");
            if !is_fred || EXTRA_FRED_ROUTES.contains(&entry.route) {
                continue;
            }
            assert!(
                commands.contains(entry.route),
                "FRED-backed catalog route {} has no ENDPOINTS command",
                entry.route
            );
        }
    }

    /// Catalog ↔ SEC `ENDPOINTS` sync (gap-matrix **L2.6**): every standardized
    /// SEC command (the keyless government wave) has a catalog route whose `sec`
    /// candidate endpoint is registered in both the fetch and ingest dispatch
    /// tables under `provider-sec`. Mirrors `fred_catalog_routes_match_provider_endpoints`.
    #[cfg(feature = "provider-sec")]
    #[test]
    fn sec_catalog_routes_match_provider_endpoints() {
        let fetch_table = fetch_dispatch_table();
        let ingest_table = ingest_dispatch_table();
        for endpoint in tdw_provider_sec::ENDPOINTS {
            let entry = tdw_endpoint_catalog::lookup(endpoint.command)
                .unwrap_or_else(|| panic!("SEC command {} has no catalog route", endpoint.command));
            let sec = entry
                .candidates
                .iter()
                .find(|c| c.provider == "sec")
                .unwrap_or_else(|| {
                    panic!("catalog route {} has no sec candidate", endpoint.command)
                });
            assert!(
                fetch_table.contains_key(&(sec.provider, sec.endpoint)),
                "SEC candidate {}/{} for route {} is not in the fetch dispatch table",
                sec.provider,
                sec.endpoint,
                endpoint.command
            );
            assert!(
                ingest_table.contains_key(&(sec.provider, sec.endpoint)),
                "SEC candidate {}/{} for route {} is not in the ingest dispatch table",
                sec.provider,
                sec.endpoint,
                endpoint.command
            );
        }
    }

    /// Catalog ↔ US-Treasury `ENDPOINTS` sync (gap-matrix **L3.2**): every
    /// standardized FiscalData command has a catalog route whose `government_us`
    /// candidate endpoint is dispatchable in both tables under
    /// `provider-government-us`.
    #[cfg(feature = "provider-government-us")]
    #[test]
    fn government_us_catalog_routes_match_provider_endpoints() {
        let fetch_table = fetch_dispatch_table();
        let ingest_table = ingest_dispatch_table();
        for endpoint in tdw_provider_government_us::ENDPOINTS {
            let entry = tdw_endpoint_catalog::lookup(endpoint.command).unwrap_or_else(|| {
                panic!(
                    "government_us command {} has no catalog route",
                    endpoint.command
                )
            });
            let candidate = entry
                .candidates
                .iter()
                .find(|c| c.provider == "government_us")
                .unwrap_or_else(|| {
                    panic!(
                        "catalog route {} has no government_us candidate",
                        endpoint.command
                    )
                });
            assert!(
                fetch_table.contains_key(&(candidate.provider, candidate.endpoint)),
                "government_us candidate {}/{} for route {} is not in the fetch table",
                candidate.provider,
                candidate.endpoint,
                endpoint.command
            );
            assert!(
                ingest_table.contains_key(&(candidate.provider, candidate.endpoint)),
                "government_us candidate {}/{} for route {} is not in the ingest table",
                candidate.provider,
                candidate.endpoint,
                endpoint.command
            );
        }
    }

    /// Catalog ↔ Federal-Reserve `ENDPOINTS` sync (gap-matrix **L3.1**): every
    /// standardized Fed command has a catalog route whose `federal_reserve`
    /// candidate endpoint is the route's `'/'→'_'` form (the FRED-style derived
    /// key) and is dispatchable in both tables under `provider-federal-reserve`.
    #[cfg(feature = "provider-federal-reserve")]
    #[test]
    fn federal_reserve_catalog_routes_match_provider_endpoints() {
        let fetch_table = fetch_dispatch_table();
        let ingest_table = ingest_dispatch_table();
        for endpoint in tdw_provider_federal_reserve::ENDPOINTS {
            let entry = tdw_endpoint_catalog::lookup(endpoint.command).unwrap_or_else(|| {
                panic!(
                    "federal_reserve command {} has no catalog route",
                    endpoint.command
                )
            });
            let expected = tdw_endpoint_catalog::endpoint_key_for_route(endpoint.command);
            let candidate = entry
                .candidates
                .iter()
                .find(|c| c.provider == "federal_reserve")
                .unwrap_or_else(|| {
                    panic!(
                        "catalog route {} has no federal_reserve candidate",
                        endpoint.command
                    )
                });
            assert_eq!(
                candidate.endpoint, expected,
                "catalog route {} federal_reserve endpoint key drifted",
                endpoint.command
            );
            assert!(
                fetch_table.contains_key(&(candidate.provider, candidate.endpoint)),
                "federal_reserve candidate {}/{} for route {} is not in the fetch table",
                candidate.provider,
                candidate.endpoint,
                endpoint.command
            );
            assert!(
                ingest_table.contains_key(&(candidate.provider, candidate.endpoint)),
                "federal_reserve candidate {}/{} for route {} is not in the ingest table",
                candidate.provider,
                candidate.endpoint,
                endpoint.command
            );
        }
    }

    /// Catalog <-> Yahoo expansion sync (gap-matrix L2.4): every Yahoo-backed
    /// catalog candidate has a fetch and an ingest dispatch binding under the
    /// `provider-yahoo-http` build, and each binding key matches the candidate's
    /// declared endpoint. Pins the catalog rows, the dispatch tables, and the
    /// registered Yahoo fetchers to one set of endpoint keys.
    #[cfg(feature = "provider-yahoo-http")]
    #[test]
    fn yahoo_catalog_candidates_are_dispatchable() {
        use std::collections::BTreeSet;

        // The nine Yahoo expansion endpoints projected by L2.4; the always-on
        // `equity_historical` fixture endpoint is covered by the equity route
        // tests and excluded here.
        const YAHOO_EXPANSION_ENDPOINTS: &[&str] = &[
            "equity_profile",
            "equity_quote",
            "price_performance",
            "dividends",
            "share_statistics",
            "analyst_consensus",
            "options_chains",
            "futures_historical",
            "futures_curve",
        ];

        let fetch_keys: BTreeSet<(&str, &str)> = fetch_dispatch_table().into_keys().collect();
        let ingest_keys: BTreeSet<(&str, &str)> = ingest_dispatch_pairs().into_iter().collect();

        let mut covered = BTreeSet::new();
        for entry in tdw_endpoint_catalog::catalog() {
            for candidate in entry.candidates {
                if candidate.provider != "yahoo"
                    || !YAHOO_EXPANSION_ENDPOINTS.contains(&candidate.endpoint)
                {
                    continue;
                }
                let key = (candidate.provider, candidate.endpoint);
                assert!(
                    fetch_keys.contains(&key),
                    "catalog route {} yahoo candidate {} has no fetch binding",
                    entry.route,
                    candidate.endpoint
                );
                assert!(
                    ingest_keys.contains(&key),
                    "catalog route {} yahoo candidate {} has no ingest binding",
                    entry.route,
                    candidate.endpoint
                );
                covered.insert(candidate.endpoint);
            }
        }

        for endpoint in YAHOO_EXPANSION_ENDPOINTS {
            assert!(
                covered.contains(endpoint),
                "yahoo expansion endpoint {endpoint} is not referenced by any catalog route"
            );
        }
    }

    /// Catalog <-> ECB/CBOE/EIA/NASDAQ projection sync (G004 part 2): every
    /// candidate these four providers contribute to a catalog route has a fetch
    /// and an ingest dispatch binding under the all-providers build, and each
    /// binding key matches the candidate's declared endpoint. Pins the new
    /// catalog rows, the dispatch tables, and the registered fetchers to one set
    /// of endpoint keys. The expected endpoint set is asserted to be fully
    /// covered so a dropped route is caught.
    #[cfg(all(
        feature = "provider-ecb",
        feature = "provider-cboe",
        feature = "provider-eia",
        feature = "provider-nasdaq",
    ))]
    #[test]
    fn g004_part2_catalog_candidates_are_dispatchable() {
        use std::collections::BTreeSet;

        // The endpoints projected by G004 part 2, by provider.
        const EXPECTED: &[(&str, &str)] = &[
            ("ecb", "reference_rates"),
            ("cboe", "index_snapshots"),
            ("cboe", "options_chains"),
            ("eia", "commodity_petroleum_status_report"),
            ("eia", "commodity_short_term_energy_outlook"),
            ("nasdaq", "equity_calendar_dividends"),
            ("nasdaq", "equity_calendar_earnings"),
            ("nasdaq", "equity_calendar_ipo"),
        ];
        const PROVIDERS: &[&str] = &["ecb", "cboe", "eia", "nasdaq"];

        let fetch_keys: BTreeSet<(&str, &str)> = fetch_dispatch_table().into_keys().collect();
        let ingest_keys: BTreeSet<(&str, &str)> = ingest_dispatch_pairs().into_iter().collect();

        let mut covered = BTreeSet::new();
        for entry in tdw_endpoint_catalog::catalog() {
            for candidate in entry.candidates {
                if !PROVIDERS.contains(&candidate.provider) {
                    continue;
                }
                let key = (candidate.provider, candidate.endpoint);
                // Only the part-2 endpoints are asserted; skip pre-existing
                // candidates these providers may contribute elsewhere.
                if !EXPECTED.contains(&key) {
                    continue;
                }
                assert!(
                    fetch_keys.contains(&key),
                    "catalog route {} candidate {}/{} has no fetch binding",
                    entry.route,
                    candidate.provider,
                    candidate.endpoint
                );
                assert!(
                    ingest_keys.contains(&key),
                    "catalog route {} candidate {}/{} has no ingest binding",
                    entry.route,
                    candidate.provider,
                    candidate.endpoint
                );
                covered.insert(key);
            }
        }

        for key in EXPECTED {
            assert!(
                covered.contains(key),
                "G004 part 2 endpoint {}/{} is not referenced by any catalog route",
                key.0,
                key.1
            );
        }
    }

    #[tokio::test]
    async fn resolve_and_fetch_falls_back_and_warns_on_retryable_error() {
        // Fallback fixture: register a failing earlier candidate (`fmp`, a
        // retryable provider error) and a working later one (`akshare` stub) in
        // an injected fetch table, then resolve `equity/price/historical` with no
        // explicit provider. The dispatch must skip the failing provider, land on
        // the working one, and append a `provider_fallback` warning.
        use tdw_provider_testkit::{FailingEquityHistoricalFetcher, StubEquityHistoricalFetcher};

        let entry =
            tdw_endpoint_catalog::lookup("equity/price/historical").expect("equity route present");

        let mut table: BTreeMap<(&'static str, &'static str), FetchBinding> = BTreeMap::new();
        table.insert(
            ("fmp", "equity_historical"),
            fetch_binding::<FailingEquityHistoricalFetcher, _, _>(),
        );
        table.insert(
            ("akshare", "hist"),
            fetch_binding::<StubEquityHistoricalFetcher, _, _>(),
        );

        let runner = CommandRunner::new(ProviderRegistry::default());
        let outcome = resolve_and_fetch(&entry, "", &json!({ "symbol": "AAPL" }), &table, &runner)
            .await
            .expect("fallback should land on the working candidate");

        // fmp precedes akshare in declaration order, so fmp is tried first
        // (fails, retryable) and akshare serves the result.
        assert_eq!(outcome.provider, "akshare");
        assert_eq!(outcome.records.len(), 1);
        assert_eq!(outcome.warnings.len(), 1);
        assert_eq!(outcome.warnings[0].category, "provider_fallback");
        assert!(
            outcome.warnings[0].message.contains("fmp")
                && outcome.warnings[0].message.contains("akshare"),
            "warning must name failed + chosen provider, got: {}",
            outcome.warnings[0].message
        );
    }

    #[tokio::test]
    async fn resolve_and_fetch_fails_fast_on_validation_error_without_fallback() {
        // A validation error (missing symbol -> Error::InvalidQuery from the
        // failing fetcher's transform_query) must NOT trigger fallback: the same
        // bad params would fail every candidate.
        use tdw_provider_testkit::{FailingEquityHistoricalFetcher, StubEquityHistoricalFetcher};

        let entry =
            tdw_endpoint_catalog::lookup("equity/price/historical").expect("equity route present");

        let mut table: BTreeMap<(&'static str, &'static str), FetchBinding> = BTreeMap::new();
        table.insert(
            ("fmp", "equity_historical"),
            fetch_binding::<FailingEquityHistoricalFetcher, _, _>(),
        );
        table.insert(
            ("akshare", "hist"),
            fetch_binding::<StubEquityHistoricalFetcher, _, _>(),
        );

        let runner = CommandRunner::new(ProviderRegistry::default());
        // No `symbol` -> transform_query returns Error::InvalidQuery (fail-fast).
        let error = resolve_and_fetch(&entry, "", &json!({}), &table, &runner)
            .await
            .expect_err("a validation error must fail fast, not fall back");
        assert!(
            matches!(error, Error::InvalidQuery(_)),
            "validation error must surface unchanged, got: {error:?}"
        );
    }

    #[test]
    fn tool_registry_routes_known_and_rejects_unknown() {
        let registry = service_tool_registry();
        let router = ToolRouter::new(registry.clone());
        // Known tool routes: the always-on `udf.run` and the catalog-driven
        // `technical.*` Compute tools.
        assert!(router.route("udf.run").is_ok());
        assert!(router.route("technical.rsi").is_ok());
        // Unknown tool is rejected by the router.
        assert!(router.route("does.not.exist").is_err());
        // The available-names helper lists the build's tools (sorted) for the
        // error; `udf.run` and every `technical.*` tool appear.
        let names = available_tool_names(&registry);
        assert!(names.contains("udf.run"), "got: {names}");
        assert!(names.contains("technical.sma"), "got: {names}");
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
                assert!(error.contains("udf.run"), "got: {error}");
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

    #[tokio::test]
    async fn dispatch_tool_unsupported_tool_returns_provider_error() {
        // The unsupported-tool branch runs only after the line-282 enforcement
        // precondition succeeds, so the reused analyst_policy() (which passes
        // enforcement) is required for the match to be reached at all.
        let policy = udf_runner_policy();
        let state = AppState::in_memory_for_tests()
            .await
            .with_policy(policy.clone());
        let error = dispatch_tool(&state, &policy, "does.not.exist", &json!({}))
            .await
            .expect_err("an unknown tool name must be rejected");

        match error {
            Error::Provider(message) => {
                assert!(
                    message.contains("unsupported tool: does.not.exist"),
                    "expected the unsupported-tool contract, got: {message}"
                );
            }
            other => panic!("expected Error::Provider, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn dispatch_tool_udf_run_invalid_arguments_returns_provider_error() {
        // The udf.run arm is entered (enforcement passes via analyst_policy),
        // then serde_json::from_value fails on a payload missing UdfRequest's
        // required fields, hitting the documented invalid-arguments contract.
        let policy = udf_runner_policy();
        let state = AppState::in_memory_for_tests()
            .await
            .with_policy(policy.clone());
        let error = dispatch_tool(&state, &policy, "udf.run", &json!({}))
            .await
            .expect_err("a malformed udf.run payload must be rejected");

        match error {
            Error::Provider(message) => {
                assert!(
                    message.contains("invalid udf.run arguments"),
                    "expected the invalid-arguments contract, got: {message}"
                );
            }
            other => panic!("expected Error::Provider, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn dispatch_tool_udf_run_happy_path_returns_runtime_and_output_envelope() {
        // A minimal request the LocalUdfSandbox accepts under default features:
        // the `upper` builtin uppercases its input. This drives the full udf.run
        // arm body — UdfRequest deserialization, sandbox.run, and the masked
        // response envelope (evidence/runtime/output).
        let policy = udf_runner_policy();
        let arguments = json!({
            "name": "upper",
            "runtime": "JavaScript",
            "source": "builtin",
            "input": "abc",
            "allow_network": false,
            "allow_filesystem": false,
        });

        let state = AppState::in_memory_for_tests()
            .await
            .with_policy(policy.clone());
        let value = dispatch_tool(&state, &policy, "udf.run", &arguments)
            .await
            .unwrap_or_else(|error| panic!("udf.run happy path should succeed: {error}"));

        // The masked envelope carries the principal-scoped evidence plus the
        // sandbox response. Empty mask_rules leave the payload unchanged.
        assert_eq!(value["evidence"]["principal"], "alice");
        assert_eq!(value["evidence"]["endpoint"], "tdw.udf.run");
        assert_eq!(value["runtime"], "JavaScript");
        assert_eq!(value["output"], "ABC");
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

    // -----------------------------------------------------------------------
    // Alert CRUD dispatcher tests (require `alerts` feature)
    // -----------------------------------------------------------------------

    /// Build a policy whose principal is `owner`.
    #[cfg(feature = "alerts")]
    fn alert_policy(owner: &str) -> PolicyEnforcementConfig {
        PolicyEnforcementConfig {
            auth: IngressAuthContext {
                claims: JwtClaims {
                    sub: owner.to_string(),
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

    /// `AppState` pre-wired with a fresh `InMemoryAlertStore`.
    #[cfg(feature = "alerts")]
    async fn alert_state(owner: &str) -> AppState {
        AppState::in_memory_for_tests()
            .await
            .with_policy(alert_policy(owner))
    }

    /// CreateAlert returns a persisted alert whose owner_id equals the
    /// verified principal — never the content of any request field.
    #[cfg(feature = "alerts")]
    #[tokio::test]
    async fn create_alert_derives_owner_from_principal() {
        let state = alert_state("alice").await;
        let env = make_envelope(Op::CreateAlert {
            symbol: "AAPL".to_string(),
            target_price: "200.00".to_string(),
            condition: "Above".to_string(),
        });
        let op_id = env.op_id.clone();
        let events = dispatch_op(&state, env).await;

        assert_eq!(events.len(), 2);
        match &events[0] {
            EventMsg::Started { op_id: sid } => assert_eq!(sid, &op_id),
            other => panic!("expected Started, got {other:?}"),
        }
        match &events[1] {
            EventMsg::Completed {
                result: Some(value),
                ..
            } => {
                // owner_id must equal the JWT sub, never a client-supplied value
                assert_eq!(value["alert"]["owner_id"], "alice");
                assert_eq!(value["alert"]["symbol"], "AAPL");
                assert_eq!(value["alert"]["target_price"], 200.0_f64);
                assert_eq!(value["alert"]["active"], true);
                assert_eq!(value["alert"]["triggered"], false);
                assert_eq!(value["evidence"]["principal"], "alice");
                assert_eq!(value["evidence"]["endpoint"], "tdw.alert.manage");
            }
            other => panic!("expected Completed, got {other:?}"),
        }
    }

    /// ListAlerts returns only the principal's own alerts (owner-scoped).
    #[cfg(feature = "alerts")]
    #[tokio::test]
    async fn list_alerts_is_owner_scoped() {
        let state_alice = alert_state("alice").await;

        // Alice creates two alerts.
        for symbol in ["AAPL", "MSFT"] {
            let env = make_envelope(Op::CreateAlert {
                symbol: symbol.to_string(),
                target_price: "100.00".to_string(),
                condition: "Above".to_string(),
            });
            let events = dispatch_op(&state_alice, env).await;
            assert!(
                matches!(&events[1], EventMsg::Completed { .. }),
                "create should succeed for {symbol}"
            );
        }

        // Bob uses a different policy (different principal) but the SAME
        // in-memory store (via Arc clone so the data is shared).
        let mut state_bob = state_alice.clone();
        state_bob.policy = Some(alert_policy("bob"));

        // Alice sees her 2 alerts.
        let env_alice = make_envelope(Op::ListAlerts {});
        let events_alice = dispatch_op(&state_alice, env_alice).await;
        match &events_alice[1] {
            EventMsg::Completed {
                result: Some(value),
                ..
            } => {
                assert_eq!(value["count"], 2, "alice should see 2 alerts");
                let alerts = value["alerts"].as_array().expect("alerts array");
                assert!(
                    alerts.iter().all(|a| a["owner_id"] == "alice"),
                    "all returned alerts must belong to alice"
                );
            }
            other => panic!("expected Completed for alice list, got {other:?}"),
        }

        // Bob sees 0 alerts (he hasn't created any).
        let env_bob = make_envelope(Op::ListAlerts {});
        let events_bob = dispatch_op(&state_bob, env_bob).await;
        match &events_bob[1] {
            EventMsg::Completed {
                result: Some(value),
                ..
            } => {
                assert_eq!(value["count"], 0, "bob should see 0 alerts");
            }
            other => panic!("expected Completed for bob list, got {other:?}"),
        }
    }

    /// DeleteAlert by another owner is refused with a non-leaking error.
    #[cfg(feature = "alerts")]
    #[tokio::test]
    async fn delete_alert_cross_owner_is_refused() {
        let state_alice = alert_state("alice").await;

        // Alice creates an alert; capture the id from the response.
        let env = make_envelope(Op::CreateAlert {
            symbol: "BTC".to_string(),
            target_price: "50000.00".to_string(),
            condition: "Above".to_string(),
        });
        let events = dispatch_op(&state_alice, env).await;
        let alert_id = match &events[1] {
            EventMsg::Completed {
                result: Some(value),
                ..
            } => value["alert"]["id"]
                .as_str()
                .expect("id should be a string")
                .to_string(),
            other => panic!("create should succeed, got {other:?}"),
        };

        // Bob tries to delete Alice's alert — must be refused.
        let mut state_bob = state_alice.clone();
        state_bob.policy = Some(alert_policy("bob"));
        let env_bob = make_envelope(Op::DeleteAlert {
            id: alert_id.clone(),
        });
        let events_bob = dispatch_op(&state_bob, env_bob).await;
        match &events_bob[1] {
            EventMsg::Failed { error, .. } => {
                assert!(
                    error.contains("not found or not owned"),
                    "error must not leak existence, got: {error}"
                );
            }
            other => panic!("expected Failed for cross-owner delete, got {other:?}"),
        }

        // Alice can still see the alert (it was NOT deleted).
        let list_env = make_envelope(Op::ListAlerts {});
        let list_events = dispatch_op(&state_alice, list_env).await;
        match &list_events[1] {
            EventMsg::Completed {
                result: Some(value),
                ..
            } => assert_eq!(value["count"], 1, "alice's alert must still exist"),
            other => panic!("expected Completed for alice list, got {other:?}"),
        }
    }

    /// DeleteAlert by the owning principal succeeds.
    #[cfg(feature = "alerts")]
    #[tokio::test]
    async fn delete_alert_own_succeeds() {
        let state = alert_state("alice").await;

        let env = make_envelope(Op::CreateAlert {
            symbol: "ETH".to_string(),
            target_price: "2000.00".to_string(),
            condition: "Below".to_string(),
        });
        let events = dispatch_op(&state, env).await;
        let alert_id = match &events[1] {
            EventMsg::Completed {
                result: Some(value),
                ..
            } => value["alert"]["id"].as_str().expect("id").to_string(),
            other => panic!("create should succeed, got {other:?}"),
        };

        let del_env = make_envelope(Op::DeleteAlert { id: alert_id });
        let del_events = dispatch_op(&state, del_env).await;
        match &del_events[1] {
            EventMsg::Completed {
                result: Some(value),
                ..
            } => {
                assert_eq!(value["deleted"], true);
            }
            other => panic!("expected Completed for own delete, got {other:?}"),
        }

        // Confirm it is gone.
        let list_env = make_envelope(Op::ListAlerts {});
        let list_events = dispatch_op(&state, list_env).await;
        match &list_events[1] {
            EventMsg::Completed {
                result: Some(value),
                ..
            } => assert_eq!(value["count"], 0, "alert should be gone after delete"),
            other => panic!("expected Completed for list-after-delete, got {other:?}"),
        }
    }

    /// SetAlertActive by another owner is refused (same non-leaking error as delete).
    #[cfg(feature = "alerts")]
    #[tokio::test]
    async fn set_alert_active_cross_owner_is_refused() {
        let state_alice = alert_state("alice").await;

        let env = make_envelope(Op::CreateAlert {
            symbol: "SOL".to_string(),
            target_price: "150.00".to_string(),
            condition: "Above".to_string(),
        });
        let events = dispatch_op(&state_alice, env).await;
        let alert_id = match &events[1] {
            EventMsg::Completed {
                result: Some(value),
                ..
            } => value["alert"]["id"].as_str().expect("id").to_string(),
            other => panic!("create should succeed, got {other:?}"),
        };

        let mut state_bob = state_alice.clone();
        state_bob.policy = Some(alert_policy("bob"));

        let env_bob = make_envelope(Op::SetAlertActive {
            id: alert_id,
            active: false,
        });
        let events_bob = dispatch_op(&state_bob, env_bob).await;
        match &events_bob[1] {
            EventMsg::Failed { error, .. } => {
                assert!(
                    error.contains("not found or not owned"),
                    "error must not leak existence, got: {error}"
                );
            }
            other => panic!("expected Failed for cross-owner toggle, got {other:?}"),
        }
    }

    /// SetAlertActive by the owning principal succeeds and toggles the flag.
    #[cfg(feature = "alerts")]
    #[tokio::test]
    async fn set_alert_active_own_succeeds() {
        let state = alert_state("alice").await;

        let env = make_envelope(Op::CreateAlert {
            symbol: "DOGE".to_string(),
            target_price: "0.10".to_string(),
            condition: "Above".to_string(),
        });
        let events = dispatch_op(&state, env).await;
        let alert_id = match &events[1] {
            EventMsg::Completed {
                result: Some(value),
                ..
            } => value["alert"]["id"].as_str().expect("id").to_string(),
            other => panic!("create should succeed, got {other:?}"),
        };

        // Disable the alert.
        let env_off = make_envelope(Op::SetAlertActive {
            id: alert_id.clone(),
            active: false,
        });
        let events_off = dispatch_op(&state, env_off).await;
        match &events_off[1] {
            EventMsg::Completed {
                result: Some(value),
                ..
            } => assert_eq!(value["active"], false),
            other => panic!("expected Completed for set_active=false, got {other:?}"),
        }

        // Re-enable the alert.
        let env_on = make_envelope(Op::SetAlertActive {
            id: alert_id,
            active: true,
        });
        let events_on = dispatch_op(&state, env_on).await;
        match &events_on[1] {
            EventMsg::Completed {
                result: Some(value),
                ..
            } => assert_eq!(value["active"], true),
            other => panic!("expected Completed for set_active=true, got {other:?}"),
        }
    }

    /// CreateAlert rejects a non-numeric target_price string.
    #[cfg(feature = "alerts")]
    #[tokio::test]
    async fn create_alert_rejects_non_numeric_target_price() {
        let state = alert_state("alice").await;
        let env = make_envelope(Op::CreateAlert {
            symbol: "AAPL".to_string(),
            target_price: "not-a-number".to_string(),
            condition: "Above".to_string(),
        });
        let events = dispatch_op(&state, env).await;
        match &events[1] {
            EventMsg::Failed { error, .. } => {
                assert!(
                    error.contains("target_price must be a decimal number"),
                    "got: {error}"
                );
            }
            other => panic!("expected Failed for bad price, got {other:?}"),
        }
    }

    /// CreateAlert rejects a non-positive target_price.
    #[cfg(feature = "alerts")]
    #[tokio::test]
    async fn create_alert_rejects_non_positive_target_price() {
        let state = alert_state("alice").await;
        let env = make_envelope(Op::CreateAlert {
            symbol: "AAPL".to_string(),
            target_price: "-1.00".to_string(),
            condition: "Above".to_string(),
        });
        let events = dispatch_op(&state, env).await;
        match &events[1] {
            EventMsg::Failed { error, .. } => {
                assert!(error.contains("finite positive number"), "got: {error}");
            }
            other => panic!("expected Failed for negative price, got {other:?}"),
        }
    }

    /// CreateAlert rejects an invalid condition string.
    #[cfg(feature = "alerts")]
    #[tokio::test]
    async fn create_alert_rejects_invalid_condition() {
        let state = alert_state("alice").await;
        let env = make_envelope(Op::CreateAlert {
            symbol: "AAPL".to_string(),
            target_price: "100.00".to_string(),
            condition: "sideways".to_string(),
        });
        let events = dispatch_op(&state, env).await;
        match &events[1] {
            EventMsg::Failed { error, .. } => {
                assert!(error.contains("\"Above\" or \"Below\""), "got: {error}");
            }
            other => panic!("expected Failed for bad condition, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // User-registration dispatcher tests (require `identity` feature)
    // -----------------------------------------------------------------------

    /// `AppState` pre-wired with a fresh `InMemoryUserStore` and an analyst
    /// policy (the gate `UserRegister` reuses for this slice).
    #[cfg(feature = "identity")]
    async fn identity_state() -> AppState {
        AppState::in_memory_for_tests()
            .await
            .with_policy(analyst_policy())
    }

    /// Happy path: a valid registration returns the persisted user (with no
    /// `password`/`password_hash` field leaked) AND emits a `user.created`
    /// envelope onto the outbox carrying the right non-secret payload.
    #[cfg(feature = "identity")]
    #[tokio::test]
    async fn register_user_persists_and_emits_user_created_event() {
        use crate::user_events::USER_CREATED_EVENT_TYPE;

        let state = identity_state().await;
        let env = make_envelope(Op::RegisterUser {
            id: "user-1".to_string(),
            email: "  Alice@Example.com ".to_string(),
            password: "correct horse battery".to_string(),
            display_name: "Alice".to_string(),
            now_ms: 1_700_000_000_000,
        });
        let events = dispatch_op(&state, env).await;

        assert_eq!(events.len(), 2);
        match &events[1] {
            EventMsg::Completed {
                result: Some(value),
                ..
            } => {
                assert_eq!(value["evidence"]["principal"], "alice");
                assert_eq!(value["evidence"]["endpoint"], "tdw.user.register");
                // Persisted user is returned with the normalized email.
                assert_eq!(value["user"]["id"], "user-1");
                assert_eq!(value["user"]["email"], "alice@example.com");
                assert_eq!(value["user"]["display_name"], "Alice");
                assert_eq!(value["user"]["created_at_ms"], 1_700_000_000_000_i64);
                assert_eq!(value["event_type"], USER_CREATED_EVENT_TYPE);
                // No secret material ever surfaces.
                assert!(
                    value["user"].get("password").is_none(),
                    "password must never be returned, got: {value}"
                );
                assert!(
                    value["user"].get("password_hash").is_none(),
                    "password_hash must never be returned, got: {value}"
                );
            }
            other => panic!("expected Completed, got {other:?}"),
        }

        // A `user.created` envelope with the non-secret payload was relayed to
        // the outbox (the same path the EventSink uses).
        let pending = state
            .outbox
            .lock()
            .unwrap_or_else(|e| panic!("outbox lock: {e}"))
            .pending_after(0);
        let created = pending
            .iter()
            .find(|record| record.envelope.event_type == USER_CREATED_EVENT_TYPE)
            .unwrap_or_else(|| panic!("a user.created envelope must be emitted; got {pending:?}"));
        let payload = &created.envelope.payload;
        assert_eq!(payload["user_id"], "user-1");
        assert_eq!(payload["email"], "alice@example.com");
        assert_eq!(payload["created_at_ms"], 1_700_000_000_000_i64);
        // The event payload must not carry secret material either.
        assert!(payload.get("password").is_none());
        assert!(payload.get("password_hash").is_none());
    }

    /// A second registration with the same (normalized) email is rejected with
    /// the duplicate-email contract (mapped from `IdentityError::EmailTaken`).
    #[cfg(feature = "identity")]
    #[tokio::test]
    async fn register_user_duplicate_email_fails() {
        let state = identity_state().await;
        let first = make_envelope(Op::RegisterUser {
            id: "user-1".to_string(),
            email: "dup@example.com".to_string(),
            password: "correct horse battery".to_string(),
            display_name: "First".to_string(),
            now_ms: 1,
        });
        let events = dispatch_op(&state, first).await;
        assert!(matches!(&events[1], EventMsg::Completed { .. }));

        let second = make_envelope(Op::RegisterUser {
            id: "user-2".to_string(),
            email: "DUP@example.com".to_string(),
            password: "another good password".to_string(),
            display_name: "Second".to_string(),
            now_ms: 2,
        });
        let events = dispatch_op(&state, second).await;
        match &events[1] {
            EventMsg::Failed { error, .. } => {
                assert!(error.contains("email already registered"), "got: {error}");
            }
            other => panic!("expected Failed for duplicate email, got {other:?}"),
        }
    }

    /// A password below the length policy is rejected with the weak-password
    /// contract (mapped from `IdentityError::WeakPassword`).
    #[cfg(feature = "identity")]
    #[tokio::test]
    async fn register_user_weak_password_fails() {
        let state = identity_state().await;
        let env = make_envelope(Op::RegisterUser {
            id: "user-weak".to_string(),
            email: "weak@example.com".to_string(),
            password: "short".to_string(),
            display_name: "Weak".to_string(),
            now_ms: 1,
        });
        let events = dispatch_op(&state, env).await;
        match &events[1] {
            EventMsg::Failed { error, .. } => {
                assert!(error.contains("weak password"), "got: {error}");
            }
            other => panic!("expected Failed for weak password, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // FunctionEnqueuer hook tests (require `functions` feature)
    // -----------------------------------------------------------------------

    #[cfg(feature = "functions")]
    mod function_enqueue_tests {
        use std::sync::{Arc, Mutex};

        use serde_json::Value;
        use tdw_protocol::{EventMsg, Op};

        use crate::AppState;
        use crate::function_enqueue::FunctionEnqueuer;

        use super::{analyst_policy, make_envelope};

        /// Mock enqueuer that records every `enqueue_for_event` call.
        #[derive(Default)]
        struct RecordingEnqueuer {
            calls: Mutex<Vec<(String, i64)>>,
        }

        impl FunctionEnqueuer for RecordingEnqueuer {
            fn enqueue_for_event(
                &self,
                event_type: &str,
                _payload: Value,
                _run_id_seed: &str,
                now_ms: i64,
            ) -> Result<usize, String> {
                self.calls
                    .lock()
                    .expect("lock")
                    .push((event_type.to_string(), now_ms));
                Ok(1)
            }
        }

        /// Mock enqueuer that always fails.
        struct FailingEnqueuer;

        impl FunctionEnqueuer for FailingEnqueuer {
            fn enqueue_for_event(
                &self,
                _event_type: &str,
                _payload: Value,
                _run_id_seed: &str,
                _now_ms: i64,
            ) -> Result<usize, String> {
                Err("queue unavailable".to_string())
            }
        }

        /// Build an `AppState` with a wired `FunctionEnqueuer` and analyst policy.
        async fn functions_state(enqueuer: Arc<dyn FunctionEnqueuer>) -> AppState {
            let mut state = AppState::in_memory_for_tests()
                .await
                .with_policy(analyst_policy());
            state.function_enqueuer = Some(enqueuer);
            state
        }

        /// Happy path: a valid registration triggers the enqueuer with
        /// `user.created` and the correct `now_ms`.
        #[tokio::test]
        async fn register_user_triggers_enqueuer() {
            use crate::dispatch_op;

            let recorder = Arc::new(RecordingEnqueuer::default());
            let state = functions_state(Arc::clone(&recorder) as Arc<dyn FunctionEnqueuer>).await;

            let env = make_envelope(Op::RegisterUser {
                id: "fn-user-1".to_string(),
                email: "fn@example.com".to_string(),
                password: "correct horse battery".to_string(),
                display_name: "FnUser".to_string(),
                now_ms: 42_000,
            });
            let events = dispatch_op(&state, env).await;

            // Registration must succeed.
            assert!(
                matches!(&events[1], EventMsg::Completed { .. }),
                "expected Completed, got {:?}",
                events[1]
            );

            // Enqueuer must have been called exactly once with user.created.
            let calls = recorder.calls.lock().expect("lock");
            assert_eq!(
                calls.len(),
                1,
                "expected 1 enqueue call, got {}",
                calls.len()
            );
            assert_eq!(calls[0].0, "user.created");
            assert_eq!(calls[0].1, 42_000);
        }

        /// Enqueue failure must NOT cause the registration to return `Failed` —
        /// the hook is strictly warn-and-continue.
        #[tokio::test]
        async fn register_user_enqueue_error_does_not_fail_dispatch() {
            use crate::dispatch_op;

            let state =
                functions_state(Arc::new(FailingEnqueuer) as Arc<dyn FunctionEnqueuer>).await;

            let env = make_envelope(Op::RegisterUser {
                id: "fn-user-fail".to_string(),
                email: "fail-enqueue@example.com".to_string(),
                password: "correct horse battery".to_string(),
                display_name: "FailUser".to_string(),
                now_ms: 1,
            });
            let events = dispatch_op(&state, env).await;

            // Despite the enqueue error, the dispatch must still succeed.
            assert!(
                matches!(&events[1], EventMsg::Completed { .. }),
                "expected Completed even with enqueue error, got {:?}",
                events[1]
            );
        }

        /// When `function_enqueuer` is `None` (default), registration succeeds
        /// and no enqueue path is exercised.
        #[tokio::test]
        async fn register_user_no_enqueuer_succeeds() {
            use crate::dispatch_op;

            let state = AppState::in_memory_for_tests()
                .await
                .with_policy(analyst_policy());
            // function_enqueuer is None by default.

            let env = make_envelope(Op::RegisterUser {
                id: "fn-user-none".to_string(),
                email: "none@example.com".to_string(),
                password: "correct horse battery".to_string(),
                display_name: "NoneUser".to_string(),
                now_ms: 1,
            });
            let events = dispatch_op(&state, env).await;
            assert!(
                matches!(&events[1], EventMsg::Completed { .. }),
                "expected Completed without enqueuer, got {:?}",
                events[1]
            );
        }
    }
}
