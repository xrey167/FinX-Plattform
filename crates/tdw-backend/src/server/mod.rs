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

use tdw_app_server::ops::{DaemonMetrics, OpsProvider};
use tdw_app_server::{CancellationToken, SubmissionHandle, serve, service_channel};
use tdw_config::{DaemonTransport, TdwConfig};
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
/// Honours `TDW_CONFIG` (a TOML file path, merged as a config layer) when set,
/// otherwise builds a minimal in-memory default. In both branches volatile
/// paths are overridden to safe in-memory/temp defaults, the TCP bind defaults
/// to `127.0.0.1:7878` (overridable via `TDW_DAEMON_TCP_BIND`), and `TDW_PROFILE`
/// overrides the resolved profile.
///
/// # Errors
///
/// Returns a [`ServerError`] if `TDW_CONFIG` is set but cannot be read, parsed,
/// or merged.
pub async fn load_config() -> Result<TdwConfig, ServerError> {
    // Base config: merge a TDW_CONFIG TOML file when set, else a minimal default
    // whose daemon binds local TCP (overridable via TDW_DAEMON_TCP_BIND, e.g.
    // `0.0.0.0:7878` in a container). The TOML branch keeps the file's own daemon
    // bind. Unset default = `127.0.0.1:7878`.
    let mut config = if let Ok(path) = std::env::var("TDW_CONFIG") {
        let contents = tokio::fs::read_to_string(&path)
            .await
            .map_err(|e| format!("TDW_CONFIG read error ({path}): {e}"))?;
        let layer = tdw_config::ConfigLayer::from_toml(
            tdw_config::ConfigLayerKind::EnvFile,
            "TDW_CONFIG",
            &contents,
        )
        .map_err(|e| format!("TDW_CONFIG parse error: {e}"))?;
        tdw_config::merge_layers(&[layer]).map_err(|e| format!("config merge error: {e}"))?
    } else {
        let mut config = TdwConfig::default();
        config.daemon.transport = DaemonTransport::Tcp;
        config.daemon.tcp_bind = Some(
            std::env::var("TDW_DAEMON_TCP_BIND").unwrap_or_else(|_| "127.0.0.1:7878".to_string()),
        );
        config
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
#[cfg(not(all(unix, feature = "transport-uds")))]
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
#[cfg(not(feature = "transport-http"))]
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
/// [`tdw_mcp::run_stdio_json_rpc_with_daemon`] /
/// [`tdw_mcp::run_streamable_http_with_daemon`], which build the server with
/// [`McpServer::with_daemon_config`](tdw_mcp::McpServer::with_daemon_config). When
/// `None`, the loop falls back to the env-derived daemon config (identical to
/// `tdw-mcp`'s standalone entrypoints).
///
/// Returns the process exit code from the underlying loop.
fn run_mcp_loop(transport: &McpTransport, daemon_addr: Option<&str>) -> i32 {
    let daemon = daemon_addr.map(|addr| {
        tdw_app_client::DaemonClientConfig::tcp(addr)
            .with_timeout(std::time::Duration::from_secs(2))
    });
    match transport {
        McpTransport::Stdio => tdw_mcp::run_stdio_json_rpc_with_daemon(daemon),
        McpTransport::Http(bind) => tdw_mcp::run_streamable_http_with_daemon(bind, daemon),
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
            let transport = cfg.mcp_transport.clone();
            let code = tokio::task::block_in_place(|| run_mcp_loop(&transport, None));
            exit_code_to_result(code)
        }

        Surfaces::Both => run_both(cfg).await,
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

    let transport = cfg.mcp_transport.clone();
    let mcp_thread = std::thread::Builder::new()
        .name("tdw-backend-mcp".to_string())
        .spawn(move || run_mcp_loop(&transport, Some(&daemon_addr)))
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
    /// `load_config` daemon-boot overrides do (an in-memory SQLite session
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
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            let attempt_addr = addr.clone();
            let result = tokio::task::spawn_blocking(move || {
                let client = DaemonClient::new(
                    DaemonClientConfig::tcp(attempt_addr).with_timeout(Duration::from_secs(1)),
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

        let server = tokio::spawn(async move { run(cfg).await });

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

        let joined = tokio::time::timeout(Duration::from_secs(10), run(cfg))
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
