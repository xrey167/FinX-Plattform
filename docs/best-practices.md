# Working Best Practices

The principles that make work on FinX-Plattform go well. This is the *why* and the
*habits*; the *steps* live in [`development-workflow.md`](development-workflow.md),
the hard rules in [`AGENTS.md`](../AGENTS.md), and cross-runtime agent guidance in
`C:\Users\ReyDa\docs\agentic-development-best-practices.md`. When this disagrees
with `AGENTS.md`, `AGENTS.md` wins.

---

## The five habits

1. **Isolate the work.** One topic per branch, one worktree per topic. Never work
   loose in the primary checkout for anything non-trivial.
2. **Evidence over assertion.** "Done" means a command ran and you read its
   output. Name the command, the artifact, and the result — never "it works".
3. **Smallest reversible diff.** Prefer the change that is easy to review and easy
   to revert. Split the moment a diff wanders off-topic.
4. **Respect the clean-room boundary.** No `finx-*`, no copied FinX-XR code or
   trait signatures, no `tdw-provider-openbb`. This is the project's reason to
   exist, not a style preference.
5. **Land through a PR.** Every change to `main` goes through a green PR, even
   solo work. The PR is the audit trail.

---

## Working environment

- **Worktrees, not stashing.** `scripts/git/new-worktree.ps1 -Name <topic>` gives
  you an isolated `target/` and lets parallel sessions run without colliding. Tear
  down with `remove-worktree.ps1` (it refuses dirty or unmerged branches by
  design — trust the guard).
- **Parallel sessions are real here.** Another session or the OmX/codex harness may
  be churning the shared checkout. Assume it. Commit and push fast; never leave
  valuable work uncommitted in a sibling worktree (`.claude/worktrees/*` can be
  wiped by parallel-session cleanup). Never `git add -A` in a shared checkout —
  stage explicit paths.
- **Don't fight the active harness.** If the OmX ultragoal harness owns the
  current goal, let it finish rather than dispatching competing parallel teams;
  catch its closeout gaps instead.
- **State belongs to its owner.** `.omc/state`, `.remember/`, `.codex/`, memory
  files — don't hand-copy state between runtimes; use the runtime's setup/doctor
  to regenerate.

## Code

- **Match the neighborhood.** New code should read like the crate it lives in —
  same naming, error handling, and module layout. The `tdw-*` crates have
  established patterns; follow them before inventing.
- **No new dependencies casually.** Adding a crate is a clean-room *and* a
  `cargo deny` decision. Justify it; prefer the workspace's existing libraries.
- **Public API changes ripple.** A change to a runtime-owned trait or registry
  shape touches many crates — map the blast radius before committing.
- **Format and lint are non-negotiable.** `cargo clippy --workspace --all-targets
  -- -D warnings` is the bar. Warnings are errors here.

## Testing

- **Test-first for behavior.** Write the failing test, then the code. Unit tests
  beside code in `mod tests`; integration/property/e2e under `tests/`.
- **Right tier for the job.** Keep billed-API calls out of default `cargo test`;
  e2e avoids billed APIs by default. Adversarial checks (injection, masking,
  authorization, OIDC claim-rejection) belong in `just test-adversarial`.
- **Coverage is a floor, not a trophy.** 82% workspace / 80% patch via Codecov.
  Don't game it with assertion-free tests.
- **Heavy gates are policy, not vibes.** Mutation, loom, and fuzz are
  nightly/release signals tracked in `docs/quality/test-policy-backlog.md` — do
  not imply they're enforced on a PR until they're wired through `xtask` and CI.

## Verification discipline

Before any "done" claim, run the smallest check that proves it, then read the
result. The full contract is one command:

```powershell
just verify-phase
```

A failed gate is a **blocker**: record the failing command, the observed output,
the affected artifact, and the fix-or-defer decision. Release readiness cannot use
quarantined tests. Keep authoring and review in **separate passes** — don't
self-approve in the same context; use a reviewer/verifier pass.

## Git & PR hygiene

- **Conventional Commits**, imperative, ~72 chars: `type(scope): subject`. Scope
  is the crate/layer (`tdw-core`, `xtask`, `ci`, `docs`, `plattform`). Group
  related changes; never bundle unrelated work.
- **Never amend or force-push** a commit already on a shared branch. Never
  force-push `main` or anyone else's branch.
- **Rebase onto `main`**, don't merge `main` back into your branch.
- **Squash-and-merge** only; delete the source branch on merge.

## Known traps (learned the hard way)

- **CI skips the wasm feature matrix.** The default CI matrix does **not** build
  `tdw-udf-wasm`/`tdw-sandbox`/`tdw-service-api` with the `wasmi`/`udf-wasm`
  features. If you touch those, build them locally with the features or you'll
  ship a green-but-broken combo.
- **The real-S3 Integration/E2E job is flaky.** It is non-required — rerun it
  rather than chasing a phantom failure.
- **`gh pr merge` lies about branch deletion** when `main` lives in a sibling
  worktree: it prints a local-delete error, but the server-side merge already
  succeeded. Verify on GitHub, don't panic.
- **Run clippy against a cold target before pushing.** A warm cache once hid a
  `needless_borrow`; the clean build caught it. Don't trust an incremental green.

## When in doubt

Explore first, then plan, then implement. Delegate multi-file or cross-cutting
work; do trivial scoped edits directly. Read the code before trusting a summary
of it — including this document.
