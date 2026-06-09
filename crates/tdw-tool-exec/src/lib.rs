#![forbid(unsafe_code)]

//! Phase 1 of the tool-execution backend: a `ToolExecutor` that resolves a registry
//! `tool`'s [`tdw_agent::ToolImplementation`] binding and dispatches it to a concrete
//! backend.
//!
//! Executed variants (Phase 1): `Builtin` (in-process [`tdw_tools::ToolRegistry`]) and
//! `Command` with `background: false` (run via a hardened direct [`std::process::Command`]
//! with command validation, a deny-by-default allow-list, and a timeout).
//!
//! `Unbound` returns [`ExecError::Unbound`] (the MCP `-32601` path). `Http`, `Mcp`, `Pty`,
//! `Wasm`, `Ref`, and `Command { background: true }` return [`ExecError::NotYetSupported`]
//! (later phases): `Http`/`Mcp` are honestly deferred because a fresh backend has neither
//! credential wiring nor server resolution, so they cannot run yet.

#![deny(clippy::pedantic, clippy::nursery)]
pub mod loop_guard;

use std::collections::BTreeMap;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

pub use loop_guard::{GuardDecision, LoopGuard, LoopGuardBuilder};
use serde_json::Value;
use tdw_agent::{EntityKind, Registry, Tool, ToolEffect, ToolImplementation, entity_from_resource};
use tdw_tools::{RegisteredTool, ToolDefinition, ToolHandler, ToolRegistry};
use thiserror::Error;

mod receipt;
pub use receipt::{ChainBreak, ChainStatus, GENESIS_PREV_HASH, ReceiptLog, ToolReceipt};

/// Env var holding the comma-separated allow-list of bare command names permitted for
/// `Command` execution. Unset/empty means *deny all* command execution.
const ALLOWED_COMMANDS_ENV: &str = "TDW_TOOL_EXEC_ALLOWED_COMMANDS";
/// Env var selecting the [`AutonomyLevel`] (`full` | `supervised` | `readonly`). Unset or
/// unparsable means [`AutonomyLevel::Full`] (today's behavior: every effect is allowed).
const AUTONOMY_ENV: &str = "TDW_TOOL_EXEC_AUTONOMY";
/// Env var overriding the command execution timeout, in whole seconds.
const TIMEOUT_SECS_ENV: &str = "TDW_TOOL_EXEC_TIMEOUT_SECS";
/// Env var toggling opt-in argument validation against the tool's `input_schema`.
/// `"1"`/`"true"`/`"on"`/`"yes"` => validate; anything else (incl. unset) => off.
const VALIDATE_ARGS_ENV: &str = "TDW_TOOL_EXEC_VALIDATE_ARGS";
/// Default command execution timeout when [`TIMEOUT_SECS_ENV`] is unset/unparsable.
const DEFAULT_TIMEOUT_SECS: u64 = 30;
/// How often the timeout loop polls the child process for exit.
const POLL_INTERVAL: Duration = Duration::from_millis(25);
/// Windows `CREATE_NO_WINDOW` process-creation flag (avoids a console flash).
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// A failure while resolving or executing a registry tool's implementation.
#[derive(Debug, Error)]
pub enum ExecError {
    /// The tool's implementation is [`ToolImplementation::Unbound`]: it is listed but not
    /// runnable. The MCP layer maps this to `-32601`.
    #[error("tool implementation is unbound")]
    Unbound,
    /// No `tool` resource with this name exists in the registry.
    #[error("tool not found in registry: {0}")]
    ToolNotFound(String),
    /// A `Builtin` implementation named a handler that is not registered.
    #[error("builtin handler not found: {0}")]
    HandlerNotFound(String),
    /// Command execution was rejected by policy (allow-list / not enabled).
    #[error("not permitted: {0}")]
    NotPermitted(String),
    /// The implementation variant (or a sub-mode of it) is deferred to a later phase.
    #[error("not yet supported: {0}")]
    NotYetSupported(&'static str),
    /// The backend (process) returned an error while executing.
    #[error("backend error: {0}")]
    Backend(String),
    /// The tool resource could not be re-typed, or arguments were malformed.
    #[error("bad arguments: {0}")]
    BadArguments(String),
    /// The tool's declared [`ToolEffect`] risk exceeds what the configured [`AutonomyLevel`]
    /// permits, so execution was refused *before dispatch*. This is a visible, recoverable
    /// observation (the agent loop can surface it and choose to escalate autonomy), not a
    /// silent failure.
    #[error("tool {tool} blocked: effect {effect} exceeds autonomy level {level}")]
    Blocked {
        /// The registry tool name that was refused.
        tool: String,
        /// The tool's declared effect (`read-only` | `write-safe` | `destructive`).
        effect: &'static str,
        /// The active autonomy level (`full` | `supervised` | `read-only`).
        level: &'static str,
    },
    /// The request arguments did not satisfy the tool's declared `input_schema`. Surfaced
    /// before dispatch; a visible, recoverable validation observation, never a panic. The MCP
    /// layer can map this to an invalid-params (`-32602`) style code.
    #[error("invalid arguments for tool {tool}: {reason}")]
    InvalidArguments {
        /// The name of the tool whose arguments failed validation.
        tool: String,
        /// A human-readable description of the first constraint that failed.
        reason: String,
    },
}

/// How much of a tool's declared [`ToolEffect`] risk the executor is permitted to run before
/// dispatch, mirroring readonly/supervised/full autonomy tiers.
///
/// The default is [`AutonomyLevel::Full`], which allows every effect — so an executor that does
/// not opt in behaves exactly as it did before this gate existed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AutonomyLevel {
    /// Allow every effect (`ReadOnly`, `WriteSafe`, `Destructive`). Behavior-preserving default.
    #[default]
    Full,
    /// Allow `ReadOnly` and `WriteSafe`; refuse `Destructive`.
    Supervised,
    /// Allow only `ReadOnly`; refuse `WriteSafe` and `Destructive`.
    ReadOnly,
}

impl AutonomyLevel {
    /// Stable lowercase label for this level, used in [`ExecError::Blocked`] and logging.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Supervised => "supervised",
            Self::ReadOnly => "read-only",
        }
    }

    /// Resolve the level from the `TDW_TOOL_EXEC_AUTONOMY` environment variable.
    ///
    /// Accepts `full`, `supervised`, or `readonly`/`read-only` (case-insensitive). Any unset,
    /// empty, or unrecognized value falls back to [`AutonomyLevel::Full`], preserving today's
    /// behavior unless an operator explicitly opts in.
    #[must_use]
    pub fn from_env() -> Self {
        std::env::var(AUTONOMY_ENV)
            .ok()
            .map_or(Self::Full, |raw| Self::parse(&raw))
    }

    /// Parse a textual level, defaulting to [`AutonomyLevel::Full`] for unrecognized input.
    ///
    /// Factored out of [`AutonomyLevel::from_env`] so the mapping can be tested without
    /// mutating process-global environment variables.
    #[must_use]
    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "supervised" => Self::Supervised,
            "readonly" | "read-only" => Self::ReadOnly,
            // "full" and anything unrecognized => behavior-preserving default.
            _ => Self::Full,
        }
    }

    /// Decide whether a tool with the given declared `effect` may run at this level.
    ///
    /// `ReadOnly` permits only `ToolEffect::ReadOnly`; `Supervised` additionally permits
    /// `WriteSafe`; `Full` permits everything.
    const fn permits(self, effect: ToolEffect) -> bool {
        match self {
            Self::Full => true,
            Self::Supervised => !matches!(effect, ToolEffect::Destructive),
            Self::ReadOnly => matches!(effect, ToolEffect::ReadOnly),
        }
    }
}

