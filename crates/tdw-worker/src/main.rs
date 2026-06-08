#![forbid(unsafe_code)]

use std::time::Duration;

use tdw_app_client::{DEFAULT_DAEMON_TCP_ADDR, DaemonClientConfig};
use tdw_app_server::{DaemonEndpoint, DaemonTransport};

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("tdw-worker error: {error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    if std::env::args().any(|arg| arg == "--contract") {
        println!("{}", tdw_worker::worker_contract_json());
        return Ok(());
    }

    if std::env::args().any(|arg| arg == "--durable-smoke") {
        println!(
            "{}",
            durable_smoke().await.map_err(|error| error.to_string())?
        );
        return Ok(());
    }

    if std::env::args().any(|arg| arg == "dead-letters") {
        return dead_letters_command().await;
    }

    if std::env::args().any(|arg| arg == "--serve" || arg == "--serve-once") {
        return serve().await;
    }

    match tdw_service_api::fetch_equity_historical("yahoo", "MSFT") {
        Ok(object) => match tdw_service_api::event_spine_sample("worker") {
            Ok(event) => {
                println!(
                    "tdw-worker job=equity_historical provider={} rows={} event_spine={}",
                    object.provider,
                    object.rows.len(),
                    event
                );
                Ok(())
            }
            Err(error) => Err(format!("tdw-worker event error: {error}")),
        },
        Err(error) => Err(error.to_string()),
    }
}

async fn serve() -> std::result::Result<(), String> {
    let once = std::env::args().any(|arg| arg == "--serve-once");
    let config = serve_config_from_env();
    let daemon = daemon_dispatch_config()?;
    let mode = if once {
        "serve-once draining"
    } else {
        "serving"
    };
    let handler_label = match &daemon {
        Some((_, endpoint)) => format!("daemon dispatch -> {endpoint}"),
        None => "ack handler".to_string(),
    };

    // Select the durable queue backend: Postgres when built with the `postgres`
    // feature and a PG URL is set, otherwise the SQLite default. The handler
    // (daemon dispatch vs offline ack) is orthogonal and applies to both.
    let report = match worker_backend_from_env() {
        #[cfg(feature = "postgres")]
        WorkerBackend::Postgres(url) => {
            eprintln!(
                "tdw-worker {mode} (backend=postgres, worker_id={}, {handler_label})",
                config.worker_id
            );
            let queue = tdw_worker::PgWorkerQueue::connect(&url)
                .await
                .map_err(|error| error.to_string())?;
            run_serve_dispatch(queue, daemon, config, once).await
        }
        WorkerBackend::Sqlite(db_url) => {
            eprintln!(
                "tdw-worker {mode} (backend=sqlite {db_url}, worker_id={}, {handler_label})",
                config.worker_id
            );
            let queue = tdw_worker::SqliteWorkerQueue::connect(&db_url)
                .await
                .map_err(|error| error.to_string())?;
            run_serve_dispatch(queue, daemon, config, once).await
        }
    }
    .map_err(|error| error.to_string())?;

    println!(
        "{}",
        serde_json::to_string(&report).map_err(|error| error.to_string())?
    );
    Ok(())
}

/// Parse the `dead-letters` subcommand from the CLI arguments.
///
/// `tdw-worker dead-letters list`
/// `tdw-worker dead-letters replay <job_id>`
#[derive(Debug, PartialEq, Eq)]
enum DeadLetterCommand {
    List,
    Replay(String),
}

fn parse_dead_letter_command() -> std::result::Result<DeadLetterCommand, String> {
    parse_dead_letter_args(std::env::args())
}

fn parse_dead_letter_args(
    args: impl Iterator<Item = String>,
) -> std::result::Result<DeadLetterCommand, String> {
    let mut args = args.skip_while(|arg| arg != "dead-letters");
    // Drop the `dead-letters` token itself.
    args.next();
    match args.next().as_deref() {
        Some("list") | None => Ok(DeadLetterCommand::List),
        Some("replay") => match args.next() {
            Some(job_id) if !job_id.trim().is_empty() => Ok(DeadLetterCommand::Replay(job_id)),
            _ => Err("dead-letters replay requires a <job_id> argument".to_string()),
        },
        Some(other) => Err(format!(
            "unknown dead-letters subcommand '{other}' (expected 'list' or 'replay <job_id>')"
        )),
    }
}

