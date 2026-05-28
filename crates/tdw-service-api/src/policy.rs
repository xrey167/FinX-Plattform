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
}

impl ServiceEndpoint {
    fn name(self) -> &'static str {
        match self {
            Self::EquityHistorical => "equity_historical",
            Self::RunQuery => "tdw.query.run",
            Self::IngestBatch => "tdw.ingest.run",
            Self::ToolCall => "tdw.udf.run",
            Self::UdfRun => "udf.run",
        }
    }

    fn required_role(self) -> &'static str {
        match self {
            Self::EquityHistorical | Self::RunQuery | Self::IngestBatch => "analyst",
            Self::ToolCall | Self::UdfRun => "udf_runner",
        }
    }

    fn policy_table(self) -> &'static str {
        match self {
            Self::EquityHistorical => "market.equity_historical",
            Self::RunQuery => "analytics.query_run",
            Self::IngestBatch => "market.ingest_batch",
            Self::ToolCall => "runtime.tool_call",
            Self::UdfRun => "runtime.udf_run",
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
    pub fn new(config: PolicyEnforcementConfig, hook_backend: B) -> Self {
        Self {
            config,
            hook_backend,
        }
    }

    pub fn udf_run(&mut self, request: UdfRequest) -> Result<Value> {
        secure_udf_run_with_backend(&self.config, request, &mut self.hook_backend)
    }

    pub fn hook_backend(&self) -> &B {
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

pub fn secure_endpoint_response(
    config: &PolicyEnforcementConfig,
    provider: &str,
    symbol: &str,
) -> Result<Value> {
    let mut backend = tdw_hooks::SystemHookHandlerBackend::default();
    secure_endpoint_response_with_backend(config, provider, symbol, &mut backend)
}

pub fn secure_endpoint_response_with_backend(
    config: &PolicyEnforcementConfig,
    provider: &str,
    symbol: &str,
    backend: &mut impl HookHandlerBackend,
) -> Result<Value> {
    secure_endpoint_by_name_with_backend(config, "equity_historical", provider, symbol, backend)
}

pub fn secure_endpoint_by_name(
    config: &PolicyEnforcementConfig,
    endpoint: &str,
    provider: &str,
    symbol: &str,
) -> Result<Value> {
    let mut backend = tdw_hooks::SystemHookHandlerBackend::default();
    secure_endpoint_by_name_with_backend(config, endpoint, provider, symbol, &mut backend)
}

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

pub fn secure_udf_run(config: &PolicyEnforcementConfig, request: UdfRequest) -> Result<Value> {
    let mut backend = tdw_hooks::SystemHookHandlerBackend::default();
    secure_udf_run_with_backend(config, request, &mut backend)
}

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
        MaskMode::Last4 => match value.as_str() {
            Some(text) => {
                let suffix = text
                    .chars()
                    .rev()
                    .take(4)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect::<String>();
                Value::String(format!("***{suffix}"))
            }
            None => Value::String("***".to_string()),
        },
    }
}
