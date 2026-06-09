use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tdw_auth::{AuthPolicy, Principal, authorize};
use tdw_auth_oidc::{JwksKey, JwtClaims, validate_claims};
use tdw_core::{Error, Result};
use tdw_event::{EventEnvelope, sample_actor_context};
use tdw_hooks::{
    HookExecutionPolicy, HookHandlerBackend, HookRegistry, HookSpec, PermissionEffect,
    PermissionRule, PermissionRules,
};
use tdw_mask::{MaskMode, MaskRule};
use tdw_sandbox::{LocalUdfSandbox, SandboxRuntime, UdfRequest};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ServiceEndpoint {
    EquityHistorical,
    RunQuery,
    IngestBatch,
    ToolCall,
    UdfRun,
    /// Used for all four price-alert CRUD ops (`CreateAlert`, `ListAlerts`,
    /// `DeleteAlert`, `SetAlertActive`). Requires the `analyst` role — the
    /// same gate as query/ingest — so any authenticated analyst can manage
    /// their own alerts without a separate privilege.
    AlertManage,
    /// User registration (`RegisterUser`). Registration is an onboarding op:
    /// in production it would be a **public**, unauthenticated endpoint
    /// (anyone can sign up). This integration slice does not introduce a new
    /// auth mechanism, so for now it reuses the same `analyst` gate as the
    /// alert ops; a production deployment would move this to a public path
    /// (no required role) once anonymous ingress is wired.
    UserRegister,
}

