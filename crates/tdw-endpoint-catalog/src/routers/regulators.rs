//! `regulators/*` catalog routes.
//!
//! Stub seeded for a later `OpenBB`-parity story. No routes are populated yet;
//! adding one is an append to [`entries`], never a new wiring point in
//! [`crate::catalog`].

use crate::CatalogEntry;

/// The `regulators` namespace's catalog entries, in declaration order.
///
/// Empty until the `regulators` router is populated by a later story.
pub const fn entries() -> Vec<CatalogEntry> {
    Vec::new()
}
