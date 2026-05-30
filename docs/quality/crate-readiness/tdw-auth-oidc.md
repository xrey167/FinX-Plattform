# tdw-auth-oidc Readiness Worksheet

Owner tranche: G005-agent-auth-hooks-tools-and-udf-crates - Agent, Auth, Hooks, Tools, and UDF Crates.

## Baseline Inventory

- Manifest: crates\tdw-auth-oidc\Cargo.toml
- Target kinds: lib
- Local dependencies: none
- External dependencies: serde ^1.0.228 features=[derive]
- Dev dependencies: none
- Reverse local dependencies: tdw-service-api
- Feature flags: none
- Test attributes detected: 2
- tests/ directory: no
- README: no
- Examples directory: no
- Scaffold/dead-code/fallback scan signals: 0 total, 0 stub-related

## Required Readiness Evidence

- [x] Manifest correctness reviewed: serde-only claim/JWKS contract remains minimal.
- [x] Dependency direction reviewed: no local dependencies; consumed by service API.
- [x] Feature flags reviewed: none.
- [x] Public API and error contracts reviewed: bool validate_claims compatibility is retained and validate_claims_strict exposes claim validation errors.
- [x] Runtime behavior reviewed: issuer, audience, subject, kid, algorithm, and role strings are checked.
- [x] Tests and coverage evidence recorded: tests cover valid claims, missing JWKS, empty subject, unsupported algorithms, and invalid roles.
- [x] Docs and examples reviewed: worksheet records the OIDC bootstrap boundary; no README/examples required.
- [x] Surface wiring reviewed: service API uses the compatibility bool path and can migrate to strict error reporting.
- [x] Scaffold, dead-code, and fallback signals classified: none.
- [x] Security and reliability risks reviewed: claims do not validate unless issuer/audience/kid/alg/role constraints pass.

## Findings

- This crate validates decoded claims against local JWKS metadata; it does not implement signature verification or JWKS fetching.
- Default accepted algorithms are constrained to RS256 and ES256.
- Follow-up boundary: token parsing, signature verification, clock validation, and JWKS cache refresh belong to the auth service adapter.

## Verification

- Focused G005 crate check passed: cargo test -p tdw-agent -p tdw-agent-store -p tdw-auth -p tdw-auth-oidc -p tdw-define -p tdw-hooks -p tdw-mask -p tdw-tools -p tdw-sandbox -p tdw-udf -p tdw-udf-external -p tdw-udf-js -p tdw-udf-python -p tdw-udf-wasm.
- Final workspace gate for G005 passed: cargo fmt --all -- --check; cargo check --workspace; cargo clippy --workspace --all-targets -- -D warnings; cargo test --workspace; cargo run -p xtask -- clean-room-audit; git diff --check.

## Verdict

Ready with follow-ups. No G005 blocker remains inside tdw-auth-oidc; follow-ups are full JWT cryptographic verification and JWKS transport.

## Policy Binding Evidence (G015)

`tdw-service-api` now calls `validate_claims_strict` at the secure ingress
boundary before authorizing or executing an endpoint. The focused G015 tests
prove an audience mismatch fails before provider or UDF work is dispatched.

## Phase 1 Closeout Evidence

- The crate now carries `//!` module docs stating its **structural-not-signature**
  scope (claim/JWKS consistency only; no JWT signature verification or JWKS
  transport), with a pointer to
  [`production-auth-oidc`](../../release/production-auth-oidc.md).
- The consumer `tdw-service-api` maps each `ClaimValidationError` to a typed
  `OidcPolicyError::InvalidClaims(..)` and asserts the exact variant per failure
  mode (`build_prod_policy_*` unit tests), and proves a prod-built policy both
  resolves a dispatch and rejects tampered ingress claims at request time
  (`prod_built_policy_*` E2E tests) — exercising this crate's strict-validation
  contract end to end.
