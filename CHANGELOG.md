# Changelog

All notable changes to FinX-Plattform are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
SemVer tags `vMAJOR.MINOR.PATCH` as defined in [`docs/release.md`](docs/release.md).

While the major version is `0`, `MINOR` is incremented for user-visible
runtime, protocol, storage, provider, or release-packaging changes, and `PATCH`
for compatible fixes, docs, CI-only changes, and packaging repairs.

## [Unreleased]

_Nothing yet._

## [0.2.0] - 2026-05-29

First substantial release after the early `v0.1.0`/`v0.1.1` packaging tags
(both cut on 2026-05-28). The 15 commits in `v0.1.1..HEAD` land the daemon
runtime, the MCP server surface, durable worker schedulers, and the live data
backend - user-visible runtime and protocol work, so per the pre-1.0 policy
this is a `MINOR` release.

### Added

- **Daemon runtime (ADR-0012).** Completed the P0-P8 daemon integration cycle
  and the stateful `tdw-mcp` stdio JSON-RPC MCP server (`initialize`,
  `tools/*`, `resources/*`, `prompts/*`, progress notifications, cancellation,
  and error paths).
- **MCP Streamable HTTP transport.** `tdw-mcp --streamable-http [bind]` serves
  the same MCP protocol over a local-first HTTP endpoint at `/mcp`, with Origin
  validation, `MCP-Protocol-Version` checks, header/body size bounds, and
  optional bearer auth via `TDW_MCP_HTTP_TOKEN` (#63).
- **Daemon-backed MCP tools.** `tdw.daemon.triage` and
  `tdw.daemon.query.submit` build `OpEnvelope` operations and route them
  through `tdw-app-client` to a live daemon, returning event evidence as
  structured MCP output; deterministic offline tools continue to run without a
  daemon.
- **MCP daemon-client transport expansion.** `tdw-app-client` submits daemon
  operations over TCP, `cfg(unix)` Unix domain sockets, and plain HTTP/SSE
  (`POST /op` + `GET /events`). Selection is configurable through
  `TDW_MCP_DAEMON_TRANSPORT`, `TDW_MCP_DAEMON_ADDR` (or `TDW_DAEMON_TCP_BIND`),
  and `TDW_MCP_DAEMON_TIMEOUT_MS`; unsupported endpoints (Windows UDS, HTTPS
  HTTP/SSE) fail closed.
- **Durable worker schedulers.** `tdw-worker` gains an embedded SQLite durable
  scheduler (priority leasing, lease expiry, retry/dead-letter, idempotent
  enqueue/complete, stats, `--durable-smoke`) and a distributed Postgres
  `PgWorkerQueue` behind the `postgres` feature that mirrors the same contract.
  The in-memory contract backend remains for offline tests.
- **G014 live data backend.** A `live` compose profile plus the `tdw-bootstrap`
  one-shot binary bring up Postgres + MinIO, apply the G013 Postgres schemas,
  and write/read back an S3 marker; documented in
  `docs/release/data-backend-runbook.md` (#47).
- **Protocol and integration test coverage.** Always-on `tdw-app-client`
  daemon-framing tests (length-delimited writes, empty/oversized frame
  rejection, terminal-event matching, HTTP/SSE submit-path derivation), an
  env-gated daemon-backed MCP integration test
  (`TDW_MCP_DAEMON_INTEGRATION_ADDR`), an env-gated durable Postgres worker test
  (`TDW_POSTGRES_TEST_URL`), and a CI worker Postgres-queue step.
- **Test-policy decisions encoded.** ADR `docs/adr/0014-test-policy-backlog.md`
  plus policy docs fix the mutation cadence (O24), the first loom model scope
  (O25), and the initial fuzz-target list (O26), with deferred enforcement
  tracked as `TEST-POLICY-001..005`.
- **Deployment guidance for the remaining product gaps.**
  [`docs/release/mcp-remote-deployment.md`](docs/release/mcp-remote-deployment.md)
  (remote MCP HTTP behind a TLS/OAuth reverse proxy) and
  [`docs/release/worker-deployment.md`](docs/release/worker-deployment.md)
  (`PgWorkerQueue` rollout, supervision, and lease/dead-letter monitoring).

### Changed

- **Bounded daemon connect.** `tdw-app-client` uses
  `TcpStream::connect_timeout` for validated TCP daemon endpoints so the
  configured timeout now covers connection establishment as well as read/write.
- **License.** Repository relicensed to dual `MIT OR Apache-2.0` (#68).
- **Reduced pedantic/nursery lint noise** across the workspace; tooling pins
  `--target-dir` on the clean-room audit and documents the WDAC gotcha (#66).

### Governance and tooling

- Agent rules tracked and worktree cleanup guarded (#48); patch-equivalent
  worktree branch deletion fixed (#49); `.mcp.json` gitignored (#61); the
  dormant `create-private-repo` bootstrap script removed (#64); the production
  functional gate documented (#46); and validated ULTRAQA characterization
  coverage salvaged.

### Notes

- `Cargo.toml` keeps `publish = false`; the workspace `version` field is not
  bumped per release (releases are tag-driven, as they were at `v0.1.1`).

## [0.1.1] - 2026-05

Packaging and fix release on top of the `v0.1.0` G014 release surface. See the
`v0.1.1` tag and its GitHub release for the packaged archives, checksums, and
attestations.

## [0.1.0] - 2026-05

Initial tagged release. G014 release-packaging surface for `tdw-service`,
`tdw-cli`, `tdw-mcp`, and `tdw-worker`: multi-target binary archives with
checksums and build-provenance attestations, plus scanned GHCR container
images. See `docs/release.md` for the full artifact and image policy.

[Unreleased]: https://github.com/xrey167/FinX-Plattform/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/xrey167/FinX-Plattform/compare/v0.1.1...v0.2.0
[0.1.1]: https://github.com/xrey167/FinX-Plattform/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/xrey167/FinX-Plattform/releases/tag/v0.1.0
