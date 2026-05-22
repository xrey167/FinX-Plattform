# tdw-knowledge Readiness Worksheet

Owner tranche: G006-knowledge-graph-tags-ml-eval-and-utility-crates - Knowledge, Graph, Tags, ML, Eval, and Utility Crates.

## Baseline Inventory

- Manifest: crates\tdw-knowledge\Cargo.toml
- Target kinds: lib
- Local dependencies: tdw-core, tdw-embed, tdw-embed-local, tdw-kg, tdw-storage-qdrant, tdw-tags
- External dependencies: serde ^1.0.228 features=[derive]; serde_json ^1.0.145; thiserror ^2.0.18; tokio ^1.52.3 kind=dev features=[macros,rt-multi-thread,sync]
- Dev dependencies: tokio
- Reverse local dependencies: tdw-service-api
- Feature flags: none
- Test attributes detected: 3
- tests/ directory: no
- README: no
- Examples directory: no
- Scaffold/dead-code/fallback scan signals: 4 total, 0 stub-related

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
