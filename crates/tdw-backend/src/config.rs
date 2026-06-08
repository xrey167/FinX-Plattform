//! Backend configuration: the serving surface selection plus the layered
//! [`TdwConfig`]. The binary phase extends this with bind/transport details.

use tdw_config::TdwConfig;

/// Which surfaces the backend serves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Surfaces {
    /// Serve only the data/daemon surface.
    DaemonOnly,
    /// Serve only the agent/MCP surface.
    McpOnly,
    /// Serve both surfaces (the default).
    #[default]
    Both,
}

/// Which transport the embedded agent/MCP surface speaks.
///
/// `Stdio` runs the line-delimited JSON-RPC loop over stdin/stdout (the default,
/// used by editor/agent hosts). `Http(bind)` runs the Streamable HTTP loop on
/// `bind` (e.g. `127.0.0.1:8788`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum McpTransport {
    /// Line-delimited JSON-RPC over stdin/stdout.
    #[default]
    Stdio,
    /// Streamable HTTP, bound to the given `host:port`.
    Http(String),
}

/// Top-level backend configuration.
#[derive(Debug, Clone, Default)]
pub struct BackendConfig {
    /// The layered TDW configuration shared by every capability group.
    pub tdw: TdwConfig,
    /// Which surfaces to serve.
    pub surfaces: Surfaces,
    /// Which transport the embedded MCP surface speaks (when served).
    pub mcp_transport: McpTransport,
}

impl BackendConfig {
    /// A `BackendConfig` from a [`TdwConfig`], serving [`Surfaces::Both`] over
    /// the default [`McpTransport::Stdio`] MCP transport.
    #[must_use]
    pub fn new(tdw: TdwConfig) -> Self {
        Self {
            tdw,
            surfaces: Surfaces::default(),
            mcp_transport: McpTransport::default(),
        }
    }

    /// Override which surfaces are served.
    #[must_use]
    pub const fn with_surfaces(mut self, surfaces: Surfaces) -> Self {
        self.surfaces = surfaces;
        self
    }

    /// Override which transport the embedded MCP surface speaks.
    #[must_use]
    pub fn with_mcp_transport(mut self, transport: McpTransport) -> Self {
        self.mcp_transport = transport;
        self
    }

    /// Build a [`BackendConfig`] from the process environment.
    ///
    /// The layered [`TdwConfig`] is resolved exactly as the daemon resolves it
    /// (via [`crate::server::load_config`], which honours `TDW_CONFIG` /
    /// `TDW_DAEMON_TCP_BIND` / `TDW_PROFILE`). The serving surfaces are read
    /// from `TDW_BACKEND_SURFACES` (`daemon-only` | `mcp-only` | `both`,
    /// default `both`). The MCP transport is read from
    /// `TDW_BACKEND_MCP_TRANSPORT` (`stdio` | `http`); when `http`, the bind is
    /// taken from `TDW_BACKEND_MCP_HTTP_BIND` (default `127.0.0.1:8788`).
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::BackendError::Init`] if the layered config cannot
    /// be loaded, or if `TDW_BACKEND_SURFACES` holds an unknown value.
    pub async fn load() -> crate::error::BackendResult<Self> {
        let tdw = crate::server::load_config()
            .await
            .map_err(|error| crate::error::BackendError::Init(error.to_string()))?;
        let surfaces = surfaces_from_env(env_value("TDW_BACKEND_SURFACES").as_deref())
            .map_err(crate::error::BackendError::Init)?;
        let mcp_transport = mcp_transport_from_env(
            env_value("TDW_BACKEND_MCP_TRANSPORT").as_deref(),
            env_value("TDW_BACKEND_MCP_HTTP_BIND").as_deref(),
        )
        .map_err(crate::error::BackendError::Init)?;
        Ok(Self {
            tdw,
            surfaces,
            mcp_transport,
        })
    }
}

/// Read an environment variable, returning `None` for unset or blank values.
fn env_value(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// Parse a `TDW_BACKEND_SURFACES` value into [`Surfaces`]. `None` (unset) maps
/// to the default [`Surfaces::Both`].
///
/// # Errors
///
/// Returns the offending value's message if it is not one of the accepted
/// spellings.
pub fn surfaces_from_env(value: Option<&str>) -> Result<Surfaces, String> {
    value.map_or_else(
        || Ok(Surfaces::default()),
        |raw| match raw.trim().to_ascii_lowercase().as_str() {
            "daemon-only" | "daemon_only" | "daemon" => Ok(Surfaces::DaemonOnly),
            "mcp-only" | "mcp_only" | "mcp" => Ok(Surfaces::McpOnly),
            "both" => Ok(Surfaces::Both),
            other => Err(format!(
                "unknown TDW_BACKEND_SURFACES value: {other}; expected daemon-only|mcp-only|both"
            )),
        },
    )
}

/// Parse a `TDW_BACKEND_MCP_TRANSPORT` value (plus an optional HTTP bind) into
/// [`McpTransport`]. `None` (unset) maps to the default [`McpTransport::Stdio`].
///
/// # Errors
///
/// Returns the offending value's message if the transport is not one of the
/// accepted spellings.
pub fn mcp_transport_from_env(
    transport: Option<&str>,
    http_bind: Option<&str>,
) -> Result<McpTransport, String> {
    transport.map_or_else(
        || Ok(McpTransport::default()),
        |raw| match raw.trim().to_ascii_lowercase().as_str() {
            "stdio" => Ok(McpTransport::Stdio),
            "http" | "streamable-http" | "streamable_http" => {
                Ok(McpTransport::Http(http_bind.map_or_else(
                    || tdw_mcp::default_streamable_http_bind().to_string(),
                    str::to_string,
                )))
            }
            other => Err(format!(
                "unknown TDW_BACKEND_MCP_TRANSPORT value: {other}; expected stdio|http"
            )),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn surfaces_default_is_both_when_unset() {
        assert_eq!(surfaces_from_env(None).expect("default"), Surfaces::Both);
    }

    #[test]
    fn surfaces_parse_each_accepted_spelling() {
        assert_eq!(
            surfaces_from_env(Some("daemon-only")).expect("daemon"),
            Surfaces::DaemonOnly
        );
        assert_eq!(
            surfaces_from_env(Some("MCP-Only")).expect("mcp"),
            Surfaces::McpOnly
        );
        assert_eq!(
            surfaces_from_env(Some("both")).expect("both"),
            Surfaces::Both
        );
        assert!(surfaces_from_env(Some("nonsense")).is_err());
    }

    #[test]
    fn mcp_transport_defaults_to_stdio_and_parses_http_bind() {
        assert_eq!(
            mcp_transport_from_env(None, None).expect("default"),
            McpTransport::Stdio
        );
        assert_eq!(
            mcp_transport_from_env(Some("stdio"), None).expect("stdio"),
            McpTransport::Stdio
        );
        assert_eq!(
            mcp_transport_from_env(Some("http"), Some("127.0.0.1:9001")).expect("http"),
            McpTransport::Http("127.0.0.1:9001".to_string())
        );
        // http with no explicit bind falls back to the tdw-mcp default.
        assert_eq!(
            mcp_transport_from_env(Some("http"), None).expect("http default bind"),
            McpTransport::Http(tdw_mcp::default_streamable_http_bind().to_string())
        );
        assert!(mcp_transport_from_env(Some("carrier-pigeon"), None).is_err());
    }
}
