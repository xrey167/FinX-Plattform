//! [`RuntimeConfig`] — application-level settings for the function runtime.

/// Application-level configuration for the function runtime.
///
/// Read from environment variables via [`RuntimeConfig::from_env`].
///
/// LLM-provider binding is deferred to the sub-PR that adds AI-step support.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeConfig {
    /// Application identifier, used to namespace runs and steps.
    ///
    /// Defaults to `"tdw"` when `TDW_FUNCTIONS_APP_ID` is not set.
    pub app_id: String,

    /// Optional HMAC signing secret for webhook request verification.
    ///
    /// Set via `TDW_FUNCTIONS_SIGNING_SECRET`.
    pub signing_secret: Option<String>,
}

impl RuntimeConfig {
    /// Build a [`RuntimeConfig`] from environment variables.
    ///
    /// | Variable                      | Default |
    /// |-------------------------------|---------|
    /// | `TDW_FUNCTIONS_APP_ID`        | `"tdw"` |
    /// | `TDW_FUNCTIONS_SIGNING_SECRET`| `None`  |
    #[must_use]
    pub fn from_env() -> Self {
        let app_id = std::env::var("TDW_FUNCTIONS_APP_ID").unwrap_or_else(|_| "tdw".to_string());
        let signing_secret = std::env::var("TDW_FUNCTIONS_SIGNING_SECRET").ok();
        Self {
            app_id,
            signing_secret,
        }
    }
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            app_id: "tdw".to_string(),
            signing_secret: None,
        }
    }
}
