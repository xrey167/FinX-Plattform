# Working in parallel: many sessions, one `main`

Multiple agents/humans often work this repo at once (separate Claude Code
sessions, teammates, CI). This is the house contract for converging all of that
onto a single line of history — `main` — **without losing or clobbering work**.

It extends [`AGENTS.md`](../AGENTS.md) (Branches / Worktrees / Commits / PRs) with
the multi-session specifics.

## The model

`main` is the one line. A pull request is the **only** join. Sessions never
share a branch; each owns a `work/<topic>` (or `fix/`, `docs/`, `chore/`) branch
in its own sibling worktree, and work only ever merges by a PR landing on `main`.
Because `main` is protected against force-push and non-fast-forward, git
physically cannot drop a commit that a merge doesn't already contain.

## The five rules

1. **One branch per session — never two sessions on one branch or shared ref.**
   Assign an owner per branch *and* per long-lived artifact (one session owns a
   release PR / the tag; feature sessions stay out of its lane). Two writers on
   one ref is the only place this model loses data.
2. **Push early, push often.** A branch that lives only on disk dies with the
   session. `git push -u origin work/<topic>` as soon as there are commits — the
   pushed branch + open PR is the durable record that survives a crash.
3. **Rebase onto `main`; never merge `main` backwards.**
   `git fetch origin && git rebase origin/main`. This is also how you recover
   from a non-fast-forward push when the remote advanced under you — rebase
   replays your commits on top, losing nothing.
4. **Small PRs, merged fast.** The longer a branch lives, the more `main` drifts
   under it and the more you rebase. One task per PR; split when a diff spreads.
5. **Tear down after merge.** Once squash-merged, remove the worktree and delete
   the local + remote branch (`scripts/git/remove-worktree.ps1 -RemoveBranch`),
   so no session later picks up a stale, already-merged branch.

## Coordination board: the shared task list

The task list (`TaskCreate` / `TaskList` / `TaskUpdate`) is **shared across
sessions** in this repo — treat it as the assignment board. Before starting,
`TaskList` to see what's claimed; claim a task by setting its `owner` and marking
it `in_progress`; express ordering with `blockedBy`. This is how sessions see
each other's intent without talking, so two of them don't build the same thing.

## What enforces this automatically

### Server-side — the "Protect main" ruleset (authoritative, unbypassable)

- **Require a pull request** + **squash-merge only** — no direct commits.
- **Block force-push and deletion** of `main`.
- **Require status checks** (`Unit (ubuntu-latest)`, `Unit (windows-latest)`,
  `Lint, Schema, and Audit`) **and "require branches to be up to date before
  merging"** — a PR cannot merge unless rebased on current `main`, so no merge
  ever silently skips another session's commit.
- **Require linear history** — keeps `main` a single readable line of squash
  commits (matches squash-merge-only).

### Local — the `pre-push` hook (fast, friendly fail before CI)

[`.githooks/pre-push`](../.githooks/pre-push) mirrors the server rules locally so
you fail in milliseconds instead of at the merge gate:

- hard-blocks direct pushes to `main`;
- warns when your branch is behind `origin/main` and need a rebase.

Wire it once per clone (relative `core.hooksPath` is shared across all worktrees
of the clone). `scripts/git/new-worktree.ps1` sets it automatically; to enable it
by hand:

```sh
git config core.hooksPath .githooks
```

## If you want zero racing: one driver, many workers

Independent, separately-launched sessions racing each other is inherently lossy
at the edges. The lower-risk alternative is **one orchestrating session** that
fans work out to teammates sharing one task list and one integration lane
(`/oh-my-claudecode:team`, or scripted `Workflow` orchestration): one driver
merges to `main` in sequence and workers never push to shared refs, so they
cannot collide.

## Recovering lost-looking work

Nothing pushed is ever truly gone: `git reflog` shows every HEAD you've been on,
`git fsck --lost-found` surfaces dangling commits, and any pushed branch or
merged PR is recoverable from the remote. The protected `main` cannot be
force-rewritten, so the integration line itself is always intact.
