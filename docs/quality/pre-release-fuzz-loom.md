# Pre-release Fuzz & Loom Recipe

Status: TEST-POLICY-005 (implemented).

Scope: release candidates only. This is a manual release-candidate step, not a
phase-exit quality gate, so it is intentionally absent from
`docs/quality/phase-exit-gates.json`. Decision record:
[`docs/adr/0014-test-policy-backlog.md`](../adr/0014-test-policy-backlog.md);
task ledger: [`docs/quality/test-policy-backlog.md`](test-policy-backlog.md).

Release readiness cannot claim fuzz/loom evidence without the green output
described below.

## One-command stable check

Run both stable suites (fuzz corpus replay + the loom relay model) in one
entry point:

```powershell
cargo run -p xtask -- prerelease-check
# or, equivalently:
just prerelease-check
```

This runs, on the stable toolchain:

1. The corpus-replay fuzz harnesses across the six parser / wire-format
   surfaces (`tdw-protocol`, `tdw-config`, `tdw-mcp`, `tdw-app-client`,
   `tdw-exec`):

   ```powershell
   cargo test -p tdw-protocol -p tdw-config -p tdw-mcp -p tdw-app-client -p tdw-exec --test fuzz_replay
   ```

2. The `tdw-app-server` loom relay model, with `RUSTFLAGS=--cfg loom` scoped to
   that single child process only (never set it globally):

   ```powershell
   $env:RUSTFLAGS = "--cfg loom"
   cargo test -p tdw-app-server --test loom_relay
   Remove-Item Env:\RUSTFLAGS
   ```

   If loom reports a too-large branch count, set
   `$env:LOOM_MAX_PREEMPTIONS = "2"`.

Expected output/artifacts: both suites pass under default `cargo test` output
(no crash reproducers written, no loom interleaving violation), and the command
prints `fuzz-smoke (corpus replay): PASS` / `loom relay model: PASS` followed by
`prerelease-check: PASS`. It exits non-zero if either suite fails.

## Deep coverage-guided fuzz short-run set (nightly toolchain)

Deep, coverage-guided fuzzing is **not** part of `prerelease-check`. Run the
bounded short-run set against each committed seed corpus from the `fuzz/`
directory on the nightly toolchain (this is the same set the scheduled
`fuzz-smoke` job in [`.github/workflows/test-policy.yml`](../../.github/workflows/test-policy.yml)
runs):

```powershell
# From the fuzz/ directory. Force the gnu target so ASan links (musl static
# libc is incompatible with the sanitizer cargo-fuzz needs).
$targets = "protocol_json","config_toml","mcp_jsonrpc","mcp_http","daemon_frame","sql_guard"
foreach ($t in $targets) {
    cargo +nightly fuzz run $t --target x86_64-unknown-linux-gnu -- -runs=10000 -max_total_time=30
}
```

To run a single target for longer when triaging a regression:

```powershell
cargo +nightly fuzz run protocol_json --target x86_64-unknown-linux-gnu -- -max_total_time=300
```

Crash / leak / timeout reproducers are written under `fuzz/artifacts/` and are
git-ignored; the scheduled CI job uploads them as the `fuzz-artifacts` artifact
on failure. If `cargo-fuzz` or a nightly toolchain is unavailable on a release
machine, the stable `prerelease-check` above is the minimum required gate and
the deep run is delegated to the scheduled CI job.

## Pre-release checklist

1. `cargo run -p xtask -- prerelease-check` is green (`prerelease-check: PASS`).
2. (Optional, recommended on RCs touching parsers/wire formats) the nightly
   short-run fuzz set above completes with no new reproducers under
   `fuzz/artifacts/`.

See the [release cut checklist](../release.md#release-cut-checklist) for where
this step sits in the full release flow.
