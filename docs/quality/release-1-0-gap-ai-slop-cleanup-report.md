# Release 1.0 Gap Cleanup Report

Scope: changed files in `work/release-1-0-gap-closure`.

## Behavior Lock

- `cargo fmt --all -- --check`
- `cargo check --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- `cargo run -p xtask -- clean-room-audit`
- `cargo run -p xtask -- prerelease-check`
- `cargo run -p tdw-service -- --smoke AAPL`
- `git diff --check`

## Cleanup Plan

1. Preserve the release-gap fixes and remove no behavior.
2. Classify fallback-like/slop signals in the changed scope.
3. Keep CI hardening explicit and evidence-preserving.
4. Record residual release risks and non-blocking debt in release evidence docs.

## Fallback Findings

Changed-scope scan for `fallback`, `workaround`, `temporary`, `bypass`,
`skip`, `swallow`, `silent default`, `TODO`, `FIXME`, and `HACK` found no new
masking fallback slop.

Classified hits:

- `docker-compose.yaml` mentions a temporary local `ports:` override for manual
  debugging. This is documentation only and does not change runtime defaults.
- Existing `serde(... skip_serializing_if ...)` and explicit skip/gate comments
  in unchanged Rust code are intentional serialization and test semantics.
- Existing shutdown comment in `tdw-backend` documents a deliberate join-handle
  cleanup path; this branch did not change that behavior.

## Passes Completed

- Fallback-like code resolution gate: no masking fallback introduced.
- Dead code deletion: not applicable; release closure edits are direct.
- Duplicate removal: not applicable; the const-fn batch import is mechanical.
- Naming/error handling cleanup: release docs now distinguish stale `v1.0.0`
  from the next SemVer-correct `v1.1.0`.
- Test reinforcement: no new tests were needed for CI retry hardening; existing
  workspace and prerelease gates cover behavior. The const-fn batch slice was
  already validated upstream in #156 after this branch rebased onto `origin/main`.

## Remaining Risks

- Docker is unavailable in the local Windows environment, so the compose retry
  hardening must be validated by GitHub Actions on the PR.
- `.batch/backlog.json` still contains pedantic/nursery debt and
  `provider:{fileset,ws}` design items; these remain non-blocking batch/product
  follow-ups per the release audit.