impl ServiceEndpoint {
    const fn name(self) -> &'static str {
        match self {
            Self::EquityHistorical => "equity_historical",
            Self::RunQuery => "tdw.query.run",
            Self::IngestBatch => "tdw.ingest.run",
            Self::ToolCall => "tdw.udf.run",
            Self::UdfRun => "udf.run",
            Self::AlertManage => "tdw.alert.manage",
            Self::UserRegister => "tdw.user.register",
        }
    }

    const fn required_role(self) -> &'static str {
        match self {
            // `UserRegister` is an onboarding op that would be public in
            // production; for now it reuses the same `analyst` gate as the
            // alert ops (no new auth mechanism is introduced here).
            Self::EquityHistorical
            | Self::RunQuery
            | Self::IngestBatch
            | Self::AlertManage
            | Self::UserRegister => "analyst",
            Self::ToolCall | Self::UdfRun => "udf_runner",
        }
    }

    const fn policy_table(self) -> &'static str {
        match self {
            Self::EquityHistorical => "market.equity_historical",
            Self::RunQuery => "analytics.query_run",
            Self::IngestBatch => "market.ingest_batch",
            Self::ToolCall => "runtime.tool_call",
            Self::UdfRun => "runtime.udf_run",
            Self::AlertManage => "market.price_alerts",
            Self::UserRegister => "system.identity_users",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IngressAuthContext {
    pub claims: JwtClaims,
    pub jwks: Vec<JwksKey>,
    pub issuer: String,
    pub audience: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyEnforcementConfig {
    pub auth: IngressAuthContext,
    pub hooks: Vec<HookSpec>,
    pub hook_execution: HookExecutionPolicy,
    pub mask_rules: Vec<MaskRule>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyEnforcementEvidence {
    pub endpoint: String,
    pub principal: String,
    pub hooks: Vec<String>,
}

pub struct SecureServiceRuntime<B: HookHandlerBackend> {
    config: PolicyEnforcementConfig,
    hook_backend: B,
}

impl<B: HookHandlerBackend> SecureServiceRuntime<B> {
    pub const fn new(config: PolicyEnforcementConfig, hook_backend: B) -> Self {
        Self {
            config,
            hook_backend,
        }
    }

    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub fn udf_run(&mut self, request: UdfRequest) -> Result<Value> {
        secure_udf_run_with_backend(&self.config, request, &mut self.hook_backend)
    }

    pub const fn hook_backend(&self) -> &B {
        &self.hook_backend
    }
}

pub fn service_hook_policy<I, S>(
    allowed_actions: I,
    allow_handler_vetoes: bool,
) -> HookExecutionPolicy
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let mut permissions = PermissionRules {
        default_permission: PermissionEffect::Deny,
        rules: Vec::new(),
    };
    for action in allowed_actions {
        let action = action.into();
        permissions.push(PermissionRule::new(
            PermissionEffect::Allow,
            action.clone(),
            action,
        ));
    }
    HookExecutionPolicy {
        permissions,
        allow_handler_vetoes,
    }
}

/// # Errors
///
/// Returns an error variant if the underlying operation fails.
pub fn secure_endpoint_response(
    config: &PolicyEnforcementConfig,
    provider: &str,
    symbol: &str,
) -> Result<Value> {
    let mut backend = tdw_hooks::SystemHookHandlerBackend::default();
    secure_endpoint_response_with_backend(config, provider, symbol, &mut backend)
}

/// # Errors
///
/// Returns an error variant if the underlying operation fails.
pub fn secure_endpoint_response_with_backend(
    config: &PolicyEnforcementConfig,
    provider: &str,
    symbol: &str,
    backend: &mut impl HookHandlerBackend,
) -> Result<Value> {
    secure_endpoint_by_name_with_backend(config, "equity_historical", provider, symbol, backend)
}

/// # Errors
///
/// Returns an error variant if the underlying operation fails.
pub fn secure_endpoint_by_name(
    config: &PolicyEnforcementConfig,
    endpoint: &str,
    provider: &str,
    symbol: &str,
) -> Result<Value> {
    let mut backend = tdw_hooks::SystemHookHandlerBackend::default();
    secure_endpoint_by_name_with_backend(config, endpoint, provider, symbol, &mut backend)
}

/// # Errors
///
/// Returns an error variant if the underlying operation fails.
pub fn secure_endpoint_by_name_with_backend(
    config: &PolicyEnforcementConfig,
    endpoint: &str,
    provider: &str,
    symbol: &str,
    backend: &mut impl HookHandlerBackend,
) -> Result<Value> {
    if endpoint != "equity_historical" {
        return Err(Error::Provider(format!(
            "endpoint denied by default: {endpoint}"
        )));
    }
    let evidence =
        enforce_request_path_with_backend(config, ServiceEndpoint::EquityHistorical, backend)?;
    let response = crate::endpoint_response(provider, symbol)?;
    Ok(json!({
        "policy": evidence,
        "response": mask_json_response(response, &config.mask_rules),
    }))
}

/// # Errors
///
/// Returns an error variant if the underlying operation fails.
pub fn secure_udf_run(config: &PolicyEnforcementConfig, request: UdfRequest) -> Result<Value> {
    let mut backend = tdw_hooks::SystemHookHandlerBackend::default();
    secure_udf_run_with_backend(config, request, &mut backend)
}

/// # Errors
///
/// Returns an error variant if the underlying operation fails.
pub fn secure_udf_run_with_backend(
    config: &PolicyEnforcementConfig,
    request: UdfRequest,
    backend: &mut impl HookHandlerBackend,
) -> Result<Value> {
    let evidence = enforce_request_path_with_backend(config, ServiceEndpoint::UdfRun, backend)?;
    let sandbox = LocalUdfSandbox;
    let response = sandbox.run(request).map_err(|error| {
        Error::Provider(format!("sandbox denied capability or request: {error}"))
    })?;
    Ok(json!({
        "policy": evidence,
        "response": mask_json_response(json!(response), &config.mask_rules),
    }))
}

/// # Errors
///
/// Returns an error variant if the underlying operation fails.
pub fn enforce_request_path_with_backend(
    config: &PolicyEnforcementConfig,
    endpoint: ServiceEndpoint,
    backend: &mut impl HookHandlerBackend,
) -> Result<PolicyEnforcementEvidence> {
    if !validate_claims(
        &config.auth.claims,
        &config.auth.jwks,
        &config.auth.issuer,
        &config.auth.audience,
    ) {
        return Err(Error::Provider("ingress jwt rejected".to_string()));
    }

    let principal = Principal {
        subject: config.auth.claims.sub.clone(),
        roles: config.auth.claims.roles.clone(),
    };
    let auth_policy = AuthPolicy {
        table: endpoint.policy_table().to_string(),
        required_role: endpoint.required_role().to_string(),
        row_filter: None,
    };
    if !authorize(&principal, &auth_policy) {
        return Err(Error::Provider(format!(
            "authorization denied for endpoint {}",
            endpoint.name()
        )));
    }

    let mut registry = HookRegistry::default();
    for hook in &config.hooks {
        registry.register(hook.clone());
    }
    let envelope = hook_event(endpoint, &principal.subject);
    let hook_outcomes = registry
        .execute_handlers(&envelope, &config.hook_execution, backend)
        .map_err(|error| Error::Provider(error.to_string()))?;
    if let Some(outcome) = hook_outcomes
        .iter()
        .find(|outcome| outcome.runtime.should_stop)
    {
        return Err(Error::Provider(format!(
            "hook vetoed request: {}",
            outcome.runtime.name
        )));
    }

    Ok(PolicyEnforcementEvidence {
        endpoint: endpoint.name().to_string(),
        principal: principal.subject,
        hooks: hook_outcomes
            .into_iter()
            .map(|outcome| outcome.runtime.name)
            .collect(),
    })
}

#[must_use]
pub fn mask_json_response(mut value: Value, rules: &[MaskRule]) -> Value {
    for rule in rules {
        mask_json_value(&mut value, rule);
    }
    value
}

fn hook_event(endpoint: ServiceEndpoint, principal: &str) -> EventEnvelope<Value> {
    let (actor, origin, trace) = sample_actor_context("tdw-service-api");
    EventEnvelope::new(
        "service.request",
        actor,
        origin,
        trace,
        "2026-05-28T00:00:00Z",
        json!({
            "endpoint": endpoint.name(),
            "principal": principal,
        }),
    )
}

fn mask_json_value(value: &mut Value, rule: &MaskRule) {
    match value {
        Value::Object(map) => {
            for (field, child) in map {
                if field == &rule.field {
                    *child = masked_leaf(child, rule.mode);
                } else {
                    mask_json_value(child, rule);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                mask_json_value(item, rule);
            }
        }
        _ => {}
    }
}

fn masked_leaf(value: &Value, mode: MaskMode) -> Value {
    match mode {
        MaskMode::Redact => Value::String("***".to_string()),
        MaskMode::Last4 => value.as_str().map_or_else(
            || Value::String("***".to_string()),
            |text| {
                let suffix = text
                    .chars()
                    .rev()
                    .take(4)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect::<String>();
                Value::String(format!("***{suffix}"))
            },
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn last4(field: &str) -> MaskRule {
        MaskRule {
            field: field.to_string(),
            mode: MaskMode::Last4,
        }
    }

    #[test]
    fn last4_masks_long_string_to_trailing_four_digits() {
        // The Last4 string path: keep only the final four characters behind the
        // `***` marker. Reached via the public mask_json_response -> the private
        // masked_leaf Last4 arm.
        let masked = mask_json_response(json!({ "account": "1234567890" }), &[last4("account")]);

        assert_eq!(masked["account"], "***7890");
    }

    #[test]
    fn last4_masks_non_string_value_to_bare_marker() {
        // as_str() is None for a numeric leaf, so the map_or_else fallback yields
        // the bare `***` marker (no suffix).
        let masked = mask_json_response(json!({ "account": 12345 }), &[last4("account")]);

        assert_eq!(masked["account"], "***");
    }

    #[test]
    fn last4_masks_short_string_keeping_all_available_chars() {
        // A string shorter than four chars exercises the chars().rev().take(4)
        // boundary: every character is retained behind the marker.
        let masked = mask_json_response(json!({ "account": "ab" }), &[last4("account")]);

        assert_eq!(masked["account"], "***ab");
    }
}
