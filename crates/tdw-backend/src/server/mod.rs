//! Shared serving glue for the `tdw-backend` library and its binary.
//!
//! This module **factors** the daemon bootstrap that previously lived only in
//! `tdw-service/src/main.rs` (`load_config`, `spawn_transport`, `resolve_profile`)
//! so both the in-process
//! [`Backend::serve`](crate::data::Backend::serve) lifecycle and the standalone
//! binary share one implementation instead of forking it. `tdw-service` now
//! calls back into these functions (it gained a `tdw-backend` dependency; no
//! cycle exists because `tdw-service` is a leaf binary that nothing depends on).
//!
//! It also adds [`run`], the surface-aware entrypoint the binary calls: it
//! composes the daemon and/or the embedded MCP loop per
//! [`BackendConfig::surfaces`](crate::config::BackendConfig), and — for
//! [`Surfaces::Both`](crate::config::Surfaces::Both) — runs the **blocking** MCP
//! loop on a dedicated named OS thread that reaches the in-process daemon only
//! over a loopback [`DaemonClient`](tdw_app_client::DaemonClient), never via the
//! async `Backend` (the MCP thread has no tokio runtime).

use std::net::SocketAddr;
#[cfg(feature = "rest-api-route")]
use std::sync::Arc;

use tdw_app_server::ops::{DaemonMetrics, OpsProvider};
use tdw_app_server::{CancellationToken, SubmissionHandle, serve, service_channel};
use tdw_config::{DaemonTransport, TdwConfig};
#[cfg(feature = "rest-api-route")]
use tdw_knowledge::runtime::KnowledgeRuntime;
use tdw_protocol::EventMsg;
use tdw_service_api::AppState;

use crate::config::{BackendConfig, McpTransport, Surfaces};
use crate::data::Backend;
use crate::error::{BackendError, BackendResult};

/// A boxed error from the serving glue (mirrors `tdw-service`'s `ServiceError`).
pub type ServerError = Box<dyn std::error::Error + Send + Sync>;

/// Resolve the layered [`TdwConfig`] for daemon boot, exactly as the original
/// `tdw-service` binary did.
///
/// Layer resolution order (later layers win):
///
/// 1. **Base**: `TDW_CONFIG` TOML file path when set, otherwise the structural
///    default with TCP transport forced and `TDW_DAEMON_TCP_BIND` applied.
/// 2. **Inline env**: `TDW_CONFIG_CONTENT` raw TOML string when set, merged as
///    an [`InlineEnv`](tdw_config::ConfigLayerKind::InlineEnv) layer on top of
///    the base. This is the mechanism compose/production profiles use to pin
///    `[knowledge.graph] backend = "bolt"` without requiring a mounted file.
/// 3. **Boot overrides**: volatile paths (`session.sqlite_path`,
///    `paths.rollout_dir`) and `TDW_PROFILE` are applied last.
///
/// # Errors
///
/// Returns a [`ServerError`] if `TDW_CONFIG` is set but cannot be read, parsed,
/// or merged, or if `TDW_CONFIG_CONTENT` is set but contains invalid TOML.
pub async fn load_config() -> Result<TdwConfig, ServerError> {
    // Step 1 — base layer: a TDW_CONFIG file or the structural default.
    let base_layers: Vec<tdw_config::ConfigLayer> = if let Ok(path) = std::env::var("TDW_CONFIG") {
        let contents = tokio::fs::read_to_string(&path)
            .await
            .map_err(|e| format!("TDW_CONFIG read error ({path}): {e}"))?;
        let layer = tdw_config::ConfigLayer::from_toml(
            tdw_config::ConfigLayerKind::EnvFile,
            "TDW_CONFIG",
            &contents,
        )
        .map_err(|e| format!("TDW_CONFIG parse error: {e}"))?;
        vec![layer]
    } else {
        // No file: synthesise a minimal default layer so merge_layers can
        // accept an empty-or-content-only list below.
        vec![]
    };

    // Step 2 — inline env layer: TDW_CONFIG_CONTENT is raw TOML merged on top
    // of the base with InlineEnv precedence (50 > EnvFile 20). This is how
    // compose/production profiles pin knowledge.graph.backend = "bolt" without
    // a mounted file. When unset this step is a no-op.
    let mut layers = base_layers;
    if let Ok(content) = std::env::var("TDW_CONFIG_CONTENT")
        && !content.trim().is_empty()
    {
        let layer = tdw_config::ConfigLayer::from_toml(
            tdw_config::ConfigLayerKind::InlineEnv,
            "TDW_CONFIG_CONTENT",
            &content,
        )
        .map_err(|e| format!("TDW_CONFIG_CONTENT parse error: {e}"))?;
        layers.push(layer);
    }

    let mut config = if layers.is_empty() {
        // Pure default: no TDW_CONFIG file, no TDW_CONFIG_CONTENT.
        let mut cfg = TdwConfig::default();
        cfg.daemon.transport = DaemonTransport::Tcp;
        cfg.daemon.tcp_bind = Some(
            std::env::var("TDW_DAEMON_TCP_BIND").unwrap_or_else(|_| "127.0.0.1:7878".to_string()),
        );
        cfg
    } else {
        // One or more layers: let merge_layers compose them over TdwConfig::default().
        // In the TDW_CONFIG-only case the file sets the tcp_bind; in all other
        // cases the structural default (`127.0.0.1:7878`) is already present and
        // TDW_DAEMON_TCP_BIND is applied below in the shared post-merge step.
        let mut cfg =
            tdw_config::merge_layers(&layers).map_err(|e| format!("config merge error: {e}"))?;
        // Apply TDW_DAEMON_TCP_BIND when set, regardless of how the base was built.
        if let Ok(bind) = std::env::var("TDW_DAEMON_TCP_BIND") {
            cfg.daemon.tcp_bind = Some(bind);
        }
        cfg
    };

    // Shared daemon-boot overrides (applied to either base): an in-memory session
    // store, a temp rollout dir, and TDW_PROFILE precedence. The profile drives
    // `build_policy` — non-prod profiles attach a local-default policy so
    // dispatches resolve; `prod`/`production` stay fail-closed until an
    // auth-backed policy is wired.
    config.session.sqlite_path = "sqlite::memory:".to_string();
    config.paths.rollout_dir = std::env::temp_dir()
        .join("tdw-rollout.jsonl")
        .to_string_lossy()
        .into_owned();
    config.profile = resolve_profile(&config.profile, std::env::var("TDW_PROFILE").ok());
    Ok(config)
}

/// Parse a raw TOML string as an [`InlineEnv`](tdw_config::ConfigLayerKind::InlineEnv)
/// layer and merge it on top of [`TdwConfig::default`].
///
/// This is the pure core of the `TDW_CONFIG_CONTENT` handling in [`load_config`].
/// Extracted as a standalone function so it can be tested without mutating
/// the process environment (which `#![forbid(unsafe_code)]` prevents in tests).
///
/// # Errors
///
/// Returns a [`ServerError`] if `content` is invalid TOML or the merged config
/// fails semantic validation.
#[cfg(test)]
fn apply_inline_content_layer(content: &str) -> Result<TdwConfig, ServerError> {
    let layer = tdw_config::ConfigLayer::from_toml(
        tdw_config::ConfigLayerKind::InlineEnv,
        "TDW_CONFIG_CONTENT",
        content,
    )
    .map_err(|e| -> ServerError { format!("TDW_CONFIG_CONTENT parse error: {e}").into() })?;
    tdw_config::merge_layers(&[layer])
        .map_err(|e| -> ServerError { format!("config merge error: {e}").into() })
}