/// Stable lowercase label for a [`ToolEffect`], used in [`ExecError::Blocked`].
const fn effect_label(effect: ToolEffect) -> &'static str {
    match effect {
        ToolEffect::ReadOnly => "read-only",
        ToolEffect::WriteSafe => "write-safe",
        ToolEffect::Destructive => "destructive",
    }
}

/// Exact `Backend` message produced on a command timeout (literal at the timeout site in
/// [`dispatch_command`]). Matched verbatim so only this specific `Backend` failure is treated
/// as recoverable.
const TIMEOUT_BACKEND_MSG: &str = "command timed out";

impl ExecError {
    /// Whether the agent could plausibly recover by adjusting its request and retrying.
    #[must_use]
    pub fn is_recoverable(&self) -> bool {
        match self {
            Self::BadArguments(_)
            | Self::InvalidArguments { .. }
            | Self::ToolNotFound(_)
            | Self::HandlerNotFound(_) => true,
            Self::Unbound
            | Self::NotPermitted(_)
            | Self::NotYetSupported(_)
            | Self::Blocked { .. } => false,
            // Temporary heuristic pending an `ExecError::Timeout` variant: only the exact
            // timeout message is recoverable; all other backend failures stay non-recoverable.
            Self::Backend(message) => message == TIMEOUT_BACKEND_MSG,
        }
    }

    /// Stable machine tag for this variant, independent of its [`Display`](std::fmt::Display)
    /// text.
    #[must_use]
    fn reason_tag(&self) -> &'static str {
        match self {
            Self::Unbound => "unbound",
            Self::ToolNotFound(_) => "tool_not_found",
            Self::HandlerNotFound(_) => "handler_not_found",
            Self::NotPermitted(_) => "not_permitted",
            Self::NotYetSupported(_) => "not_yet_supported",
            Self::Backend(message) if message == TIMEOUT_BACKEND_MSG => "backend_timeout",
            Self::Backend(_) => "backend",
            Self::BadArguments(_) => "bad_arguments",
            Self::Blocked { .. } => "blocked",
            Self::InvalidArguments { .. } => "invalid_arguments",
        }
    }

    /// Short stable hint describing how the agent might respond to this variant.
    #[must_use]
    fn variant_hint(&self) -> &'static str {
        match self {
            Self::Unbound => "tool is listed but not runnable; choose another tool",
            Self::ToolNotFound(_) => "no such tool; check the tool name",
            Self::HandlerNotFound(_) => "builtin handler is not registered; check the name",
            Self::NotPermitted(_) => "blocked by policy; do not retry",
            Self::NotYetSupported(_) => "this implementation variant is not available yet",
            Self::Backend(message) if message == TIMEOUT_BACKEND_MSG => {
                "the command timed out; retry may succeed"
            }
            Self::Backend(_) => "backend failure; do not blindly retry",
            Self::BadArguments(_) | Self::InvalidArguments { .. } => "fix the arguments and retry",
            Self::Blocked { .. } => "blocked by autonomy policy; do not retry without escalation",
        }
    }

    /// Render this error as a model-visible tool-result observation.
    ///
    /// Shape: `{tool, is_error: true, reason, detail, hint}`.
    #[must_use]
    pub fn to_observation(&self, tool: &str) -> Value {
        serde_json::json!({
            "tool": tool,
            "is_error": true,
            "reason": self.reason_tag(),
            "detail": self.to_string(),
            "hint": self.variant_hint(),
        })
    }
}

/// Per-tool failure counter that caps how many times a given tool may fail before the agent
/// should stop retrying it. Pure and synchronous: no async, no I/O.
#[derive(Clone, Debug, Default)]
pub struct FailureBudget {
    per_tool: BTreeMap<String, u32>,
    cap: u32,
}

impl FailureBudget {
    /// A new budget allowing up to `cap` failures per tool.
    #[must_use]
    pub const fn new(cap: u32) -> Self {
        Self {
            per_tool: BTreeMap::new(),
            cap,
        }
    }

    /// Record a failure for `tool`, returning whether the tool is still within budget.
    pub fn record(&mut self, tool: &str) -> bool {
        let count = self
            .per_tool
            .entry(tool.to_string())
            .and_modify(|count| *count = count.saturating_add(1))
            .or_insert(1);
        *count <= self.cap
    }
}

/// The structured result of a successful tool execution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolOutcome {
    /// The structured JSON value returned by the backend.
    pub structured: Value,
}

/// Policy governing direct `Command` execution: the deny-by-default allow-list and the
/// per-command timeout.
///
/// The production default is [`CommandPolicy::from_env`], which reads
/// `TDW_TOOL_EXEC_ALLOWED_COMMANDS` (comma-separated bare command names; unset/empty =
/// *deny all*) and `TDW_TOOL_EXEC_TIMEOUT_SECS` (whole seconds; default 30). Tests may build
/// an explicit policy to avoid mutating process-global environment variables.
#[derive(Clone, Debug)]
pub struct CommandPolicy {
    /// `None` => deny all command execution. `Some(list)` => only these bare names may run.
    allowed: Option<Vec<String>>,
    /// Per-command wall-clock timeout before the child is killed.
    timeout: Duration,
}

impl CommandPolicy {
    /// Build a policy from explicit values (primarily for tests / embedders).
    ///
    /// `allowed = None` denies all command execution; `Some(list)` permits exactly the bare
    /// command names in `list`.
    #[must_use]
    pub const fn new(allowed: Option<Vec<String>>, timeout: Duration) -> Self {
        Self { allowed, timeout }
    }

    /// Build a policy from the `TDW_TOOL_EXEC_ALLOWED_COMMANDS` / `TDW_TOOL_EXEC_TIMEOUT_SECS`
    /// environment variables (the production default).
    #[must_use]
    pub fn from_env() -> Self {
        let allowed = std::env::var(ALLOWED_COMMANDS_ENV).ok().and_then(|raw| {
            let names: Vec<String> = raw
                .split(',')
                .map(str::trim)
                .filter(|entry| !entry.is_empty())
                .map(str::to_string)
                .collect();
            // Unset OR present-but-empty both deny all execution.
            if names.is_empty() { None } else { Some(names) }
        });
        let timeout = std::env::var(TIMEOUT_SECS_ENV)
            .ok()
            .and_then(|raw| raw.trim().parse::<u64>().ok())
            // Reject 0 (instant-kill footgun); fall back to the default.
            .filter(|secs| *secs > 0)
            .map_or(
                Duration::from_secs(DEFAULT_TIMEOUT_SECS),
                Duration::from_secs,
            );
        Self { allowed, timeout }
    }

    /// Deny-by-default allow-list check for a bare `command` name.
    fn authorize(&self, command: &str) -> Result<(), ExecError> {
        match &self.allowed {
            None => Err(ExecError::NotPermitted(
                "command execution not enabled (set TDW_TOOL_EXEC_ALLOWED_COMMANDS)".to_string(),
            )),
            Some(list) if list.iter().any(|entry| entry == command) => Ok(()),
            Some(_) => Err(ExecError::NotPermitted(format!(
                "command not in allow-list: {command}"
            ))),
        }
    }
}

impl Default for CommandPolicy {
    fn default() -> Self {
        Self::from_env()
    }
}

/// Opt-in toggle for validating request `args` against the resolved tool's declared
/// `input_schema` *before* dispatch.
///
/// Default is [`SchemaValidation::Off`] (behavior-preserving): an executor that does not opt
/// in dispatches exactly as before. The production default is [`SchemaValidation::from_env`],
/// which reads `TDW_TOOL_EXEC_VALIDATE_ARGS`. Embedders may opt in explicitly via
/// [`ToolExecutor::with_arg_validation`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SchemaValidation {
    /// Do not validate args (behavior-preserving default).
    #[default]
    Off,
    /// Validate args against `input_schema`; reject mismatches with
    /// [`ExecError::InvalidArguments`].
    On,
}

