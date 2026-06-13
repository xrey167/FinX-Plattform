//! Shared server harness: boot a transport on an ephemeral loopback port and
//! hand back a handle that owns the listener task.
//!
//! Every "real backend" example reuses this so a `main()` can boot the server,
//! print the bound address, and serve until Ctrl-C, while a test boots the same
//! server, makes a request, and cancels it deterministically.

use std::net::SocketAddr;
use std::sync::Arc;

use tdw_app_server::{
    CancellationToken, KnowledgeGraphHandler, WorkspaceConfig, serve_agent_http, serve_rest_http,
    serve_workspace_http, serve_workspace_http_with_graph,
};
use tdw_service_api::{AgentBridgeState, AppState, RestApiState};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

/// A booted example server: its bound address plus the cancel token and join
/// handle for the accept loop. Dropping the handle does not stop the server;
/// call [`RunningServer::shutdown`] for a clean stop.
#[derive(Debug)]
pub struct RunningServer {
    addr: SocketAddr,
    cancel: CancellationToken,
    task: JoinHandle<()>,
}

impl RunningServer {
    /// The loopback address the server is listening on.
    #[must_use]
    pub const fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// The base URL (`http://<addr>`) clients should target.
    #[must_use]
    pub fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// Signal the accept loop to stop and await the task. Safe to call once.
    pub async fn shutdown(self) {
        self.cancel.cancel();
        let _ = self.task.await;
    }

    /// Block until the server task finishes (e.g. it was cancelled elsewhere).
    ///
    /// # Errors
    ///
    /// Returns the join error if the server task panicked.
    pub async fn join(self) -> Result<(), tokio::task::JoinError> {
        self.task.await
    }
}

/// Bind an ephemeral loopback listener, returning it with its resolved address.
///
/// # Errors
///
/// Returns any bind error from the OS.
async fn bind_ephemeral() -> std::io::Result<(TcpListener, SocketAddr)> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    Ok((listener, addr))
}

/// Boot the catalog-derived **REST** surface (`GET /api/v1/<route>`) over an
/// in-memory daemon. This is the surface the Python SDK drives.
///
/// # Errors
///
/// Returns any error binding the ephemeral listener.
pub async fn start_rest() -> std::io::Result<RunningServer> {
    let state = AppState::in_memory_for_tests().await;
    let handler = RestApiState::new(state).into_handler();
    let (listener, addr) = bind_ephemeral().await?;
    let cancel = CancellationToken::new();
    let cancel_srv = cancel.clone();
    let task = tokio::spawn(async move {
        let _ = serve_rest_http(listener, handler, None, cancel_srv).await;
    });
    Ok(RunningServer { addr, cancel, task })
}

/// Boot the **Workspace data backend** surface (`/widgets.json`, `/apps.json`,
/// `/widget-data/...`) over an in-memory daemon, with the given CORS/auth config.
///
/// # Errors
///
/// Returns any error binding the ephemeral listener.
pub async fn start_workspace(config: WorkspaceConfig) -> std::io::Result<RunningServer> {
    let state = AppState::in_memory_for_tests().await;
    let handler = RestApiState::new(state).into_handler();
    let (listener, addr) = bind_ephemeral().await?;
    let cancel = CancellationToken::new();
    let cancel_srv = cancel.clone();
    let task = tokio::spawn(async move {
        let _ = serve_workspace_http(listener, handler, config, cancel_srv).await;
    });
    Ok(RunningServer { addr, cancel, task })
}

/// Boot the **Workspace data backend** surface WITH the read-only knowledge
/// graph handler attached (K-M6), so `GET /widget-data/knowledge/graph` serves a
/// live ego-graph through the same `serve_workspace_http_with_graph` path the
/// daemon uses.
///
/// # Errors
///
/// Returns any error binding the ephemeral listener.
pub async fn start_workspace_with_graph(
    config: WorkspaceConfig,
    graph: Arc<dyn KnowledgeGraphHandler>,
) -> std::io::Result<RunningServer> {
    let state = AppState::in_memory_for_tests().await;
    let handler = RestApiState::new(state).into_handler();
    let (listener, addr) = bind_ephemeral().await?;
    let cancel = CancellationToken::new();
    let cancel_srv = cancel.clone();
    let task = tokio::spawn(async move {
        let _ = serve_workspace_http_with_graph(listener, handler, Some(graph), config, cancel_srv)
            .await;
    });
    Ok(RunningServer { addr, cancel, task })
}

/// Boot the **agent (copilot) bridge** surface (`/agents.json`, `POST /v1/query`)
/// backed by the offline `StubLanguageModel`, with the given CORS/auth config.
///
/// # Errors
///
/// Returns any error binding the ephemeral listener.
pub async fn start_agent(config: WorkspaceConfig) -> std::io::Result<RunningServer> {
    let handler = AgentBridgeState::new().into_handler();
    let (listener, addr) = bind_ephemeral().await?;
    let query_url = format!("http://{addr}/v1/query");
    let cancel = CancellationToken::new();
    let cancel_srv = cancel.clone();
    let task = tokio::spawn(async move {
        let _ = serve_agent_http(listener, handler, config, query_url, cancel_srv).await;
    });
    Ok(RunningServer { addr, cancel, task })
}