/// Operate on the dead-letter queue: list dead-lettered jobs as JSON, or replay
/// one back onto the queue. Uses the same backend selection as `--serve`.
async fn dead_letters_command() -> std::result::Result<(), String> {
    let command = parse_dead_letter_command()?;
    match worker_backend_from_env() {
        #[cfg(feature = "postgres")]
        WorkerBackend::Postgres(url) => {
            let queue = tdw_worker::PgWorkerQueue::connect(&url)
                .await
                .map_err(|error| error.to_string())?;
            match command {
                DeadLetterCommand::List => {
                    let letters = queue.dead_letters().await.map_err(|e| e.to_string())?;
                    print_dead_letters(&letters)
                }
                DeadLetterCommand::Replay(job_id) => {
                    queue
                        .replay_dead_letter(&job_id)
                        .await
                        .map_err(|e| e.to_string())?;
                    report_replay(&job_id, "postgres");
                    Ok(())
                }
            }
        }
        WorkerBackend::Sqlite(db_url) => {
            let queue = tdw_worker::SqliteWorkerQueue::connect(&db_url)
                .await
                .map_err(|error| error.to_string())?;
            match command {
                DeadLetterCommand::List => {
                    let letters = queue.dead_letters().await.map_err(|e| e.to_string())?;
                    print_dead_letters(&letters)
                }
                DeadLetterCommand::Replay(job_id) => {
                    queue
                        .replay_dead_letter(&job_id)
                        .await
                        .map_err(|e| e.to_string())?;
                    report_replay(&job_id, "sqlite");
                    Ok(())
                }
            }
        }
    }
}

fn print_dead_letters(letters: &[tdw_worker::DeadLetterRecord]) -> std::result::Result<(), String> {
    println!(
        "{}",
        serde_json::to_string(letters).map_err(|error| error.to_string())?
    );
    Ok(())
}

/// Audit log line for a dead-letter replay, emitted to stderr so it lands in
/// operator logs alongside the worker's other diagnostics.
fn report_replay(job_id: &str, backend: &str) {
    eprintln!("tdw-worker dead-letters replay job_id={job_id} backend={backend} status=requeued");
}

/// Which durable backend `--serve` runs against.
enum WorkerBackend {
    #[cfg(feature = "postgres")]
    Postgres(String),
    Sqlite(String),
}

/// Postgres when `--features postgres` is built and `TDW_WORKER_PG_URL`
/// (or `DATABASE_URL`) is set; otherwise `SQLite` (`TDW_WORKER_DB`, default
/// `sqlite://tdw-worker.sqlite`).
fn worker_backend_from_env() -> WorkerBackend {
    #[cfg(feature = "postgres")]
    if let Some(url) = non_empty_env("TDW_WORKER_PG_URL").or_else(|| non_empty_env("DATABASE_URL"))
    {
        return WorkerBackend::Postgres(url);
    }
    let db_url =
        std::env::var("TDW_WORKER_DB").unwrap_or_else(|_| "sqlite://tdw-worker.sqlite".to_string());
    WorkerBackend::Sqlite(db_url)
}

