# Release 1.0 Readiness

Scope: `work/release-1-0` release-readiness pass for the clean-room
FinX-Plattform workspace.

## Decision

Release 1.0 was cut as `v1.0.0`, but current `origin/main` has release-relevant
changes after that tag. The corrective path is not to move `v1.0.0`; it is to
ship a new SemVer tag from green `main`. Because the post-tag delta includes
user-visible auth and worker CLI features, the next release is `v1.1.0`.

## Release-Critical Fixes

- `tdw-service-api` now exposes `provider-yahoo-http`, enabling
  `tdw-provider-yahoo/http` and selecting `YahooHttpEquityHistoricalFetcher`
  for both `default_registry()` and Yahoo execution paths when selected. Yahoo
  is a replacement under the existing `yahoo/equity_historical` key, not an
  extra registry entry.
- `all-http-providers` now includes the Yahoo HTTP fetcher, so the aggregate
  provider feature covers every feature-gated HTTP provider.
- `tdw-bootstrap`, `tdw-cli`, and `tdw-proto` now have deterministic unit tests,
  clearing their batch test-gap entries.
- `README.md` no longer describes the workspace as a pre-1.0 release-candidate
  branch; it identifies the stable 1.x release line.
- Post-`v1.0.0` changes for operator configuration, cryptographic OIDC
  verification, safe daemon defaults, worker dead-letter operations, and
  bounded worker concurrency are reconciled into the `v1.1.0` changelog.
- The requested `FinX-Plattform-batch*` sweep found
  `FinX-Plattform-batch-lint-debt-004`; after rebasing, that work is already
  present on `origin/main` via #156, so this branch only records the release
  audit outcome.

## Non-Blocking Follow-Ups

- `.batch/backlog.json` still records pedantic/nursery clippy debt for the batch
  harness. The AGENTS release gate uses `cargo clippy --workspace --all-targets
  -- -D warnings`; pedantic/nursery cleanup stays in batch-improvement scope.
- `provider:fileset` and `provider:ws` remain `needs-design`, not missing HTTP
  implementations. `tdw-provider-fileset` is a local fixture/fileset provider,
  and `tdw-provider-ws` is a streaming transport shape rather than a standard
  request/response HTTP fetcher. They should be designed as separate product
  slices, not forced into the standard `provider-*-http` pattern.

## Required Evidence Before Tag

Run the AGENTS gate locally:

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -p xtask -- clean-room-audit
```

Run the release pre-check from `docs/release.md` before cutting `v1.1.0`:

```powershell
cargo run -p xtask -- prerelease-check
```

After merge, confirm `main` CI and CodeQL are green, tag `v1.1.0`, and verify
the release workflow publishes the binary archives, checksum files, attestations,
and GHCR images from the same main commit.
