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

use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;
use tdw_agent::{EntityKind, Registry, Tool, ToolImplementation, entity_from_resource};
use tdw_tools::{RegisteredTool, ToolDefinition, ToolHandler, ToolRegistry};
use thiserror::Error;

/// Env var holding the comma-separated allow-list of bare command names permitted for
/// `Command` execution. Unset/empty means *deny all* command execution.
const ALLOWED_COMMANDS_ENV: &str = "TDW_TOOL_EXEC_ALLOWED_COMMANDS";
/// Env var overriding the command execution timeout, in whole seconds.
const TIMEOUT_SECS_ENV: &str = "TDW_TOOL_EXEC_TIMEOUT_SECS";
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

/// Resolves a registry `tool`'s implementation binding and dispatches it to a backend.
///
/// Holds an in-process [`ToolRegistry`] for `Builtin` handlers; `Command { background: false }`
/// runs via a hardened direct [`std::process::Command`] governed by a [`CommandPolicy`].
#[derive(Default)]
pub struct ToolExecutor {
    builtins: ToolRegistry,
    command_policy: CommandPolicy,
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
    /// [`ExecError::Unbound`] for an unbound tool, [`ExecError::NotYetSupported`] for a
    /// deferred variant, [`ExecError::NotPermitted`] for a command rejected by policy,
    /// [`ExecError::BadArguments`] for a malformed tool resource or command, or
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

        match tool.implementation {
            ToolImplementation::Unbound => Err(ExecError::Unbound),
            ToolImplementation::Builtin { handler } => self.dispatch_builtin(&handler, args),
            ToolImplementation::Command {
                command,
                args: command_args,
                background,
            } => {
                if background {
                    return Err(ExecError::NotYetSupported("background command (Phase 2)"));
                }
                dispatch_command(&self.command_policy, &command, &command_args)
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
        }
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

    let stdout_reader = spawn_reader(child.stdout.take());
    let stderr_reader = spawn_reader(child.stderr.take());

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
fn spawn_reader<R>(stream: Option<R>) -> Option<thread::JoinHandle<Vec<u8>>>
where
    R: std::io::Read + Send + 'static,
{
    stream.map(|mut stream| {
        thread::spawn(move || {
            let mut buffer = Vec::new();
            let _ = stream.read_to_end(&mut buffer);
            buffer
        })
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
        // A command that runs ~5s on both platforms; the 1s timeout must kill it.
        // `cmd /c sleep 5` does not exist on Windows, so use a per-OS script.
        let script = if cfg!(windows) {
            "ping -n 6 127.0.0.1 >NUL"
        } else {
            "sleep 5"
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
            .with_command_policy(policy(Some(&[shell_bin()]), Duration::from_secs(1)));

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
    fn unknown_tool_reports_tool_not_found() {
        let registry = Registry::new();
        let executor = ToolExecutor::new();

        let error = executor
            .execute(&registry, "tool.exec.nope", &serde_json::json!({}))
            .expect_err("unknown tool must error");
        assert!(matches!(error, ExecError::ToolNotFound(name) if name == "tool.exec.nope"));
    }
}