/// Resolve the effective profile: `TDW_PROFILE` overrides `current` when non-empty.
///
/// Pure (env read happens at the call site) so the override precedence is unit-testable
/// without mutating the process environment. Otherwise `current` is kept.
#[must_use]
pub fn resolve_profile(current: &str, env_profile: Option<String>) -> String {
    match env_profile {
        Some(profile) if !profile.trim().is_empty() => profile.trim().to_string(),
        _ => current.to_string(),
    }
}

/// A spawned transport: its [`JoinHandle`](tokio::task::JoinHandle) plus the
/// actual address the listener bound (resolved AFTER binding, so an ephemeral
/// `127.0.0.1:0` request yields the OS-assigned port).
pub struct TransportTask {
    /// The spawned transport server task.
    pub join: tokio::task::JoinHandle<std::io::Result<()>>,
    /// The address the transport actually bound (post-OS-assignment).
    pub bound_addr: String,
}

/// Bind a TCP listener on `bind` and return it alongside the concrete address it
/// bound (so an ephemeral `…:0` request resolves to the OS-assigned port via
/// `local_addr`). `label` only flavors the error/log text (e.g. `"TCP"`,
/// `"HTTP/SSE"`), so the TCP and HTTP/SSE transports share one bind path.
///
/// # Errors
///
/// Returns a [`ServerError`] if `bind` is not a valid socket address or the bind
/// fails.
async fn bind_tcp(
    bind: &str,
    label: &str,
) -> Result<(tokio::net::TcpListener, String), ServerError> {
    let addr: SocketAddr = bind
        .parse()
        .map_err(|e| format!("invalid {label} bind address ({bind}): {e}"))?;
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| format!("{label} bind failed ({addr}): {e}"))?;
    let bound_addr = listener
        .local_addr()
        .map_err(|e| format!("local_addr: {e}"))?
        .to_string();
    eprintln!("tdw-backend: {label} listener bound on {bound_addr}");
    Ok((listener, bound_addr))
}

/// Spawn the configured daemon transport, returning its task and the address it
/// actually bound.
///
/// For TCP the [`TcpListener`](tokio::net::TcpListener) is bound **here** (so a
/// `127.0.0.1:0` request resolves to a concrete OS-assigned port via
/// `local_addr`) and the pre-bound listener is handed to
/// [`serve_tcp`](tdw_app_server::serve_tcp). UDS/HTTP are feature-gated and
/// fail-closed when the corresponding transport feature is not compiled.
///
/// # Errors
///
/// Returns a [`ServerError`] if the address is invalid, binding fails, or the
/// requested transport was not compiled into this build.
pub async fn spawn_transport(
    config: &TdwConfig,
    handle: SubmissionHandle,
    events_rx: tokio::sync::mpsc::UnboundedReceiver<EventMsg>,
    cancel: CancellationToken,
) -> Result<TransportTask, ServerError> {
    match config.daemon.transport {
        DaemonTransport::Tcp => {
            let bind = config
                .daemon
                .tcp_bind
                .as_deref()
                .unwrap_or("127.0.0.1:7878");
            let (listener, bound_addr) = bind_tcp(bind, "TCP").await?;
            let join = tokio::spawn(async move {
                tdw_app_server::serve_tcp(listener, handle, events_rx, cancel).await
            });
            Ok(TransportTask { join, bound_addr })
        }

        DaemonTransport::Uds => spawn_uds(config, handle, events_rx, cancel).await,

        DaemonTransport::HttpSse => spawn_http(config, handle, events_rx, cancel).await,
    }
}

// Feature-gated UDS helper — only compiled on Unix with the transport-uds feature.
#[cfg(all(unix, feature = "transport-uds"))]
async fn spawn_uds(
    config: &TdwConfig,
    handle: SubmissionHandle,
    events_rx: tokio::sync::mpsc::UnboundedReceiver<EventMsg>,
    cancel: CancellationToken,
) -> Result<TransportTask, ServerError> {
    use std::path::PathBuf;
    let path = PathBuf::from(&config.daemon.uds_path);
    let bound_addr = config.daemon.uds_path.clone();
    eprintln!("tdw-backend: UDS listener on {path:?}");
    let join =
        tokio::spawn(
            async move { tdw_app_server::serve_uds(path, handle, events_rx, cancel).await },
        );
    Ok(TransportTask { join, bound_addr })
}

// UDS is explicit: fail startup instead of silently binding a different transport.
// `async` must match the feature-enabled `spawn_uds` signature above; both are `.await`ed at the same call site.
#[cfg(not(all(unix, feature = "transport-uds")))]
#[allow(clippy::unused_async)]
async fn spawn_uds(
    _config: &TdwConfig,
    _handle: SubmissionHandle,
    _events_rx: tokio::sync::mpsc::UnboundedReceiver<EventMsg>,
    _cancel: CancellationToken,
) -> Result<TransportTask, ServerError> {
    Err("UDS transport requested but this binary was not built for transport-uds on Unix".into())
}

// Feature-gated HTTP/SSE helper.
#[cfg(feature = "transport-http")]
async fn spawn_http(
    config: &TdwConfig,
    handle: SubmissionHandle,
    events_rx: tokio::sync::mpsc::UnboundedReceiver<EventMsg>,
    cancel: CancellationToken,
) -> Result<TransportTask, ServerError> {
    let bind = config
        .daemon
        .http_bind
        .as_deref()
        .unwrap_or("127.0.0.1:7879");
    let (listener, bound_addr) = bind_tcp(bind, "HTTP/SSE").await?;
    let join = tokio::spawn(async move {
        tdw_app_server::serve_http(listener, handle, events_rx, cancel).await
    });
    Ok(TransportTask { join, bound_addr })
}

// HTTP/SSE is explicit: fail startup instead of silently binding a different transport.
// `async` must match the feature-enabled `spawn_http` signature above; both are `.await`ed at the same call site.
#[cfg(not(feature = "transport-http"))]
#[allow(clippy::unused_async)]
async fn spawn_http(
    _config: &TdwConfig,
    _handle: SubmissionHandle,
    _events_rx: tokio::sync::mpsc::UnboundedReceiver<EventMsg>,
    _cancel: CancellationToken,
) -> Result<TransportTask, ServerError> {
    Err(
        "HTTP/SSE transport requested but this binary was not built with feature 'transport-http'"
            .into(),
    )
}

/// Operability provider for the daemon's `/health`, `/ready`, `/metrics`
/// surface. `/ready` probes the durable stores via [`AppState::readiness`];
/// `/metrics` renders the live [`DaemonMetrics`] (dispatch outcome counters +
/// in-flight gauge) shared with the daemon's [`ServiceLoop`].
#[derive(Clone)]
struct DaemonOps {
    state: AppState,
    metrics: DaemonMetrics,
}

impl OpsProvider for DaemonOps {
    async fn ready(&self) -> (bool, String) {
        match self.state.readiness().await {
            Ok(()) => (true, "ready: durable stores reachable\n".to_string()),
            Err(error) => (false, format!("not ready: stores unreachable: {error}\n")),
        }
    }

    async fn metrics(&self) -> String {
        self.metrics.render()
    }
}

