//! The unified entity taxonomy shared by the agent plane and the warehouse plane.
//!
//! This is the single source of truth for entity classification across the platform
//! (A1 of the knowledge-system overhaul): the classified [`EntityKind`] registry and its
//! manifest [`Group`]s, the cross-cutting facets ([`DataFacets`], [`EvalFacets`]), and the
//! orthogonal [`Origin`] (tier × source) classification scheme.
//!
//! It is a leaf crate by design — pure data types, no I/O — so every layer (knowledge
//! graph, tags, retrieval, agents, MCP) can depend on one taxonomy without cycles.
//! `tdw-agent` re-exports these types from their original module paths, so existing
//! consumers keep compiling unchanged.
//!
//! K-M4: the [`functional_predicates`] module adds the functional-predicate metadata
//! surface used by contradiction-driven temporal invalidation.

#![forbid(unsafe_code)]
#![deny(clippy::pedantic, clippy::nursery)]

pub mod facets;
pub mod functional_predicates;
pub mod kind;
pub mod origin;

pub use facets::{
    Adaptivity, DataFacets, EvalFacets, Materialization, OpsMetrics, Plane, ValidationState,
    ValidationStatus,
};
pub use functional_predicates::{
    FunctionalPredicateSet, TAXONOMY_DEFAULTS, validate_functional_rel,
};
pub use kind::{EntityKind, Group};
pub use origin::{Origin, Source, Tier};
