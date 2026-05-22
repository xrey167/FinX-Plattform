# Protocol And Config Boundary

G002 adds the first explicit agentic CLI substrate:

- `tdw-protocol` owns serializable `Op`, `EventMsg`, `OpEnvelope`, IDs, approval
  decisions, and replay frames. It has no dependencies on TDW runtime, service,
  storage, agent, or provider crates.
- `tdw-config` owns the explicit layer order and JSON Schema output for TDW
  config. Layer precedence is user defaults, env-pointed file, project config,
  split-file config, inline env JSON, then CLI flags.
- `tdw-service-api::protocol_config_sample` proves service-facing code can
  consume protocol/config contracts directly.

Schema artifacts are generated with:

```powershell
cargo run -q -p xtask -- protocol schema-check
cargo run -q -p xtask -- config schema-check
```

The current provider and storage traits remain in `tdw-core` until the protocol
boundary is stable enough for later stories to migrate client-facing commands
onto `Op` and `EventMsg`.
