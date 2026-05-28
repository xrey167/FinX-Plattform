# Post-Merge Follow-Ups Release Decision

Date: 2026-05-28

Decision: do not cut the next patch release from this worktree.

## Rationale

- The current branch is `work/post-merge-followups` and is still unmerged.
- The branch has been rebased onto `origin/main` at:
  `f977be3 chore(repo): pin --target-dir on audit-clean-room and document WDAC gotcha (#66)`.
- The follow-up work is one local commit ahead of `origin/main` and still needs
  the normal PR review and CI path.
- Releasing from this worktree would bypass the branch-protection and
  release-evidence contract in `AGENTS.md` and `docs/release.md`.

## Release Condition

Cut the next patch release only after:

- this branch is rebased onto current `main` or otherwise made patch-equivalent
  to `main`;
- a PR is opened and CI is green;
- the PR is merged through the protected `main` path;
- release evidence is refreshed from the merged commit.

## Evidence From This Gate

The follow-up branch is release-candidate material after merge, not a release
source itself. The final aggregate gate records local verification and review
evidence; it does not create a tag or run the release workflow.
