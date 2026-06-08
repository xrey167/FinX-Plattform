# tdw-service-api

The daemon's secure request path and composition root. This crate owns
`AppState` (the engines + registry + event-spine stores + policy), the async op
`dispatcher`, the ingress `policy` guard (OIDC claim validation, role
authorization, hooks, response masking), the streaming-ingest entrypoints, and a
large set of deterministic "evidence" samples the surface binaries print.

Every daemon request passes the sync policy guard before any work runs, and ops
are dispatched through `AppState`'s `Dispatcher`/`EventSink` implementations
(defined here, consumed by `tdw-app-server`). Engine selection is feature-gated:
the default build is fully offline (in-memory/recording engines); the
`real-engines` bundle wires the live Postgres/ClickHouse/Qdrant/Meilisearch/S3
backends for the `live` profile.

## Binaries produced

None. Library crate (the composition root for `tdw-service` / `tdw-backend`).

## Feature flags

Storage / engine selection (all off by default → offline):

| Feature | Effect |
| --- | --- |
| `storage-fs` | local-disk blob engine for the `service` profile |
| `real-postgres` | real `PgEngine` relational backend |
| `real-clickhouse` | real ClickHouse HTTP OLAP backend |
| `real-qdrant` | real Qdrant HTTP vector backend |
| `real-meilisearch` | real Meilisearch HTTP lexical backend |
| `real-s3` | real S3/MinIO blob backend |
| `daemon-postgres` | Postgres-backed daemon session + rollout stores |
| `real-engines` | aggregate of all `real-*` + `daemon-postgres` (the `live` set) |
| `udf-wasm` | enable the WASM UDF runtime in `tdw-sandbox` |
| `ws` | live Binance trade websocket subscribe path |
| `provider-*`, `all-http-providers`, `provider-yahoo-http`, `provider-binance-http` | wire live HTTP fetchers into `default_registry()` (default = 3 offline providers) |

## Key environment variables

Selected at runtime (full list in
[`docs/CONFIGURATION.md`](../../docs/CONFIGURATION.md)):

- Engine selection: `TDW_PROFILE=live` or `TDW_ENGINES=real` requests the real
  engines (fail-closed if a required URL/feature is missing).
- Engine endpoints: `TDW_POSTGRES_URL` (or `TDW_DAEMON_PG_URL`/`DATABASE_URL`),
  `TDW_CLICKHOUSE_URL` (+ `_USER`/`_PASSWORD`), `TDW_QDRANT_URL` (+ `_API_KEY`),
  `TDW_MEILI_URL` (+ `_API_KEY`), `TDW_S3_ENDPOINT`/`_BUCKET`/`_ACCESS_KEY`/
  `_SECRET_KEY` (+ `_REGION`).
- Auth (prod policy): `TDW_OIDC_ISSUER`, `TDW_OIDC_AUDIENCE`, `TDW_OIDC_JWKS`,
  `TDW_OIDC_SUBJECT`, `TDW_OIDC_KID` (+ optional `TDW_OIDC_ROLES`).
  `TDW_DAEMON_OPEN_POLICY=1` opts into the local-dev policy regardless of profile.

## Quickstart (library)

Build an offline `AppState` and dispatch an op end to end:

```rust,ignore
use tdw_service_api::dispatch_op;
use tdw_protocol::{ActorKind, ActorRef, Op, OpEnvelope, SessionId};

let state = tdw_service_api::AppState::in_memory_for_tests().await; // local-dev policy
let env = OpEnvelope::new(
    SessionId::new("s1")?, 1,
    ActorRef { actor_id: "user".into(), kind: ActorKind::User, tenant_id: None },
    Op::RunQuery { sql: "select 1".into(), plan_id: None, cost_hint: None },
);
let events = dispatch_op(&state, env).await; // [Started, Completed|Failed]
# Ok::<(), Box<dyn std::error::Error>>(())
```

`list_providers()` / `default_registry()` enumerate the registered providers
(3 offline by default). The `*_sample` functions return deterministic evidence
the surface binaries print.

See [`examples/basic.rs`](examples/basic.rs):
`cargo run -p tdw-service-api --example tdw_service_api_basic`.

## See also

- [`ARCHITECTURE.md`](ARCHITECTURE.md) — composition root, policy guard, dispatch.
- `tdw-app-server` — the transport/service-loop that drives `AppState`.
- `tdw-service` / `tdw-backend` — the daemon binaries that boot `AppState`.
