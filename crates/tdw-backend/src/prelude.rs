//! Convenience re-exports. `use tdw_backend::prelude::*;` brings the facade
//! surface available so far into scope. Later phases extend this.

pub use crate::agent::AgentBackend;
pub use crate::config::{BackendConfig, Surfaces};
pub use crate::data::Backend;
pub use crate::error::{BackendError, BackendResult};

pub use tdw_config::TdwConfig;
pub use tdw_service_api::AppState;
