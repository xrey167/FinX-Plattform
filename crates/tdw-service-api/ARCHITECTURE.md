# tdw-service-api architecture

`tdw-service-api` is the daemon's composition root and secure request path. It
assembles the engines, registry, event-spine stores, and policy into `AppState`;
implements the `Dispatcher`/`EventSink` traits over it; and enforces auth +
hooks + masking on every request before any work runs.

## Module map

| Path | Contents |
| --- | --- |
| `src/lib.rs` | `default_registry`, `list_providers`, fetch/index helpers, the deterministic `*_sample` evidence functions, re-exports |
| `src/app_state.rs` | `AppState`, engine selection, policy construction (`OidcPolicyError`), session/rollout backends, stream control |
| `src/dispatcher.rs` | `dispatch_op` + `impl Dispatcher for AppState` (the op router) |
| `src/event_sink.rs` | `impl EventSink for AppState` (outbox + rollout + cost ledger) |
| `src/policy.rs` | `ServiceEndpoint`, `PolicyEnforcementConfig`, `enforce_request_path_with_backend`, masking, secure-endpoint wrappers |
| `src/stream_ingest.rs` | `run_stream_ingest` / `run_ws_ingest` |

## Key types

### `AppState` (composition root)

Holds `Arc<dyn …>` engines (`olap`, `relational`, `blob`, `vector`, `lexical`),
the `registry`, event-spine stores (`bus`, `outbox`, `snapshot`), the
`SessionBackend` (SQLite or `Pg`) and `RolloutBackend` (JSONL or `Pg`), the
optional `policy` (+ `policy_attach_error`), and the live `streams` registry.

Construction:

- `from_config(TdwConfig)` — the production path. Selects engines via
  `live_engines_requested(profile)` (true for `live` or `TDW_ENGINES=real`);
  builds durable stores; builds the policy.
- `in_memory_for_tests()` — offline SQLite-in-memory + temp JSONL + local-dev
  policy.
- `with_policy(..)`, and feature-gated `with_real_postgres/clickhouse/qdrant(..)`.

### Engine selection (adapter pattern, step 3)

`select_{blob,relational,olap,vector,lexical}_engine(live)` each branch on the
`live` flag: in the live path they wire the real engine from `TDW_*` env (fail
closed via `require_engine_env` / `missing_feature_err` on a missing var or
absent cargo feature); otherwise they return the offline in-memory/recording
engine. `build_durable_stores` chooses SQLite/JSONL or Postgres (when
`daemon-postgres` + a PG URL).

### Policy

- `ServiceEndpoint` (`EquityHistorical | RunQuery | IngestBatch | ToolCall |
  UdfRun`) — each maps to a logical name, a required role (`analyst` or
  `udf_runner`), and a policy table.
- `IngressAuthContext { claims, jwks, issuer, audience }` and
  `PolicyEnforcementConfig { auth, hooks, hook_execution, mask_rules }`.
- `enforce_request_path_with_backend(config, endpoint, backend)` is the sync
  guard: structural OIDC claim validation (`tdw-auth-oidc`), role authorization
  (`tdw-auth`), hook execution (`tdw-hooks`), returning
  `PolicyEnforcementEvidence`. `mask_json_response` applies `mask_rules`.
- Policy construction: `build_policy` → local-dev policy for non-prod profiles
  (or `TDW_DAEMON_OPEN_POLICY=1`); for `prod`/`production` it reads the five
  required `TDW_OIDC_*` vars and validates them with `validate_claims_strict`.
  `OidcPolicyError` names the actionable cause (`MissingEnvVars`,
  `MalformedJwksPair`, `DuplicateKid`, `InvalidClaims`).

### Dispatch

`dispatch_op(state, env) -> Vec<EventMsg>` emits `Started` then a terminal
`Completed`/`Failed`. `run_dispatch` first requires a configured policy (deny by
default if absent), then routes by `Op`:

- `RunQuery` → policy guard → `relational.fetch_json(sql)` → masked rows.
- `IngestBatch` → guard → per-symbol fetch via `CommandRunner` → idempotent
  `INSERT … FORMAT JSONEachRow` to `state.olap` with a per-(op,symbol) dedup
  token.
- `ToolCall` (`udf.run`) → guard → `LocalUdfSandbox`.
- `StreamStart`/`StreamStop` → guard (reuses `IngestBatch`) →
  `start_binance_stream` / `stop_stream`.
- `Shutdown` → `{"shutdown":"requested"}`; the rest acknowledge.

### Event sink

`persist_event` appends to the in-memory outbox and the rollout backend;
`record_cost` upserts a session row and appends a cost-ledger entry.

## Runtime flow (registry-driven dispatch)

```text
OpEnvelope ─▶ AppState::dispatch ─▶ dispatch_op
   require policy (deny-by-default if None)
   enforce_request_path_with_backend  (OIDC claims → authorize → hooks)
   route by Op:
     RunQuery   → relational.fetch_json → mask
     IngestBatch→ CommandRunner(registry) fetch → olap.execute(INSERT … dedup)
     ToolCall   → LocalUdfSandbox
     Stream*    → AppState stream control
   ─▶ [Started, Completed | Failed]
        │  (ServiceLoop persists each via EventSink: outbox + rollout + cost)
```

## Security posture

- **Deny by default**: dispatch fails closed when no policy is attached.
- **Structural OIDC (post-#150)**: prod policy validates issuer/audience/kid∈JWKS/
  allowed-algorithm/role shape with `validate_claims_strict`; cryptographic JWT
  signature verification + remote JWKS fetch are handled in `tdw-auth-oidc`. A
  fully-unset prod config stays fail-closed; a *partial* one is a typed error.
- **Real engines fail closed**: a `live` boot missing a required `TDW_*` URL or
  the matching cargo feature errors at startup rather than degrading to offline.
- **Masking** is applied to every dispatched response via `mask_rules`.
- **Offline default**: no feature flags ⇒ in-memory engines and exactly 3 offline
  providers; no network.

## Integration points

- `tdw-app-server` — drives `AppState` via `Dispatcher`/`EventSink`/`ServiceLoop`.
- `tdw-config` — `TdwConfig` drives engine + policy selection.
- `tdw-auth` / `tdw-auth-oidc` / `tdw-hooks` / `tdw-mask` — the policy guard.
- `tdw-runtime` + `tdw-provider-*` — the ingest/fetch path.
- `tdw-storage-*` — the engine implementations.
- `tdw-session` / `tdw-rollout` / `tdw-outbox` / `tdw-bus` — the event spine.