/// Run the lease loop with the configured handler over any `ServeQueue` backend.
///
/// The *inner* handler is selected as before: `DaemonJobHandler` when a daemon
/// is configured, otherwise the offline `LoggingAckHandler`.
///
/// When the binary is built with the `functions` feature **and** the runtime
/// flag `TDW_WORKER_FUNCTIONS` is truthy **and** email config is present
/// (`TDW_SMTP_HOST` / `TDW_EMAIL_FROM`), the inner handler is wrapped in an
/// inline `FnRoutingHandler`: `tdw.functions` jobs (e.g. the welcome function)
/// are routed to the inline function registry, every other job is forwarded
/// unchanged to the inner handler.
///
/// The `functions` feature is **OFF by default**; with the feature absent (or
/// the flag unset, or email config missing) this function is byte-for-byte
/// identical to its prior behavior.
async fn run_serve_dispatch<Q: tdw_worker::ServeQueue + Clone + 'static>(
    queue: Q,
    daemon: Option<(DaemonClientConfig, String)>,
    config: tdw_worker::ServeConfig,
    once: bool,
) -> tdw_worker::Result<tdw_worker::ServeReport> {
    if let Some((daemon_config, _)) = daemon {
        let inner = tdw_worker::DaemonJobHandler::new(daemon_config);
        #[cfg(feature = "functions")]
        if let Some(registry) = build_function_registry() {
            return run_serve(
                queue,
                FnRoutingHandler::new(std::sync::Arc::new(registry), inner),
                config,
                once,
            )
            .await;
        }
        run_serve(queue, inner, config, once).await
    } else {
        let inner = tdw_worker::LoggingAckHandler;
        #[cfg(feature = "functions")]
        if let Some(registry) = build_function_registry() {
            return run_serve(
                queue,
                FnRoutingHandler::new(std::sync::Arc::new(registry), inner),
                config,
                once,
            )
            .await;
        }
        run_serve(queue, inner, config, once).await
    }
}

// ---------------------------------------------------------------------------
// functions feature: inline registry, routing handler, welcome function, mailer
//
// `tdw-functions` and `tdw-functions-app` cannot be normal deps of `tdw-worker`
// because `tdw-functions` has `tdw-cron` as a dev-dep and `tdw-cron` depends on
// `tdw-worker` — a package-level cycle Cargo rejects regardless of feature flags.
// Everything needed for the R2 slice is therefore self-contained here.
// Swap-in of the shared `tdw-functions::FunctionRegistry` + `RoutingJobHandler`
// is tracked as a follow-up once the cycle is resolved upstream.
// ---------------------------------------------------------------------------

/// `tool_name` sentinel for function-job envelopes.
/// Mirrors `tdw_functions::job::FUNCTIONS_TOOL_NAME` so jobs enqueued by that
/// crate (or any conforming producer) are routed here without modification.
#[cfg(feature = "functions")]
const FUNCTIONS_TOOL_NAME: &str = "tdw.functions";

/// Stable id of the welcome function. Mirrors
/// `tdw_functions_app::WELCOME_FUNCTION_ID`.
#[cfg(feature = "functions")]
const WELCOME_FUNCTION_ID: &str = "welcome.on-user-created";

/// Event name that activates the welcome function. Mirrors
/// `tdw_functions_app::USER_CREATED_EVENT`. Only needed by the test-only
/// `subscribers` assertion; not used at runtime.
#[cfg(all(feature = "functions", test))]
const USER_CREATED_EVENT: &str = "user.created";

// ---------------------------------------------------------------------------
// Inline WelcomeMailer port
// ---------------------------------------------------------------------------

/// Port for delivering the welcome email.
///
/// Mirrors `tdw_functions_app::WelcomeMailer` so the registry builder is
/// unit-testable with a mock without constructing a live SMTP transport.
#[cfg(feature = "functions")]
trait WelcomeMailer: Send + Sync {
    fn send_welcome(&self, to_email: &str, body: &str) -> Result<(), String>;
}

/// Production mailer backed by [`tdw_email::TransactionalMailer`] (smtp sub-feature).
///
/// Owns a dedicated current-thread Tokio runtime to drive the async send
/// synchronously, keeping the `WelcomeMailer` port sync.
#[cfg(feature = "functions")]
struct SmtpWelcomeMailer {
    mailer: tdw_email::TransactionalMailer,
    runtime: tokio::runtime::Runtime,
}

#[cfg(feature = "functions")]
impl SmtpWelcomeMailer {
    fn new(config: tdw_email::EmailConfig) -> Result<Self, String> {
        let mailer = tdw_email::TransactionalMailer::new(config)
            .map_err(|e| format!("SMTP mailer init: {e}"))?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| format!("SMTP runtime init: {e}"))?;
        Ok(Self { mailer, runtime })
    }
}