impl SchemaValidation {
    /// Stable string form (`"off"` / `"on"`), for logging/diagnostics.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::On => "on",
        }
    }

    /// Resolve from `TDW_TOOL_EXEC_VALIDATE_ARGS` (the production default).
    ///
    /// `"1"`/`"true"`/`"on"`/`"yes"` (case-insensitive) => [`Self::On`]; anything else,
    /// including unset, => [`Self::Off`].
    #[must_use]
    pub fn from_env() -> Self {
        std::env::var(VALIDATE_ARGS_ENV)
            .ok()
            .map_or(Self::Off, |raw| Self::parse(&raw))
    }

    /// Parse a raw string into a mode (factored out for env-free testing).
    #[must_use]
    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "on" | "yes" => Self::On,
            _ => Self::Off,
        }
    }
}

/// Resolves a registry `tool`'s implementation binding and dispatches it to a backend.
///
/// Holds an in-process [`ToolRegistry`] for `Builtin` handlers; `Command { background: false }`
/// runs via a hardened direct [`std::process::Command`] governed by a [`CommandPolicy`].
///
/// The [`AutonomyLevel`] gate (default [`AutonomyLevel::Full`]) is consulted before dispatch and
/// refuses tools whose declared [`ToolEffect`] risk exceeds the level.
///
/// When [`SchemaValidation::On`] (via [`ToolExecutor::with_arg_validation`] or the
/// `TDW_TOOL_EXEC_VALIDATE_ARGS` env default), request `args` are checked against the resolved
/// tool's `input_schema` *before* dispatch; a mismatch returns
/// [`ExecError::InvalidArguments`] rather than reaching the backend.
///
/// Optionally records an append-only, hash-chained [`ReceiptLog`] of successful
/// executions (off by default; opt in via [`ToolExecutor::with_receipts`]). When
/// recording is enabled the executor is **not** `Sync` and must not be shared across
/// threads for concurrent `execute` (see [`ReceiptLog`]).
pub struct ToolExecutor {
    builtins: ToolRegistry,
    command_policy: CommandPolicy,
    autonomy: AutonomyLevel,
    validate: SchemaValidation,
    /// `None` = receipt recording disabled (default); `Some(log)` = recording.
    receipts: Option<ReceiptLog>,
}

impl Default for ToolExecutor {
    fn default() -> Self {
        Self {
            builtins: ToolRegistry::default(),
            command_policy: CommandPolicy::default(),
            // Production default: read the level from the environment (defaults to `Full`
            // when unset), mirroring `CommandPolicy::from_env`.
            autonomy: AutonomyLevel::from_env(),
            validate: SchemaValidation::from_env(),
            receipts: None,
        }
    }
}

impl ToolExecutor {
    /// A new executor with no registered `Builtin` handlers and an env-derived command policy.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Override the [`CommandPolicy`] (allow-list + timeout) for `Command` execution.
    #[must_use]
    pub fn with_command_policy(mut self, policy: CommandPolicy) -> Self {
        self.command_policy = policy;
        self
    }

    /// Set the [`AutonomyLevel`] gate consulted before dispatch.
    ///
    /// Defaults to [`AutonomyLevel::Full`] (every effect allowed), so calling this is an
    /// explicit opt-in to a stricter posture.
    #[must_use]
    pub const fn with_autonomy(mut self, level: AutonomyLevel) -> Self {
        self.autonomy = level;
        self
    }

    /// Enable the append-only, hash-chained [`ReceiptLog`].
    ///
    /// Off by default; calling this is an explicit opt-in. When not enabled there is
    /// zero behavioral or performance change versus an executor without receipts.
    ///
    /// A receipts-enabled executor is **not** `Sync`: do not share it across threads
    /// for concurrent `execute` (see [`ReceiptLog`]).
    #[must_use]
    pub fn with_receipts(mut self) -> Self {
        self.receipts = Some(ReceiptLog::new());
        self
    }

    /// Read-only access to the [`ReceiptLog`], or `None` if recording is disabled.
    #[must_use]
    pub const fn receipts(&self) -> Option<&ReceiptLog> {
        self.receipts.as_ref()
    }

    /// Opt into (or out of) pre-dispatch argument validation against the tool's
    /// `input_schema`.
    ///
    /// Defaults to the env-derived [`SchemaValidation::from_env`]; [`SchemaValidation::On`]
    /// makes [`Self::execute`] reject argument/schema mismatches with
    /// [`ExecError::InvalidArguments`] before any backend runs.
    #[must_use]
    pub const fn with_arg_validation(mut self, mode: SchemaValidation) -> Self {
        self.validate = mode;
        self
    }

    /// Register an in-process `Builtin` handler under `name`.
    ///
    /// `name` is used both as the registered tool name and the permission pattern; it must
    /// be a valid dotted tool name (ASCII alphanumeric plus `_`/`-`, dot-separated).
    ///
    /// # Errors
    ///
    /// Returns [`ExecError::BadArguments`] if `name` is not a valid tool name or already
    /// registered.
    pub fn with_builtin(mut self, name: &str, handler: ToolHandler) -> Result<Self, ExecError> {
        let definition = ToolDefinition {
            name: name.to_string(),
            description: format!("builtin handler {name}"),
            input_schema: serde_json::json!({ "type": "object" }),
            output_schema: serde_json::json!({ "type": "object" }),
            permission_pattern: name.to_string(),
        };
        self.builtins
            .register(RegisteredTool::new(definition, handler))
            .map_err(|error| ExecError::BadArguments(error.to_string()))?;
        Ok(self)
    }

    /// Resolve `name`'s `tool` resource in `registry`, then dispatch its implementation.
    ///
    /// # Errors
    ///
    /// Returns [`ExecError::ToolNotFound`] if no `tool` named `name` is registered,
    /// [`ExecError::Unbound`] for an unbound tool, [`ExecError::InvalidArguments`] when
    /// argument validation is enabled (see [`Self::with_arg_validation`]) and `args` do not
    /// satisfy the tool's `input_schema`, [`ExecError::NotYetSupported`] for a deferred
    /// variant, [`ExecError::NotPermitted`] for a command rejected by policy,
    /// [`ExecError::BadArguments`] for a malformed tool resource or command,
    /// [`ExecError::Blocked`] when the tool's declared effect exceeds the configured
    /// [`AutonomyLevel`], or
    /// [`ExecError::Backend`] for a process failure.
    pub fn execute(
        &self,
        registry: &Registry,
        name: &str,
        args: &Value,
    ) -> Result<ToolOutcome, ExecError> {
        let resource = registry
            .get(EntityKind::Tool, name)
            .ok_or_else(|| ExecError::ToolNotFound(name.to_string()))?;
        let tool: Tool = entity_from_resource(resource)
            .map_err(|error| ExecError::BadArguments(error.to_string()))?;

        // Risk gate: refuse before dispatch if the tool's declared effect exceeds the
        // configured autonomy level. `Full` (the default) permits everything, so this is a
        // no-op unless an embedder opted in.
        if !self.autonomy.permits(tool.effect) {
            return Err(ExecError::Blocked {
                tool: name.to_string(),
                effect: effect_label(tool.effect),
                level: self.autonomy.as_str(),
            });
        }

        if matches!(self.validate, SchemaValidation::On)
            && let Err(reason) = validate_args(&tool.input_schema, args)
        {
            return Err(ExecError::InvalidArguments {
                tool: name.to_string(),
                reason,
            });
        }

        let result = match tool.implementation {
            ToolImplementation::Unbound => Err(ExecError::Unbound),
            ToolImplementation::Builtin { handler } => self.dispatch_builtin(&handler, args),
            ToolImplementation::Command {
                command,
                args: command_args,
                background,
            } => {
                if background {
                    Err(ExecError::NotYetSupported("background command (Phase 2)"))
                } else {
                    dispatch_command(&self.command_policy, &command, &command_args)
                }
            }
            ToolImplementation::Http { .. } => {
                Err(ExecError::NotYetSupported("http (needs credential wiring)"))
            }
            ToolImplementation::Mcp { .. } => Err(ExecError::NotYetSupported(
                "mcp proxy (needs server resolution)",
            )),
            ToolImplementation::Pty { .. } => Err(ExecError::NotYetSupported("pty (Phase 3)")),
            ToolImplementation::Wasm { .. } => Err(ExecError::NotYetSupported("wasm (Phase 4)")),
            ToolImplementation::Ref { .. } => {
                Err(ExecError::NotYetSupported("ref resolution (Phase 4)"))
            }
        };

        // Record a receipt only on success and only when opted in. Errors are not
        // receipts: the chain tracks completed effects, not rejected attempts.
        if let (Ok(outcome), Some(log)) = (&result, &self.receipts) {
            log.append(name, args, outcome);
        }
        result
    }

