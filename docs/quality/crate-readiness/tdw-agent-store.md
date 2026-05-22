# tdw-agent-store Readiness Worksheet

Owner tranche: G005-agent-auth-hooks-tools-and-udf-crates - Agent, Auth, Hooks, Tools, and UDF Crates.

## Baseline Inventory

- Manifest: crates\tdw-agent-store\Cargo.toml
- Target kinds: lib
- Local dependencies: tdw-agent
- External dependencies: serde ^1.0.228 features=[derive]
- Dev dependencies: none
- Reverse local dependencies: tdw-eval-runner, tdw-service-api
- Feature flags: none
- Test attributes detected: 2
- tests/ directory: no
- README: no
- Examples directory: no
- Scaffold/dead-code/fallback scan signals: 0 total, 0 stub-related

## Required Readiness Evidence

- [x] Manifest correctness reviewed: serde plus tdw-agent is the expected minimal shape.
- [x] Dependency direction reviewed: depends only on tdw-agent and is consumed by eval/service surfaces.
- [x] Feature flags reviewed: none.
- [x] Public API and error contracts reviewed: existing upsert methods remain available and checked try_* methods now expose validation failures.
- [x] Runtime behavior reviewed: checked agent, workflow, and eval-run inserts prevent invalid cards, cyclic/missing workflow edges, empty statuses, and non-finite metrics.
- [x] Tests and coverage evidence recorded: tests cover persistence plus invalid agent/workflow/eval rejection.
- [x] Docs and examples reviewed: worksheet records the store boundary; no separate README/examples required.
- [x] Surface wiring reviewed: eval runner and service API can continue using compatibility APIs while follow-ups migrate to checked APIs.
- [x] Scaffold, dead-code, and fallback signals classified: none.
- [x] Security and reliability risks reviewed: checked paths prevent poisoned store state from untrusted agent metadata.

## Findings

- The crate is an in-memory bootstrap store, not a durable database adapter.
- Checked insertion APIs now exist for production call sites that need validation evidence.
- Follow-up boundary: migrate service/eval runtime paths from compatibility upserts to try_* APIs when error propagation is widened.

## Verification

- Focused G005 crate check passed: cargo test -p tdw-agent -p tdw-agent-store -p tdw-auth -p tdw-auth-oidc -p tdw-define -p tdw-hooks -p tdw-mask -p tdw-tools -p tdw-sandbox -p tdw-udf -p tdw-udf-external -p tdw-udf-js -p tdw-udf-python -p tdw-udf-wasm.
- Final workspace gate for G005 passed: cargo fmt --all -- --check; cargo check --workspace; cargo clippy --workspace --all-targets -- -D warnings; cargo test --workspace; cargo run -p xtask -- clean-room-audit; git diff --check.

## Verdict

Ready with follow-ups. No G005 blocker remains inside tdw-agent-store; follow-ups are checked API adoption by runtime consumers.