/// Bind and spawn the daemon's ops listener when `TDW_DAEMON_HTTP_BIND` is set.
/// Returns the listener task, or `None` when unset (the default) or on bind
/// failure (logged), so the daemon still serves its transport.
async fn spawn_daemon_ops(
    state: &AppState,
    metrics: DaemonMetrics,
    cancel: CancellationToken,
) -> Option<tokio::task::JoinHandle<std::io::Result<()>>> {
    let bind = std::env::var("TDW_DAEMON_HTTP_BIND")
        .ok()
        .filter(|value| !value.trim().is_empty())?;
    let listener = match tokio::net::TcpListener::bind(&bind).await {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("tdw-backend: ops listener bind failed on {bind}: {error}");
            return None;
        }
    };
    eprintln!("tdw-backend: ops listener on http://{bind} (/health /ready /metrics)");
    let provider = DaemonOps {
        state: state.clone(),
        metrics,
    };
    Some(tokio::spawn(async move {
        tdw_app_server::ops::serve_ops(listener, provider, cancel).await
    }))
}

/// Bind and spawn the daemon's catalog-derived REST listener when
/// `TDW_DAEMON_REST_BIND` is set (and the `rest-api-route` feature is compiled).
///
/// Serves `GET /api/v1/{route...}?<params>` (resolved through the same
/// policy-guarded `Op::FetchData` path) and `GET /openapi.json`. Returns the
/// listener task, or `None` when unset (the default) or on bind failure
/// (logged), so the daemon still serves its primary transport. The REST surface
/// is unauthenticated at the transport — the same posture as `POST /op` — so the
/// loopback default is the safe one; a non-loopback bind warrants the same
/// reverse-proxy/auth fronting as the TCP transport.
#[cfg(feature = "rest-api-route")]
async fn spawn_daemon_rest(
    state: &AppState,
    knowledge: Option<Arc<KnowledgeRuntime>>,
    cancel: CancellationToken,
) -> Option<tokio::task::JoinHandle<std::io::Result<()>>> {
    let bind = std::env::var("TDW_DAEMON_REST_BIND")
        .ok()
        .filter(|value| !value.trim().is_empty())?;
    let listener = match tokio::net::TcpListener::bind(&bind).await {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("tdw-backend: REST listener bind failed on {bind}: {error}");
            return None;
        }
    };
    eprintln!("tdw-backend: REST listener on http://{bind} (/api/v1/<route> /openapi.json)");
    let handler = tdw_service_api::RestApiState::new(state.clone()).into_handler();
    // Wire the knowledge status handler when a co-resident KnowledgeRuntime is
    // available so GET /api/v1/knowledge/status returns the live snapshot.
    let knowledge_status = knowledge
        .map(|runtime| tdw_service_api::KnowledgeStatusAdapter::new(runtime).into_handler());
    Some(tokio::spawn(async move {
        tdw_app_server::serve_rest_http(listener, handler, knowledge_status, cancel).await
    }))
}

/// Bind and spawn the daemon's OpenBB Workspace bridge listener when
/// `TDW_WORKSPACE_BIND` is set (and the `workspace-route` feature is compiled).
///
/// Serves `GET /widgets.json`, `GET /apps.json` (both derived from the endpoint
/// catalog), and `GET /widget-data/{route...}?<params>` (resolved through the
/// same policy-guarded `Op::FetchData` path the REST surface uses), with CORS
/// and an optional `X-TDW-API-KEY` check.
///
/// **Fail-closed**, mirroring `TDW_MCP_HTTP_TOKEN` semantics: a non-loopback
/// bind with no `TDW_WORKSPACE_API_KEY` set refuses to start the listener (logs
/// and returns `None`), since any browser/host reaching the port could otherwise
/// drive the daemon's fetch path unauthenticated. A loopback bind needs no key.
///
/// Returns `None` when unset (the default), on a fail-closed refusal, or on bind
/// failure (logged), so the daemon still serves its primary transport.
///
/// Decide whether the workspace listener must refuse to start: a non-loopback
/// bind with no API key is the fail-closed case. Pure (no env / no I/O) so the
/// fail-closed rule is unit-testable without mutating the process environment.
#[cfg(any(feature = "workspace-route", feature = "agent-route"))]
#[must_use]
fn workspace_bind_refused(bind: &str, has_api_key: bool) -> bool {
    !bind_is_loopback(bind) && !has_api_key
}

#[cfg(feature = "workspace-route")]
async fn spawn_daemon_workspace(
    state: &AppState,
    knowledge: Option<Arc<KnowledgeRuntime>>,
    cancel: CancellationToken,
) -> Option<tokio::task::JoinHandle<std::io::Result<()>>> {
    let bind = std::env::var("TDW_WORKSPACE_BIND")
        .ok()
        .filter(|value| !value.trim().is_empty())?;
    let config = tdw_app_server::WorkspaceConfig::from_env();
    if workspace_bind_refused(&bind, config.api_key.is_some()) {
        eprintln!(
            "tdw-backend: REFUSING to start workspace listener on non-loopback bind '{bind}' without TDW_WORKSPACE_API_KEY set (fail-closed). Set the key or bind 127.0.0.1."
        );
        return None;
    }
    let listener = match tokio::net::TcpListener::bind(&bind).await {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("tdw-backend: workspace listener bind failed on {bind}: {error}");
            return None;
        }
    };
    eprintln!(
        "tdw-backend: workspace listener on http://{bind} (/widgets.json /apps.json /widget-data/<route>)"
    );
    let handler = tdw_service_api::RestApiState::new(state.clone()).into_handler();
    // K-M6: wire the read-only knowledge graph-visualization handler when a
    // co-resident KnowledgeRuntime is available so
    // GET /widget-data/knowledge/graph serves the live ego-graph. Absent → the
    // route falls through and 400s (the standalone daemon has no graph).
    let graph = knowledge
        .map(|runtime| tdw_service_api::KnowledgeGraphAdapter::new(runtime).into_handler());
    Some(tokio::spawn(async move {
        tdw_app_server::serve_workspace_http_with_graph(listener, handler, graph, config, cancel)
            .await
    }))
}

/// Bind and spawn the daemon's OpenBB Workspace **agent protocol** bridge
/// listener when `TDW_AGENT_BIND` is set (and the `agent-route` feature is
/// compiled).
///
/// Serves `GET /agents.json` (one default copilot) and `POST /v1/query` (the
/// `openbb-ai` SSE copilot protocol), driving the offline `StubLanguageModel`
/// through the pure `tdw-openbb-agent` sequencer. CORS + the optional
/// `X-TDW-API-KEY` check are shared with the workspace family.
///
/// **Fail-closed**, mirroring [`spawn_daemon_workspace`]: a non-loopback bind
/// with no `TDW_WORKSPACE_API_KEY` set refuses to start (logs and returns
/// `None`). Returns `None` when unset, on a fail-closed refusal, or on bind
/// failure (logged), so the daemon still serves its primary transport.
#[cfg(feature = "agent-route")]
async fn spawn_daemon_agent(
    cancel: CancellationToken,
) -> Option<tokio::task::JoinHandle<std::io::Result<()>>> {
    let bind = std::env::var("TDW_AGENT_BIND")
        .ok()
        .filter(|value| !value.trim().is_empty())?;
    let config = tdw_app_server::WorkspaceConfig::from_env();
    if workspace_bind_refused(&bind, config.api_key.is_some()) {
        eprintln!(
            "tdw-backend: REFUSING to start agent listener on non-loopback bind '{bind}' without TDW_WORKSPACE_API_KEY set (fail-closed). Set the key or bind 127.0.0.1."
        );
        return None;
    }
    let listener = match tokio::net::TcpListener::bind(&bind).await {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("tdw-backend: agent listener bind failed on {bind}: {error}");
            return None;
        }
    };
    // The agents.json `endpoints.query` URL must point back at this listener.
    let query_url = format!("http://{bind}/v1/query");
    eprintln!("tdw-backend: agent listener on http://{bind} (/agents.json /v1/query)");
    let handler = tdw_service_api::AgentBridgeState::new().into_handler();
    Some(tokio::spawn(async move {
        tdw_app_server::serve_agent_http(listener, handler, config, query_url, cancel).await
    }))
}