    fn dispatch_builtin(&self, handler: &str, args: &Value) -> Result<ToolOutcome, ExecError> {
        let registered = self
            .builtins
            .get(handler)
            .ok_or_else(|| ExecError::HandlerNotFound(handler.to_string()))?;
        let structured = registered
            .call(args.clone())
            .map_err(|error| ExecError::Backend(error.to_string()))?;
        Ok(ToolOutcome { structured })
    }
}

/// Validate `args` against a JSON-Schema-ish `schema`. Supports a pragmatic subset:
/// root/object `type`, `required: [..]`, and per-property `type` from `properties`.
/// Unknown keywords and absent constraints are ignored (lenient by design). Never panics.
fn validate_args(schema: &Value, args: &Value) -> Result<(), String> {
    // If schema is not an object (e.g. `true`/null/missing), accept (nothing to check).
    let Some(schema_obj) = schema.as_object() else {
        return Ok(());
    };
    // Root `type` check (default "object" per Tool contract: root must be an object).
    let root_type = schema_obj.get("type");
    check_type_spec(root_type, "object", args).map_err(|message| format!("root: {message}"))?;
    // Only object arguments have required/properties semantics. Compound schemas like
    // `["object", "null"]` are accepted for null and validated for objects.
    if type_spec_includes(root_type, "object", true) && args.is_object() {
        let obj = args
            .as_object()
            .ok_or_else(|| "expected object arguments".to_string())?;
        // Required fields present (and non-null).
        if let Some(req) = schema_obj.get("required").and_then(Value::as_array) {
            for field in req.iter().filter_map(Value::as_str) {
                match obj.get(field) {
                    None | Some(Value::Null) => {
                        return Err(format!("missing required field: {field}"));
                    }
                    Some(_) => {}
                }
            }
        }
        // Per-property type checks for present keys only (extra keys allowed: open-world).
        if let Some(props) = schema_obj.get("properties").and_then(Value::as_object) {
            for (key, pschema) in props {
                if let Some(actual) = obj.get(key)
                    && let Some(expected) = pschema.as_object().and_then(|p| p.get("type"))
                {
                    check_type_spec(Some(expected), "object", actual)
                        .map_err(|message| format!("field {key}: {message}"))?;
                }
            }
        }
    }
    Ok(())
}

fn check_type_spec(
    expected: Option<&Value>,
    default_type: &'static str,
    value: &Value,
) -> Result<(), String> {
    match expected {
        None => check_type(default_type, value),
        Some(Value::String(expected)) => check_type(expected, value),
        Some(Value::Array(types)) => {
            let names = types.iter().filter_map(Value::as_str);
            if names
                .clone()
                .any(|expected| check_type(expected, value).is_ok())
            {
                Ok(())
            } else if let Some(first) = names.into_iter().next() {
                check_type(first, value)
            } else {
                Ok(())
            }
        }
        // Unknown/compound forms are accepted leniently.
        Some(_) => Ok(()),
    }
}

fn type_spec_includes(
    expected: Option<&Value>,
    needle: &'static str,
    default_when_absent: bool,
) -> bool {
    match expected {
        None => default_when_absent,
        Some(Value::String(expected)) => expected == needle,
        Some(Value::Array(types)) => types.iter().filter_map(Value::as_str).any(|t| t == needle),
        Some(_) => false,
    }
}

/// Single-keyword type predicate over the JSON-Schema primitive type names.
/// `integer` => JSON number that is i64/u64 (no fractional part). `number` => any JSON number.
/// Unknown type names are accepted (lenient). Never panics.
fn check_type(expected: &str, value: &Value) -> Result<(), String> {
    let ok = match expected {
        "string" => value.is_string(),
        "number" => value.is_number(),
        "integer" => {
            value.is_i64()
                || value.is_u64()
                || value
                    .as_f64()
                    .is_some_and(|number| number.is_finite() && number.fract() == 0.0)
        }
        "boolean" => value.is_boolean(),
        "array" => value.is_array(),
        "object" => value.is_object(),
        "null" => value.is_null(),
        // Unknown/compound (e.g. type arrays) => don't reject.
        _ => true,
    };
    if ok {
        Ok(())
    } else {
        Err(format!(
            "expected {expected}, got {}",
            json_type_name(value)
        ))
    }
}

/// Short JSON type name for error messages.
const fn json_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Reject a `command` that is anything other than a bare program name.
///
/// Bare names only: PATH lookup, no absolute/relative paths, no shell metacharacters, no
/// control characters, and no `..` traversal.
fn validate_command(command: &str) -> Result<(), ExecError> {
    if command.is_empty() {
        return Err(ExecError::BadArguments(
            "command must not be empty".to_string(),
        ));
    }
    if command.contains("..") {
        return Err(ExecError::BadArguments(format!(
            "command must be a bare name (no path traversal): {command}"
        )));
    }
    for ch in command.chars() {
        if matches!(ch, '/' | '\\' | ';' | '&' | '|' | '<' | '>' | '`' | '$')
            || ch.is_control()
            || ch.is_whitespace()
        {
            return Err(ExecError::BadArguments(format!(
                "command must be a bare name (illegal character {ch:?}): {command}"
            )));
        }
    }
    Ok(())
}

/// Run a validated, allow-listed bare command directly with a timeout and captured output.
///
/// stdout/stderr are drained on dedicated threads to avoid pipe-buffer deadlock; the child
/// is polled for exit and killed on timeout.
///
/// # Security
///
/// The command name is validated (bare name, no metachars/paths) and allow-listed
/// (deny-by-default). The `args` come from the **tool definition**, not the request, so a
/// caller cannot influence argv. Note: allow-listing a shell interpreter (`cmd`, `bash`,
/// `powershell`) trusts the registry author's `Command{args}` — an operator who allow-lists
/// an interpreter is trusting whoever can write tool definitions. This is a registry-trust
/// boundary, not a remote bypass.
fn dispatch_command(
    policy: &CommandPolicy,
    command: &str,
    command_args: &[String],
) -> Result<ToolOutcome, ExecError> {
    validate_command(command)?;
    policy.authorize(command)?;

    let mut builder = Command::new(command);
    builder
        .args(command_args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        builder.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = builder
        .spawn()
        .map_err(|error| ExecError::Backend(format!("failed to spawn command: {error}")))?;

    let stdout_reader = child.stdout.take().map(spawn_reader);
    let stderr_reader = child.stderr.take().map(spawn_reader);

    let timeout = policy.timeout;
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if started.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = join_reader(stdout_reader);
                    let _ = join_reader(stderr_reader);
                    return Err(ExecError::Backend("command timed out".to_string()));
                }
                thread::sleep(POLL_INTERVAL);
            }
            Err(error) => {
                let _ = child.kill();
                let _ = join_reader(stdout_reader);
                let _ = join_reader(stderr_reader);
                return Err(ExecError::Backend(format!(
                    "failed to await command: {error}"
                )));
            }
        }
    };

    let stdout = join_reader(stdout_reader);
    let stderr = join_reader(stderr_reader);
    let exit_code = status.code().map_or(Value::Null, Value::from);

    Ok(ToolOutcome {
        structured: serde_json::json!({
            "stdout": String::from_utf8_lossy(&stdout),
            "stderr": String::from_utf8_lossy(&stderr),
            "exitCode": exit_code,
        }),
    })
}

