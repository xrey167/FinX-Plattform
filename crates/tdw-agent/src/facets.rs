//! Cross-cutting facets — moved to [`tdw_taxonomy::facets`] (knowledge-system A1).
//!
//! This module remains as a re-export shim so every existing `crate::facets::*` path and
//! downstream `tdw_agent::{DataFacets, EvalFacets, …}` consumer keeps compiling
//! unchanged. The canonical definitions live in `tdw-taxonomy`.

pub use tdw_taxonomy::facets::{
    DataFacets, EvalFacets, Materialization, OpsMetrics, Plane, ValidationState, ValidationStatus,
};