/// Emit the same startup policy diagnostics the original `tdw-service` binary
/// did, reporting whether a policy is attached for the resolved profile.
fn report_policy_state(state: &AppState, config: &TdwConfig) {
    if state.policy.is_some() {
        eprintln!(
            "tdw-backend: daemon starting in '{}' profile with a policy attached; dispatches will resolve",
            config.profile
        );
    } else if let Some(error) = &state.policy_attach_error {
        eprintln!(
            "tdw-backend: daemon starting in '{}' profile with no policy attached: {error}; configure TDW_OIDC_* correctly so dispatches resolve",
            config.profile
        );
    } else {
        eprintln!(
            "tdw-backend: daemon starting in '{}' profile with no policy attached; dispatches will return Failed until an auth-backed policy is wired (configure TDW_OIDC_*)",
            config.profile
        );
    }
}

/// Whether `bind` (a `host:port` socket spec) targets a loopback interface.
///
/// Used to decide whether a non-loopback (network-reachable) bind warrants a
/// prominent security warning. Unparseable/host-only specs are treated as
/// non-loopback so the warning errs on the side of caution.
#[must_use]
pub fn bind_is_loopback(bind: &str) -> bool {
    let host = bind
        .rsplit_once(':')
        .map_or(bind, |(host, _)| host)
        .trim_matches(['[', ']']);
    match host {
        "localhost" => true,
        other => other
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback()),
    }
}

/// Emit a prominent warning when the daemon's TCP transport binds a
/// non-loopback address while no auth-backed policy is attached.
///
/// Exposing the daemon on `0.0.0.0` (or any routable host) without ingress auth
/// is the highest-risk misconfiguration: any host that can reach the port can
/// drive the daemon. The safe default is loopback (`127.0.0.1:7878`); operators
/// who deliberately bind wider must wire a policy (configure `TDW_OIDC_*`) and
/// front the daemon with a token/mTLS/reverse-proxy layer — see
/// `docs/release/data-backend-runbook.md`.
fn warn_on_unauthenticated_nonloopback_bind(config: &TdwConfig, state: &AppState) {
    if config.daemon.transport != DaemonTransport::Tcp {
        return;
    }
    let bind = config
        .daemon
        .tcp_bind
        .as_deref()
        .unwrap_or("127.0.0.1:7878");
    if !bind_is_loopback(bind) && state.policy.is_none() {
        eprintln!(
            "tdw-backend: SECURITY WARNING — daemon TCP transport is bound to non-loopback address '{bind}' with no auth-backed policy attached. Any host that can reach this port can drive the daemon. Configure TDW_OIDC_* and front the daemon with a token/mTLS/reverse-proxy layer (see docs/release/data-backend-runbook.md), or bind 127.0.0.1."
        );
    }
}

/// Run the daemon to completion in the current tokio runtime, blocking until
/// ctrl-c or a dispatched `Shutdown` cancels it.
///
/// This is the faithful lift of the original `tdw-service` daemon bootstrap:
/// build the [`AppState`] from `config`, wire the [`service_channel`], spawn the
/// relay and transport, then [`serve`] until cancellation and await the
/// transport task.
///
/// # Errors
///
/// Returns a [`ServerError`] if the composition root cannot be built, the
/// transport cannot bind, or the service loop errors.
pub async fn run_daemon(config: &TdwConfig) -> Result<(), ServerError> {
    let state = AppState::from_config(config.clone())
        .await
        .map_err(|e| format!("AppState::from_config failed: {e}"))?;
    report_policy_state(&state, config);

    // A *partial* OIDC configuration (some but not all `TDW_OIDC_*` set, or an
    // invalid JWKS/claims set) is an operator mistake, not a fail-closed default:
    // refuse to start with the actionable diagnostic rather than silently
    // running with no auth-backed policy. A fully-unset OIDC config keeps the
    // existing fail-closed behavior (`policy_attach_error` is `None` there).
    if let Some(error) = &state.policy_attach_error {
        return Err(format!(
            "refusing to start daemon in '{}' profile: {error}; set the listed TDW_OIDC_* variables (or unset all of them to run fail-closed)",
            config.profile
        )
        .into());
    }

    warn_on_unauthenticated_nonloopback_bind(config, &state);

    // K-E1: eagerly validate the graph backend at boot so the bolt-connected
    // log line (or in-memory NOTICE) appears before any op is dispatched.
    // A misconfigured or unreachable backend fails the daemon here, at startup,
    // rather than silently succeeding and dying on first knowledge use.
    crate::data::validate_graph_backend(&config.knowledge.graph)
        .await
        .map_err(|e| -> ServerError {
            format!("graph backend startup check failed: {e}").into()
        })?;

    let metrics = DaemonMetrics::new();
    let (handle, events_rx, service_loop) = service_channel(state.clone(), state.clone());
    let service_loop = service_loop.with_metrics(metrics.clone());
    let cancel = CancellationToken::new();
    let relay = tdw_app_server::spawn_inmemory_relay(
        state.outbox.clone(),
        state.bus.clone(),
        std::time::Duration::from_millis(50),
        cancel.clone(),
    );

    let transport = spawn_transport(config, handle, events_rx, cancel.clone()).await?;

    // Optional ops surface (/health, /ready, /metrics), env-gated and off by
    // default; bound on TDW_DAEMON_HTTP_BIND. It shares the cancellation token
    // so a graceful drain stops accepting ops requests too.
    let ops_task = spawn_daemon_ops(&state, metrics, cancel.clone()).await;

    // Optional catalog-derived REST surface (GET /api/v1/<route>, /openapi.json),
    // env-gated on TDW_DAEMON_REST_BIND and off by default. Shares the
    // cancellation token so a graceful drain stops accepting REST requests too.
    // Standalone daemon (run_daemon) has no co-resident Backend so the
    // knowledge status endpoint is unavailable (None → 404 for that path).
    #[cfg(feature = "rest-api-route")]
    let rest_task = spawn_daemon_rest(&state, None, cancel.clone()).await;

    // Optional OpenBB Workspace bridge surface (GET /widgets.json, /apps.json,
    // /widget-data/<route>), env-gated on TDW_WORKSPACE_BIND and off by default.
    // Fail-closed on a non-loopback bind without TDW_WORKSPACE_API_KEY. Shares
    // the cancellation token so a graceful drain stops accepting requests too.
    // Standalone daemon (run_daemon) has no co-resident Backend, so the K-M6
    // knowledge graph widget data route is unavailable (None → falls through).
    #[cfg(feature = "workspace-route")]
    let workspace_task = spawn_daemon_workspace(&state, None, cancel.clone()).await;

    // Optional OpenBB Workspace agent-protocol bridge (GET /agents.json,
    // POST /v1/query SSE), env-gated on TDW_AGENT_BIND and off by default.
    // Fail-closed on a non-loopback bind without TDW_WORKSPACE_API_KEY. Shares
    // the cancellation token so a graceful drain stops accepting requests too.
    #[cfg(feature = "agent-route")]
    let agent_task = spawn_daemon_agent(cancel.clone()).await;

    // Phase B — a standalone daemon's only memory surface is the persisted file
    // set named by `TDW_MEMORY_DIR` (runtime memory ingest is via the library
    // `Backend` API — `upsert_memory` — which the binary does not expose, since
    // there is no memory `Op` on the transport). So consolidation here ages the
    // on-disk `*.json5` memories in place; spawn the scheduler ONLY when such a
    // dir is configured, otherwise it would tick over an empty, unreachable store.
    let consolidation_task = if crate::data::memory_dir_configured() {
        let memory =
            std::sync::Arc::new(tokio::sync::Mutex::new(crate::data::build_memory_store()));
        Some(tdw_agent_store::spawn_consolidation_scheduler(
            memory,
            crate::data::consolidation_tick(),
            cancel.clone(),
        ))
    } else {
        None
    };

    println!(
        "tdw-backend: daemon up. transport={:?} addr={}. ctrl-c to exit.",
        config.daemon.transport, transport.bound_addr
    );

    serve(service_loop, relay, cancel.clone()).await?;
    // The scheduler observes the same token; abort if it lingers so the daemon
    // teardown stays bounded.
    if let Some(task) = consolidation_task {
        task.abort();
        let _ = task.await;
    }
    // The ops listener observes the same cancellation token; await its clean
    // exit so the drain is bounded.
    if let Some(task) = ops_task {
        let _ = task.await;
    }
    // The REST listener observes the same cancellation token; await its clean
    // exit so the drain is bounded.
    #[cfg(feature = "rest-api-route")]
    if let Some(task) = rest_task {
        let _ = task.await;
    }
    // The workspace listener observes the same cancellation token; await its
    // clean exit so the drain is bounded.
    #[cfg(feature = "workspace-route")]
    if let Some(task) = workspace_task {
        let _ = task.await;
    }
    // The agent listener observes the same cancellation token; await its clean
    // exit so the drain is bounded.
    #[cfg(feature = "agent-route")]
    if let Some(task) = agent_task {
        let _ = task.await;
    }
    let _ = transport.join.await;
    Ok(())
}

