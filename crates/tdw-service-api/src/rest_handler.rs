//! `RestApiHandler` implementation wiring the daemon's catalog fetch path to
//! the `tdw-app-server` REST route family (feature = "rest-api-route").
//!
//! This is the *caller-side* of the transport-decoupling seam: `tdw-app-server`
//! defines the [`RestApiHandler`] trait but does not depend on
//! `tdw-service-api`; this crate (which owns [`AppState`] and the dispatcher)
//! implements it on a thin [`RestApiState`] wrapper and hands an
//! `Arc<dyn RestApiHandler>` to `tdw_app_server::serve_rest_http`. The handler
//! delegates to [`crate::dispatcher::rest_fetch_data`], so REST never bypasses
//! the policy / hook / mask guards.

#![cfg(feature = "rest-api-route")]

use std::sync::Arc;

use serde_json::Value;
use tdw_app_server::{RestApiHandler, RestError};

use crate::AppState;
use crate::dispatcher::{RestFetchError, rest_fetch_data};

/// Adapter that implements [`RestApiHandler`] over a shared [`AppState`].
///
/// Holds the `AppState` by value (it is `Clone` and cheap to share); construct
/// one via [`RestApiState::new`] and wrap it in an `Arc` for
/// `tdw_app_server::serve_rest_http`.
#[derive(Clone)]
pub struct RestApiState {
    state: AppState,
}

impl RestApiState {
    /// Wrap an [`AppState`] as a [`RestApiHandler`].
    #[must_use]
    pub const fn new(state: AppState) -> Self {
        Self { state }
    }

    /// Build an `Arc<dyn RestApiHandler>` ready to hand to
    /// `tdw_app_server::serve_rest_http`.
    #[must_use]
    pub fn into_handler(self) -> Arc<dyn RestApiHandler> {
        Arc::new(self)
    }
}

#[async_trait::async_trait]
impl RestApiHandler for RestApiState {
    async fn fetch_route(&self, route: &str, params: Value) -> Result<Value, RestError> {
        rest_fetch_data(&self.state, route, params)
            .await
            .map_err(|error| match error {
                RestFetchError::UnknownRoute(message) => RestError::UnknownRoute(message),
                RestFetchError::InvalidParams(message) => RestError::InvalidParams(message),
                RestFetchError::Provider(message) => RestError::Provider(message),
            })
    }
}
