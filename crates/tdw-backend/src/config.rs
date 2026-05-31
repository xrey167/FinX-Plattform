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

/// Top-level backend configuration.
#[derive(Debug, Clone, Default)]
pub struct BackendConfig {
    /// The layered TDW configuration shared by every capability group.
    pub tdw: TdwConfig,
    /// Which surfaces to serve.
    pub surfaces: Surfaces,
}

impl BackendConfig {
    /// A `BackendConfig` from a [`TdwConfig`], serving [`Surfaces::Both`].
    #[must_use]
    pub fn new(tdw: TdwConfig) -> Self {
        Self {
            tdw,
            surfaces: Surfaces::default(),
        }
    }

    /// Override which surfaces are served.
    #[must_use]
    pub fn with_surfaces(mut self, surfaces: Surfaces) -> Self {
        self.surfaces = surfaces;
        self
    }
}
