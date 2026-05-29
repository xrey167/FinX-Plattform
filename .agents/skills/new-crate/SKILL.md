---
name: new-crate
description: Scaffold a new tdw-* crate in the FinX-Plattform workspace. Creates Cargo.toml, src/lib.rs, AGENTS.md stub; registers the crate in the workspace root Cargo.toml [workspace.dependencies] table; updates crates/AGENTS.md hierarchy. Use when the user says "new crate", "scaffold crate", or "add tdw- crate".
disable-model-invocation: true
---

# new-crate

Scaffold a new `tdw-<name>` crate consistently with the existing workspace conventions.

## Inputs
Ask the user (only if not provided):
1. **Crate name** — must be `tdw-<kebab>` (e.g. `tdw-storage-clickhouse`).
2. **Family** — one of: `provider`, `storage`, `llm`, `protocol`, `runtime`, `tool`, `util`.
3. **One-line purpose** — goes into Cargo.toml `description` and AGENTS.md header.

## Steps

1. **Verify location.** Must be inside a worktree (`FinX-Plattform/` or `FinX-Plattform-<topic>/`). Refuse if `git rev-parse --show-toplevel` fails or returns the workspace root `FinX-Finance`.

2. **Create `crates/<name>/Cargo.toml`** using the template:
   ```toml
   [package]
   name = "<name>"
   version.workspace = true
   edition.workspace = true
   rust-version.workspace = true
   authors.workspace = true
   publish.workspace = true
   license.workspace = true
   description = "<purpose>"

   [lints]
   workspace = true

   [dependencies]
   thiserror.workspace = true
   ```

3. **Create `crates/<name>/src/lib.rs`** — empty module with a doc comment matching the purpose.

4. **Create `crates/<name>/AGENTS.md`** following the sibling pattern (Purpose / Key Files / Dependencies / Testing). Mirror the depth used in neighbouring `tdw-*` crates in the same family.

5. **Edit workspace root `Cargo.toml`**: add a line to `[workspace.dependencies]` in alphabetical order:
   ```toml
   <name> = { path = "crates/<name>" }
   ```
   The `members = ["crates/*", ...]` glob picks it up automatically — do not edit members.

6. **Update `crates/AGENTS.md`** — add the new crate to its family section.

7. **Verify**:
   - `cargo metadata --no-deps --format-version=1 -q` succeeds.
   - `cargo check -p <name>` succeeds.
   - `cargo fmt -p <name>` makes no further changes.

## Refuse / Escalate

- Do not run if `git status` shows unstaged changes to `Cargo.toml` — risk of merge conflicts with stacked PRs (see G010/G013 worktrees).
- Do not pick a name colliding with any existing `[workspace.dependencies]` entry.
