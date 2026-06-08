# tdw-config architecture

`tdw-config` turns an ordered set of partial config layers into one typed
`TdwConfig`. It owns the precedence rules and the deep-merge, but never reads
files or the environment — callers supply layer contents.

## Module map

A single `src/lib.rs`.

## Key types

### The config tree (`TdwConfig`)

`TdwConfig` is the root, with `Default` providing the offline baseline:

| Field | Type | Default highlights |
| --- | --- | --- |
| `profile` | `String` | `"default"` |
| `paths` | `PathsConfig { data_dir, rollout_dir }` | `~/.tdw`, `~/.tdw/sessions` |
| `daemon` | `DaemonConfig { transport, uds_path, tcp_bind, http_bind }` | TCP `127.0.0.1:8787` |
| `session` | `SessionConfig { sqlite_path, jsonl_archive }` | `~/.tdw/session.sqlite`, archive on |
| `worker` | `WorkerConfig { backend, sqlite_path, postgres_url_env }` | SQLite backend |
| `permissions` | `PermissionConfig { default_action, last_match_wins }` | `Ask`, last-match-wins |
| `model` | `ModelConfig { provider, model, base_url }` | `openai-compatible`, `unset` |
| `protocol` | `ProtocolConfig { max_event_bytes, replay_enabled }` | 1 MiB, replay on |

Enums: `DaemonTransport` (`Tcp | Uds | HttpSse`), `WorkerBackend`
(`Sqlite | Postgres`), `PermissionAction` (`Allow | Ask | Deny`).

Note: `DaemonTransport` is re-exported by `tdw-app-server` so the transport vocab
is shared across the daemon stack.

### Layers

- `ConfigLayerKind` — `UserDefaults | EnvFile | ProjectConfig | SplitFile |
  InlineEnv | CliFlags`, each with a fixed `precedence()` (10..=60).
- `ConfigLayer { kind, name, value }` — one partial config as a JSON `Value`.
  Build from raw TOML with `ConfigLayer::from_toml(kind, name, toml_str)` or from
  an already-built JSON value with `ConfigLayer::new(...)`.
- `ConfigLayerDescriptor { kind, source, precedence }` — the documented source of
  each layer, returned by `default_layer_order()`.

### Errors

`ConfigError::Toml { name, source }` for a bad TOML layer;
`ConfigError::Json { name, source }` for a serde conversion failure.

## Merge algorithm

`merge_layers(&[ConfigLayer]) -> Result<TdwConfig>`:

1. Sort layers ascending by `kind.precedence()` (so higher precedence is applied
   last and wins).
2. Start from `TdwConfig::default()` serialized to a JSON value.
3. `deep_merge` each layer's value over the accumulator: objects merge key-by-key
   recursively (`merge_object`); any non-object value is overwritten wholesale.
4. Deserialize the merged value back into `TdwConfig`.

The effect: later (higher-precedence) layers override individual leaf values
without deleting sibling/nested values the earlier layers set.

## Runtime flow

```text
files / env / flags  (read by the CALLER, e.g. tdw-backend::server::load_config)
        │  (contents)
        ▼
ConfigLayer::from_toml / ::new   ──▶  Vec<ConfigLayer>
        │
        ▼
merge_layers  ──(sort by precedence, deep-merge over default)──▶  TdwConfig
        │
        ▼
consumed by tdw-service-api::AppState::from_config, the daemon bootstrap, etc.
```

## Schema export

`config_schema()` returns the `TdwConfig` JSON Schema; `schema_bundle()` adds the
`config_layer_descriptor` schema. The hidden `__fuzz_config_toml` shim parses
arbitrary bytes as a TOML layer and must never panic (nightly cargo-fuzz target).

## Security posture

No trust decisions. The config tree merely *names* the security-relevant
settings the daemon enforces elsewhere: `permissions.default_action`,
`daemon.transport`/`tcp_bind` (loopback default), and `worker.postgres_url_env`
(the name of the env var that holds the URL, never the URL itself).

## Integration points

- `tdw-service-api::AppState::from_config` selects engines and policy from a
  `TdwConfig`.
- `tdw-backend::server::load_config` builds the layers from files/env then calls
  `merge_layers`.
- `tdw-mcp` reads `TDW_CONFIG` / `TDW_CONFIG_CONTENT` into layers to resolve the
  daemon endpoint.
