# Contributing

## Branches And Worktrees

Use short-lived feature branches. Prefer the worktree helper:

```powershell
.\scripts\git\new-worktree.ps1 -Name phase-01-core
```

Worktrees are created as siblings of this repository so generated build output and
parallel edits do not collide.

## Required Local Checks

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -p xtask -- clean-room-audit
```

## Clean-Room Rule

Do not copy code, trait signatures, or tests from FinX-XR. FinX-XR may be read only
for high-level pattern awareness when a plan explicitly calls for it.

## Local Toolchain Gotchas

### `os error 5 (Zugriff verweigert / Access denied)` on build scripts

Symptom: a `cargo build`, `cargo check`, `cargo test`, or `cargo run -p xtask`
invocation fails on a build script with a message like:

```
could not execute process `<path>\debug\build\<crate>\build-script-build` (never executed)
Caused by:
  Zugriff verweigert (os error 5)
```

Cause: cargo wrote the build script to a path that the host's code-integrity
policy (Windows Defender Application Control / SmartScreen / a third-party EDR
minifilter) refuses to spawn from. The most common trigger is a user-side
`CARGO_TARGET_DIR` environment variable pointing at a non-system drive
(e.g. `E:\cargo-target`).

Workarounds, in order of preference:

1. Use the `cargo audit-clean-room` alias for the clean-room gate — it pins
   `--target-dir target` and so always builds into the workspace tree.
2. Pass `--target-dir target` (or another known-allowed path) on the cargo
   command line. Command-line flags take precedence over the env var.
3. Unset `CARGO_TARGET_DIR` for the cargo invocation:
   - PowerShell: `$prev = $env:CARGO_TARGET_DIR; Remove-Item Env:CARGO_TARGET_DIR; cargo <command>; $env:CARGO_TARGET_DIR = $prev`
   - bash / git-bash: `env -u CARGO_TARGET_DIR cargo <command>`

The `[env]` table in `.cargo/config.toml` does **not** help here — it sets
environment variables for child processes that cargo spawns, but cargo reads
its own `CARGO_TARGET_DIR` from the parent environment first.
