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
use tdw_domain::QuoteSnapshot;
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
#[allow(unused_variables)] // `symbol` is used only inside the cfg-gated provider block
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
        feature = "provider-finnhub",
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
        let state = AppState::in_memory_for_tests()
            .await
            .with_policy(analyst_policy());
        let env = make_envelope(Op::IngestBatch {
            provider: "yahoo".to_string(),
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
                assert_eq!(value["provider"], "yahoo");
                assert_eq!(value["endpoint"], "equity_historical");
            }
            other => panic!("expected Completed, got {other:?}"),
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
        // directly, exactly as before the resolution layer existed.
        let state = AppState::in_memory_for_tests()
            .await
            .with_policy(analyst_policy());
        let env = make_envelope(Op::IngestBatch {
            provider: "yahoo".to_string(),
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
                assert_eq!(value["provider"], "yahoo");
                assert_eq!(value["endpoint"], "equity_historical");
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

    #[test]
    fn dispatch_tool_unsupported_tool_returns_provider_error() {
        // The unsupported-tool branch runs only after the line-282 enforcement
        // precondition succeeds, so the reused analyst_policy() (which passes
        // enforcement) is required for the match to be reached at all.
        let policy = udf_runner_policy();
        let error = dispatch_tool(&policy, "does.not.exist", &json!({}))
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

    #[test]
    fn dispatch_tool_udf_run_invalid_arguments_returns_provider_error() {
        // The udf.run arm is entered (enforcement passes via analyst_policy),
        // then serde_json::from_value fails on a payload missing UdfRequest's
        // required fields, hitting the documented invalid-arguments contract.
        let policy = udf_runner_policy();
        let error = dispatch_tool(&policy, "udf.run", &json!({}))
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

    #[test]
    fn dispatch_tool_udf_run_happy_path_returns_runtime_and_output_envelope() {
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

        let value = dispatch_tool(&policy, "udf.run", &arguments)
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
}
