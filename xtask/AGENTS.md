<!-- Parent: ../AGENTS.md -->
<!-- Generated: 2026-05-27 | Updated: 2026-05-27 -->

# xtask

## Purpose

Repository maintenance and verification commands packaged as a Cargo binary
crate — the Rust-native equivalent of a Makefile. Invoked from the workspace
root via `cargo run -p xtask -- <command>`. xtask wires through to
`tdw-agent`, `tdw-config`, `tdw-event`, `tdw-migration`, `tdw-protocol`, and
`tdw-sql-codegen`, so it stays consistent with the rest of the workspace
without needing a parallel scripting language.

## Key Files

| File | Description |
|------|-------------|
| `Cargo.toml` | Member of the workspace (`publish = false`); depends on the five tdw-* crates it drives. |
| `src/main.rs` | Single-file dispatcher; one `fn` per subcommand. |

## Commands

| Command | What it does |
|---------|--------------|
| `bench` | Scaffolds `docs/perf-history.json` (the workload-anchored SLO record). |
| `bench-compare <baseline>` | Compares a bench run against a baseline. |
| `quality-gate write` | Writes `docs/quality/phase-exit-gates.json`. Edit `quality_gates()` to change. |
| `quality-gate check` | Verifies the committed gates file is up to date; fails CI if stale. |
| `ddl-export <postgres\|clickhouse>` | Emits idempotent DDL for `tdw-domain` structs via `tdw-sql-codegen`. |
| `migrate up\|down\|status` | Offline migration plan over the `tdw-migration` registry. No remote DB writes. |
| `schema-sync` | Writes agent JSON Schemas to `docs/schemas/agent/`. |
| `events schema-check` | Writes event JSON Schemas to `docs/schemas/event/`. |
| `protocol schema-check` | Writes protocol JSON Schemas to `docs/schemas/protocol/`. |
| `config schema-check` | Writes config JSON Schemas to `docs/schemas/config/`. |
| `clean-room-audit` | Greps `Cargo.toml` + `crates/` for forbidden tokens (`finx-`, `tesser-`, `tdw-provider-openbb`). Required CI gate. |

## For AI Agents

### Working In This Directory

- **Subcommands are flat.** Two-word commands like `migrate up` are parsed
  positionally; do not introduce flag parsing libraries (`clap`, `pico-args`)
  unless explicitly approved.
- **xtask is allowed to write to `docs/`.** That is its job. Generated files
  live under `docs/schemas/*/`, `docs/quality/`, and `docs/perf-history.json`.
- The `clean-room-audit` command is intentionally a string scan with the
  forbidden tokens built out of pieces (`"finx" + "-"`) to avoid the audit
  flagging itself. Keep that pattern when adding new forbidden markers.

### Testing

```powershell
cargo test -p xtask
```

Two tests guard the policy: every required gate is present, and the gates
JSON is stable + enforces failure evidence.

## Dependencies

### Internal

- `tdw-agent`, `tdw-config`, `tdw-event`, `tdw-migration`, `tdw-protocol`,
  `tdw-sql-codegen` — the crates whose data xtask exports.

### External

- `serde_json` (workspace).

<!-- MANUAL: Notes added below this line are preserved on regeneration. -->
