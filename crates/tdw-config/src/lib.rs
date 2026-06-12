#![forbid(unsafe_code)]
#![deny(clippy::pedantic, clippy::nursery)]
use std::collections::BTreeMap;

use schemars::{JsonSchema, schema_for};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use thiserror::Error;

pub type Result<T> = std::result::Result<T, ConfigError>;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("invalid TOML layer {name}: {source}")]
    Toml {
        name: String,
        source: toml::de::Error,
    },
    #[error("invalid JSON conversion for layer {name}: {source}")]
    Json {
        name: String,
        source: serde_json::Error,
    },
    #[error("invalid configuration: {0}")]
    Validation(String),
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
pub enum ConfigLayerKind {
    UserDefaults,
    EnvFile,
    ProjectConfig,
    SplitFile,
    InlineEnv,
    CliFlags,
}

impl ConfigLayerKind {
    #[must_use]
    pub const fn precedence(self) -> u8 {
        match self {
            Self::UserDefaults => 10,
            Self::EnvFile => 20,
            Self::ProjectConfig => 30,
            Self::SplitFile => 40,
            Self::InlineEnv => 50,
            Self::CliFlags => 60,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ConfigLayerDescriptor {
    pub kind: ConfigLayerKind,
    pub source: &'static str,
    pub precedence: u8,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ConfigLayer {
    pub kind: ConfigLayerKind,
    pub name: String,
    pub value: Value,
}

impl ConfigLayer {
    pub fn new(kind: ConfigLayerKind, name: impl Into<String>, value: Value) -> Self {
        Self {
            kind,
            name: name.into(),
            value,
        }
    }

    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub fn from_toml(kind: ConfigLayerKind, name: impl Into<String>, input: &str) -> Result<Self> {
        let name = name.into();
        let parsed = toml::from_str::<toml::Value>(input).map_err(|source| ConfigError::Toml {
            name: name.clone(),
            source,
        })?;
        let value = serde_json::to_value(parsed).map_err(|source| ConfigError::Json {
            name: name.clone(),
            source,
        })?;
        Ok(Self { kind, name, value })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PathsConfig {
    pub data_dir: String,
    pub rollout_dir: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DaemonConfig {
    pub transport: DaemonTransport,
    pub uds_path: String,
    pub tcp_bind: Option<String>,
    pub http_bind: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum DaemonTransport {
    Tcp,
    Uds,
    HttpSse,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SessionConfig {
    pub sqlite_path: String,
    pub jsonl_archive: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum WorkerBackend {
    Sqlite,
    Postgres,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WorkerConfig {
    pub backend: WorkerBackend,
    pub sqlite_path: String,
    pub postgres_url_env: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PermissionConfig {
    pub default_action: PermissionAction,
    pub last_match_wins: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum PermissionAction {
    Allow,
    Ask,
    Deny,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ModelConfig {
    pub provider: String,
    pub model: String,
    pub base_url: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProtocolConfig {
    pub max_event_bytes: u64,
    pub replay_enabled: bool,
}

/// Single-user identity for the knowledge finding surface (knowledge-system K-X6).
///
/// Until K-L5 delivers per-session identity negotiation this is a single
/// operator-configured id that binds to every `tdw.kg.finding` / `tdw.kg.link`
/// tool call on this daemon. The id is validated at config load with the same
/// grammar as [`tdw_knowledge::proposals::validate_agent_id`]:
/// `[A-Za-z0-9:._-]+`, non-empty, ≤ 128 bytes.
///
/// Set `[knowledge.user] id = "analyst"` in your daemon TOML (default).
/// When K-L5 lands, per-session identity will be injected at the MCP layer
/// and this config key will be superseded — note that in the transition plan.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct UserConfig {
    /// The operator-configured user identity bound to the finding tools.
    ///
    /// Grammar: `[A-Za-z0-9:._-]+`, non-empty, ≤ 128 bytes.
    /// Default: `"analyst"`.
    #[serde(default = "UserConfig::default_id")]
    pub id: String,
}

impl UserConfig {
    fn default_id() -> String {
        "analyst".to_string()
    }
}

impl Default for UserConfig {
    fn default() -> Self {
        Self {
            id: Self::default_id(),
        }
    }
}

/// Scheduled retrieval-eval settings for the in-daemon self-eval (K-L3).
///
/// Placed in `tdw-config` (not `tdw-eval-runner`) so operator TOML can configure
/// it without creating a dependency cycle:
/// `tdw-cron → tdw-worker → tdw-service-api → tdw-eval-runner → tdw-cron`.
///
/// The composition root (`tdw-backend`) converts this into a
/// `tdw_eval_runner::scheduled_eval::ScheduledEvalConfig` at startup.
///
/// # Example (`tdw.toml`)
///
/// ```toml
/// [knowledge.evals]
/// split_id = "golden-split-v1"
/// cadence  = "0 3 * * MON"
/// max_recall_drop = 0.05
/// max_mrr_drop    = 0.05
/// max_ndcg_drop   = 0.05
/// ```
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ScheduledEvalConfig {
    /// The `fixed_split_id` identifying which golden case set to run.
    ///
    /// When `None` or empty the daemon registers no cron trigger and status
    /// reports `Unconfigured` — loudly, not silently.
    #[serde(default)]
    pub split_id: Option<String>,

    /// 5-field cron expression for the eval cadence.
    ///
    /// Default: `"0 3 * * MON"` (weekly, Monday 03:00 UTC).
    #[serde(default = "ScheduledEvalConfig::default_cadence")]
    pub cadence: String,

    /// Maximum allowed absolute drop in mean recall\@k before a regression is
    /// declared. Must be in `[0.0, 1.0]`. Default: `0.05` (5 percentage points).
    #[serde(default = "ScheduledEvalConfig::default_threshold")]
    pub max_recall_drop: f64,

    /// Maximum allowed absolute drop in MRR before a regression is declared.
    /// Must be in `[0.0, 1.0]`. Default: `0.05`.
    #[serde(default = "ScheduledEvalConfig::default_threshold")]
    pub max_mrr_drop: f64,

    /// Maximum allowed absolute drop in mean nDCG\@k before a regression is
    /// declared. Must be in `[0.0, 1.0]`. Default: `0.05`.
    #[serde(default = "ScheduledEvalConfig::default_threshold")]
    pub max_ndcg_drop: f64,

    /// Filesystem path to the golden-split fixture file (JSON).
    ///
    /// Must point to a `GoldenSplitFixture` JSON object
    /// (`{ "documents": [...], "cases": [...], "baseline": <report>|null }`).
    /// When `None`, the daemon falls back to the fixture embedded in the
    /// `tdw-eval-runner` crate at `baselines/<split_id>.json`.
    ///
    /// A missing or unparseable file causes the eval worker to write
    /// `Stale` (no alarm) — a load error is a configuration problem, never
    /// a retrieval regression.
    #[serde(default)]
    pub split_fixture_path: Option<String>,
}

impl ScheduledEvalConfig {
    fn default_cadence() -> String {
        "0 3 * * MON".to_string()
    }

    const fn default_threshold() -> f64 {
        0.05
    }
}

impl Default for ScheduledEvalConfig {
    fn default() -> Self {
        Self {
            split_id: None,
            cadence: Self::default_cadence(),
            max_recall_drop: Self::default_threshold(),
            max_mrr_drop: Self::default_threshold(),
            max_ndcg_drop: Self::default_threshold(),
            split_fixture_path: None,
        }
    }
}

/// Knowledge-system settings (knowledge-system B6 + F1 + K-L1 + K-L3 + K-X6).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct KnowledgeConfig {
    #[serde(default)]
    pub embedding: EmbeddingConfig,
    /// Graph-database backend for the live daemon's knowledge graph (F1).
    ///
    /// The default backend is `in-memory` so that `tdw-service` starts without
    /// any external services on a first run. Graph data is **not persisted**
    /// across restarts in this mode. Set `backend = "bolt"` in your daemon TOML
    /// (or in the `full`/`live` Compose profiles) for any environment where
    /// graph data must survive a restart.
    ///
    /// This is an **explicit default**, not a fallback: both `in-memory` and
    /// `bolt` are first-class. There is NO silent fallback — selecting `bolt`
    /// and having Memgraph unreachable is a hard `Init` error at daemon startup.
    #[serde(default)]
    pub graph: GraphConfig,
    /// Auto-tagging rule-set configuration (knowledge-system K-L1).
    ///
    /// When absent (the default), no tag rules are loaded and the fact is
    /// logged loudly at boot. When present, `rules_dir` must exist and
    /// contain valid `*.json` rule files — a missing directory or malformed
    /// file is a **hard `Init` error**.
    #[serde(default)]
    pub rules: RulesConfig,
    /// Inference engine limits (knowledge-system K-L1).
    ///
    /// Absent fields fall back to the defaults defined in `tdw-infer`'s
    /// [`RunLimits`]: `max_iterations = 32`, `max_derived = 10 000`.
    #[serde(default)]
    pub infer: InferLimitsConfig,
    /// Single-user identity for the finding tools (knowledge-system K-X6).
    ///
    /// Defaults to `id = "analyst"`. Override in `[knowledge.user]` in your
    /// daemon TOML. Per-session identity arrives with K-L5.
    #[serde(default)]
    pub user: UserConfig,
    /// Scheduled in-daemon retrieval self-eval (K-L3).
    ///
    /// When `split_id` is set, the daemon registers a cron trigger that runs a
    /// golden-split regression test on the configured cadence and surfaces the
    /// result as `eval_freshness` in `tdw.kg.status`.  When `split_id` is absent
    /// (the default), no trigger is registered and status reports `Unconfigured`.
    #[serde(default)]
    pub evals: ScheduledEvalConfig,
}

/// Auto-tagging rule-set configuration (knowledge-system K-L1).
///
/// `rules_dir` is optional — absent means "no rules loaded"; the daemon logs
/// this loudly at boot so the operator knows auto-tagging AND inference are
/// disabled. When set, the directory must exist and every rule file must parse
/// correctly; a nonexistent directory or malformed file is a **hard `Init`
/// error**, never a silent skip.
///
/// # File-naming convention (K-L1)
///
/// Two distinct rule types live in separate files identified by their suffix:
///
/// - **`*.tag.json`** — [`tdw_tag_rules::TagRule`] array; drives auto-tagging
///   via the `RuleEngine`. These run synchronously inside the indexer.
/// - **`*.infer.json`** — [`tdw_infer::InferRule`] array (serde shapes
///   `DeriveEdge` / `PropagateTag` as documented in `tdw-infer`); drives
///   forward-chaining inference via the `InferEngine`. These fire after every
///   ingest batch via `run_incremental`.
///
/// Files with any other extension (e.g. plain `.json`) are **ignored** so
/// `README.json` or schema files don't accidentally parse as rules. Lexicographic
/// sort order within each group is deterministic.
///
/// # Load limits
///
/// `max_files`, `max_file_size_kb`, and `max_total_rules` cap the total rules
/// loaded at boot and on every hot-reload tick. Exceeding any limit is a **hard
/// `Init` error** at boot and a loud log + keep-old on reload.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RulesConfig {
    /// Directory containing `*.tag.json` (tag rules) and `*.infer.json`
    /// (inference rules) files.  `None` disables all rule-driven derivation.
    #[serde(default)]
    pub rules_dir: Option<String>,
    /// Maximum number of rule files (tag + infer combined) permitted in the
    /// directory. A hard `Init` error (or loud reload rejection) beyond this.
    /// Default: 64.
    #[serde(default)]
    pub max_files: Option<usize>,
    /// Maximum size of any single rule file in kilobytes. A hard `Init` error
    /// (or loud reload rejection) if any file exceeds this. Default: 256 KiB.
    #[serde(default)]
    pub max_file_size_kb: Option<u64>,
    /// Maximum total number of rules (tag + infer combined) loaded from the
    /// directory. A hard `Init` error (or loud reload rejection) beyond this.
    /// Default: 1 000.
    #[serde(default)]
    pub max_total_rules: Option<usize>,
}

/// Inference-engine run limits (knowledge-system K-L1).
///
/// Both fields are optional and default to the constants defined in `tdw-infer`
/// (`max_iterations = 32`, `max_derived = 10 000`). Exceeding either bound is
/// logged as an error, never silently truncated (B7 contract).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct InferLimitsConfig {
    /// Maximum stratum-iterations across one inference run.
    /// Defaults to `tdw-infer`'s `RunLimits::default().max_iterations` (32).
    #[serde(default)]
    pub max_iterations: Option<usize>,
    /// Maximum new facts (edges + tags) one run may derive.
    /// Defaults to `tdw-infer`'s `RunLimits::default().max_derived` (10 000).
    #[serde(default)]
    pub max_derived: Option<usize>,
}

/// Which graph-database backend the knowledge graph uses.
///
/// Both `in-memory` (the zero-config default) and `bolt` (Memgraph/Neo4j,
/// production) are first-class. There is **NO silent fallback**: a `bolt`
/// backend whose URI is unreachable at daemon startup is a hard `Init` error,
/// and a `bolt` backend requested without the `bolt` Cargo feature is a hard
/// `Init` error. The `in-memory` default is an explicit choice, not a fallback.
///
/// **Production posture**: compose/production configs **must** set
/// `backend = "bolt"` explicitly. The `full` and `live` Docker Compose
/// profiles do this via environment. The `in-memory` default is intentional
/// for zero-config first runs and offline development only.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GraphConfig {
    /// `in-memory` (default; zero-config, not persisted) or `bolt`
    /// (Memgraph/Neo4j; production). See [`KnowledgeConfig::graph`].
    pub backend: String,
    /// Bolt endpoint URI, e.g. `bolt://127.0.0.1:7687`. Required when
    /// `backend = bolt`; ignored for `in-memory`.
    #[serde(default)]
    pub bolt_uri: Option<String>,
    /// Bolt username. Defaults to the empty string (Memgraph auth-disabled).
    #[serde(default)]
    pub bolt_user: String,
    /// Environment variable holding the Bolt password. The variable is read at
    /// daemon startup, not at config load. Defaults to `TDW_GRAPH_PASSWORD`.
    /// The resolved value is the empty string when the variable is unset
    /// (Memgraph auth-disabled default).
    #[serde(default)]
    pub bolt_password_env: String,
    /// Bolt database name. Memgraph's default is `"memgraph"`; Neo4j's is
    /// `"neo4j"`. neo4rs defaults to `"neo4j"` when unset, which fails against
    /// Memgraph — so we default explicitly to `"memgraph"` here.
    #[serde(default = "GraphConfig::default_bolt_db")]
    pub bolt_db: String,
}

impl GraphConfig {
    fn default_bolt_db() -> String {
        "memgraph".to_string()
    }
}

impl Default for GraphConfig {
    /// Returns the zero-config default: `backend = "in-memory"`.
    ///
    /// This allows `tdw-service` to start without any external graph database
    /// on a first run. Graph data is **not persisted** across restarts in this
    /// mode; a startup notice is printed to stderr as a reminder.
    ///
    /// Production environments **must** override `backend = "bolt"` in their
    /// daemon TOML or Compose environment. The `full` and `live` Compose
    /// profiles set `bolt` explicitly — this default does not affect them.
    ///
    /// Bolt field defaults (`bolt_uri`, `bolt_user`, `bolt_password_env`,
    /// `bolt_db`) are kept at their production-appropriate values so that
    /// operator configs that only set `backend = "bolt"` work without having
    /// to repeat every field.
    fn default() -> Self {
        Self {
            backend: "in-memory".to_string(),
            bolt_uri: Some("bolt://127.0.0.1:7687".to_string()),
            bolt_user: String::new(),
            bolt_password_env: "TDW_GRAPH_PASSWORD".to_string(),
            bolt_db: Self::default_bolt_db(),
        }
    }
}

/// Which embedder the knowledge index uses.
///
/// BOTH the local model and the API providers are first-class; this switch
/// selects one — there is NO silent fallback (a requested-but-unavailable
/// provider is a startup error). Collections are namespaced per embedder
/// model id, so switching never mixes dimensions; re-embed existing
/// documents with `tdw kg reindex`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EmbeddingConfig {
    /// `hash` (deterministic offline default) | `local` (on-disk model via
    /// the `local-model` build feature) | `openai` | `google`.
    pub provider: String,
    /// Provider-specific model id (API model name; informational for `local`,
    /// whose id derives from the model directory).
    #[serde(default)]
    pub model: Option<String>,
    /// Directory holding `config.json` + `tokenizer.json` +
    /// `model.safetensors` for the `local` provider.
    #[serde(default)]
    pub model_dir: Option<String>,
    /// When set, the constructed embedder's dimension must match — a cheap
    /// guard against pointing `model_dir` at the wrong model.
    #[serde(default)]
    pub expected_dims: Option<usize>,
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        Self {
            provider: "hash".to_string(),
            model: None,
            model_dir: None,
            expected_dims: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TdwConfig {
    pub profile: String,
    pub paths: PathsConfig,
    pub daemon: DaemonConfig,
    pub session: SessionConfig,
    pub worker: WorkerConfig,
    pub permissions: PermissionConfig,
    pub model: ModelConfig,
    pub protocol: ProtocolConfig,
    /// Defaulted so pre-B6 config files keep deserializing unchanged.
    #[serde(default)]
    pub knowledge: KnowledgeConfig,
}

impl Default for TdwConfig {
    fn default() -> Self {
        Self {
            profile: "default".to_string(),
            paths: PathsConfig {
                data_dir: "~/.tdw".to_string(),
                rollout_dir: "~/.tdw/sessions".to_string(),
            },
            daemon: DaemonConfig {
                transport: DaemonTransport::Tcp,
                uds_path: "~/.tdw/daemon.sock".to_string(),
                tcp_bind: Some("127.0.0.1:8787".to_string()),
                http_bind: None,
            },
            session: SessionConfig {
                sqlite_path: "~/.tdw/session.sqlite".to_string(),
                jsonl_archive: true,
            },
            worker: WorkerConfig {
                backend: WorkerBackend::Sqlite,
                sqlite_path: "~/.tdw/worker.sqlite".to_string(),
                postgres_url_env: "TDW_POSTGRES_URL".to_string(),
            },
            permissions: PermissionConfig {
                default_action: PermissionAction::Ask,
                last_match_wins: true,
            },
            model: ModelConfig {
                provider: "openai-compatible".to_string(),
                model: "unset".to_string(),
                base_url: None,
            },
            protocol: ProtocolConfig {
                max_event_bytes: 1_048_576,
                replay_enabled: true,
            },
            knowledge: KnowledgeConfig::default(),
        }
    }
}

impl TdwConfig {
    /// Validate the *semantics* of a fully merged config — the checks serde's
    /// structural decode can't make. Called at the end of [`merge_layers`] so an
    /// invalid config fails fast at load instead of much later at bind/use.
    ///
    /// Enforces: bind addresses parse as socket addresses; the active transport
    /// has the bind it needs; `protocol.max_event_bytes > 0`; the model id is
    /// non-empty/control-free; and `model.base_url` (when set) is an http(s)
    /// URL with no whitespace/control characters.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::Validation`] describing the first invalid field.
    #[allow(clippy::too_many_lines)]
    pub fn validate(&self) -> Result<()> {
        use std::net::SocketAddr;
        use std::str::FromStr;

        // Bind addresses, when present, must parse as socket addresses.
        for (field, bind) in [
            ("daemon.tcp_bind", &self.daemon.tcp_bind),
            ("daemon.http_bind", &self.daemon.http_bind),
        ] {
            if let Some(bind) = bind
                && SocketAddr::from_str(bind).is_err()
            {
                return Err(ConfigError::Validation(format!(
                    "{field} is not a valid socket address: {bind:?}"
                )));
            }
        }

        // The active transport must have the bind/path it will use, so a
        // misconfigured transport is caught at load rather than at bind time.
        match self.daemon.transport {
            DaemonTransport::Tcp if self.daemon.tcp_bind.is_none() => {
                return Err(ConfigError::Validation(
                    "daemon.transport = Tcp requires daemon.tcp_bind".to_string(),
                ));
            }
            DaemonTransport::HttpSse if self.daemon.http_bind.is_none() => {
                return Err(ConfigError::Validation(
                    "daemon.transport = HttpSse requires daemon.http_bind".to_string(),
                ));
            }
            DaemonTransport::Uds if self.daemon.uds_path.trim().is_empty() => {
                return Err(ConfigError::Validation(
                    "daemon.transport = Uds requires a non-empty daemon.uds_path".to_string(),
                ));
            }
            _ => {}
        }

        // A zero cap would reject every event.
        if self.protocol.max_event_bytes == 0 {
            return Err(ConfigError::Validation(
                "protocol.max_event_bytes must be greater than 0".to_string(),
            ));
        }

        // Model id: non-empty, no control characters.
        if self.model.model.trim().is_empty() || self.model.model.chars().any(char::is_control) {
            return Err(ConfigError::Validation(format!(
                "model.model is not a valid model id: {:?}",
                self.model.model
            )));
        }

        // Optional base URL: http(s) scheme, no whitespace/control characters.
        if let Some(base_url) = &self.model.base_url {
            let has_unsafe = base_url
                .chars()
                .any(|character| character.is_whitespace() || character.is_control());
            if has_unsafe || !(base_url.starts_with("http://") || base_url.starts_with("https://"))
            {
                return Err(ConfigError::Validation(format!(
                    "model.base_url must be an http(s) URL with no whitespace/control chars: \
                     {base_url:?}"
                )));
            }
        }

        // Knowledge embedder: a known provider token, and `local` must say
        // where its model lives — there is no silent fallback at runtime, so
        // an unusable selection should already fail at config load.
        let embedding = &self.knowledge.embedding;
        match embedding.provider.as_str() {
            "hash" | "openai" | "google" => {}
            "local" => {
                if embedding
                    .model_dir
                    .as_deref()
                    .is_none_or(|dir| dir.trim().is_empty())
                {
                    return Err(ConfigError::Validation(
                        "knowledge.embedding.provider = local requires \
                         knowledge.embedding.model_dir"
                            .to_string(),
                    ));
                }
            }
            other => {
                return Err(ConfigError::Validation(format!(
                    "knowledge.embedding.provider must be hash|local|openai|google, got {other:?}"
                )));
            }
        }

        // Graph backend: known token, and `bolt` must supply a URI.
        let graph = &self.knowledge.graph;
        match graph.backend.as_str() {
            "in-memory" => {}
            "bolt" => {
                if graph
                    .bolt_uri
                    .as_deref()
                    .is_none_or(|uri| uri.trim().is_empty())
                {
                    return Err(ConfigError::Validation(
                        "knowledge.graph.backend = bolt requires knowledge.graph.bolt_uri"
                            .to_string(),
                    ));
                }
            }
            other => {
                return Err(ConfigError::Validation(format!(
                    "knowledge.graph.backend must be bolt|in-memory, got {other:?}"
                )));
            }
        }

        validate_knowledge(&self.knowledge)?;

        // Scheduled eval: cadence must be a 5-field cron expression; thresholds
        // must be in [0.0, 1.0].  Validation runs regardless of whether
        // `split_id` is set so operators catch typos before enabling the eval.
        let evals = &self.knowledge.evals;

        // Cadence: exactly 5 whitespace-separated fields, each field containing
        // only cron-legal characters ([0-9*/,\-?LWC#]).  A full semantic parse
        // requires `tdw-cron` (which would introduce a dependency cycle here);
        // this structural check catches the most common mistakes (wrong field
        // count, stray characters) at config-load time.
        {
            let fields: Vec<&str> = evals.cadence.split_whitespace().collect();
            if fields.len() != 5 {
                return Err(ConfigError::Validation(format!(
                    "knowledge.evals.cadence must be a 5-field cron expression \
                     (e.g. \"0 3 * * MON\"), got {:?}",
                    evals.cadence
                )));
            }
            let legal: fn(char) -> bool = |c| {
                c.is_ascii_digit()
                    || matches!(c, '*' | '/' | ',' | '-' | '?' | 'L' | 'W' | 'C' | '#' | 'A'..='Z' | 'a'..='z')
            };
            for field in &fields {
                if field.chars().any(|c| !legal(c)) {
                    return Err(ConfigError::Validation(format!(
                        "knowledge.evals.cadence contains an invalid cron field {:?} \
                         in expression {:?}",
                        field, evals.cadence
                    )));
                }
            }
        }

        // Regression thresholds must be in [0.0, 1.0].
        for (name, value) in [
            ("knowledge.evals.max_recall_drop", evals.max_recall_drop),
            ("knowledge.evals.max_mrr_drop", evals.max_mrr_drop),
            ("knowledge.evals.max_ndcg_drop", evals.max_ndcg_drop),
        ] {
            if !(0.0..=1.0).contains(&value) {
                return Err(ConfigError::Validation(format!(
                    "{name} must be in [0.0, 1.0], got {value}"
                )));
            }
        }

        Ok(())
    }
}

/// Validate all `[knowledge.*]` sub-sections (K-L1 + K-X6).
///
/// Extracted from [`TdwConfig::validate`] so that function stays under the
/// 100-line function-length lint limit.
fn validate_knowledge(knowledge: &KnowledgeConfig) -> Result<()> {
    validate_rules_infer(&knowledge.rules, &knowledge.infer)?;
    // User identity: grammar [A-Za-z0-9:._-]+, non-empty, ≤128 bytes.
    validate_user_id(&knowledge.user.id)
}

fn validate_rules_infer(rules: &RulesConfig, infer: &InferLimitsConfig) -> Result<()> {
    // Rules dir: when set, the path must not be empty; existence is checked at
    // daemon boot (a nonexistent configured dir is a hard Init error there —
    // validate() only rejects structurally invalid values like whitespace-only).
    if let Some(dir) = &rules.rules_dir
        && dir.trim().is_empty()
    {
        return Err(ConfigError::Validation(
            "knowledge.rules.rules_dir must be a non-empty path when set".to_string(),
        ));
    }
    // Load limits: when set, values must be greater than zero.
    if let Some(max_files) = rules.max_files
        && max_files == 0
    {
        return Err(ConfigError::Validation(
            "knowledge.rules.max_files must be greater than 0 when set".to_string(),
        ));
    }
    if let Some(max_kb) = rules.max_file_size_kb
        && max_kb == 0
    {
        return Err(ConfigError::Validation(
            "knowledge.rules.max_file_size_kb must be greater than 0 when set".to_string(),
        ));
    }
    if let Some(max_total) = rules.max_total_rules
        && max_total == 0
    {
        return Err(ConfigError::Validation(
            "knowledge.rules.max_total_rules must be greater than 0 when set".to_string(),
        ));
    }
    // Infer limits: when set, values must be greater than zero.
    if let Some(max_iter) = infer.max_iterations
        && max_iter == 0
    {
        return Err(ConfigError::Validation(
            "knowledge.infer.max_iterations must be greater than 0 when set".to_string(),
        ));
    }
    if let Some(max_derived) = infer.max_derived
        && max_derived == 0
    {
        return Err(ConfigError::Validation(
            "knowledge.infer.max_derived must be greater than 0 when set".to_string(),
        ));
    }
    Ok(())
}

/// Validate a `knowledge.user.id` value.
///
/// Grammar: `[A-Za-z0-9:._-]+`, non-empty, ≤ 128 bytes.
/// Mirrors `tdw_knowledge::proposals::validate_agent_id` — kept inline so
/// `tdw-config` stays free of that cross-crate dependency.
fn validate_user_id(user_id: &str) -> Result<()> {
    if user_id.is_empty() {
        return Err(ConfigError::Validation(
            "knowledge.user.id must not be empty".to_string(),
        ));
    }
    if user_id.len() > 128 {
        return Err(ConfigError::Validation(format!(
            "knowledge.user.id is too long ({} bytes, max 128)",
            user_id.len()
        )));
    }
    if let Some(bad) = user_id
        .chars()
        .find(|c| !c.is_ascii_alphanumeric() && !matches!(c, ':' | '.' | '_' | '-'))
    {
        return Err(ConfigError::Validation(format!(
            "knowledge.user.id {user_id:?} contains invalid character {bad:?} \
             — only [A-Za-z0-9:._-] allowed"
        )));
    }
    Ok(())
}

#[must_use]
pub fn default_layer_order() -> Vec<ConfigLayerDescriptor> {
    [
        (ConfigLayerKind::UserDefaults, "~/.tdw/config.toml"),
        (ConfigLayerKind::EnvFile, "$TDW_CONFIG"),
        (
            ConfigLayerKind::ProjectConfig,
            "<project-root>/.tdw/config.toml",
        ),
        (
            ConfigLayerKind::SplitFile,
            "<project-root>/.tdw/{providers,udfs,hooks}/*.toml",
        ),
        (ConfigLayerKind::InlineEnv, "$TDW_CONFIG_CONTENT"),
        (ConfigLayerKind::CliFlags, "CLI flags"),
    ]
    .into_iter()
    .map(|(kind, source)| ConfigLayerDescriptor {
        kind,
        source,
        precedence: kind.precedence(),
    })
    .collect()
}

/// # Errors
///
/// Returns an error variant if the underlying operation fails.
pub fn merge_layers(layers: &[ConfigLayer]) -> Result<TdwConfig> {
    let mut ordered = layers.to_vec();
    ordered.sort_by_key(|layer| layer.kind.precedence());

    let mut merged =
        serde_json::to_value(TdwConfig::default()).map_err(|source| ConfigError::Json {
            name: "default".to_string(),
            source,
        })?;
    for layer in ordered {
        deep_merge(&mut merged, layer.value);
    }

    let config: TdwConfig = serde_json::from_value(merged).map_err(|source| ConfigError::Json {
        name: "merged".to_string(),
        source,
    })?;
    // Structural decode succeeded; now enforce the semantic contract at the
    // ingestion boundary so an invalid config fails fast here, not at bind/use.
    config.validate()?;
    Ok(config)
}

#[must_use]
pub fn config_schema() -> Value {
    schema_json::<TdwConfig>()
}

#[must_use]
pub fn schema_bundle() -> BTreeMap<&'static str, Value> {
    BTreeMap::from([
        ("tdw_config", config_schema()),
        (
            "config_layer_descriptor",
            schema_json::<ConfigLayerDescriptor>(),
        ),
    ])
}

// ---------------------------------------------------------------------------
// Per-provider credential registry (gap-matrix item L5.7)
// ---------------------------------------------------------------------------

/// One per-provider credential entry: which env var (and optional config-file
/// key) supplies the provider's API key/token.
///
/// This is the single documented source of truth for where each keyed provider
/// reads its credential from, mirroring the `credentials` section convention in
/// the `OpenBB` `user_settings.json` surface (per-provider key fields). The table
/// is static and build-independent; resolving an actual value is the consumer's
/// job via [`resolve_credential`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CredentialEntry {
    /// Provider key, e.g. `"fred"`, `"eia"` (matches the catalog candidate key).
    pub provider: &'static str,
    /// Environment variable that supplies the credential, e.g. `"FRED_API_KEY"`.
    pub env_var: &'static str,
    /// Optional dotted config-file key (`credentials.<field>`) mirroring the
    /// `OpenBB` `user_settings.json` field naming, e.g. `"fred_api_key"`.
    pub config_key: Option<&'static str>,
    /// One-line human-readable description of the credential.
    pub doc: &'static str,
}

/// The documented per-provider credential registry — the single source of truth
/// for which environment variable (and optional config-file key) each keyed
/// provider uses.
///
/// ## Architecture contract (G008 CORE2)
///
/// * **`tdw-config` (this crate)** — sole declaration surface. Every keyed
///   provider has exactly one entry here. New providers register here first.
/// * **`tdw-core::http_support`** — resolution logic only (`read_required_key`,
///   `read_optional_key`). It has no knowledge of provider names; it receives
///   the env-var name from the provider crate, which must match `env_var` in
///   this table.
///
/// Entries are in provider-key lexicographic order so the deterministic-order
/// test and snapshot diffs stay stable.
///
/// ### Required vs. optional
///
/// Providers whose `config_key` is `None` and whose `doc` says "optional" use
/// `read_optional_key` — they degrade gracefully (unauthenticated tier) when
/// the variable is absent. All other entries are required keys.
///
/// ### Multi-variable providers
///
/// Some providers accept several env-var aliases (e.g. `HuggingFace`). Only the
/// primary / preferred name appears in `env_var`; aliases are noted in `doc`.
const CREDENTIAL_REGISTRY: &[CredentialEntry] = &[
    // --- alpaca ---
    CredentialEntry {
        provider: "alpaca_key",
        env_var: "APCA_API_KEY_ID",
        config_key: Some("apca_api_key_id"),
        doc: "Alpaca brokerage API key ID.",
    },
    CredentialEntry {
        provider: "alpaca_secret",
        env_var: "APCA_API_SECRET_KEY",
        config_key: Some("apca_api_secret_key"),
        doc: "Alpaca brokerage API secret key (paired with alpaca_key).",
    },
    // --- alpha vantage ---
    CredentialEntry {
        provider: "alpha_vantage",
        env_var: "TDW_ALPHA_VANTAGE_API_KEY",
        config_key: Some("alpha_vantage_api_key"),
        doc: "Alpha Vantage market-data API key.",
    },
    // --- benzinga ---
    CredentialEntry {
        provider: "benzinga",
        env_var: "TDW_BENZINGA_API_KEY",
        config_key: Some("benzinga_api_key"),
        doc: "Benzinga news API key.",
    },
    // --- bls (optional) ---
    CredentialEntry {
        provider: "bls",
        env_var: "TDW_BLS_API_KEY",
        config_key: Some("bls_api_key"),
        doc: "BLS (Bureau of Labor Statistics) API key (optional — unauthenticated \
              tier available without a key).",
    },
    // --- ccdata ---
    CredentialEntry {
        provider: "ccdata",
        env_var: "TDW_CCDATA_API_KEY",
        config_key: Some("ccdata_api_key"),
        doc: "CryptoCompare / CCData market-data API key.",
    },
    // --- coingecko (optional) ---
    CredentialEntry {
        provider: "coingecko",
        env_var: "COINGECKO_API_KEY",
        config_key: Some("coingecko_api_key"),
        doc: "CoinGecko cryptocurrency API key (optional — free tier available \
              without a key).",
    },
    // --- databento ---
    CredentialEntry {
        provider: "databento",
        env_var: "TDW_DATABENTO_API_KEY",
        config_key: Some("databento_api_key"),
        doc: "Databento historical market-data API key.",
    },
    // --- eia ---
    CredentialEntry {
        provider: "eia",
        env_var: "TDW_EIA_API_KEY",
        config_key: Some("eia_api_key"),
        doc: "U.S. Energy Information Administration API key (free registration). \
              The provider crate reads TDW_EIA_API_KEY; EIA_API_KEY is the \
              OpenBB-parity name (documentation only — not read by any crate today).",
    },
    // --- finnhub ---
    CredentialEntry {
        provider: "finnhub",
        env_var: "TDW_FINNHUB_API_KEY",
        config_key: Some("finnhub_api_key"),
        doc: "Finnhub stock / forex / crypto API key.",
    },
    // --- fmp ---
    CredentialEntry {
        provider: "fmp",
        env_var: "TDW_FMP_API_KEY",
        config_key: Some("fmp_api_key"),
        doc: "Financial Modeling Prep API key. The provider crate reads only \
              TDW_FMP_API_KEY — there is no runtime alias fallback. FMP_API_KEY \
              is the OpenBB-parity name recorded here for documentation purposes \
              only; it is not read by the provider today.",
    },
    // --- fred ---
    CredentialEntry {
        provider: "fred",
        env_var: "FRED_API_KEY",
        config_key: Some("fred_api_key"),
        doc: "FRED (St. Louis Fed) API key.",
    },
    // --- huggingface (optional) ---
    CredentialEntry {
        provider: "huggingface",
        env_var: "HF_TOKEN",
        config_key: Some("hf_token"),
        doc: "HuggingFace inference API token (optional — some public models work \
              without authentication). Aliases tried in order: HF_TOKEN (primary), \
              HUGGINGFACE_API_TOKEN, HF_API_TOKEN.",
    },
    // --- polygon ---
    CredentialEntry {
        provider: "polygon",
        env_var: "POLYGON_API_KEY",
        config_key: Some("polygon_api_key"),
        doc: "Polygon.io API key (used for FX / crypto / index aggregate bars).",
    },
    // --- seeking alpha ---
    CredentialEntry {
        provider: "seeking_alpha",
        env_var: "TDW_SEEKING_ALPHA_API_KEY",
        config_key: Some("seeking_alpha_api_key"),
        doc: "Seeking Alpha news / analysis API key.",
    },
    // --- tiingo ---
    CredentialEntry {
        provider: "tiingo",
        env_var: "TDW_TIINGO_API_KEY",
        config_key: Some("tiingo_api_key"),
        doc: "Tiingo financial data API key.",
    },
    // --- trading economics ---
    CredentialEntry {
        provider: "trading_economics",
        env_var: "TDW_TRADING_ECONOMICS_API_KEY",
        config_key: Some("trading_economics_api_key"),
        doc: "Trading Economics macroeconomic data API key.",
    },
    // --- velodata ---
    CredentialEntry {
        provider: "velodata",
        env_var: "TDW_VELODATA_API_KEY",
        config_key: Some("velodata_api_key"),
        doc: "Velodata on-chain / derivatives API key.",
    },
];

/// The full per-provider credential registry, in deterministic provider order.
#[must_use]
pub const fn credential_registry() -> &'static [CredentialEntry] {
    CREDENTIAL_REGISTRY
}

/// Look up the credential entry for `provider`, if one is registered.
///
/// Lookup is by exact provider-key equality (the same key used in the endpoint
/// catalog's `ProviderCandidate::provider`). Returns `None` for keyless /
/// unregistered providers.
#[must_use]
pub fn credential_for_provider(provider: &str) -> Option<&'static CredentialEntry> {
    CREDENTIAL_REGISTRY
        .iter()
        .find(|entry| entry.provider == provider)
}

/// Resolve a provider's credential value from the process environment via the
/// registry's `env_var`.
///
/// Returns `None` when the provider is not registered, when the env var is
/// unset, or when it is set to an empty / whitespace-only value (treated as
/// absent so a blank key never reaches a provider). Config-file resolution is
/// the caller's responsibility — this helper covers only the env-var path, which
/// is the one every provider crate already uses today.
#[must_use]
pub fn resolve_credential(provider: &str) -> Option<String> {
    let entry = credential_for_provider(provider)?;
    let value = std::env::var(entry.env_var).ok()?;
    if value.trim().is_empty() {
        None
    } else {
        Some(value)
    }
}

/// Fuzz shim: attempt to parse arbitrary bytes as a TOML config layer.
///
/// Must never panic on adversarial input; parse failures are the expected
/// graceful outcome. Shared with the nightly cargo-fuzz target.
#[doc(hidden)]
pub fn __fuzz_config_toml(data: &[u8]) {
    let input = String::from_utf8_lossy(data);
    let _ = ConfigLayer::from_toml(ConfigLayerKind::ProjectConfig, "fuzz", &input);
}

fn deep_merge(base: &mut Value, overlay: Value) {
    match (base, overlay) {
        (Value::Object(base), Value::Object(overlay)) => merge_object(base, overlay),
        (base, overlay) => *base = overlay,
    }
}

fn merge_object(base: &mut Map<String, Value>, overlay: Map<String, Value>) {
    for (key, value) in overlay {
        match (base.get_mut(&key), value) {
            (Some(base_value), Value::Object(overlay_object)) => {
                if let Value::Object(base_object) = base_value {
                    merge_object(base_object, overlay_object);
                } else {
                    *base_value = Value::Object(overlay_object);
                }
            }
            (_, value) => {
                base.insert(key, value);
            }
        }
    }
}

fn schema_json<T: JsonSchema>() -> Value {
    serde_json::to_value(schema_for!(T))
        .unwrap_or_else(|error| panic!("config schema should serialize: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn default_layer_order_matches_contract() {
        let order = default_layer_order();
        assert_eq!(order.len(), 6);
        assert_eq!(order[0].kind, ConfigLayerKind::UserDefaults);
        assert_eq!(order[5].kind, ConfigLayerKind::CliFlags);
        assert!(
            order
                .windows(2)
                .all(|pair| pair[0].precedence < pair[1].precedence)
        );
    }

    #[test]
    fn later_layers_override_without_deleting_nested_values() {
        let layers = vec![
            ConfigLayer::new(
                ConfigLayerKind::ProjectConfig,
                "project",
                json!({
                    "model": { "provider": "anthropic", "model": "claude" },
                    "daemon": { "transport": "Uds", "uds_path": "/tmp/tdw.sock" }
                }),
            ),
            ConfigLayer::new(
                ConfigLayerKind::CliFlags,
                "cli",
                json!({
                    "model": { "model": "override" },
                    "protocol": { "max_event_bytes": 512 }
                }),
            ),
        ];

        let config = merge_layers(&layers).expect("layers merge");

        assert_eq!(config.model.provider, "anthropic");
        assert_eq!(config.model.model, "override");
        assert_eq!(config.daemon.uds_path, "/tmp/tdw.sock");
        assert_eq!(config.protocol.max_event_bytes, 512);
        assert_eq!(config.permissions.default_action, PermissionAction::Ask);
    }

    #[test]
    fn default_config_passes_validation() {
        TdwConfig::default()
            .validate()
            .expect("the shipped default config must be valid");
    }

    #[test]
    fn validate_rejects_semantic_errors() {
        // Unparseable bind address.
        let mut cfg = TdwConfig::default();
        cfg.daemon.tcp_bind = Some("not-an-address".to_string());
        assert!(matches!(cfg.validate(), Err(ConfigError::Validation(_))));

        // Tcp transport without a tcp_bind.
        let mut cfg = TdwConfig::default();
        cfg.daemon.tcp_bind = None;
        assert!(matches!(cfg.validate(), Err(ConfigError::Validation(_))));

        // HttpSse transport without an http_bind.
        let mut cfg = TdwConfig::default();
        cfg.daemon.transport = DaemonTransport::HttpSse;
        cfg.daemon.http_bind = None;
        assert!(matches!(cfg.validate(), Err(ConfigError::Validation(_))));

        // Zero event-size cap.
        let mut cfg = TdwConfig::default();
        cfg.protocol.max_event_bytes = 0;
        assert!(matches!(cfg.validate(), Err(ConfigError::Validation(_))));

        // Empty model id.
        let mut cfg = TdwConfig::default();
        cfg.model.model = "  ".to_string();
        assert!(matches!(cfg.validate(), Err(ConfigError::Validation(_))));

        // Non-http(s) base URL.
        let mut cfg = TdwConfig::default();
        cfg.model.base_url = Some("ftp://example.com".to_string());
        assert!(matches!(cfg.validate(), Err(ConfigError::Validation(_))));

        // A valid https base URL passes.
        let mut cfg = TdwConfig::default();
        cfg.model.base_url = Some("https://api.example.com/v1".to_string());
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn merge_layers_rejects_a_semantically_invalid_layer() {
        // Validation runs at the merge boundary, so a structurally-valid but
        // semantically-invalid override is rejected by merge_layers itself.
        let layers = vec![ConfigLayer::new(
            ConfigLayerKind::CliFlags,
            "cli",
            json!({ "protocol": { "max_event_bytes": 0 } }),
        )];
        assert!(matches!(
            merge_layers(&layers),
            Err(ConfigError::Validation(_))
        ));
    }

    #[test]
    fn parses_toml_layer() {
        let layer = ConfigLayer::from_toml(
            ConfigLayerKind::ProjectConfig,
            "project",
            r#"
profile = "dev"

[session]
sqlite_path = ".tdw/session.sqlite"

[worker]
backend = "Postgres"
sqlite_path = ".tdw/worker.sqlite"
postgres_url_env = "TDW_WORKER_POSTGRES_URL"
"#,
        )
        .expect("toml parses");

        let config = merge_layers(&[layer]).expect("layer merges");
        assert_eq!(config.profile, "dev");
        assert_eq!(config.session.sqlite_path, ".tdw/session.sqlite");
        assert!(config.session.jsonl_archive);
        assert_eq!(config.worker.backend, WorkerBackend::Postgres);
        assert_eq!(config.worker.sqlite_path, ".tdw/worker.sqlite");
        assert_eq!(config.worker.postgres_url_env, "TDW_WORKER_POSTGRES_URL");
    }

    #[test]
    fn graph_config_defaults_to_in_memory() {
        // K-E1: the zero-config default is in-memory so that a first run
        // requires no external services. Bolt field defaults are still
        // populated so that operator configs can override just `backend`.
        let cfg = TdwConfig::default();
        assert_eq!(cfg.knowledge.graph.backend, "in-memory");
        // Bolt URI default is still set (used when operator adds backend=bolt).
        assert!(cfg.knowledge.graph.bolt_uri.is_some());
        // The default config must pass validation.
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn graph_config_in_memory_passes_without_uri() {
        let mut cfg = TdwConfig::default();
        cfg.knowledge.graph.backend = "in-memory".to_string();
        cfg.knowledge.graph.bolt_uri = None;
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn graph_config_bolt_with_uri_passes() {
        // Bolt validation path: URI present → OK.
        let mut cfg = TdwConfig::default();
        cfg.knowledge.graph.backend = "bolt".to_string();
        cfg.knowledge.graph.bolt_uri = Some("bolt://127.0.0.1:7687".to_string());
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn graph_config_bolt_without_uri_fails() {
        let mut cfg = TdwConfig::default();
        cfg.knowledge.graph.backend = "bolt".to_string();
        cfg.knowledge.graph.bolt_uri = None;
        assert!(matches!(cfg.validate(), Err(ConfigError::Validation(_))));
    }

    #[test]
    fn graph_config_unknown_backend_fails() {
        let mut cfg = TdwConfig::default();
        cfg.knowledge.graph.backend = "neo4j-unknown".to_string();
        assert!(matches!(cfg.validate(), Err(ConfigError::Validation(_))));
    }

    #[test]
    fn exports_config_schema_bundle() {
        let bundle = schema_bundle();
        assert!(bundle.contains_key("tdw_config"));
        assert!(bundle.contains_key("config_layer_descriptor"));
    }

    #[test]
    fn rules_config_defaults_absent_and_validates_whitespace_dir() {
        let cfg = TdwConfig::default();
        assert!(cfg.knowledge.rules.rules_dir.is_none());
        assert!(cfg.validate().is_ok());

        // A whitespace-only rules_dir is rejected at config validation.
        let mut cfg = TdwConfig::default();
        cfg.knowledge.rules.rules_dir = Some("   ".to_string());
        assert!(matches!(cfg.validate(), Err(ConfigError::Validation(_))));

        // A real path string is accepted (existence is checked at daemon boot, not here).
        let mut cfg = TdwConfig::default();
        cfg.knowledge.rules.rules_dir = Some("/tmp/rules".to_string());
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn infer_limits_config_defaults_absent_and_rejects_zero() {
        let cfg = TdwConfig::default();
        assert!(cfg.knowledge.infer.max_iterations.is_none());
        assert!(cfg.knowledge.infer.max_derived.is_none());
        assert!(cfg.validate().is_ok());

        let mut cfg = TdwConfig::default();
        cfg.knowledge.infer.max_iterations = Some(0);
        assert!(matches!(cfg.validate(), Err(ConfigError::Validation(_))));

        let mut cfg = TdwConfig::default();
        cfg.knowledge.infer.max_derived = Some(0);
        assert!(matches!(cfg.validate(), Err(ConfigError::Validation(_))));

        // Valid non-zero overrides both pass.
        let mut cfg = TdwConfig::default();
        cfg.knowledge.infer.max_iterations = Some(10);
        cfg.knowledge.infer.max_derived = Some(500);
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn credential_registry_wires_fred_and_eia() {
        let fred = credential_for_provider("fred").expect("fred credential registered");
        assert_eq!(fred.env_var, "FRED_API_KEY");
        assert_eq!(fred.config_key, Some("fred_api_key"));

        // EIA: authoritative env var is TDW_EIA_API_KEY (what the provider crate
        // actually reads); EIA_API_KEY is the OpenBB-parity name (doc only).
        let eia = credential_for_provider("eia").expect("eia credential registered");
        assert_eq!(eia.env_var, "TDW_EIA_API_KEY");
        assert_eq!(eia.config_key, Some("eia_api_key"));

        assert!(credential_for_provider("yahoo").is_none());
    }

    #[test]
    fn credential_registry_wires_fmp_and_polygon() {
        // FMP: authoritative env var is TDW_FMP_API_KEY (finding #4 — FMP_API_KEY
        // was the pre-G008 OpenBB-parity alias; TDW_FMP_API_KEY is what the
        // provider crate actually reads today).
        let fmp = credential_for_provider("fmp").expect("fmp credential registered");
        assert_eq!(fmp.env_var, "TDW_FMP_API_KEY");
        assert_eq!(fmp.config_key, Some("fmp_api_key"));

        let polygon = credential_for_provider("polygon").expect("polygon credential registered");
        assert_eq!(polygon.env_var, "POLYGON_API_KEY");
        assert_eq!(polygon.config_key, Some("polygon_api_key"));
    }

    #[test]
    fn credential_registry_covers_all_g008_providers() {
        // Every provider migrated under G008 CORE2 must have a registry entry.
        for provider in &[
            "alpaca_key",
            "alpaca_secret",
            "alpha_vantage",
            "benzinga",
            "bls",
            "ccdata",
            "coingecko",
            "databento",
            "eia",
            "finnhub",
            "fmp",
            "fred",
            "huggingface",
            "polygon",
            "seeking_alpha",
            "tiingo",
            "trading_economics",
            "velodata",
        ] {
            assert!(
                credential_for_provider(provider).is_some(),
                "provider {provider} missing from CREDENTIAL_REGISTRY"
            );
        }
        // Keyless providers must NOT appear.
        assert!(credential_for_provider("yahoo").is_none());
        assert!(credential_for_provider("ecb").is_none());
    }

    #[test]
    fn credential_registry_is_in_deterministic_provider_order() {
        let providers: Vec<&str> = credential_registry()
            .iter()
            .map(|entry| entry.provider)
            .collect();
        let mut sorted = providers.clone();
        sorted.sort_unstable();
        assert_eq!(providers, sorted, "registry must be in provider-key order");
    }
}
