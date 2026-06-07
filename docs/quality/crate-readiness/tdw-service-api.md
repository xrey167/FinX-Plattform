# tdw-service-api Readiness Worksheet

Owner tranche: G007-client-service-mcp-acp-runtime-and-worker-crates - Client, Service, MCP, ACP, Runtime, and Worker Crates.

## Baseline Inventory

- Manifest: crates\tdw-service-api\Cargo.toml
- Target kinds: lib
- Local dependencies: tdw-acp, tdw-actor, tdw-agent, tdw-agent-store, tdw-app-client, tdw-app-server, tdw-auth, tdw-auth-oidc, tdw-bus, tdw-cdc, tdw-config, tdw-core, tdw-define, tdw-domain, tdw-embed, tdw-embed-local, tdw-entity-resolver, tdw-eval-runner, tdw-event, tdw-exec, tdw-feature-store, tdw-graph, tdw-hooks, tdw-kg, tdw-knowledge, tdw-llm, tdw-llm-anthropic, tdw-llm-openai-compat, tdw-mask, tdw-outbox, tdw-pipe, tdw-protocol, tdw-provider-fileset, tdw-provider-ws-mock, tdw-provider-yahoo, tdw-replay, tdw-rollout, tdw-runtime, tdw-sandbox, tdw-snapshot, tdw-spatial, tdw-stage, tdw-storage-meilisearch, tdw-storage-qdrant, tdw-storage-s3, tdw-table-format, tdw-tag-rules, tdw-tags, tdw-tools, tdw-tui, tdw-udf, tdw-workflow-engine
- External dependencies: bytes ^1.11.0; serde ^1.0.228 features=[derive]; serde_json ^1.0.145
- Dev dependencies: none
- Reverse local dependencies: tdw-cli, tdw-mcp, tdw-service, tdw-worker
- Feature flags: none
- Test attributes detected: 18
- tests/ directory: no
- README: no
- Examples directory: no
- Scaffold/dead-code/fallback scan signals: 51 total, 0 stub-related

## Required Readiness Evidence

- [x] Manifest correctness reviewed.
- [x] Dependency direction reviewed.
- [x] Feature flags reviewed or marked not applicable.
- [x] Public API and error contracts reviewed.
- [x] Runtime behavior reviewed.
- [x] Tests and coverage evidence recorded.
- [x] Docs and examples reviewed.
- [x] Surface wiring reviewed where applicable.
- [x] Scaffold, dead-code, and fallback signals classified.
- [x] Security and reliability risks reviewed.

## Findings

- Service API remains the deliberate composition layer for CLI/MCP/service/worker entrypoints, now explicitly wiring ACP validation, app-client/app-server queue behavior, checked exec, TUI rendering, and replay in `client_event_sample`.
- Added local dependencies on `tdw-app-client` and `tdw-app-server` to prove real integration instead of duplicating daemon/client behavior in the service API.
- Scan signals are named deterministic sample functions, test assertions, and the mock streamer fixture used for integration evidence; no stub, copied FinX-XR, OpenBB, or scaffold-only service route was found.

## Verification

- Focused G007 command passed: `cargo test -p tdw-acp -p tdw-app-client -p tdw-app-server -p tdw-exec -p tdw-runtime -p tdw-service-api -p tdw-tui -p tdw-cli -p tdw-mcp -p tdw-service -p tdw-worker`.

## Verdict

Ready with follow-ups. Service API is the bootstrap composition boundary for G007 surfaces; production transport servers remain future integration work.

## Smoke Evidence (G009)

Participates in the [end-to-end functional smoke](../end-to-end-smoke.md). The smoke composition is exercised by:

- `tdw-test-utils::smoke::run_end_to_end_smoke` (library entry)
- `crates/tdw-test-utils/tests/end_to_end_smoke.rs` (integration tests)
- `tdw-service` and `tdw-cli` binaries (programmatic harness output)

Verified with `cargo test -p tdw-test-utils --test end_to_end_smoke` — green.

## Policy Binding Evidence (G015)

G015 request-path binding landed in `tdw-service-api`:

- `secure_endpoint_response` validates incoming OIDC claims with
  `tdw-auth-oidc`, authorizes the endpoint with `tdw-auth`, executes registered
  hooks through `tdw-hooks::HookRegistry::execute_handlers`, honors service hook
  vetoes only when the deployment policy permits them, and applies `tdw-mask`
  rules to the outgoing JSON response.
- `PolicyEnforcementConfig` now carries `HookExecutionPolicy`, so service hooks
  are deny-by-default until an explicit deployment allow rule is configured.
- `SecureServiceRuntime` and the `*_with_backend` secure helpers bind a concrete
  `HookHandlerBackend` to the service request path; tests prove MCP handlers run
  through the bound backend and denied hook actions fail before backend calls.
- `secure_endpoint_by_name` is deny-by-default for unknown service endpoint
  names.
- `secure_udf_run` validates the same ingress/auth boundary before dispatching
  through `tdw-sandbox::LocalUdfSandbox`, preserving network/filesystem
  capability denial from `tdw-udf`.
- Focused tests cover accepted and denied endpoint calls, invalid ingress
  claims, hook veto, unknown endpoint denial, response masking, accepted UDF
  execution, and denied network-capability UDF execution.
- The `prod`/`production` profile is fail-closed **by default** (no policy
  attached → dispatches return `Failed`), but `build_policy` now attempts to
  attach an auth-backed policy from `TDW_OIDC_*` environment configuration
  (`TDW_OIDC_ISSUER`, `TDW_OIDC_AUDIENCE`, `TDW_OIDC_JWKS`, `TDW_OIDC_SUBJECT`,
  `TDW_OIDC_KID`, optional `TDW_OIDC_ROLES`). The pure `build_prod_policy` helper
  parses + validates these inputs via `tdw-auth-oidc::validate_claims_strict` and
  now returns a typed `Result<_, OidcPolicyError>` (`MissingEnvVars`,
  `MalformedJwksPair`, `DuplicateKid`, `InvalidClaims`) instead of a bare `None`,
  so a misconfigured prod boot reports the **specific** cause. The env→policy
  wrapper `build_prod_policy_from_lookup` is injectable (tested without mutating
  process env): all-six-unset → `Ok(None)` (intentional fail-closed), partial
  config → `Err(MissingEnvVars(..))` listing every missing var, all-valid →
  `Ok(Some(_))`. `AppState::policy_attach_error` carries the cause, and
  `run_daemon` turns it into a **hard startup error** (the daemon refuses to
  start on partial/invalid config). This is the structural claim/JWKS pre-filter;
  cryptographic JWT verification (signature + `exp`/`nbf`/`iat`/iss/aud) lives in
  `tdw-auth-oidc::verify_jwt` — see
  [`production-auth-oidc`](../../release/production-auth-oidc.md).
- Evidence: per-variant `build_prod_policy_*` unit tests assert the exact
  `OidcPolicyError`; `build_prod_policy_from_lookup_*` tests cover the
  unset/partial/valid wrapper semantics; and end-to-end tests prove a prod-built
  policy resolves a `RunQuery` dispatch to `Completed`
  (`prod_built_policy_lets_dispatch_resolve_to_completed`) and rejects tampered
  ingress claims (`prod_built_policy_rejects_invalid_ingress_claims`).

Verification: `cargo test -p tdw-service-api --target-dir target` passed (all
tests, including the rewritten typed-error unit tests, the injected-lookup
wrapper tests, and the two prod-policy E2E tests).
