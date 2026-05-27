# tdw-mask Readiness Worksheet

Owner tranche: G005-agent-auth-hooks-tools-and-udf-crates - Agent, Auth, Hooks, Tools, and UDF Crates.

## Baseline Inventory

- Manifest: crates\tdw-mask\Cargo.toml
- Target kinds: lib
- Local dependencies: tdw-hooks
- External dependencies: serde ^1.0.228 features=[derive]
- Dev dependencies: none
- Reverse local dependencies: tdw-service-api
- Feature flags: none
- Test attributes detected: 3
- tests/ directory: no
- README: no
- Examples directory: no
- Scaffold/dead-code/fallback scan signals: 1 total, 0 stub-related

## Required Readiness Evidence

- [x] Manifest correctness reviewed: serde plus tdw-hooks matches mask rule and hook export needs.
- [x] Dependency direction reviewed: depends only on hooks; consumed by service API.
- [x] Feature flags reviewed: none.
- [x] Public API and error contracts reviewed: compatibility apply_masks remains fail-closed and try_apply_masks exposes invalid field errors.
- [x] Runtime behavior reviewed: rule validation rejects unsafe field names before masking; compatibility path redacts rather than leaking on invalid rules.
- [x] Tests and coverage evidence recorded: tests cover Last4 masking, fail-closed compatibility behavior, hook export, and invalid field rejection.
- [x] Docs and examples reviewed: worksheet records mask behavior; no README/examples required.
- [x] Surface wiring reviewed: service API consumes mask rules and masking hook.
- [x] Scaffold, dead-code, and fallback signals classified: remaining scan signal is the explicit fail-closed compatibility wrapper.
- [x] Security and reliability risks reviewed: checked path prevents malformed field selectors from being treated as row keys and compatibility path redacts all values on invalid rules.

## Findings

- Masking remains deterministic and string-based for bootstrap.
- Checked masking path rejects field names with shell/query-like separators.
- Compatibility wrapper now fails closed by redacting all values when invalid rules are provided.
- Follow-up boundary: richer data-type aware masking and policy-driven row/column binding belong in the service/query layer.

## Verification

- Focused G005 crate check passed: cargo test -p tdw-agent -p tdw-agent-store -p tdw-auth -p tdw-auth-oidc -p tdw-define -p tdw-hooks -p tdw-mask -p tdw-tools -p tdw-sandbox -p tdw-udf -p tdw-udf-external -p tdw-udf-js -p tdw-udf-python -p tdw-udf-wasm.
- Final G008 cleanup check passed: cargo test -p tdw-mask.
- Final workspace gate for G005 passed: cargo fmt --all -- --check; cargo check --workspace; cargo clippy --workspace --all-targets -- -D warnings; cargo test --workspace; cargo run -p xtask -- clean-room-audit; git diff --check.

## Verdict

Ready with follow-ups. No G005 blocker remains inside tdw-mask; follow-ups are service-level policy binding.

## Policy Binding Evidence (G015)

`tdw-service-api` now applies configured `MaskRule` values to outgoing secure
endpoint and secure UDF JSON responses. Focused tests prove the provider field
is redacted on an otherwise successful response.
