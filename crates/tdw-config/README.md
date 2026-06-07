# tdw-config

Layered configuration model for TDW. Defines the typed `TdwConfig` tree (profile,
paths, daemon, session, worker, permissions, model, protocol) plus the
precedence-ordered layer system that merges user defaults, env files, project
config, split files, inline env, and CLI flags into one effective config.

Pure data + merge logic: `#![forbid(unsafe_code)]`, no I/O. Callers read files
and pass their contents in as layers; this crate parses (TOML → JSON value),
deep-merges by precedence, and deserializes into `TdwConfig`.

## Binaries produced

None. Library crate.

## Feature flags

None.

## Key environment variables

This crate does not read the environment itself, but it names the layer sources
that operators populate (see `default_layer_order()`):

| Layer | Source | Precedence |
| --- | --- | --- |
| `UserDefaults` | `~/.tdw/config.toml` | 10 |
| `EnvFile` | `$TDW_CONFIG` (path to a TOML file) | 20 |
| `ProjectConfig` | `<project-root>/.tdw/config.toml` | 30 |
| `SplitFile` | `<project-root>/.tdw/{providers,udfs,hooks}/*.toml` | 40 |
| `InlineEnv` | `$TDW_CONFIG_CONTENT` (inline TOML) | 50 |
| `CliFlags` | CLI flags | 60 |

Higher precedence wins. The downstream `TDW_*` runtime variables that the
resolved config feeds (daemon bind, profile, engine endpoints, OIDC) are
documented in [`docs/CONFIGURATION.md`](../../docs/CONFIGURATION.md).

## Quickstart (library)

Merge a project TOML layer under a CLI-flag override and read the result:

```rust
use tdw_config::{ConfigLayer, ConfigLayerKind, merge_layers};
use serde_json::json;

let project = ConfigLayer::from_toml(
    ConfigLayerKind::ProjectConfig,
    "project",
    r#"profile = "service""#,
)?;
let cli = ConfigLayer::new(
    ConfigLayerKind::CliFlags,
    "cli",
    json!({ "protocol": { "max_event_bytes": 512 } }),
);

let config = merge_layers(&[project, cli])?;
assert_eq!(config.profile, "service");
assert_eq!(config.protocol.max_event_bytes, 512);
// Untouched values come from TdwConfig::default().
assert!(config.protocol.replay_enabled);
# Ok::<(), Box<dyn std::error::Error>>(())
```

`tdw_config::config_schema()` / `schema_bundle()` export the JSON Schema for
tooling.

See [`examples/basic.rs`](examples/basic.rs):
`cargo run -p tdw-config --example basic`.

## See also

- [`ARCHITECTURE.md`](ARCHITECTURE.md) — the config tree and merge algorithm.
- [`docs/CONFIGURATION.md`](../../docs/CONFIGURATION.md) — operator `TDW_*` reference.
