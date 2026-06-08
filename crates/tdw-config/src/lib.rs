#![forbid(unsafe_code)]

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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TdwConfig {
    pub profile: String,
    pub paths: PathsConfig,
    pub daemon: DaemonConfig,
    pub session: SessionConfig,
    pub worker: WorkerConfig,
    pub permissions: PermissionConfig,
    pub model: ModelConfig,
    pub protocol: ProtocolConfig,
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
        }
    }
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

    serde_json::from_value(merged).map_err(|source| ConfigError::Json {
        name: "merged".to_string(),
        source,
    })
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
