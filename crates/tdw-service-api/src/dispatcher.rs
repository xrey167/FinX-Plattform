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

use async_trait::async_trait;
use serde_json::{Value, json};
use tdw_app_server::Dispatcher;
use tdw_core::{Error, Result};
use tdw_hooks::SystemHookHandlerBackend;
use tdw_protocol::{EventMsg, Op, OpEnvelope, TimeRange};
use tdw_provider_fileset::FilesetEquityHistoricalFetcher;
use tdw_provider_yahoo::YahooEquityHistoricalFetcher;
use tdw_runtime::CommandRunner;
use tdw_sandbox::{LocalUdfSandbox, SandboxRuntime, UdfRequest};

use crate::{
    AppState, PolicyEnforcementConfig, ServiceEndpoint, enforce_request_path_with_backend,
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
        } => dispatch_tool(policy, tool_name, arguments).await,
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

    // Bronze landing table keyed by (provider, endpoint). Provider rows are
    // written verbatim via JSONEachRow; normalization to `raw.market_data_bar`
    // is a downstream (silver) concern.
    let table = match (provider, endpoint) {
        ("fileset" | "yahoo", "equity_historical") => "raw.equity_historical",
        (provider, endpoint) => {
            return Err(Error::Provider(format!(
                "unsupported ingest provider/endpoint: {provider}/{endpoint}"
            )));
        }
    };

    let runner = CommandRunner::new((*state.registry).clone());
    let mut per_symbol = Vec::with_capacity(symbols.len());
    let mut total_rows = 0usize;

    for symbol in symbols {
        let mut params = json!({ "symbol": symbol });
        if let Some(range) = range {
            params["range"] = json!({ "start": range.start, "end": range.end });
        }
        let object = match provider {
            "fileset" => runner.run(&FilesetEquityHistoricalFetcher, params).await?,
            "yahoo" => runner.run(&YahooEquityHistoricalFetcher, params).await?,
            _ => unreachable!("provider/endpoint validated above"),
        };
        // Per-(op, symbol) dedup token: stable across retries of the same op
        // (same session_id + sequence) yet distinct per symbol, so a multi-symbol
        // op does not dedup later symbols' blocks against the first.
        let token = tdw_storage_clickhouse::ingest_dedup_token(
            env.session_id.as_str(),
            env.sequence,
            &format!("{table}:{symbol}"),
        );
        let rows = persist_batch(state, table, &token, &object).await?;
        total_rows += rows;
        per_symbol.push(json!({ "symbol": symbol, "rows": rows, "dedup_token": token }));
    }

    Ok(mask_json_response(
        json!({
            "evidence": evidence,
            "provider": provider,
            "endpoint": endpoint,
            "table": table,
            "rows": total_rows,
            "symbols": per_symbol,
        }),
        &policy.mask_rules,
    ))
}

/// Persist a fetched batch as an idempotent `INSERT … FORMAT JSONEachRow` and
/// return the row count. The caller supplies the deduplication token (so a
/// client retry of the same op is dropped by ClickHouse rather than
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
    policy: &PolicyEnforcementConfig,
    tool_name: &str,
    arguments: &Value,
) -> Result<Value> {
    let mut backend = SystemHookHandlerBackend::default();
    let evidence =
        enforce_request_path_with_backend(policy, ServiceEndpoint::ToolCall, &mut backend)?;
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
        other => Err(Error::Provider(format!("unsupported tool: {other}"))),
    }
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
                    error.contains("unsupported ingest provider/endpoint"),
                    "got: {error}"
                );
            }
            other => panic!("expected Failed, got {other:?}"),
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
}
