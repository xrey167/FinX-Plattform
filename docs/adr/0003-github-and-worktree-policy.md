# ADR-0003: GitHub And Worktree Policy

## Decision

Initialize Git locally and ship GitHub-ready files, but do not create a remote or
push without explicit approval. Use sibling Git worktrees for phase branches.

## Consequences

- Remote creation is script-backed and reviewable.
- Worktree naming stays consistent across agents.
- External GitHub state remains user-controlled.