#[cfg(feature = "functions")]
impl WelcomeMailer for SmtpWelcomeMailer {
    fn send_welcome(&self, to_email: &str, body: &str) -> Result<(), String> {
        let msg = tdw_email::EmailMessage {
            from: String::new(),
            to: to_email.to_string(),
            subject: "Welcome to FinX".to_string(),
            text: format!("Welcome to FinX! Your account ({to_email}) is ready."),
            html: body.to_string(),
        };
        self.runtime
            .block_on(self.mailer.send(&msg))
            .map_err(|e| format!("welcome mail send: {e}"))
    }
}

// ---------------------------------------------------------------------------
// Inline function registry
// ---------------------------------------------------------------------------

/// Payload shape stored in `Op::ToolCall { arguments }` for function jobs.
/// Mirrors `tdw_functions::job::FunctionJob`.
#[cfg(feature = "functions")]
#[derive(serde::Deserialize)]
struct FunctionJobPayload {
    function_id: String,
    run_id: String,
    payload: serde_json::Value,
    #[serde(default)]
    mode: FunctionJobMode,
}

#[cfg(feature = "functions")]
#[derive(serde::Deserialize, Default, PartialEq, Eq)]
enum FunctionJobMode {
    #[default]
    Invoke,
    Resume,
}

/// Type alias for the boxed async handler closure stored in [`FnEntry`].
#[cfg(feature = "functions")]
type FnHandlerBox = Box<
    dyn Fn(
            serde_json::Value,
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send>>
        + Send
        + Sync,
>;

/// An entry in the inline registry: the function id, the subscribed event (test
/// only), and an async handler closure.
#[cfg(feature = "functions")]
struct FnEntry {
    function_id: &'static str,
    /// The event name this function subscribes to. Only read by `subscribers`
    /// in tests; not needed at runtime (routing is by `tool_name` on the job
    /// envelope, not by event re-dispatch).
    #[cfg(test)]
    event: Option<&'static str>,
    handler: FnHandlerBox,
}

/// Minimal inline function registry.
///
/// Holds a list of [`FnEntry`] values. On `invoke`, the entry matching
/// `function_id` is called with the job `payload`. `subscribers` is test-only
/// and asserts the welcome function is wired to the correct event.
#[cfg(feature = "functions")]
struct InlineFunctionRegistry {
    entries: Vec<FnEntry>,
}

#[cfg(feature = "functions")]
impl InlineFunctionRegistry {
    fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    fn register(&mut self, entry: FnEntry) {
        self.entries.push(entry);
    }

    async fn invoke(&self, function_id: &str, payload: serde_json::Value) -> Result<(), String> {
        let entry = self
            .entries
            .iter()
            .find(|e| e.function_id == function_id)
            .ok_or_else(|| format!("unknown function: {function_id}"))?;
        (entry.handler)(payload).await
    }

    /// Return function ids subscribed to `event_name` (test-only).
    #[cfg(test)]
    fn subscribers(&self, event_name: &str) -> Vec<&str> {
        self.entries
            .iter()
            .filter(|e| e.event == Some(event_name))
            .map(|e| e.function_id)
            .collect()
    }
}

// ---------------------------------------------------------------------------
// FnRoutingHandler
// ---------------------------------------------------------------------------

/// Routes worker jobs: `tdw.functions` `ToolCall` jobs go to the inline
/// registry; every other job is forwarded to the `inner` handler unchanged.
#[cfg(feature = "functions")]
struct FnRoutingHandler<H> {
    registry: std::sync::Arc<InlineFunctionRegistry>,
    inner: H,
}

#[cfg(feature = "functions")]
impl<H> FnRoutingHandler<H> {
    fn new(registry: std::sync::Arc<InlineFunctionRegistry>, inner: H) -> Self {
        Self { registry, inner }
    }
}

#[cfg(feature = "functions")]
#[async_trait::async_trait]
impl<H: tdw_worker::JobHandler> tdw_worker::JobHandler for FnRoutingHandler<H> {
    async fn handle(&self, job: &tdw_worker::WorkerJob) -> std::result::Result<(), String> {
        use tdw_protocol::Op;
        let is_fn_job = matches!(
            &job.envelope.op,
            Op::ToolCall { tool_name, .. } if tool_name == FUNCTIONS_TOOL_NAME
        );
        if is_fn_job {
            dispatch_function_job(&self.registry, job).await
        } else {
            self.inner.handle(job).await
        }
    }
}

/// Decode the `FunctionJobPayload` from a `tdw.functions` envelope and invoke
/// the matching entry in the registry.
#[cfg(feature = "functions")]
async fn dispatch_function_job(
    registry: &InlineFunctionRegistry,
    job: &tdw_worker::WorkerJob,
) -> std::result::Result<(), String> {
    use tdw_protocol::Op;
    let arguments = match &job.envelope.op {
        Op::ToolCall { arguments, .. } => arguments.clone(),
        other => {
            return Err(format!(
                "dispatch_function_job: expected ToolCall, got {other:?}"
            ));
        }
    };
    let fp: FunctionJobPayload = serde_json::from_value(arguments)
        .map_err(|e| format!("function job deserialize failed: {e}"))?;

    match fp.mode {
        FunctionJobMode::Invoke => registry.invoke(&fp.function_id, fp.payload).await,
        FunctionJobMode::Resume => Err(format!(
            "Resume mode not supported by inline registry for run {}",
            fp.run_id
        )),
    }
}

// ---------------------------------------------------------------------------
// Registry builder
// ---------------------------------------------------------------------------

/// Build the inline function registry when `TDW_WORKER_FUNCTIONS` is truthy
/// and email config is present. Returns `None` (safe fallback) otherwise.
///
/// Swapping in the shared `tdw-functions::FunctionRegistry` + `RoutingJobHandler`
/// is a follow-up: see PR description for the cycle root cause.
#[cfg(feature = "functions")]
fn build_function_registry() -> Option<InlineFunctionRegistry> {
    if !functions_flag_enabled() {
        return None;
    }
    let Some(email_config) = tdw_email::EmailConfig::from_env() else {
        eprintln!(
            "tdw-worker: TDW_WORKER_FUNCTIONS is set but email config is absent \
             (TDW_SMTP_HOST / TDW_EMAIL_FROM unset) — function jobs stay DISABLED"
        );
        return None;
    };
    let mailer = match SmtpWelcomeMailer::new(email_config) {
        Ok(m) => std::sync::Arc::new(m) as std::sync::Arc<dyn WelcomeMailer>,
        Err(e) => {
            eprintln!(
                "tdw-worker: welcome mailer construction failed ({e}) \
                 — function jobs stay DISABLED"
            );
            return None;
        }
    };
    Some(build_function_registry_from(mailer))
}

/// Whether `TDW_WORKER_FUNCTIONS` is truthy (`1`, `true`, `yes`, `on`).
/// Everything else — including unset/empty — is OFF (safe by default).
#[cfg(feature = "functions")]
fn functions_flag_enabled() -> bool {
    non_empty_env("TDW_WORKER_FUNCTIONS")
        .map(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

/// Build the registry from an already-constructed `mailer`.
///
/// Split from [`build_function_registry`] so unit tests can inject a no-op
/// `WelcomeMailer` mock without touching the environment or constructing a live
/// SMTP transport.
#[cfg(feature = "functions")]
fn build_function_registry_from(
    mailer: std::sync::Arc<dyn WelcomeMailer>,
) -> InlineFunctionRegistry {
    let mut registry = InlineFunctionRegistry::new();
    registry.register(FnEntry {
        function_id: WELCOME_FUNCTION_ID,
        #[cfg(test)]
        event: Some(USER_CREATED_EVENT),
        handler: Box::new(move |payload| {
            let mailer = std::sync::Arc::clone(&mailer);
            Box::pin(async move { run_welcome(mailer, payload).await })
        }),
    });
    registry
}

/// Execute the welcome function: parse the `user.created` payload, compose the
/// email body via `tdw_email::render_template`, and deliver via the mailer.
#[cfg(feature = "functions")]
async fn run_welcome(
    mailer: std::sync::Arc<dyn WelcomeMailer>,
    payload: serde_json::Value,
) -> Result<(), String> {
    #[derive(serde::Deserialize)]
    struct UserCreated {
        email: String,
        #[serde(default)]
        display_name: Option<String>,
    }
    let user: UserCreated =
        serde_json::from_value(payload).map_err(|e| format!("welcome: invalid payload: {e}"))?;
    if user.email.is_empty() {
        return Err("welcome: payload missing email".to_string());
    }
    let name = user
        .display_name
        .as_deref()
        .filter(|n| !n.is_empty())
        .unwrap_or(user.email.as_str());
    let mut vars = std::collections::BTreeMap::new();
    vars.insert("name", name.to_string());
    vars.insert(
        "body",
        "Thanks for creating your FinX account. We're glad to have you on board.".to_string(),
    );
    vars.insert(
        "cta",
        "Sign in any time to start exploring the platform.".to_string(),
    );
    vars.insert("unsubscribe_url", "#".to_string());
    let body = tdw_email::render_template("welcome", &vars)
        .map_err(|e| format!("welcome: template render: {e}"))?;
    mailer.send_welcome(&user.email, &body)
}

async fn run_serve<Q: tdw_worker::ServeQueue + Clone + 'static, H: tdw_worker::JobHandler>(
    queue: Q,
    handler: H,
    config: tdw_worker::ServeConfig,
    once: bool,
) -> tdw_worker::Result<tdw_worker::ServeReport> {
    if once {
        // `--serve-once` drains the ready backlog and exits; no long-running ops
        // listener is wired for the one-shot path.
        let runner = tdw_worker::WorkerRunner::new(queue, handler, config);
        return runner.run_until_idle().await;
    }

    // The ops listener (/health, /ready, /metrics) is OFF by default; it binds
    // only when TDW_WORKER_HTTP_BIND is set. It shares the serve loop's queue
    // handle (same backend pool) so /ready reflects real queue connectivity.
    let cancel = tdw_app_server::CancellationToken::new();
    let ops_task = spawn_worker_ops(&queue, cancel.clone()).await;

    let runner = tdw_worker::WorkerRunner::new(queue, handler, config);
    let cancel_for_drain = cancel.clone();
    let report = runner
        .run(async move {
            // SIGTERM (container/systemd stop) or Ctrl-C: stop accepting ops
            // requests and let the lease loop drain its in-flight jobs (its own
            // drain pattern never cancels a leased job).
            tdw_app_server::shutdown_signal().await;
            cancel_for_drain.cancel();
        })
        .await;

    // Ensure the ops listener is torn down even if the loop returned without a
    // signal (e.g. resolved-shutdown), then await it.
    cancel.cancel();
    if let Some(task) = ops_task {
        let _ = task.await;
    }
    report
}

/// Bind and spawn the worker's ops listener when `TDW_WORKER_HTTP_BIND` is set.
/// Returns the listener task, or `None` when the env var is unset (default).
/// A bind failure is logged and treated as "no listener" so the worker still
/// serves jobs.
async fn spawn_worker_ops<Q: tdw_worker::ServeQueue + Clone + 'static>(
    queue: &Q,
    cancel: tdw_app_server::CancellationToken,
) -> Option<tokio::task::JoinHandle<std::io::Result<()>>> {
    let bind = non_empty_env("TDW_WORKER_HTTP_BIND")?;
    let listener = match tokio::net::TcpListener::bind(&bind).await {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("tdw-worker ops listener bind failed on {bind}: {error}");
            return None;
        }
    };
    eprintln!("tdw-worker ops listener on http://{bind} (/health /ready /metrics)");
    let provider = tdw_worker::WorkerOps::new(queue.clone());
    Some(tokio::spawn(async move {
        tdw_app_server::ops::serve_ops(listener, provider, cancel).await
    }))
}