/// Run the embedded agent/MCP loop on the **current** (blocking) thread.
///
/// `daemon_addr`, when `Some`, points the embedded MCP server's loopback
/// [`DaemonClient`](tdw_app_client::DaemonClient) at the in-process daemon's
/// bound TCP address. This is the **real mechanism**, and it avoids any
/// `std::env::set_var` (forbidden here under `forbid(unsafe_code)` on Rust
/// 2024): a [`DaemonClientConfig::tcp`] is threaded into the `tdw-mcp` loop via
/// [`tdw_mcp::run_stdio_json_rpc_with_knowledge`] /
/// [`tdw_mcp::run_streamable_http_with_knowledge`], which build the server with
/// [`McpServer::with_daemon_config`](tdw_mcp::McpServer::with_daemon_config). When
/// `daemon_addr` is `None`, the loop falls back to the env-derived daemon config
/// (identical to `tdw-mcp`'s standalone entrypoints).
///
/// `knowledge`, `feedback`, and `indexer` are the co-resident handles from
/// `Backend` (knowledge-system F1/K-E3): when `Some` they are injected into the
/// MCP server so the knowledge read tools, `tdw.kg.feedback`, and `tdw.kg.ingest`
/// are live on the embedded surface without a loopback round-trip to the daemon.
/// When `None` (standalone `McpOnly` surface without a co-resident `Backend`)
/// the server omits those tools, exactly as the `tdw-mcp` standalone binary does.
///
/// Returns the process exit code from the underlying loop.
#[allow(clippy::too_many_arguments)]
fn run_mcp_loop(
    transport: &McpTransport,
    daemon_addr: Option<&str>,
    knowledge: Option<std::sync::Arc<tdw_knowledge::runtime::KnowledgeRuntime>>,
    feedback: Option<std::sync::Arc<tokio::sync::Mutex<tdw_agent_store::RetrievalFeedbackStore>>>,
    indexer: Option<std::sync::Arc<tokio::sync::Mutex<tdw_knowledge::indexer::KnowledgeIndexer>>>,
    infer: Option<std::sync::Arc<tokio::sync::Mutex<tdw_infer::InferEngine>>>,
    contradiction: Option<std::sync::Arc<tdw_infer::contradiction::ContradictionDetector>>,
    watchlist: Option<
        std::sync::Arc<std::sync::Mutex<tdw_mcp::knowledge_watchlist_tools::WatchlistStore>>,
    >,
    questions: Option<
        std::sync::Arc<std::sync::Mutex<tdw_mcp::knowledge_question_tools::OpenQuestionStore>>,
    >,
) -> i32 {
    let daemon = daemon_addr.map(|addr| {
        tdw_app_client::DaemonClientConfig::tcp(addr)
            .with_timeout(std::time::Duration::from_secs(2))
    });
    match transport {
        McpTransport::Stdio => tdw_mcp::run_stdio_json_rpc_with_knowledge(
            daemon,
            knowledge,
            feedback,
            indexer,
            infer,
            contradiction,
            watchlist,
            questions,
        ),
        McpTransport::Http(bind) => tdw_mcp::run_streamable_http_with_knowledge(
            bind,
            daemon,
            knowledge,
            feedback,
            indexer,
            infer,
            contradiction,
            watchlist,
            questions,
        ),
    }
}

/// Surface-aware entrypoint: compose and run the configured surfaces.
///
/// * [`Surfaces::DaemonOnly`] → run the daemon to completion ([`run_daemon`]).
/// * [`Surfaces::McpOnly`] → run the blocking MCP loop on the current thread.
/// * [`Surfaces::Both`] → [`Backend::serve`](crate::data::Backend::serve) on the
///   tokio runtime, then run the **blocking** MCP loop on a dedicated named OS
///   thread (`tdw-backend-mcp`) that reaches data only via the loopback
///   [`DaemonClient`](tdw_app_client::DaemonClient) at the daemon's bound
///   address. On MCP-loop exit the daemon is signalled and the backend is shut
///   down.
///
/// # Errors
///
/// Returns a [`BackendError`] if the daemon cannot be built/served or shut down,
/// or if the MCP thread cannot be spawned/joined.
pub async fn run(cfg: BackendConfig) -> BackendResult<()> {
    match cfg.surfaces {
        Surfaces::DaemonOnly => run_daemon(&cfg.tdw)
            .await
            .map_err(|error| BackendError::Init(error.to_string())),

        Surfaces::McpOnly => {
            // No async daemon — run the blocking loop on this thread directly.
            // `spawn_blocking` is intentionally NOT used for the long-lived loop.
            // No co-resident Backend in McpOnly mode: knowledge/feedback are None
            // (the standalone surface reaches knowledge via the daemon's loopback
            // transport, not in-process injection).
            //
            // K-X5: load the watchlist store from TDW_WATCHLIST_DIR so that
            // subscriptions created in a previous session survive the restart.
            // McpOnly has no co-resident Background so the store is owned here
            // and injected directly — same pattern as Both mode.
            let watchlist = Some(std::sync::Arc::new(std::sync::Mutex::new(
                tdw_mcp::knowledge_watchlist_tools::load_store_from_env(),
            )));
            // K-X8: load the question store from TDW_QUESTIONS_DIR so that
            // parked questions created in a previous session survive the restart.
            let questions = Some(std::sync::Arc::new(std::sync::Mutex::new(
                tdw_mcp::knowledge_question_tools::load_store_from_env(),
            )));
            let transport = cfg.mcp_transport.clone();
            let code = tokio::task::block_in_place(|| {
                run_mcp_loop(
                    &transport, None, None, None, None, None, None, watchlist, questions,
                )
            });
            exit_code_to_result(code)
        }

        Surfaces::Both => Box::pin(run_both(cfg)).await,
    }
}

