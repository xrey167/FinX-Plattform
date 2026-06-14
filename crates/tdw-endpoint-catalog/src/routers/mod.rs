//! Per-router catalog modules.
//!
//! Each router owns the routes for one top-level namespace (`equity`, `crypto`,
//! …). A router exposes one `entries()` function returning that namespace's
//! [`CatalogEntry`] list in declaration order; [`crate::catalog`] aggregates
//! them in a fixed router order. Routers seeded for later stories return an
//! empty list for now — the module exists so adding routes is an append, never
//! a new wiring point.

pub mod commodity;
pub mod crypto;
pub mod currency;
pub mod derivatives;
pub mod econometrics;
pub mod economy;
pub mod equity;
pub mod etf;
pub mod fixedincome;
pub mod forecast;
pub mod imf_utils;
pub mod index;
pub mod news;
pub mod portfolio;
pub mod quantitative;
pub mod regulators;
pub mod technical;
pub mod uscongress;
