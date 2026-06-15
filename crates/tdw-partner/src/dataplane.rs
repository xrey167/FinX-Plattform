//! The [`DataPlane`] port over the warehouse dispatcher (the partner design §1.4).
//!
//! `PartnerCore` needs to fetch data for a resolved route, but the dispatcher
//! lives in the 4900-line `tdw-service-api` crate. Depending on it directly from
//! `tdw-partner` would invert the layering (`tdw-knowledge` ↔ `tdw-service-api`
//! would become bidirectional through this crate). Instead the execute step is
//! expressed as a thin async port: the Workspace adapter in `tdw-service-api`
//! implements it by calling `dispatcher::rest_fetch_data` / `dispatch_op`, while
//! the pure crate stays a leaf.
//!
//! This is the anti-over-engineering line: the port is one method
//! (`fetch(route, params) -> Value`), not a plugin framework.

use async_trait::async_trait;
use serde_json::Value;

/// An error returned by a [`DataPlane::fetch`].
#[derive(Debug, thiserror::Error)]
pub enum DataPlaneError {
    /// The route was rejected or no provider could serve it.
    #[error("data plane fetch failed for route {route:?}: {message}")]
    Fetch {
        /// The route that failed.
        route: String,
        /// A human-readable failure reason.
        message: String,
    },
}

/// The port `PartnerCore` executes a resolved route through.
///
/// Implemented by `tdw-service-api` over the catalog-driven dispatch path
/// (`rest_fetch_data` / `dispatch_op`). Offline/test surfaces implement it with
/// a fixture map. The route is always one the resolver already validated against
/// the catalog (`is_valid_route`), so an implementation never has to re-grammar
/// it — only resolve a provider and fetch.
#[async_trait]
pub trait DataPlane: Send + Sync {
    /// Fetch `route` with `params`, returning the standardized result envelope.
    ///
    /// # Errors
    ///
    /// Returns [`DataPlaneError::Fetch`] when the route cannot be served.
    async fn fetch(&self, route: &str, params: Value) -> Result<Value, DataPlaneError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Arc;

    /// A fixture data plane: a route→result map, mirroring how the real
    /// dispatcher would round-trip a fetch without any network.
    #[derive(Default)]
    struct FixtureDataPlane {
        rows: HashMap<String, Value>,
    }

    #[async_trait]
    impl DataPlane for FixtureDataPlane {
        async fn fetch(&self, route: &str, _params: Value) -> Result<Value, DataPlaneError> {
            self.rows
                .get(route)
                .cloned()
                .ok_or_else(|| DataPlaneError::Fetch {
                    route: route.to_string(),
                    message: "no fixture for route".to_string(),
                })
        }
    }

    #[tokio::test]
    async fn route_to_fetch_round_trip() {
        let mut rows = HashMap::new();
        rows.insert(
            "equity/price/historical".to_string(),
            serde_json::json!({"rows": [{"close": 190.0}]}),
        );
        let plane: Arc<dyn DataPlane> = Arc::new(FixtureDataPlane { rows });

        let out = plane
            .fetch(
                "equity/price/historical",
                serde_json::json!({"symbol": "AAPL"}),
            )
            .await
            .expect("fixture fetch");
        assert_eq!(out["rows"][0]["close"], 190.0);

        // An unknown route surfaces as a typed fetch error, not a panic.
        let err = plane
            .fetch("nope/route", Value::Null)
            .await
            .expect_err("unknown route errors");
        assert!(matches!(err, DataPlaneError::Fetch { .. }));
    }
}
