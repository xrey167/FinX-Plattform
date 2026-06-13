# Review Gate: CI-Only Checks the Four-Command List Misses

The four-command verification list in [`AGENTS.md`](../AGENTS.md) —
`cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
`cargo test --workspace`, `cargo run -p xtask -- clean-room-audit` — is the
*floor*, not the gate. It runs **debug**, single-target, single-snapshot. CI runs
more. Every gap below caused a real CI failure during the knowledge-system-2 wave:
the PR passed all four commands locally, auto-merge was armed, and CI went red
anyway.

**Meta-principle: the local gate must mirror what
[`.github/workflows/ci.yml`](../.github/workflows/ci.yml) actually runs.** Before
arming `gh pr merge --squash --auto`, open that file and run the same
fmt/clippy/schema/audit/release steps it runs. This doc is the distilled checklist;
`ci.yml` is the source of truth.

---

## The checklist (run before arming auto-merge)

### 1. Release build + release tests — not just debug

The debug `cargo check` / `cargo test` you ran does **not** cover the **Windows
Release** required job (`cargo build --workspace --release`) or release-only
determinism behavior.

```
cargo build --release --workspace
cargo test --release            # for any crate with determinism-sensitive or release-only behavior
```

Determinism assertions must compare **only deterministic outputs** — fixed-seed
quality metrics, bit-exact floats via `.to_bits()`. **Never** assert on:

- latency or wall-clock timing,
- `HashMap` / `HashSet` iteration order.

> Real failure: **K-M5 (#405)** passed debug `cargo check`/`cargo test` but failed
> *Windows Release* and the *release determinism* check in CI.

### 2. Regenerate the schema-drift golden after touching MCP tools or endpoints

The **Lint, Schema, and Audit** required job regenerates schemas and runs
`git diff --exit-code`. If you added/removed an MCP tool, endpoint, event, or SDK
surface and did not regenerate, CI fails on the drift. Run the exact regen the job
runs (from `ci.yml`, the `lint` job):

```
cargo run -p xtask -- schema-sync     && git diff --exit-code -- docs/schemas/agent
cargo run -p xtask -- events schema-check && git diff --exit-code -- docs/schemas/event
cargo run -p xtask -- openapi-sync    && git diff --exit-code -- docs/schemas/openapi.json
cargo run -p xtask -- pysdk-sync      && git diff --exit-code -- sdk/python/finx_platform
```

**CRITICAL:** regenerate the golden **again after EACH rebase** onto a `main` that
gained other new tools. A golden regenerated against an older `main` goes stale the
moment `main` moves — the drift check compares against current `main`'s surface, not
the one you branched from.

> Real failures: **K-X9 (#413)**, **K-X10 (#412)** — stale schema goldens after
> rebase.

### 3. Coverage and the pedantic+nursery ratchet are INFORMATIONAL — do not chase them

They show **red** but do **not** gate merge. Don't burn time turning them green.

**Required checks that DO gate merge:**

- Unit (ubuntu) / Unit (windows)
- Lint, Schema, and Audit
- Windows Release
- aarch64 Linux Build
- Integration, Property, and E2E Subset
- Change Detection
- CodeQL / Analyze

**Informational (non-blocking):** Coverage, pedantic+nursery clippy ratchet.

> Real case: **K-M6 (#415)** auto-merged cleanly with *Coverage* red — as designed.

### 4. Pin the clock in date-sensitive tests

Never read wall-clock time in a test. A test that calls `Utc::now()` (or equivalent)
passes today and fails in CI on a different date. Inject the clock:

```rust
McpServer::with_now(/* fixed instant */)   // or the crate's injected-now seam
```

> Real failure: **#414** (fix-thesis-clock) — date-brittle test.

### 5. After ANY additive rebase, run the full-target workspace check

```
cargo check --workspace --all-targets
```

A rebase that resolves the hot-conflict files can silently drop a tool registration
or leave a `<<<<<<<` conflict marker behind, causing **broad** workspace CI failure.
Watch these two in particular:

- `crates/tdw-backend/src/data/mod.rs` (`from_config`)
- `crates/tdw-mcp/src/lib.rs` (tool registrations)

> Real failure: **K-R8 (#411)** — dropped registration / leftover marker after
> rebase.

---

## TL;DR

Run the four `AGENTS.md` commands, then add these CI-only steps before arming
auto-merge:

1. `cargo build --release --workspace` + release tests for determinism-sensitive crates.
2. Regenerate schema goldens (`schema-sync` / `events schema-check` / `openapi-sync` / `pysdk-sync`) + `git diff --exit-code` — **again after every rebase**.
3. Ignore Coverage + ratchet (informational); only the required checks gate.
4. Pin the clock in date-sensitive tests.
5. `cargo check --workspace --all-targets` after any additive rebase.

When in doubt, read [`.github/workflows/ci.yml`](../.github/workflows/ci.yml) and run
what it runs.
