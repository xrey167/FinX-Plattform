---
description: Drive a change through the FinX dev workflow (worktree -> TDD -> gates -> PR)
argument-hint: <topic> [short description of the change]
allowed-tools: Bash, Read, Edit, Write, Glob, Grep
---

You are driving the FinX-Plattform development & implementation workflow defined
in `docs/development-workflow.md`. Follow it exactly; that doc and `AGENTS.md` are
authoritative.

Task topic / description: **$ARGUMENTS**

Branch/worktree name (first token of the arguments, kebab-case): **$1**

Work through these steps. Stop and report if any gate fails — a failed gate is a
blocker, not a footnote.

## 1. Frame
- Restate the scope as ONE logical change. If it needs two unrelated diffs, say
  so and ask which to do first.
- Pick the branch prefix: `work/` (feature/refactor), `fix/`, `docs/`, or
  `chore/`. Default to `work/` unless the change is clearly a fix/docs/chore.
- Confirm the clean-room boundary holds: no `finx-*`, no copied FinX-XR code, no
  `tdw-provider-openbb`.

## 2. Worktree
- Create the isolated worktree with the helper (never bare `git worktree add`,
  never branch in the primary checkout):
  ```powershell
  .\scripts\git\new-worktree.ps1 -Name $1
  ```
- All subsequent edits happen in the sibling `FinX-Plattform-$1` worktree on
  branch `work/$1`.

## 3. Plan
- List the files you expect to create/modify and the exact verification you will
  run. Write it to the PR draft or `.omc/notepad.md`. Command output is the only
  acceptable evidence of "done".

## 4. Implement (TDD)
- Write the test first (unit beside code in `mod tests`; integration/property/e2e
  under `tests/`; adversarial in `just test-adversarial`).
- Smallest viable diff; match surrounding style. Coverage target 82% workspace /
  80% patch.
- Commit in Conventional Commits: `type(scope): subject`, imperative, ~72 chars.
  Group related changes only.

## 5. Verify — phase-exit gate
- Run the full gate and capture output:
  ```powershell
  just verify-phase
  ```
- If you touched `tdw-udf-wasm`, `tdw-sandbox`, or `tdw-service-api`, ALSO build
  the `wasmi` / `udf-wasm` feature combos — CI's default matrix skips them.
- On any failure: record the failing command, the output, the affected artifact,
  and the fix-or-defer decision. Do not proceed until green or explicitly
  deferred.

## 6. PR
- Push and open the PR:
  ```powershell
  git push -u origin work/$1
  gh pr create --fill
  ```
- Fill the template body (`.github/pull_request_template.md`): Summary,
  Verification (the four commands actually run), Clean-Room Checklist. PR title
  mirrors the lead commit subject. Keep it small.

## 7. Merge & teardown (only when CI is green and branch is up to date)
- Squash-and-merge only; delete the source branch on merge. A local
  branch-delete error from `gh pr merge` when `main` is in a sibling worktree is
  benign — the server merge succeeded.
- Tear down:
  ```powershell
  .\scripts\git\remove-worktree.ps1 -Path ..\FinX-Plattform-$1 -RemoveBranch
  ```

Report progress after each numbered step. Never commit directly to `main`; never
force-push `main` or a shared branch; rebase onto `main` rather than merging it
back.
