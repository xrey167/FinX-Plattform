# tdw-eval-runner Readiness Worksheet

Owner tranche: G006-knowledge-graph-tags-ml-eval-and-utility-crates - Knowledge, Graph, Tags, ML, Eval, and Utility Crates.

## Baseline Inventory

- Manifest: crates\tdw-eval-runner\Cargo.toml
- Target kinds: lib, test
- Local dependencies: tdw-agent, tdw-agent-store, tdw-core, tdw-embed, tdw-knowledge, tdw-retrieve, tdw-storage-meilisearch, tdw-storage-qdrant
- External dependencies: serde, serde_json, tdw-llm
- Dev dependencies: tdw-embed-local, tdw-kg, tdw-llm-anthropic, tokio
- Reverse local dependencies: tdw-backend, tdw-service-api
- Feature flags: `local-model` (activates `tdw-embed-local/model` for live BERT retrieval-eval leg; not enabled in CI)
- Test attributes detected: 19 unit + 3 integration (always-run) + 1 compile-gated integration (local-model feature; compile-verified, runtime execution env-gated on TDW_LOCAL_MODEL_DIR, not CI-live-verified — B6 precedent)
- tests/ directory: yes (retrieval_eval.rs, live_real_model.rs)
- README: no
- Examples directory: yes
- Scaffold/dead-code/fallback scan signals: 0 total, 0 stub-related

## Required Readiness Evidence

- [x] Manifest correctness reviewed.
- [x] Dependency direction reviewed: eval runner correctly sits above agent, agent-store, knowledge, and retrieve.
- [x] Feature flags reviewed: `local-model` gates the live BERT embedder leg; default CI path is fully offline.
- [x] Public API and error contracts reviewed: `retrieval_eval` module exports `RetrievalEvalCase`, `DriftKey`, `CaseScore`, `RetrievalEvalReport`, `recall_at_k`, `reciprocal_rank`, `ndcg_at_k`, `score_case`, `build_in_memory_retriever`, `run_retrieval_eval`.
- [x] Runtime behavior reviewed: `run_retrieval_eval` errors loudly on malformed cases; no silent fallback; temporal leakage structurally impossible via retriever's as_of filter.
- [x] Tests and coverage evidence recorded: 19 unit tests cover all metric functions with hand-computed values; 3 integration tests cover determinism, temporal-leakage regression, and serde round-trip; local-model leg compile-verified under `--features local-model`, runtime execution env-gated on TDW_LOCAL_MODEL_DIR and not CI-live-verified (B6 precedent).
- [x] Docs and examples reviewed: module-level docstring covers CI-vs-env-gated distinction, drift tracking, and leakage safety contract.
- [x] Surface wiring reviewed: existing EvalRunner tests continue to pass; no regressions.
- [x] Scaffold, dead-code, and fallback signals classified: none.
- [x] Security and reliability risks reviewed: no network, no docker, no wall-clock in the always-run path; DriftKey injected by caller.

## Findings

- B11 retrieval-quality harness added as `retrieval_eval` module: fixed `RetrievalEvalCase` sets (identified by `fixed_split_id`), pure metric functions (recall@k, MRR, nDCG@k), `DriftKey` matching B8's `KnowledgeVersions` triple, and `run_retrieval_eval` runner over a `Retriever`.
- Leakage safety is structural: the retriever's own temporal filter hides future/undated documents; regression test asserts zero recall for a future doc under a past `as_of`.
- Per-embedder: hash embedder always runs; local BERT embedder is env-gated behind `TDW_LOCAL_MODEL_DIR` + `local-model` feature.
- `RetrievalEvalReport` is fully serializable so successive runs can be diff-ed by the caller; no hidden global state, no wall-clock.

## Verification

- Always-run path: `cargo test -p tdw-eval-runner --target-dir target` → 19 unit + 4 integration tests passed, 0 failed.
- Local-model compile gate: `cargo test -p tdw-eval-runner --features local-model --no-run --target-dir target` → compiles clean (runtime execution not CI-live-verified; self-skips when TDW_LOCAL_MODEL_DIR is unset — B6 precedent).
- Clippy (default + feature): `cargo clippy -p tdw-eval-runner --all-targets --target-dir target -- -D warnings` → 0 warnings, 0 errors.
- Fmt: `cargo fmt -p tdw-eval-runner --check` → clean.

## Verdict

Ready with follow-ups. B11 retrieval-eval harness complete; local-model leg compile-verified; no blockers.
