# Development & Implementation Workflow

The single canonical path from an idea to merged code. It stitches together the
rules that already live in [`AGENTS.md`](../AGENTS.md),
[`docs/worktrees.md`](worktrees.md), [`docs/quality-gates.md`](quality-gates.md),
[`docs/testing.md`](testing.md), [`docs/github.md`](github.md), and
[`docs/release.md`](release.md). When this doc and those disagree, the
domain-specific doc wins — fix the link here.

The `/feature` slash command (`.claude/commands/feature.md`) automates steps 1–7.
For the *principles* behind these steps (habits, anti-patterns, known traps), see
[`best-practices.md`](best-practices.md).

---

## At a glance

```
1. Frame      pick one task, pick a branch name
2. Branch     scripts/git/new-worktree.ps1 -Name <topic>   ->  work/<topic>
3. Plan       write the change list + the verification you will run
4. Implement  TDD: test first, then code, smallest viable diff
5. Verify     just verify-phase   (the phase-exit gate)
6. PR         conventional commits, push, gh pr create (template)
7. Merge      green CI -> squash-merge -> remove-worktree.ps1
```

One logical task per branch. One phase task or fix per PR. If the diff starts
touching unrelated areas, split it.

---

## 1. Frame the task

- Scope it to **one logical change**. If it needs two unrelated diffs, it is two
  tasks.
- Choose the branch prefix (kebab-case after the prefix):
  - `work/<topic>` — feature, phase, or refactor work (created by the helper).
  - `fix/<topic>` — bug fix outside a tracked phase.
  - `docs/<topic>` — documentation only.
  - `chore/<topic>` — tooling, CI, deps, gitignore.
- If the work comes from the plan set, open a **Phase Task** issue
  (`.github/ISSUE_TEMPLATE/phase_task.yml`): source plan, scope, and the
  verification you will run before closing.

**Clean-room boundary (non-negotiable):** no `finx-*` crates or deps, no copied
FinX-XR code or trait signatures, no `tdw-provider-openbb`. This is enforced by
`cargo run -p xtask -- clean-room-audit` and re-checked at PR time.

## 2. Create the worktree

Never branch in the primary checkout and never use bare `git worktree add`.
Always use the helper:

```powershell
.\scripts\git\new-worktree.ps1 -Name phase-01-core
```

This fetches+prunes, creates branch `work/phase-01-core`, the sibling directory
`FinX-Plattform-phase-01-core`, and wires the shared pre-push guardrail
(`core.hooksPath .githooks`). The branch name is always `work/<Name>` — do not
rename. One worktree per topic; if a worktree drifts off-topic, finish or abandon
it first. See [`docs/worktrees.md`](worktrees.md) and
[`docs/multi-session.md`](multi-session.md).

## 3. Plan the change

Before editing, write down (in the PR description draft, the issue, or
`.omc/notepad.md`):

- The files you expect to create/modify.
- The **verification you will run** — name the gates and the artifact each
  produces. "It works" is not evidence; command output is.

For larger work, explore first, then plan, then implement.

## 4. Implement (TDD)

- **Test first.** Unit tests live beside code in `mod tests`. Integration →
  `tests/integration/`, property → `tests/property/`, e2e → `tests/e2e/`
  (avoid billed APIs by default), benches → `benches/` or `xtask`. Adversarial
  checks (injection, masking, authorization, OIDC claim-rejection) stay in
  `just test-adversarial`. See [`docs/testing.md`](testing.md).
- Smallest viable diff. Match surrounding style, naming, and comment density.
- Commit in **Conventional Commits**: `type(scope): subject`, imperative,
  ~72 chars. Allowed types: `feat`, `fix`, `docs`, `chore`, `refactor`, `perf`,
  `test`, `build`, `ci`, `revert`, `bootstrap`. Scope is the crate/layer
  (`tdw-core`, `xtask`, `ci`, `dbt`, `docs`, `plattform`). Group related changes;
  never bundle unrelated work. Never amend/force-push a commit already on a
  shared branch.
- Coverage target: 82% workspace / 80% patch (Codecov-gated).

