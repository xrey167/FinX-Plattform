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

/// Resolve the effective profile: a non-empty `TDW_PROFILE` value overrides
/// `current`; otherwise `current` is kept. Pure (env read happens at the call
/// site) so the override precedence is unit-testable without mutating the
/// process environment.
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
            let bind = config.daemon.tcp_bind.as_deref().unwrap_or("127.0.0.1:7878");
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
    let join = tokio::spawn(async move {
        tdw_app_server::serve_uds(path, handle, events_rx, cancel).await
    });
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
    let bind = config.daemon.http_bind.as_deref().unwrap_or("127.0.0.1:7879");
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

    let (handle, events_rx, service_loop) = service_channel(state.clone(), state.clone());
    let cancel = CancellationToken::new();
    let relay = tdw_app_server::spawn_inmemory_relay(
        state.outbox.clone(),
        state.bus.clone(),
        std::time::Duration::from_millis(50),
        cancel.clone(),
    );

    let transport = spawn_transport(config, handle, events_rx, cancel.clone()).await?;

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