/// Spawn a thread that drains a piped stream into a byte buffer.
fn spawn_reader<R>(mut stream: R) -> thread::JoinHandle<Vec<u8>>
where
    R: std::io::Read + Send + 'static,
{
    thread::spawn(move || {
        let mut buffer = Vec::new();
        let _ = stream.read_to_end(&mut buffer);
        buffer
    })
}

/// Join a reader thread, returning its captured bytes (empty on join failure / absent stream).
fn join_reader(handle: Option<thread::JoinHandle<Vec<u8>>>) -> Vec<u8> {
    handle
        .map(|handle| handle.join().unwrap_or_default())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tdw_agent::{Adaptivity, EntityMeta, Origin, RegistryEntity, Source, Tier, ToolEffect};

    // The workspace forbids `unsafe_code` (so `std::env::set_var`, which is `unsafe` in
    // edition 2024, is unavailable). Instead of mutating process-global environment variables
    // — which would race other tests — these tests inject an explicit, per-executor
    // [`CommandPolicy`]. This is race-free by construction: no shared global state is touched.
    // [`CommandPolicy::from_env`] (the production default) reuses the identical allow-list and
    // timeout logic exercised here via [`CommandPolicy::new`].
    fn policy(allowed: Option<&[&str]>, timeout: Duration) -> CommandPolicy {
        CommandPolicy::new(
            allowed.map(|names| names.iter().map(|name| (*name).to_string()).collect()),
            timeout,
        )
    }

    fn allow(names: &[&str]) -> CommandPolicy {
        policy(Some(names), Duration::from_secs(30))
    }

    /// `(command, args)` for a shell that runs `script`, portable across CI runners.
    /// On Windows: `cmd /c <script>`; on Unix: `sh -c "<script>"`.
    fn shell_command(script: &str) -> (String, Vec<String>) {
        if cfg!(windows) {
            (
                "cmd".to_string(),
                vec!["/c".to_string(), script.to_string()],
            )
        } else {
            ("sh".to_string(), vec!["-c".to_string(), script.to_string()])
        }
    }

    /// The shell binary name, for the executor allow-list.
    fn shell_bin() -> &'static str {
        if cfg!(windows) { "cmd" } else { "sh" }
    }

    fn tool_with_impl(name: &str, implementation: ToolImplementation) -> Tool {
        Tool {
            meta: EntityMeta::new(
                name,
                name,
                "0.1.0",
                Origin {
                    tier: Tier::Domain,
                    source: Source::Internal,
                },
                Adaptivity::None,
                false,
            )
            .with_title(name)
            .with_description("tool-exec test fixture"),
            input_schema: serde_json::json!({ "type": "object" }),
            output_schema: None,
            effect: ToolEffect::ReadOnly,
            idempotent: true,
            open_world: false,
            implementation,
        }
    }

    /// Like [`tool_with_impl`] but with an explicit declared [`ToolEffect`], for exercising the
    /// autonomy gate.
    fn tool_with_effect(
        name: &str,
        effect: ToolEffect,
        implementation: ToolImplementation,
    ) -> Tool {
        let mut tool = tool_with_impl(name, implementation);
        tool.effect = effect;
        tool
    }

    /// Build a `Builtin` tool whose `input_schema` is `schema`, dispatching to `handler`.
    /// Used to prove the pre-dispatch arg-validation gate against a chosen schema.
    fn tool_with_schema(name: &str, schema: Value, handler: &str) -> Tool {
        let mut tool = tool_with_impl(
            name,
            ToolImplementation::Builtin {
                handler: handler.to_string(),
            },
        );
        tool.input_schema = schema;
        tool
    }

    fn registry_with(tool: &Tool) -> Registry {
        Registry::from_resources([tool
            .to_resource()
            .unwrap_or_else(|error| panic!("tool resource: {error}"))])
        .unwrap_or_else(|error| panic!("registry should build: {error}"))
    }

    // Fixed by the `ToolHandler = fn(Value) -> Result<Value>` type alias; used as a fn
    // pointer so the Result cannot be unwrapped away.
    #[allow(clippy::unnecessary_wraps)]
    #[allow(clippy::needless_pass_by_value)]
    fn echo_handler(input: Value) -> tdw_tools::Result<Value> {
        Ok(serde_json::json!({ "echoed": input }))
    }

    #[test]
    fn command_allow_listed_runs_and_returns_stdout() {
        let (command, args) = shell_command("echo hi");
        let tool = tool_with_impl(
            "tool.exec.echo",
            ToolImplementation::Command {
                command,
                args,
                background: false,
            },
        );
        let registry = registry_with(&tool);
        let executor = ToolExecutor::new().with_command_policy(allow(&[shell_bin()]));

        let outcome = executor
            .execute(&registry, "tool.exec.echo", &serde_json::json!({}))
            .unwrap_or_else(|error| panic!("command should execute: {error}"));

        assert_eq!(outcome.structured["exitCode"], 0);
        assert!(
            outcome.structured["stdout"]
                .as_str()
                .unwrap_or_default()
                .contains("hi")
        );
    }

    #[test]
    fn command_without_allow_list_is_not_permitted() {
        let (command, args) = shell_command("echo hi");
        let tool = tool_with_impl(
            "tool.exec.denied",
            ToolImplementation::Command {
                command,
                args,
                background: false,
            },
        );
        let registry = registry_with(&tool);
        // `None` allow-list = deny all command execution.
        let executor =
            ToolExecutor::new().with_command_policy(policy(None, Duration::from_secs(30)));

        let error = executor
            .execute(&registry, "tool.exec.denied", &serde_json::json!({}))
            .expect_err("disabled command execution must error");
        assert!(matches!(error, ExecError::NotPermitted(_)));
    }

    #[test]
    fn command_with_shell_metachar_is_bad_arguments() {
        let tool = tool_with_impl(
            "tool.exec.meta",
            ToolImplementation::Command {
                command: "foo;bar".to_string(),
                args: Vec::new(),
                background: false,
            },
        );
        let registry = registry_with(&tool);
        // Even allow-listed, the shell-metachar name is rejected before the allow-list check.
        let executor = ToolExecutor::new().with_command_policy(allow(&["foo;bar"]));

        let error = executor
            .execute(&registry, "tool.exec.meta", &serde_json::json!({}))
            .expect_err("shell metachar command must error");
        assert!(matches!(error, ExecError::BadArguments(_)));
    }

    #[test]
    fn command_not_in_allow_list_is_not_permitted() {
        let tool = tool_with_impl(
            "tool.exec.other",
            ToolImplementation::Command {
                command: "powershell".to_string(),
                args: Vec::new(),
                background: false,
            },
        );
        let registry = registry_with(&tool);
        let executor = ToolExecutor::new().with_command_policy(allow(&[shell_bin()]));

        let error = executor
            .execute(&registry, "tool.exec.other", &serde_json::json!({}))
            .expect_err("command outside allow-list must error");
        assert!(matches!(error, ExecError::NotPermitted(_)));
    }

    #[test]
    fn command_exceeding_timeout_is_killed() {
        // A command that runs ~2s on both platforms; the 200ms timeout must kill it.
        // `cmd /c sleep 2` does not exist on Windows, so use a per-OS script.
        let script = if cfg!(windows) {
            "ping -n 3 127.0.0.1 >NUL"
        } else {
            "sleep 2"
        };
        let (command, args) = shell_command(script);
        let tool = tool_with_impl(
            "tool.exec.slow",
            ToolImplementation::Command {
                command,
                args,
                background: false,
            },
        );
        let registry = registry_with(&tool);
        let executor = ToolExecutor::new()
            .with_command_policy(policy(Some(&[shell_bin()]), Duration::from_millis(200)));

        let error = executor
            .execute(&registry, "tool.exec.slow", &serde_json::json!({}))
            .expect_err("slow command must time out");
        assert!(matches!(error, ExecError::Backend(message) if message == "command timed out"));
    }

    #[test]
    fn http_tool_is_not_yet_supported() {
        let tool = tool_with_impl(
            "tool.exec.http",
            ToolImplementation::Http {
                url: tdw_agent::Reference::Http("https://example.test/hook".to_string()),
            },
        );
        let registry = registry_with(&tool);
        let executor = ToolExecutor::new();

        let error = executor
            .execute(&registry, "tool.exec.http", &serde_json::json!({}))
            .expect_err("http must defer");
        assert!(matches!(error, ExecError::NotYetSupported(_)));
    }

    #[test]
    fn mcp_tool_is_not_yet_supported() {
        let tool = tool_with_impl(
            "tool.exec.mcp",
            ToolImplementation::Mcp {
                server: "some.server".to_string(),
                tool_name: "remote.tool".to_string(),
            },
        );
        let registry = registry_with(&tool);
        let executor = ToolExecutor::new();

        let error = executor
            .execute(&registry, "tool.exec.mcp", &serde_json::json!({}))
            .expect_err("mcp must defer");
        assert!(matches!(error, ExecError::NotYetSupported(_)));
    }

    #[test]
    fn unbound_tool_returns_unbound_error() {
        let tool = tool_with_impl("tool.exec.unbound", ToolImplementation::Unbound);
        let registry = registry_with(&tool);
        let executor = ToolExecutor::new();

        let error = executor
            .execute(&registry, "tool.exec.unbound", &serde_json::json!({}))
            .expect_err("unbound tool must error");
        assert!(matches!(error, ExecError::Unbound));
    }

    #[test]
    fn builtin_tool_dispatches_registered_handler() {
        let tool = tool_with_impl(
            "tool.exec.builtin",
            ToolImplementation::Builtin {
                handler: "tool.exec.builtin.handler".to_string(),
            },
        );
        let registry = registry_with(&tool);
        let executor = ToolExecutor::new()
            .with_builtin("tool.exec.builtin.handler", echo_handler)
            .unwrap_or_else(|error| panic!("builtin should register: {error}"));

        let outcome = executor
            .execute(
                &registry,
                "tool.exec.builtin",
                &serde_json::json!({ "ping": true }),
            )
            .unwrap_or_else(|error| panic!("builtin should execute: {error}"));

        assert_eq!(
            outcome.structured["echoed"],
            serde_json::json!({ "ping": true })
        );
    }

    #[test]
    fn missing_builtin_handler_reports_handler_not_found() {
        let tool = tool_with_impl(
            "tool.exec.missing",
            ToolImplementation::Builtin {
                handler: "tool.exec.absent".to_string(),
            },
        );
        let registry = registry_with(&tool);
        let executor = ToolExecutor::new();

        let error = executor
            .execute(&registry, "tool.exec.missing", &serde_json::json!({}))
            .expect_err("missing handler must error");
        assert!(matches!(error, ExecError::HandlerNotFound(name) if name == "tool.exec.absent"));
    }

    #[test]
    fn background_command_is_not_yet_supported() {
        let tool = tool_with_impl(
            "tool.exec.bg",
            ToolImplementation::Command {
                command: shell_bin().to_string(),
                args: Vec::new(),
                background: true,
            },
        );
        let registry = registry_with(&tool);
        let executor = ToolExecutor::new();

        let error = executor
            .execute(&registry, "tool.exec.bg", &serde_json::json!({}))
            .expect_err("background command must defer");
        assert!(matches!(error, ExecError::NotYetSupported(_)));
    }

    #[test]
    fn destructive_tool_under_supervised_is_blocked() {
        let (command, args) = shell_command("echo hi");
        let tool = tool_with_effect(
            "tool.exec.destructive",
            ToolEffect::Destructive,
            ToolImplementation::Command {
                command,
                args,
                background: false,
            },
        );
        let registry = registry_with(&tool);
        // Allow-list the shell so a non-blocked path WOULD run: the block must come from the
        // autonomy gate, not the command policy.
        let executor = ToolExecutor::new()
            .with_command_policy(allow(&[shell_bin()]))
            .with_autonomy(AutonomyLevel::Supervised);

        let error = executor
            .execute(&registry, "tool.exec.destructive", &serde_json::json!({}))
            .expect_err("destructive tool under supervised must be blocked");
        assert!(matches!(
            error,
            ExecError::Blocked {
                ref tool,
                effect: "destructive",
                level: "supervised",
            } if tool == "tool.exec.destructive"
        ));
    }

    #[test]
    fn destructive_tool_under_full_runs() {
        let (command, args) = shell_command("echo hi");
        let tool = tool_with_effect(
            "tool.exec.destructive.full",
            ToolEffect::Destructive,
            ToolImplementation::Command {
                command,
                args,
                background: false,
            },
        );
        let registry = registry_with(&tool);
        // Full is the explicit value here; behavior must match the unconfigured default.
        let executor = ToolExecutor::new()
            .with_command_policy(allow(&[shell_bin()]))
            .with_autonomy(AutonomyLevel::Full);

        let outcome = executor
            .execute(
                &registry,
                "tool.exec.destructive.full",
                &serde_json::json!({}),
            )
            .unwrap_or_else(|error| panic!("destructive tool under full should run: {error}"));
        assert_eq!(outcome.structured["exitCode"], 0);
    }

    #[test]
    fn destructive_tool_under_default_full_runs() {
        // Behavior-preservation guard: an executor that never opts into a stricter level still
        // runs a destructive tool (default autonomy == Full).
        let (command, args) = shell_command("echo hi");
        let tool = tool_with_effect(
            "tool.exec.destructive.default",
            ToolEffect::Destructive,
            ToolImplementation::Command {
                command,
                args,
                background: false,
            },
        );
        let registry = registry_with(&tool);
        let executor = ToolExecutor::new().with_command_policy(allow(&[shell_bin()]));

        let outcome = executor
            .execute(
                &registry,
                "tool.exec.destructive.default",
                &serde_json::json!({}),
            )
            .unwrap_or_else(|error| {
                panic!("default-autonomy destructive tool should run: {error}")
            });
        assert_eq!(outcome.structured["exitCode"], 0);
    }

    #[test]
    fn read_only_tool_under_read_only_runs() {
        let tool = tool_with_effect(
            "tool.exec.ro",
            ToolEffect::ReadOnly,
            ToolImplementation::Builtin {
                handler: "tool.exec.ro.handler".to_string(),
            },
        );
        let registry = registry_with(&tool);
        let executor = ToolExecutor::new()
            .with_builtin("tool.exec.ro.handler", echo_handler)
            .unwrap_or_else(|error| panic!("builtin should register: {error}"))
            .with_autonomy(AutonomyLevel::ReadOnly);

        let outcome = executor
            .execute(&registry, "tool.exec.ro", &serde_json::json!({ "ok": 1 }))
            .unwrap_or_else(|error| panic!("read-only tool under read-only should run: {error}"));
        assert_eq!(outcome.structured["echoed"], serde_json::json!({ "ok": 1 }));
    }

    #[test]
    fn write_safe_tool_under_read_only_is_blocked() {
        let tool = tool_with_effect(
            "tool.exec.ws",
            ToolEffect::WriteSafe,
            ToolImplementation::Builtin {
                handler: "tool.exec.ws.handler".to_string(),
            },
        );
        let registry = registry_with(&tool);
        let executor = ToolExecutor::new()
            .with_builtin("tool.exec.ws.handler", echo_handler)
            .unwrap_or_else(|error| panic!("builtin should register: {error}"))
            .with_autonomy(AutonomyLevel::ReadOnly);

        let error = executor
            .execute(&registry, "tool.exec.ws", &serde_json::json!({}))
            .expect_err("write-safe tool under read-only must be blocked");
        assert!(matches!(
            error,
            ExecError::Blocked {
                effect: "write-safe",
                level: "read-only",
                ..
            }
        ));
    }

    #[test]
    fn autonomy_parse_maps_known_levels_and_defaults_to_full() {
        // Exercise the parsing logic without mutating process-global env (mirrors the
        // CommandPolicy test note).
        assert_eq!(AutonomyLevel::parse("full"), AutonomyLevel::Full);
        assert_eq!(
            AutonomyLevel::parse("Supervised"),
            AutonomyLevel::Supervised
        );
        assert_eq!(AutonomyLevel::parse("readonly"), AutonomyLevel::ReadOnly);
        assert_eq!(AutonomyLevel::parse("read-only"), AutonomyLevel::ReadOnly);
        assert_eq!(
            AutonomyLevel::parse("  READONLY  "),
            AutonomyLevel::ReadOnly
        );
        // Unrecognized / empty => behavior-preserving default.
        assert_eq!(AutonomyLevel::parse("bogus"), AutonomyLevel::Full);
        assert_eq!(AutonomyLevel::parse(""), AutonomyLevel::Full);
        // Default trait impl is also Full.
        assert_eq!(AutonomyLevel::default(), AutonomyLevel::Full);
    }

    #[test]
    fn unknown_tool_reports_tool_not_found() {
        let registry = Registry::new();
        let executor = ToolExecutor::new();

        let error = executor
            .execute(&registry, "tool.exec.nope", &serde_json::json!({}))
            .expect_err("unknown tool must error");
        assert!(matches!(error, ExecError::ToolNotFound(name) if name == "tool.exec.nope"));
    }

    // --- self-healing observation helpers ------------------------------------------------

    #[test]
    fn is_recoverable_classifies_error_variants() {
        assert!(ExecError::BadArguments("bad".to_string()).is_recoverable());
        assert!(
            ExecError::InvalidArguments {
                tool: "tool.x".to_string(),
                reason: "missing field".to_string(),
            }
            .is_recoverable()
        );
        assert!(ExecError::ToolNotFound("t".to_string()).is_recoverable());
        assert!(ExecError::HandlerNotFound("h".to_string()).is_recoverable());

        assert!(!ExecError::Unbound.is_recoverable());
        assert!(!ExecError::NotPermitted("nope".to_string()).is_recoverable());
        assert!(!ExecError::NotYetSupported("later").is_recoverable());
        assert!(
            !ExecError::Blocked {
                tool: "tool.x".to_string(),
                effect: "destructive",
                level: "read-only",
            }
            .is_recoverable()
        );

        assert!(!ExecError::Backend("failed to spawn command: x".to_string()).is_recoverable());
        assert!(ExecError::Backend(TIMEOUT_BACKEND_MSG.to_string()).is_recoverable());
        assert!(
            !ExecError::Backend("command timed out while draining".to_string()).is_recoverable()
        );
    }

    #[test]
    fn to_observation_has_stable_shape_and_tags() {
        let obs = ExecError::BadArguments("missing field".to_string()).to_observation("tool.x");
        assert_eq!(obs["tool"], "tool.x");
        assert_eq!(obs["is_error"], true);
        assert_eq!(obs["reason"], "bad_arguments");
        assert_eq!(obs["detail"], "bad arguments: missing field");
        assert!(obs["hint"].is_string());

        let obs = ExecError::NotPermitted("blocked".to_string()).to_observation("tool.y");
        assert_eq!(obs["tool"], "tool.y");
        assert_eq!(obs["is_error"], true);
        assert_eq!(obs["reason"], "not_permitted");

        let obs = ExecError::Backend(TIMEOUT_BACKEND_MSG.to_string()).to_observation("tool.z");
        assert_eq!(obs["reason"], "backend_timeout");

        let obs = ExecError::Backend("spawn failed".to_string()).to_observation("tool.z");
        assert_eq!(obs["reason"], "backend");

        let obs = ExecError::InvalidArguments {
            tool: "tool.arg".to_string(),
            reason: "missing symbol".to_string(),
        }
        .to_observation("tool.arg");
        assert_eq!(obs["reason"], "invalid_arguments");

        let obs = ExecError::Blocked {
            tool: "tool.write".to_string(),
            effect: "write-safe",
            level: "read-only",
        }
        .to_observation("tool.write");
        assert_eq!(obs["reason"], "blocked");
    }

    #[test]
    fn failure_budget_record_returns_true_up_to_cap_then_false() {
        let mut budget = FailureBudget::new(2);
        assert!(budget.record("tool.a"));
        assert!(budget.record("tool.a"));
        assert!(!budget.record("tool.a"));
        assert!(!budget.record("tool.a"));

        assert!(budget.record("tool.b"));
    }

    #[test]
    fn failure_budget_cap_zero_is_immediately_exceeded() {
        let mut budget = FailureBudget::new(0);
        assert!(!budget.record("tool.a"));
    }

    // --- argument-schema validation (opt-in) ---------------------------------------------

    /// Build a registry + validating executor for a schema-bearing builtin in one step.
    fn validating_setup(name: &str, schema: Value) -> (Registry, ToolExecutor) {
        let handler = format!("{name}.handler");
        let tool = tool_with_schema(name, schema, &handler);
        let registry = registry_with(&tool);
        let executor = ToolExecutor::new()
            .with_builtin(&handler, echo_handler)
            .unwrap_or_else(|error| panic!("builtin should register: {error}"))
            .with_arg_validation(SchemaValidation::On);
        (registry, executor)
    }

    #[test]
    fn validation_off_by_default_skips_check() {
        // No opt-in: a missing required field is *not* rejected (behavior-preserving).
        let tool = tool_with_schema(
            "tool.exec.val.off",
            serde_json::json!({ "type": "object", "required": ["symbol"] }),
            "tool.exec.val.off.handler",
        );
        let registry = registry_with(&tool);
        let executor = ToolExecutor::new()
            .with_builtin("tool.exec.val.off.handler", echo_handler)
            .unwrap_or_else(|error| panic!("builtin should register: {error}"));

        let outcome = executor
            .execute(&registry, "tool.exec.val.off", &serde_json::json!({}))
            .unwrap_or_else(|error| panic!("validation off must skip check: {error}"));
        assert_eq!(outcome.structured["echoed"], serde_json::json!({}));
    }

    #[test]
    fn missing_required_field_is_invalid_arguments() {
        let (registry, executor) = validating_setup(
            "tool.exec.val.req",
            serde_json::json!({ "type": "object", "required": ["symbol"] }),
        );

        let error = executor
            .execute(&registry, "tool.exec.val.req", &serde_json::json!({}))
            .expect_err("missing required field must be invalid");
        match error {
            ExecError::InvalidArguments { tool, reason } => {
                assert_eq!(tool, "tool.exec.val.req");
                assert!(
                    reason.contains("symbol"),
                    "reason should name field: {reason}"
                );
            }
            other => panic!("expected InvalidArguments, got {other:?}"),
        }
    }

    #[test]
    fn present_required_field_passes() {
        let (registry, executor) = validating_setup(
            "tool.exec.val.present",
            serde_json::json!({ "type": "object", "required": ["symbol"] }),
        );

        let outcome = executor
            .execute(
                &registry,
                "tool.exec.val.present",
                &serde_json::json!({ "symbol": "AAPL" }),
            )
            .unwrap_or_else(|error| panic!("present required field must pass: {error}"));
        assert_eq!(
            outcome.structured["echoed"],
            serde_json::json!({ "symbol": "AAPL" })
        );
    }

    #[test]
    fn wrong_property_type_is_invalid_arguments() {
        let (registry, executor) = validating_setup(
            "tool.exec.val.wrongtype",
            serde_json::json!({ "type": "object", "properties": { "limit": { "type": "integer" } } }),
        );

        let error = executor
            .execute(
                &registry,
                "tool.exec.val.wrongtype",
                &serde_json::json!({ "limit": "ten" }),
            )
            .expect_err("wrong property type must be invalid");
        match error {
            ExecError::InvalidArguments { reason, .. } => {
                assert!(
                    reason.contains("limit"),
                    "reason should name field: {reason}"
                );
            }
            other => panic!("expected InvalidArguments, got {other:?}"),
        }
    }

    #[test]
    fn correct_property_type_passes() {
        let (registry, executor) = validating_setup(
            "tool.exec.val.righttype",
            serde_json::json!({ "type": "object", "properties": { "limit": { "type": "integer" } } }),
        );

        let outcome = executor
            .execute(
                &registry,
                "tool.exec.val.righttype",
                &serde_json::json!({ "limit": 5 }),
            )
            .unwrap_or_else(|error| panic!("correct property type must pass: {error}"));
        assert_eq!(
            outcome.structured["echoed"],
            serde_json::json!({ "limit": 5 })
        );
    }

    #[test]
    fn extra_unknown_field_allowed() {
        let (registry, executor) = validating_setup(
            "tool.exec.val.extra",
            serde_json::json!({ "type": "object", "required": ["symbol"] }),
        );

        let outcome = executor
            .execute(
                &registry,
                "tool.exec.val.extra",
                &serde_json::json!({ "symbol": "x", "extra": 1 }),
            )
            .unwrap_or_else(|error| panic!("extra field must be allowed: {error}"));
        assert_eq!(
            outcome.structured["echoed"],
            serde_json::json!({ "symbol": "x", "extra": 1 })
        );
    }

    #[test]
    fn null_required_field_treated_as_missing() {
        let (registry, executor) = validating_setup(
            "tool.exec.val.null",
            serde_json::json!({ "type": "object", "required": ["symbol"] }),
        );

        let error = executor
            .execute(
                &registry,
                "tool.exec.val.null",
                &serde_json::json!({ "symbol": Value::Null }),
            )
            .expect_err("null required field must be invalid");
        assert!(matches!(error, ExecError::InvalidArguments { .. }));
    }

    #[test]
    fn non_object_schema_accepts_anything() {
        // A non-object schema (`true`) has nothing to check: lenient/never-panic guard.
        let (registry, executor) =
            validating_setup("tool.exec.val.anyschema", serde_json::json!(true));

        let outcome = executor
            .execute(
                &registry,
                "tool.exec.val.anyschema",
                &serde_json::json!({ "anything": 1 }),
            )
            .unwrap_or_else(|error| panic!("non-object schema must accept anything: {error}"));
        assert_eq!(
            outcome.structured["echoed"],
            serde_json::json!({ "anything": 1 })
        );
    }

    #[test]
    fn validate_gate_runs_before_backend_dispatch() {
        // The handler is intentionally *not* registered; if the gate fails to short-circuit
        // we would see HandlerNotFound, not InvalidArguments.
        let tool = tool_with_schema(
            "tool.exec.val.shortcircuit",
            serde_json::json!({ "type": "object", "required": ["symbol"] }),
            "tool.exec.val.shortcircuit.absent",
        );
        let registry = registry_with(&tool);
        let executor = ToolExecutor::new().with_arg_validation(SchemaValidation::On);

        let error = executor
            .execute(
                &registry,
                "tool.exec.val.shortcircuit",
                &serde_json::json!({}),
            )
            .expect_err("validation must short-circuit before dispatch");
        assert!(matches!(error, ExecError::InvalidArguments { .. }));
    }

    #[test]
    fn schema_validation_parse_maps_known_values_and_defaults_off() {
        assert_eq!(SchemaValidation::parse("1"), SchemaValidation::On);
        assert_eq!(SchemaValidation::parse("true"), SchemaValidation::On);
        assert_eq!(SchemaValidation::parse("ON"), SchemaValidation::On);
        assert_eq!(SchemaValidation::parse("yes"), SchemaValidation::On);
        assert_eq!(SchemaValidation::parse("0"), SchemaValidation::Off);
        assert_eq!(SchemaValidation::parse("bogus"), SchemaValidation::Off);
        assert_eq!(SchemaValidation::parse(""), SchemaValidation::Off);
        assert_eq!(SchemaValidation::default(), SchemaValidation::Off);
    }

    #[test]
    fn integer_type_accepts_i64_rejects_float() {
        let (registry, executor) = validating_setup(
            "tool.exec.val.int",
            serde_json::json!({ "type": "object", "properties": { "n": { "type": "integer" } } }),
        );

        let error = executor
            .execute(
                &registry,
                "tool.exec.val.int",
                &serde_json::json!({ "n": 1.5 }),
            )
            .expect_err("float must fail integer type");
        assert!(matches!(error, ExecError::InvalidArguments { .. }));

        let outcome = executor
            .execute(
                &registry,
                "tool.exec.val.int",
                &serde_json::json!({ "n": 3 }),
            )
            .unwrap_or_else(|error| panic!("i64 must pass integer type: {error}"));
        assert_eq!(outcome.structured["echoed"], serde_json::json!({ "n": 3 }));
    }

    #[test]
    fn compound_root_type_allows_null_without_object_validation() {
        let (registry, executor) = validating_setup(
            "tool.exec.val.compound",
            serde_json::json!({
                "type": ["object", "null"],
                "required": ["symbol"],
                "properties": { "symbol": { "type": "string" } }
            }),
        );

        let outcome = executor
            .execute(
                &registry,
                "tool.exec.val.compound",
                &serde_json::json!(null),
            )
            .unwrap_or_else(|error| panic!("compound object/null schema must allow null: {error}"));
        assert_eq!(outcome.structured["echoed"], serde_json::json!(null));
    }

    #[test]
    fn integer_type_accepts_mathematically_integral_float() {
        let (registry, executor) = validating_setup(
            "tool.exec.val.intfloat",
            serde_json::json!({ "type": "object", "properties": { "n": { "type": "integer" } } }),
        );

        let outcome = executor
            .execute(
                &registry,
                "tool.exec.val.intfloat",
                &serde_json::json!({ "n": 5.0 }),
            )
            .unwrap_or_else(|error| panic!("5.0 must pass integer type: {error}"));
        assert_eq!(
            outcome.structured["echoed"],
            serde_json::json!({ "n": 5.0 })
        );
    }
}
