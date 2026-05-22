# FinX-Plattform Agent Rules

## Scope And Boundary

- Treat this directory as the Git/workspace root.
- Preserve the clean-room boundary: no `finx-*` crates or dependencies, no copied
  FinX-XR code, and no `tdw-provider-openbb`.
- Keep changes scoped and verified with fresh command output.
- Use `cargo fmt --all -- --check`, `cargo check --workspace`, `cargo test --workspace`,
  and `cargo run -p xtask -- clean-room-audit` for bootstrap-level verification.

## Branches

- `main` is the only long-lived branch and is the integration target. Never commit
  directly to `main` once a remote exists; land changes through a PR.
- Use short-lived feature branches off `main`. One logical task per branch.
- Branch name prefixes (kebab-case after the prefix):
  - `work/<topic>` — feature, phase, or refactor work. Created automatically by
    `scripts/git/new-worktree.ps1`. Examples: `work/phase-01-core`,
    `work/layer-e-event-spine`.
  - `fix/<topic>` — bug fix that is not part of a tracked phase.
  - `docs/<topic>` — documentation-only change.
  - `chore/<topic>` — tooling, CI, dependency bump, gitignore, etc.
  - `review/<topic>` — read-only review or audit branch; do not push WIP commits
    onto a `review/` branch.
- Do not reuse branch names. Once merged or abandoned, the local branch and its
  worktree are deleted in the same change (see Worktrees).
- Rebase onto `main` rather than merging `main` back into a feature branch. Resolve
  conflicts locally; never force-push to `main` or to anyone else's branch.

## Worktrees

- All non-trivial work happens in a sibling worktree, not in the primary checkout.
  This isolates `target/` artifacts and lets agents work in parallel without
  colliding.
- Create with the helper, never with bare `git worktree add`:

  ```powershell
  .\scripts\git\new-worktree.ps1 -Name phase-01-core
  ```

  This creates branch `work/phase-01-core` and sibling directory
  `FinX-Plattform-phase-01-core` next to the primary checkout. The branch name is
  always `work/<Name>` — do not rename.
- Worktree lifecycle:
  1. Create with `new-worktree.ps1`.
  2. Develop, commit, push (`git push -u origin work/<topic>`).
  3. Open PR, merge.
  4. Tear down: `git worktree remove <path>` then `git branch -d work/<topic>`,
     then `git fetch --prune` to drop the remote tracking ref.
- Background agents may also create `.claude/worktrees/<name>` worktrees for
  isolation. Those are session-scoped and removed by `ExitWorktree`; do not push
  them as long-lived branches.
- Do not commit a worktree's path into the repo. The `.worktrees/` and
  `.claude/worktrees/` entries in `.gitignore` are defensive guards; the standard
  sibling-worktree layout puts checkouts outside the repo entirely.
- One worktree per topic. If a worktree drifts from its topic, finish or abandon
  it before starting new work in the same checkout.

## Commits

- Conventional Commits. Format: `type(scope): subject` in the imperative mood,
  subject ~72 chars max.
- Allowed `type` values: `feat`, `fix`, `docs`, `chore`, `refactor`, `perf`,
  `test`, `build`, `ci`, `revert`, `bootstrap` (only for foundational scaffolding
  commits — prefer `feat`/`chore` after bootstrap).
- Scope is the crate, layer, or area (`tdw-core`, `xtask`, `ci`, `dbt`, `docs`,
  `plattform`).
- Group related changes; do not bundle unrelated work into one commit.
- Never amend or force-push a commit that has already been pushed to a shared
  branch.

## Pull Requests

- Every change to `main` lands via PR, even solo work. PRs document intent and
  run CI.
- PR title mirrors the lead commit subject; PR body uses
  `.github/pull_request_template.md` and must include:
  - **Summary** — what changed and why.
  - **Verification** — the four checklist commands (fmt-check, clippy, test,
    clean-room-audit) actually run locally.
  - **Clean-Room Checklist** — no `finx-*`, no copied FinX-XR, no
    `tdw-provider-openbb`.
- A PR must be green on CI (`ci.yml` matrix + `codeql.yml`) before merge.
- Keep PRs small: prefer one phase task or one fix per PR. Split when a diff
  starts touching unrelated areas.
- Merge style: squash-and-merge for feature branches. Delete the source branch on
  merge.

## Remote (GitHub)

- Approved remote: `xrey167/FinX-Plattform`, visibility `private`.
- The remote is created with `scripts/github/create-private-repo.ps1`. Changing
  **owner**, **repository name**, or **visibility** of an existing remote
  requires a new explicit approval — do not move the repo silently.
- After remote exists:
  - `origin/main` must be protected: require PR, require CI, no force-push, no
    direct push, allow squash-and-merge only.
  - Tags follow `vMAJOR.MINOR.PATCH` once a release process is defined; not used
    pre-release.

## Verification Before Calling A Task Done

1. `cargo fmt --all -- --check`
2. `cargo clippy --workspace --all-targets -- -D warnings`
3. `cargo test --workspace`
4. `cargo run -p xtask -- clean-room-audit`
5. PR opened (or, for in-progress work, branch pushed and `gh pr status`
   reviewed). Tool output, not assumption, is the evidence.
