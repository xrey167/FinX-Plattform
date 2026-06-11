//! `news/*` catalog routes (gap-matrix item **L2.12**).
//!
//! Standardized news cluster. Each route declares its provider candidates as
//! `(provider, endpoint_key)` where the endpoint key is the concrete dispatch
//! key the runtime registers for that provider's normalizing fetcher (not the
//! `'/'→'_'` route form, since the Benzinga news fetchers use short endpoint
//! keys). Per-provider conformance tests in `tdw-service-api` keep these rows
//! and the providers' dispatch tables in sync without this crate depending on
//! the providers.

use schemars::{Schema, schema_for};
use tdw_core::query_params::StandardParams;
use tdw_domain::NewsArticle;

use crate::{CatalogEntry, EndpointKind, ProviderCandidate};

const NEWS_COMPANY: &[ProviderCandidate] = &[ProviderCandidate::new("benzinga", "news_company")];
const NEWS_WORLD: &[ProviderCandidate] = &[ProviderCandidate::new("benzinga", "news_world")];

fn standard_params() -> Schema {
    schema_for!(StandardParams)
}

fn news_article() -> Schema {
    schema_for!(NewsArticle)
}

/// The `news` namespace's catalog entries, in declaration order.
pub fn entries() -> Vec<CatalogEntry> {
    vec![
        CatalogEntry {
            route: "news/company",
            kind: EndpointKind::Fetch,
            params_schema: standard_params,
            model: news_article,
            candidates: NEWS_COMPANY,
            bronze_table: Some("raw.news_article"),
            doc: "Symbol-filtered company news articles (benzinga).",
            chartable: false,
        },
        CatalogEntry {
            route: "news/world",
            kind: EndpointKind::Fetch,
            params_schema: standard_params,
            model: news_article,
            candidates: NEWS_WORLD,
            bronze_table: Some("raw.news_article"),
            doc: "General market / world news articles, no symbol filter (benzinga).",
            chartable: false,
        },
    ]
}
