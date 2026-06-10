//! The entity-kind registry — moved to [`tdw_taxonomy::kind`] (knowledge-system A1).
//!
//! This module remains as a re-export shim so every existing `crate::kind::EntityKind`
//! path and downstream `tdw_agent::EntityKind` consumer keeps compiling unchanged. The
//! canonical definition (50 kinds incl. the warehouse `domain` group) lives in
//! `tdw-taxonomy`, the shared leaf crate both planes depend on.

pub use tdw_taxonomy::kind::{EntityKind, Group};