/// Resolve daemon-dispatch config from the environment. Returns `None` (ack
/// handler) unless `TDW_WORKER_DISPATCH=daemon`, `TDW_WORKER_DAEMON_ADDR`, or
/// `TDW_WORKER_DAEMON_TRANSPORT` is set. Validates the endpoint, failing closed
/// on unsupported transports (e.g. UDS on Windows).
fn daemon_dispatch_config() -> std::result::Result<Option<(DaemonClientConfig, String)>, String> {
    let dispatch = non_empty_env("TDW_WORKER_DISPATCH");
    let addr = non_empty_env("TDW_WORKER_DAEMON_ADDR");
    let transport_raw = non_empty_env("TDW_WORKER_DAEMON_TRANSPORT");
    let enabled =
        matches!(dispatch.as_deref(), Some("daemon")) || addr.is_some() || transport_raw.is_some();
    if !enabled {
        return Ok(None);
    }

    let transport = match transport_raw.as_deref() {
        Some(value) => parse_daemon_transport(value)?,
        None => DaemonTransport::Tcp,
    };
    let address = addr.unwrap_or_else(|| default_daemon_address(transport));
    let timeout = match non_empty_env("TDW_WORKER_DAEMON_TIMEOUT_MS") {
        Some(value) => Duration::from_millis(
            value
                .parse()
                .map_err(|error| format!("invalid TDW_WORKER_DAEMON_TIMEOUT_MS: {error}"))?,
        ),
        None => Duration::from_secs(2),
    };

    let endpoint_label = format!("{}:{address}", daemon_transport_label(transport));
    let daemon_config =
        DaemonClientConfig::new(DaemonEndpoint { transport, address }).with_timeout(timeout);
    daemon_config
        .validate()
        .map_err(|error| format!("invalid daemon client config: {error:?}"))?;
    Ok(Some((daemon_config, endpoint_label)))
}