/// [`Surfaces::Both`]: serve the daemon in-process, then run the blocking MCP
/// loop on a dedicated OS thread pointed at the daemon's loopback address.
async fn run_both(cfg: BackendConfig) -> BackendResult<()> {
    let mut backend = Backend::from_config(cfg.tdw.clone()).await?;
    backend.serve(&cfg).await?;
    let daemon_addr = backend
        .bound_addr()
        .map(str::to_string)
        .ok_or_else(|| BackendError::Init("daemon did not expose a bound address".to_string()))?;

    // Inject the co-resident knowledge runtime, feedback store, indexer,
    // inference engine, and contradiction detector into the embedded MCP server
    // (knowledge-system F1/K-E3/K-L1/K-M4). All are cheap Arc clones.
    let knowledge = Some(backend.knowledge_runtime_handle());
    let feedback = Some(backend.feedback_store_handle());
    let indexer = Some(backend.knowledge_indexer_handle());
    let infer = Some(backend.infer_engine_handle());
    let contradiction_detector = backend.contradiction_detector_handle();
    let watchlist = Some(backend.watchlist_store_handle());
    let questions = Some(backend.question_store_handle());

    // Optional catalog-derived REST surface, env-gated on TDW_DAEMON_REST_BIND.
    // In Both mode the co-resident knowledge runtime is wired so
    // GET /api/v1/knowledge/status returns the live snapshot (K-E2).
    #[cfg(feature = "rest-api-route")]
    let _rest_task = {
        let cancel = tdw_app_server::CancellationToken::new();
        spawn_daemon_rest(backend.app_state(), knowledge.clone(), cancel).await
    };

    // Optional OpenBB Workspace bridge surface, env-gated on TDW_WORKSPACE_BIND.
    // In Both mode the co-resident knowledge runtime is wired so the K-M6
    // GET /widget-data/knowledge/graph route serves the live ego-graph.
    #[cfg(feature = "workspace-route")]
    let _workspace_task = {
        let cancel = tdw_app_server::CancellationToken::new();
        spawn_daemon_workspace(backend.app_state(), knowledge.clone(), cancel).await
    };

    let transport = cfg.mcp_transport.clone();
    let mcp_thread = std::thread::Builder::new()
        .name("tdw-backend-mcp".to_string())
        .spawn(move || {
            run_mcp_loop(
                &transport,
                Some(&daemon_addr),
                knowledge,
                feedback,
                indexer,
                infer,
                Some(contradiction_detector),
                watchlist,
                questions,
            )
        })
        .map_err(BackendError::Io)?;

    // Wait for the MCP loop to finish on a blocking thread so we do not stall
    // the async runtime. The MCP loop owns the process lifetime in `Both` mode
    // (stdio EOF or HTTP listener close ends it).
    let code = tokio::task::spawn_blocking(move || mcp_thread.join())
        .await?
        .map_err(|_| BackendError::Init("tdw-backend-mcp thread panicked".to_string()))?;

    // Signal the daemon to stop and reclaim its tasks before returning.
    backend.shutdown().await?;
    exit_code_to_result(code)
}

