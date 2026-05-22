# tdw-agent Readiness Worksheet

Owner tranche: G005-agent-auth-hooks-tools-and-udf-crates - Agent, Auth, Hooks, Tools, and UDF Crates.

## Baseline Inventory

- Manifest: crates\tdw-agent\Cargo.toml
- Target kinds: lib
- Local dependencies: none
- External dependencies: schemars ^1.2.1; serde ^1.0.228 features=[derive]; serde_json ^1.0.145; thiserror ^2.0.18; validator ^0.20.0 features=[derive]
- Dev dependencies: none
- Reverse local dependencies: tdw-agent-store, tdw-eval-runner, tdw-service-api, tdw-workflow-engine, xtask
- Feature flags: none
- Test attributes detected: 5
- tests/ directory: yes
- README: no
- Examples directory: no
- Scaffold/dead-code/fallback scan signals: 7 total, 0 stub-related

## Required Readiness Evidence

- [ ] Manifest correctness reviewed.
- [ ] Dependency direction reviewed.
- [ ] Feature flags reviewed or marked not applicable.
- [ ] Public API and error contracts reviewed.
- [ ] Runtime behavior reviewed.
- [ ] Tests and coverage evidence recorded.
- [ ] Docs and examples reviewed.
- [ ] Surface wiring reviewed where applicable.
- [ ] Scaffold, dead-code, and fallback signals classified.
- [ ] Security and reliability risks reviewed.

## Findings

- Pending tranche audit.

## Verification

- Pending tranche audit. Record focused crate commands and any workspace commands here.

## Verdict

Pending tranche audit. This baseline worksheet is not a production-readiness attestation yet.
