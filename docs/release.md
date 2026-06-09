# Release Packaging

Scope: G014 release packaging for `tdw-service`, `tdw-cli`, `tdw-mcp`,
and `tdw-worker`.

## Version Policy

FinX-Plattform uses SemVer tags in the form `vMAJOR.MINOR.PATCH`. The platform
reached `v1.0.0` on 2026-06-07.

From `v1.0.0` onward (SemVer):

- Increment `MAJOR` for backward-incompatible protocol, persistence, public API,
  or operator-contract changes (e.g. a default that breaks an existing exposed
  deployment without an opt-out).
- Increment `MINOR` for backward-compatible user-visible runtime, protocol,
  storage, provider, or release-packaging additions.
- Increment `PATCH` for compatible fixes, docs, CI-only changes, and packaging
  repairs that do not change runtime behavior.
- Document every breaking change, and any default that is breaking only for
  exposed/non-default deployments, in the release notes under *Upgrade notes*.

The workspace `Cargo.toml` `version` field is intentionally **not** bumped per
release: releases are tag-driven, and the field has stayed pinned since the
early tags. Do not couple a `Cargo.toml` version bump to a tag cut unless the
crates are being published to a registry (they carry `publish = false`).

The GitHub release workflow only packages tag pushes matching
`vMAJOR.MINOR.PATCH`.

User-visible changes per tag are recorded in the top-level
[`CHANGELOG.md`](../CHANGELOG.md), which follows Keep a Changelog. Each release
cut adds a dated section there before the tag is pushed.

## Release Artifacts

`.github/workflows/release.yml` builds these binaries:

- `tdw-service`
- `tdw-cli`
- `tdw-mcp`
- `tdw-worker`

Each binary is built for:

- `x86_64-unknown-linux-gnu`
- `aarch64-apple-darwin`
- `x86_64-pc-windows-msvc`

The workflow uploads compressed archives plus `.sha256` checksum files to the
GitHub Release. It also requests GitHub build-provenance attestations for the
published archive and checksum files.

## Container Images

CI builds and scans a container image for every packaged binary:

- `docker/tdw-service.Dockerfile`
- `docker/tdw-cli.Dockerfile`
- `docker/tdw-mcp.Dockerfile`
- `docker/tdw-worker.Dockerfile`

On `main` pushes, the scanned images are pushed to GHCR as:

```text
ghcr.io/<owner>/finx-plattform-tdw-service:sha-<git-sha>
ghcr.io/<owner>/finx-plattform-tdw-service:latest
ghcr.io/<owner>/finx-plattform-tdw-cli:sha-<git-sha>
ghcr.io/<owner>/finx-plattform-tdw-cli:latest
ghcr.io/<owner>/finx-plattform-tdw-mcp:sha-<git-sha>
ghcr.io/<owner>/finx-plattform-tdw-mcp:latest
ghcr.io/<owner>/finx-plattform-tdw-worker:sha-<git-sha>
ghcr.io/<owner>/finx-plattform-tdw-worker:latest
```

The scan gate fails on unfixed high or critical vulnerabilities reported by
Trivy for the built image.

## Local Compose Smoke

Validate the Compose model without a Docker daemon:

```powershell
docker compose --profile minimal config
docker compose --profile full config
docker compose --profile tools config
```

Run the packaged smoke path through Compose:

```powershell
docker compose --profile full run --rm --build tdw-service --smoke AAPL
docker compose --profile full run --rm --build tdw-worker
docker compose --profile full run --rm --build tdw-worker --durable-smoke
docker compose --profile tools run --rm --build tdw-cli AAPL
docker compose --profile tools run --rm --build tdw-mcp
```

`tdw-service` and `tdw-cli` execute the G009 deterministic smoke path. The
default `tdw-worker` run keeps the fast worker evidence path, and
`tdw-worker --durable-smoke` exercises the embedded SQLite durable scheduler.
The `full` profile also declares the production storage services so the same
composition can be extended by G015/G016 without changing the service graph.

## Pre-release Fuzz & Loom Check

Before cutting a release candidate, run the stable fuzz-smoke and loom evidence
in one command (TEST-POLICY-005):

```powershell
cargo run -p xtask -- prerelease-check
# or, if you prefer just:
just prerelease-check
```

This single entry point runs two stable suites on the stable toolchain:

1. The corpus-replay fuzz harnesses (`tests/fuzz_replay.rs`) across the six
   parser/wire-format surfaces — `tdw-protocol`, `tdw-config`, `tdw-mcp`,
   `tdw-app-client`, and `tdw-exec` — via
   `cargo test -p tdw-protocol -p tdw-config -p tdw-mcp -p tdw-app-client -p tdw-exec --test fuzz_replay`.
2. The `tdw-app-server` loom relay model via
   `cargo test -p tdw-app-server --test loom_relay`, with `RUSTFLAGS=--cfg loom`
   scoped to that one child process only (never set globally).

Expected output/artifacts: both suites pass under default `cargo test` output
(no crash reproducers written, no loom interleaving violation), and the command
prints a `fuzz-smoke (corpus replay): PASS` / `loom relay model: PASS` summary
followed by `prerelease-check: PASS`. It exits non-zero if either suite fails,
so release readiness cannot claim fuzz/loom evidence without green output.

Deep, coverage-guided fuzzing is **not** part of this command: that remains the
nightly `fuzz-smoke` CI job (`.github/workflows/nightly.yml`) and the manual
`cargo +nightly fuzz run <target>` path (targets `protocol_json`, `config_toml`,
`mcp_jsonrpc`, `mcp_http`, `daemon_frame`, `sql_guard`). Crash reproducers from
those deep runs are uploaded as CI artifacts on failure.

## Release Cut Checklist

1. Ensure `main` is green on CI and CodeQL.
2. Choose the next SemVer tag using the policy above.
3. Reconcile the changelog: run `git log <previous-tag>..HEAD --oneline`,
   confirm every user-visible change is captured under the new version section
   in [`CHANGELOG.md`](../CHANGELOG.md), and date that section. The `MINOR` vs
   `PATCH` choice must match the change set per the policy above.
4. Run the [pre-release fuzz & loom recipe](quality/pre-release-fuzz-loom.md):
   `cargo run -p xtask -- prerelease-check`; confirm the loom relay model and
   fuzz-smoke corpus replay are green (`prerelease-check: PASS`).
5. Create and push the tag:

   ```powershell
   git tag v0.MINOR.PATCH
   git push origin v0.MINOR.PATCH
   ```

6. Wait for the Release workflow to publish all 12 archives, checksum files,
   and attestations.
7. Confirm the CI image job pushed fresh GHCR `sha-<git-sha>` images from the
   same commit.

## Remaining Follow-up

This page defines the G014 release surface. G016 remains responsible for a
final aggregate proof that a release tag exists, images were published from
`main`, and the full production-functional gate is clear.