## 5. Verify — the phase-exit gate

The gate is the repo contract for any "done" claim. Run the full local set:

```powershell
just verify-phase
```

which runs, in order:

| Recipe | What it checks |
| --- | --- |
| `fmt-check` | `cargo fmt --all -- --check` |
| `lint` | `cargo clippy --workspace --all-targets -- -D warnings` |
| `test-unit` | `cargo test --workspace --no-default-features` |
| `test-integration` | `--features integration` |
| `test-property` | `--features property` |
| `test-e2e` | `--features e2e` |
| `test-adversarial` | tag-rules / mask / auth / auth-oidc |
| `schema-sync` | regenerate agent schemas (must be drift-free) |
| `event-schema-check` | event schema drift check |
| `bench` | benchmark harness |
| `quality-gate-check` | generated gate contract still satisfied |
| `deny` | `cargo deny check` (licenses/advisories) |
| `audit` | `cargo run -p xtask -- clean-room-audit` |

The minimal PR contract (mirrored in the PR template) is the four-command subset:
`fmt --check`, `clippy -D warnings`, `cargo test --workspace`, `clean-room-audit`.
`just ci-local` reproduces the CI matrix locally.

**A failed gate is a blocker, not a footnote.** Record the failing command, the
observed output, the affected artifact, and the fix-or-defer decision. Do not
claim a checkpoint complete until it is fixed or explicitly carried in the
ultragoal evidence. Release readiness cannot use quarantined tests.

> CI does **not** build the `wasmi` / `udf-wasm` feature combinations in the
> default matrix — if you touch `tdw-udf-wasm`, `tdw-sandbox`, or
> `tdw-service-api`, run them locally with those features (see
> [memory: CI wasmi blind spot]).

Heavier release-only evidence (`just coverage`, `just windows-release`,
`just prerelease-check`) runs at release checkpoints, not every PR — see
[`docs/release.md`](release.md).

## 6. Open the PR

```powershell
git push -u origin work/<topic>
gh pr create --fill   # then complete the template body
```

- PR title mirrors the lead commit subject.
- Body uses `.github/pull_request_template.md`: **Summary**, **Verification**
  (the four checklist commands actually run locally), **Clean-Room Checklist**.
- Keep PRs small — one phase task or one fix.
- Rebase onto `main`; never merge `main` back into the branch, never force-push
  `main` or anyone else's branch.

## 7. Merge & tear down

- Wait for **green CI** — the `ci.yml` matrix + `codeql.yml`, and the branch must
  be up to date with `main`. The real-S3 Integration/E2E job is flaky; rerun it
  (it is non-required) rather than chasing a phantom failure
  ([memory: FinX PR/CI workflow]).
- **Squash-and-merge** only; delete the source branch on merge.
- `gh pr merge` may print a local-branch-delete error when `main` lives in a
  sibling worktree — the **server-side merge still succeeded**; ignore it.
- Tear down the worktree with the guarded helper (refuses dirty or
  not-merged/not-patch-equivalent branches):

```powershell
.\scripts\git\remove-worktree.ps1 -Path ..\FinX-Plattform-<topic> -RemoveBranch
```

---

## Releases (separate cadence)

Cutting a version is not part of every feature PR. Tags follow
`vMAJOR.MINOR.PATCH`; the process, pre-1.0 increment policy, target matrix, and
GHCR image policy live in [`docs/release.md`](release.md). Run `just coverage`,
`just windows-release`, and `just prerelease-check` as release evidence.

## Quick reference

| Need | Command |
| --- | --- |
| New isolated worktree | `.\scripts\git\new-worktree.ps1 -Name <topic>` |
| Format | `just fmt` / check: `just fmt-check` |
| Lint | `just lint` |
| Full gate | `just verify-phase` |
| Reproduce CI locally | `just ci-local` |
| Refresh gate contract | `just quality-gate` (after changing Justfile/CI/xtask) |
| Open PR | `gh pr create --fill` |
| Tear down worktree | `.\scripts\git\remove-worktree.ps1 -Path <path> -RemoveBranch` |