/// Map a process exit code from an MCP loop into a [`BackendResult`]: `0` is
/// success, anything else is a generic init failure naming the code.
fn exit_code_to_result(code: i32) -> BackendResult<()> {
    if code == 0 {
        Ok(())
    } else {
        Err(BackendError::Init(format!(
            "embedded MCP loop exited with code {code}"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tdw_config::DaemonTransport;

    // -- Step 1: pure-function unit tests -----------------------------------

    #[test]
    fn resolve_profile_prefers_non_empty_env_over_current() {
        // (a) Some non-empty -> returns the env value, overriding `current`.
        let resolved = resolve_profile("dev", Some("service".to_string()));
        assert_eq!(resolved, "service");
    }

    #[test]
    fn resolve_profile_trims_whitespace_padded_env() {
        // (b) Some('  prod  ') whitespace-padded -> returns trimmed 'prod'.
        let resolved = resolve_profile("dev", Some("  prod  ".to_string()));
        assert_eq!(resolved, "prod");
    }

    #[test]
    fn resolve_profile_empty_env_falls_through_to_current() {
        // (c) Some('') empty -> falls through to `current`.
        let resolved = resolve_profile("dev", Some(String::new()));
        assert_eq!(resolved, "dev");
    }

    #[test]
    fn resolve_profile_all_whitespace_env_falls_through_to_current() {
        // (d) Some('   ') all-whitespace -> falls through to `current`.
        let resolved = resolve_profile("dev", Some("   ".to_string()));
        assert_eq!(resolved, "dev");
    }

    #[test]
    fn resolve_profile_none_keeps_current() {
        // (e) None -> returns `current` unchanged.
        let resolved = resolve_profile("production", None);
        assert_eq!(resolved, "production");
    }

    #[test]
    fn exit_code_to_result_zero_is_ok() {
        assert!(exit_code_to_result(0).is_ok());
    }

    #[test]
    fn exit_code_to_result_nonzero_is_init_err_naming_the_code() {
        let err = exit_code_to_result(3).expect_err("non-zero code must be an error");
        match err {
            BackendError::Init(msg) => {
                assert!(
                    msg.contains("code 3"),
                    "error message must name the exit code, got: {msg}"
                );
            }
            other => panic!("expected BackendError::Init, got {other:?}"),
        }
    }

    // -- Step 2: bind / transport unit tests --------------------------------

    /// Build a `(SubmissionHandle, events_rx, ServiceLoop)` triple from a real
    /// in-memory `AppState`, returning only the pieces `spawn_transport` needs.
    /// The `ServiceLoop` is dropped (the transport tests never drive it; they
    /// only exercise the bind / fail-closed paths and then cancel).
    async fn transport_inputs() -> (
        SubmissionHandle,
        tokio::sync::mpsc::UnboundedReceiver<EventMsg>,
    ) {
        let state = AppState::in_memory_for_tests().await;
        let (handle, events_rx, _service_loop) = service_channel(state.clone(), state);
        (handle, events_rx)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn spawn_transport_tcp_binds_ephemeral_port_then_shuts_down() {
        let mut config = TdwConfig::default();
        config.daemon.transport = DaemonTransport::Tcp;
        config.daemon.tcp_bind = Some("127.0.0.1:0".to_string());

        let (handle, events_rx) = transport_inputs().await;
        let cancel = CancellationToken::new();

        let task = spawn_transport(&config, handle, events_rx, cancel.clone())
            .await
            .expect("ephemeral TCP bind should succeed");

        // local_addr resolution: the OS assigned a concrete, non-zero port.
        assert!(
            task.bound_addr.parse::<SocketAddr>().is_ok(),
            "bound_addr must be a valid SocketAddr, got: {}",
            task.bound_addr
        );
        assert_ne!(
            task.bound_addr
                .rsplit(':')
                .next()
                .expect("port segment present"),
            "0",
            "ephemeral :0 must resolve to a concrete OS-assigned port"
        );

        // Hermetic teardown: cancel and join so no task leaks.
        cancel.cancel();
        let joined = tokio::time::timeout(std::time::Duration::from_secs(3), task.join)
            .await
            .expect("transport task must join promptly after cancel");
        assert!(joined.is_ok(), "transport task join should not panic");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn spawn_transport_tcp_invalid_address_errors() {
        let mut config = TdwConfig::default();
        config.daemon.transport = DaemonTransport::Tcp;
        config.daemon.tcp_bind = Some("not-an-addr".to_string());

        let (handle, events_rx) = transport_inputs().await;
        let cancel = CancellationToken::new();

        // `TransportTask` (the Ok type) is not `Debug`, so match rather than
        // `expect_err` to extract the error without requiring it to format Ok.
        let result = spawn_transport(&config, handle, events_rx, cancel).await;
        let msg = match result {
            Ok(_) => panic!("an unparseable bind address must error, not bind"),
            Err(e) => e.to_string(),
        };
        assert!(
            msg.contains("invalid") && msg.contains("not-an-addr"),
            "error must describe the invalid address, got: {msg}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn spawn_transport_uds_fails_closed_under_default_features() {
        // Default features do not compile transport-uds: the stub must fail
        // closed rather than silently bind a different transport.
        let mut config = TdwConfig::default();
        config.daemon.transport = DaemonTransport::Uds;

        let (handle, events_rx) = transport_inputs().await;
        let cancel = CancellationToken::new();

        let result = spawn_transport(&config, handle, events_rx, cancel).await;
        let msg = match result {
            Ok(_) => panic!("UDS must fail closed without the transport-uds feature"),
            Err(e) => e.to_string(),
        };
        assert!(
            msg.contains("not built"),
            "UDS stub must report a not-built error, got: {msg}"
        );
        assert!(
            msg.contains("transport-uds"),
            "UDS stub must name the missing transport-uds feature, got: {msg}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn spawn_transport_http_fails_closed_under_default_features() {
        // Default features do not compile transport-http: the stub must fail
        // closed.
        let mut config = TdwConfig::default();
        config.daemon.transport = DaemonTransport::HttpSse;

        let (handle, events_rx) = transport_inputs().await;
        let cancel = CancellationToken::new();

        let result = spawn_transport(&config, handle, events_rx, cancel).await;
        let msg = match result {
            Ok(_) => panic!("HTTP/SSE must fail closed without the transport-http feature"),
            Err(e) => e.to_string(),
        };
        assert!(
            msg.contains("not built"),
            "HTTP/SSE stub must report a not-built error, got: {msg}"
        );
        assert!(
            msg.contains("transport-http"),
            "HTTP/SSE stub must name the missing transport-http feature, got: {msg}"
        );
    }

    // -- Step 4: load_config default-branch unit test (TDW_CONFIG unset) -----

    #[tokio::test]
    async fn load_config_default_branch_applies_daemon_boot_overrides() {
        // The TDW_CONFIG file-read branch is only taken when TDW_CONFIG is set;
        // skip the default-branch assertions if a runner happens to set it.
        if std::env::var("TDW_CONFIG").is_ok() {
            return;
        }

        let config = load_config()
            .await
            .expect("default-branch load_config must succeed");

        // Default branch sets TCP transport (line 64) ...
        assert_eq!(config.daemon.transport, DaemonTransport::Tcp);
        // ... overrides the session store to in-memory (line 76) ...
        assert_eq!(config.session.sqlite_path, "sqlite::memory:");
        // ... and always has a concrete tcp_bind (lines 65-67).
        assert!(config.daemon.tcp_bind.is_some());

        // The exact default bind only holds when TDW_DAEMON_TCP_BIND is unset.
        if std::env::var("TDW_DAEMON_TCP_BIND").is_err() {
            assert_eq!(config.daemon.tcp_bind.as_deref(), Some("127.0.0.1:7878"));
        }

        // The resolved profile is consistent with the resolve_profile contract:
        // load_config feeds TDW_PROFILE through resolve_profile over the base
        // profile, so the result equals resolve_profile of that same input.
        let expected_profile = resolve_profile(&config.profile, std::env::var("TDW_PROFILE").ok());
        assert_eq!(config.profile, expected_profile);
    }

    /// K-E1 review finding 1: `TDW_CONFIG_CONTENT` is the mechanism compose/production
    /// profiles use to pin knowledge.graph.backend = "bolt". This test proves the
    /// layer-merge path works by calling `apply_inline_content_layer` directly —
    /// the pure helper extracted from `load_config` — without mutating the process
    /// environment (which is forbidden by `#![forbid(unsafe_code)]`).
    ///
    /// The test also acts as a contract for the compose pin: the same TOML that
    /// `docker-compose.yaml` injects via `TDW_CONFIG_CONTENT` is passed here, and
    /// the assertion proves it overrides the `in-memory` default to `bolt`.
    #[test]
    fn inline_content_layer_overrides_graph_backend() {
        let toml = "[knowledge.graph]\nbackend = \"bolt\"\nbolt_uri = \"bolt://127.0.0.1:7687\"\n";
        let config = apply_inline_content_layer(toml)
            .expect("valid TDW_CONFIG_CONTENT TOML must produce a valid config");
        assert_eq!(
            config.knowledge.graph.backend, "bolt",
            "inline content layer must override the in-memory default to bolt"
        );
        assert_eq!(
            config.knowledge.graph.bolt_uri.as_deref(),
            Some("bolt://127.0.0.1:7687"),
            "inline content layer must carry bolt_uri into the merged config"
        );
    }

    #[test]
    fn loopback_binds_are_recognized() {
        assert!(bind_is_loopback("127.0.0.1:7878"));
        assert!(bind_is_loopback("localhost:7878"));
        assert!(bind_is_loopback("[::1]:7878"));
        assert!(bind_is_loopback("127.0.0.5:7878"));
    }

    #[test]
    fn non_loopback_binds_are_recognized() {
        assert!(!bind_is_loopback("0.0.0.0:7878"));
        assert!(!bind_is_loopback("192.168.1.10:7878"));
        assert!(!bind_is_loopback("[::]:7878"));
        // A host-only / unparseable spec errs on the side of caution.
        assert!(!bind_is_loopback("example.com:7878"));
    }

    #[cfg(feature = "workspace-route")]
    #[test]
    fn workspace_bind_refused_only_on_nonloopback_without_key() {
        // Non-loopback + no key -> fail-closed refusal.
        assert!(workspace_bind_refused("0.0.0.0:7900", false));
        // Non-loopback + key set -> allowed.
        assert!(!workspace_bind_refused("0.0.0.0:7900", true));
        // Loopback never refuses, key or not.
        assert!(!workspace_bind_refused("127.0.0.1:7900", false));
        assert!(!workspace_bind_refused("localhost:7900", false));
    }

    // -- Round 18: daemon serving-path behavioral suite ---------------------
    //
    // These tests drive the previously-0%-covered daemon RUN path
    // (`run_daemon`, `run(DaemonOnly)`, `run(Both)`) to completion over an
    // ephemeral loopback TCP port, cancelling via a loopback `DaemonClient`
    // `Op::Shutdown` (the same proven mechanism as
    // `data::tests::serve_binds_ephemeral_port_submits_via_loopback_then_shuts_down`).
    //
    // Hermetic and edition-2024 safe: NO `std::env::set_var` anywhere; the
    // daemon is steered purely via an explicit `TdwConfig` (Tcp + a reserved
    // loopback port) and a loopback client. The `env_blocked_subset`
    // (partial-OIDC early-return, `TDW_MEMORY_DIR` consolidation branch, and
    // the `report_policy_state` attach-error / no-policy arms) is deliberately
    // NOT exercised here. Under the default profile `AppState::from_config`
    // attaches a local-default policy, so `state.policy.is_some()` and the
    // present-arm of `report_policy_state` fires while `policy_attach_error`
    // stays `None` (so the partial-OIDC `Err` return is correctly skipped).

    use std::time::Duration;
    use tdw_app_client::{DaemonClient, DaemonClientConfig};
    use tdw_protocol::{ActorKind, ActorRef, Op, OpEnvelope, SessionId};

    /// Reserve a concrete free loopback port by binding an ephemeral listener,
    /// reading the OS-assigned address, then dropping the listener so the
    /// daemon under test can rebind it. This is how the serving-path tests
    /// learn a port to point the loopback client at, since `run_daemon` /
    /// `run` consume their config and do not surface the bound address the way
    /// `Backend::serve` does.
    async fn reserve_loopback_addr() -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("reserving an ephemeral loopback port should succeed");
        let addr = listener
            .local_addr()
            .expect("local_addr on the reserved listener")
            .to_string();
        drop(listener);
        addr
    }

    /// Build a `TdwConfig` whose daemon binds `addr` over TCP and whose
    /// durable stores are fully in-memory/temp, exactly as the production
    /// `load_config` daemon-boot overrides do (an in-memory `SQLite` session
    /// store + a temp rollout dir). Using the raw `TdwConfig::default()` here
    /// would point the session store at `~/.tdw/session.sqlite`, so
    /// `AppState::from_config` would fail and the daemon would never bind.
    fn in_memory_daemon_config(addr: &str) -> TdwConfig {
        let mut config = TdwConfig::default();
        config.daemon.transport = DaemonTransport::Tcp;
        config.daemon.tcp_bind = Some(addr.to_string());
        config.session.sqlite_path = "sqlite::memory:".to_string();
        config.paths.rollout_dir = std::env::temp_dir()
            .join("tdw-rollout-r18-a.jsonl")
            .to_string_lossy()
            .into_owned();
        // F1: the default graph backend is "bolt" but test builds lack the
        // `bolt` feature (and have no Memgraph available). Force in-memory so
        // `Backend::from_config` / `build_graph_engine` succeeds in CI.
        config.knowledge.graph.backend = "in-memory".to_string();
        config
    }

    /// Build a `Shutdown` op envelope, mirroring `data::tests::make_envelope`.
    fn shutdown_envelope() -> OpEnvelope {
        OpEnvelope::new(
            SessionId::new("session-server-test").expect("session id"),
            1,
            ActorRef {
                actor_id: "user:test".to_string(),
                kind: ActorKind::User,
                tenant_id: Some("default".to_string()),
            },
            Op::Shutdown,
        )
    }

    /// Submit a loopback `Op::Shutdown` to the daemon at `addr`, retrying the
    /// connect a few times so the spawned daemon has a moment to bind. Returns
    /// once the daemon emits a terminal `Completed` event for the op.
    async fn submit_loopback_shutdown(addr: String) {
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        loop {
            let attempt_addr = addr.clone();
            let result = tokio::task::spawn_blocking(move || {
                let client = DaemonClient::new(
                    DaemonClientConfig::tcp(attempt_addr).with_timeout(Duration::from_secs(5)),
                );
                client.submit_and_wait(&shutdown_envelope())
            })
            .await
            .expect("spawn_blocking join");

            match result {
                Ok(submission) => {
                    assert!(
                        submission
                            .events
                            .iter()
                            .any(|event| matches!(event, EventMsg::Completed { .. })),
                        "the daemon must emit a terminal Completed event for the shutdown op"
                    );
                    return;
                }
                Err(error) => {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "loopback client never reached the daemon: {error}"
                    );
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            }
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn run_daemon_serves_then_shuts_down_via_loopback() {
        // Drives `run_daemon` end-to-end: AppState::from_config, the
        // report_policy_state present-arm (default profile attaches a policy),
        // service_channel + relay + spawn_transport wiring, the no-memory-dir
        // (None) consolidation arm, `serve` until the loopback Shutdown
        // cancels it, and the bounded Ok(()) teardown.
        let addr = reserve_loopback_addr().await;

        let config = in_memory_daemon_config(&addr);

        let daemon = tokio::spawn(async move { run_daemon(&config).await });

        submit_loopback_shutdown(addr).await;

        let joined = tokio::time::timeout(Duration::from_secs(5), daemon)
            .await
            .expect("run_daemon must join promptly after the loopback Shutdown")
            .expect("run_daemon task should not panic");
        joined.expect("run_daemon should return Ok(()) on a clean Shutdown-driven teardown");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn run_daemon_only_surface_serves_then_shuts_down_via_loopback() {
        // Drives `run(BackendConfig{surfaces: DaemonOnly})`: the DaemonOnly
        // match arm and its BackendError::Init map closure on the success
        // branch (which never fires here), delegating to `run_daemon`.
        let addr = reserve_loopback_addr().await;

        let cfg = BackendConfig {
            tdw: in_memory_daemon_config(&addr),
            surfaces: Surfaces::DaemonOnly,
            ..Default::default()
        };

        let server = tokio::spawn(async move { Box::pin(run(cfg)).await });

        submit_loopback_shutdown(addr).await;

        let joined = tokio::time::timeout(Duration::from_secs(5), server)
            .await
            .expect("run(DaemonOnly) must join promptly after the loopback Shutdown")
            .expect("run task should not panic");
        joined.expect("run(DaemonOnly) should return Ok(()) on a clean Shutdown-driven teardown");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn run_both_surface_wires_daemon_and_runs_mcp_loop_to_completion() {
        // Drives `run(BackendConfig{surfaces: Both, mcp_transport: Stdio})` ->
        // `run_both`: Backend::from_config, backend.serve, bound_addr
        // resolution, the named `tdw-backend-mcp` OS-thread spawn, the
        // spawn_blocking join, backend.shutdown, and exit_code_to_result.
        //
        // In Both mode the embedded stdio MCP loop owns the process lifetime
        // and ends on stdin EOF. Under the test runner stdin is already at EOF
        // (no input is piped), so `run_stdio_json_rpc_with_daemon` iterates
        // zero lines and returns code 0 promptly; that drives `run_both`
        // through the OS-thread join, `backend.shutdown` (which cancels the
        // in-process daemon and reclaims its tasks), and `exit_code_to_result`
        // to a clean `Ok(())`. We therefore do NOT inject a loopback Shutdown
        // here — the MCP-EOF path self-terminates the surface, which is the
        // real Both-mode lifecycle.
        //
        // We assert `run` returns within a bounded timeout — `Ok(())` on the
        // clean (code 0) exit, or a deterministic `BackendError::Init` mapped
        // by `exit_code_to_result` on a non-zero code. Per the plan this test
        // is upside-only (the DaemonOnly + run_daemon tests already clear the
        // coverage floor), so a non-zero exit code is an accepted outcome, not
        // a failure.
        let addr = reserve_loopback_addr().await;

        let cfg = BackendConfig {
            tdw: in_memory_daemon_config(&addr),
            surfaces: Surfaces::Both,
            mcp_transport: McpTransport::Stdio,
        };

        let joined = tokio::time::timeout(Duration::from_secs(10), Box::pin(run(cfg)))
            .await
            .expect("run(Both) must complete within the bounded timeout");
        match joined {
            Ok(()) => {}
            Err(BackendError::Init(msg)) => {
                assert!(
                    msg.contains("MCP loop exited with code"),
                    "a non-zero MCP exit must be reported via exit_code_to_result, got: {msg}"
                );
            }
            Err(other) => panic!("unexpected run(Both) error: {other:?}"),
        }
    }
}
