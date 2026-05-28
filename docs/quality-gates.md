# Quality Gates

The phase-exit gate is the repo contract for implementation claims. Every
story checkpoint must cite the commands it ran and the artifact each command
produced.

Run the local gate set:

```powershell
just verify-phase
```

Run heavier release evidence separately:

```powershell
just coverage
just windows-release
```

Deferred heavy-test gates are tracked in
`docs/quality/test-policy-backlog.md`. Mutation score floors, broader loom
models, and fuzz target enforcement are policy tasks until their commands are
implemented and the generated gate contract is refreshed.

CI also asserts generated schema drift after `schema-sync` and
`event-schema-check` with `git diff --exit-code -- docs/schemas/agent` and
`git diff --exit-code -- docs/schemas/event`.

Refresh the machine-readable gate contract after changing `Justfile`, CI, or
`xtask` quality policy:

```powershell
just quality-gate
```

Failed gates are blockers. The checkpoint evidence must name the failing
command, the observed output, the affected artifact, and the fix or deferral
decision. Release readiness cannot use quarantined tests.
