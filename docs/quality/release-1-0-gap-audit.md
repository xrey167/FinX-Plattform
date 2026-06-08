# Release 1.0 Gap Audit

Date: 2026-06-07
Branch: `work/release-1-0-gap-closure`

## Verdict

`v1.0.0` is not the complete current release state. It was tagged at
`fa0b6bf`, while current `origin/main` is `d2978f3` and includes five
additional release-relevant commits:

- `b979736` - operator configuration reference and compose setup docs.
- `480bd33` - cryptographic OIDC verification and secure transport defaults.
- `bb689fe` - worker dead-letter list/replay CLI and bounded concurrency.
- `fea2590` - live-stack and tools smoke CI, aarch64 release leg, and
  multi-arch image release work.
- `d2978f3` - `batch-lint-debt-004` missing-const-for-fn burn-down.

The correct release action is a new 1.x tag after this branch lands and main is
green. Because the unreleased delta contains user-visible auth and worker
features, the next SemVer release is `v1.1.0`.

## Batch Worktree Sweep

The requested `C:\Users\ReyDa\FinX-Finance\FinX-Plattform-batch*` sweep found
one sibling checkout:

- `FinX-Plattform-batch-lint-debt-004`

That checkout initially held uncommitted `missing_const_for_fn` edits in 12
Rust files. During this audit, `origin/main` advanced to `d2978f3` and landed
that same batch as #156 with `.batch/ledger/batch-lint-debt-004.md`. After
rebasing, this branch no longer carries duplicate batch code changes.

## Current Blocking State

- `v1.0.0` exists but is stale relative to `origin/main`.
- The latest completed main CI failure was run `27077801630` for `bb689fe`:
  `Integration, Property, and E2E Subset` failed before `G014 dockerized
  service smoke` could execute because Docker Hub timed out while pulling
  `clickhouse` and `postgres`.
- Newer main CI runs for `fea2590` and `d2978f3` are queued at the time of this
  audit and must go green before the next tag is cut.
- This branch hardens that gate with an explicit retrying pull step for the
  full-profile compose dependencies. The plain local
  `cargo run -p tdw-service -- --smoke AAPL` succeeds, so the observed failure
  is isolated to image-pull availability, not the service smoke path.

## Non-Blocking State

- `.batch/backlog.json` still contains pedantic/nursery clippy debt. The
  release gate remains `cargo clippy --workspace --all-targets -- -D warnings`;
  pedantic/nursery cleanup stays in batch-improvement scope.
- `provider:fileset` and `provider:ws` remain `needs-design`. They are not
  missing standard HTTP fetchers: fileset is a local fixture/fileset provider,
  and ws is a streaming transport shape. They should be implemented as separate
  product slices, not forced into `provider-*-http`.

## Evidence Commands

```powershell
git log --oneline v1.0.0..origin/main
git tag --list 'v*' --sort=-version:refname
gh run view 27077801630 --json status,conclusion,jobs
gh api repos/xrey167/FinX-Plattform/actions/jobs/79918039089/logs
gh run list --workflow CI --branch main --limit 8 --json databaseId,status,conclusion,headSha,createdAt,url
Get-ChildItem -Directory C:\Users\ReyDa\FinX-Finance -Filter 'FinX-Plattform-batch*'
cargo run -p xtask -- improve-scan
cargo run -p tdw-service -- --smoke AAPL
```