fn parse_daemon_transport(value: &str) -> std::result::Result<DaemonTransport, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "tcp" => Ok(DaemonTransport::Tcp),
        "uds" | "unix" => Ok(DaemonTransport::Uds),
        "http-sse" | "http" | "sse" => Ok(DaemonTransport::HttpSse),
        other => Err(format!("unsupported TDW_WORKER_DAEMON_TRANSPORT: {other}")),
    }
}

fn default_daemon_address(transport: DaemonTransport) -> String {
    match transport {
        DaemonTransport::Tcp => DEFAULT_DAEMON_TCP_ADDR.to_string(),
        DaemonTransport::Uds => "/tmp/tdw-daemon.sock".to_string(),
        DaemonTransport::HttpSse => "http://127.0.0.1:7879/events".to_string(),
    }
}

const fn daemon_transport_label(transport: DaemonTransport) -> &'static str {
    match transport {
        DaemonTransport::Tcp => "tcp",
        DaemonTransport::Uds => "uds",
        DaemonTransport::HttpSse => "http-sse",
    }
}

fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

fn serve_config_from_env() -> tdw_worker::ServeConfig {
    let mut config = tdw_worker::ServeConfig::default();
    match std::env::var("TDW_WORKER_ID") {
        Ok(id) if !id.trim().is_empty() => config.worker_id = id,
        _ => {}
    }
    if let Some(ttl) = std::env::var("TDW_WORKER_LEASE_TTL_MS")
        .ok()
        .and_then(|value| value.parse().ok())
    {
        config.lease_ttl_ms = ttl;
    }
    if let Some(poll) = std::env::var("TDW_WORKER_POLL_MS")
        .ok()
        .and_then(|value| value.parse().ok())
    {
        config.poll_interval_ms = poll;
    }
    if let Some(concurrency) = std::env::var("TDW_WORKER_CONCURRENCY")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
    {
        config.max_concurrent = clamp_concurrency(concurrency);
    }
    config
}

/// Upper bound on `TDW_WORKER_CONCURRENCY`. Bounds in-flight jobs so a typo
/// (e.g. `100000`) cannot exhaust DB connections / file descriptors.
const MAX_WORKER_CONCURRENCY: usize = 256;

/// Clamp the requested worker concurrency into `1..=MAX_WORKER_CONCURRENCY`,
/// warning on stderr when the request is clamped. `0`/garbage never stalls the
/// serve loop, and an over-large value never exhausts resources.
fn clamp_concurrency(requested: usize) -> usize {
    let clamped = requested.clamp(1, MAX_WORKER_CONCURRENCY);
    if clamped != requested {
        eprintln!(
            "tdw-worker: TDW_WORKER_CONCURRENCY={requested} clamped to {clamped} \
             (valid range 1..={MAX_WORKER_CONCURRENCY})"
        );
    }
    clamped
}

async fn durable_smoke() -> tdw_worker::Result<String> {
    let queue = tdw_worker::SqliteWorkerQueue::connect("sqlite::memory:").await?;
    let outcome = queue
        .enqueue(tdw_worker::sample_shutdown_job("worker-smoke-1")?)
        .await?;
    let lease = queue.lease_next("worker-smoke").await?;
    if let Some(lease) = &lease {
        queue.complete(&lease.job_id).await?;
    }
    let stats = queue.stats().await?;

    Ok(serde_json::json!({
        "worker": "tdw-worker",
        "durable_smoke": true,
        "backend": "sqlite",
        "inserted": outcome.inserted,
        "lease": lease,
        "stats": stats
    })
    .to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(parts: &[&str]) -> impl Iterator<Item = String> {
        parts
            .iter()
            .map(|part| (*part).to_string())
            .collect::<Vec<_>>()
            .into_iter()
    }

    #[test]
    fn clamp_concurrency_bounds_to_valid_range() {
        assert_eq!(clamp_concurrency(0), 1, "zero must not stall the loop");
        assert_eq!(clamp_concurrency(1), 1);
        assert_eq!(clamp_concurrency(8), 8);
        assert_eq!(
            clamp_concurrency(MAX_WORKER_CONCURRENCY),
            MAX_WORKER_CONCURRENCY
        );
        assert_eq!(
            clamp_concurrency(MAX_WORKER_CONCURRENCY + 1),
            MAX_WORKER_CONCURRENCY,
            "over-large request is capped"
        );
        assert_eq!(clamp_concurrency(100_000), MAX_WORKER_CONCURRENCY);
    }

    #[test]
    fn parse_dead_letter_args_defaults_to_list() {
        assert!(matches!(
            parse_dead_letter_args(args(&["tdw-worker", "dead-letters"])),
            Ok(DeadLetterCommand::List)
        ));
        assert!(matches!(
            parse_dead_letter_args(args(&["tdw-worker", "dead-letters", "list"])),
            Ok(DeadLetterCommand::List)
        ));
    }

    #[test]
    fn parse_dead_letter_args_parses_replay() {
        match parse_dead_letter_args(args(&["tdw-worker", "dead-letters", "replay", "job-7"])) {
            Ok(DeadLetterCommand::Replay(job_id)) => assert_eq!(job_id, "job-7"),
            other => panic!("expected replay, got {other:?}"),
        }
    }

    #[test]
    fn parse_dead_letter_args_rejects_bad_input() {
        assert!(parse_dead_letter_args(args(&["tdw-worker", "dead-letters", "replay"])).is_err());
        assert!(parse_dead_letter_args(args(&["tdw-worker", "dead-letters", "bogus"])).is_err());
    }
}

#[cfg(all(test, feature = "functions"))]
mod function_tests {
    use std::sync::Arc;

    use super::{
        USER_CREATED_EVENT, WELCOME_FUNCTION_ID, WelcomeMailer, build_function_registry_from,
    };

    /// No-op mailer: registry construction must succeed without a live SMTP transport.
    struct NoopMailer;
    impl WelcomeMailer for NoopMailer {
        fn send_welcome(&self, _to: &str, _body: &str) -> Result<(), String> {
            Ok(())
        }
    }

    #[test]
    fn build_function_registry_from_registers_welcome() {
        let mailer = Arc::new(NoopMailer) as Arc<dyn WelcomeMailer>;
        let registry = build_function_registry_from(mailer);

        // The welcome function must subscribe to the user.created event.
        let subs = registry.subscribers(USER_CREATED_EVENT);
        assert!(
            subs.contains(&WELCOME_FUNCTION_ID),
            "welcome function must subscribe to {USER_CREATED_EVENT}: {subs:?}"
        );
    }
}
