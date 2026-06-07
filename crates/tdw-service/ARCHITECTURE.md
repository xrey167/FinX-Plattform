# tdw-service architecture

`tdw-service` is a thin daemon entrypoint. It owns no composition logic — that
lives in `tdw-backend::server` — so the standalone daemon and the unified
`tdw-backend` binary serve identically.

## Module map

| Path | Contents |
| --- | --- |
| `src/main.rs` | the `tdw-service` binary: `--smoke` branch + daemon branch |

The daemon branch is two calls into `tdw-backend`:

```rust,ignore
let config = tdw_backend::server::load_config().await?;
tdw_backend::server::run_daemon(&config).await?;
```

## Command tree

- `tdw-service --smoke [SYMBOL]` — run the offline `tdw-test-utils`
  `run_end_to_end_smoke` and print the `SmokeReport` JSON, then exit.
- `tdw-service` (no flags) — daemon mode.

## Boot sequence (daemon dispatch path)

```text
load_config (tdw-backend)
   merge TDW_CONFIG layer or default ; TDW_DAEMON_TCP_BIND ; in-memory session ;
   temp rollout ; resolve_profile(TDW_PROFILE)
        │
        ▼
run_daemon (tdw-backend)
   AppState::from_config            (engines + policy per profile)
   report_policy_state              (logs whether a policy is attached)
   refuse start on PARTIAL OIDC config (actionable error)
   warn_on_unauthenticated_nonloopback_bind
   service_channel(state, state)    (AppState is both Dispatcher and EventSink)
   spawn_inmemory_relay(outbox → bus, 50ms)
   spawn_transport(config)          (TCP / UDS / HTTP-SSE; fail-closed if absent)
   serve(loop, relay, cancel)       (until ctrl-c / dispatched Shutdown)
```

`resolve_profile` and `spawn_transport` are unit-tested in this crate's
`#[cfg(test)]` block (profile precedence + fail-closed transport errors).

## Security posture

- **Loopback default** (`127.0.0.1:7878`); a non-loopback bind with no policy
  triggers a prominent security warning from `tdw-backend::server`.
- **OIDC fail-closed (post-#150)**: a fully-unset prod OIDC config runs
  fail-closed; a *partial* config refuses to start with the list of missing
  `TDW_OIDC_*` vars. Non-prod profiles attach a local-dev policy so dispatch
  resolves.
- **Fail-closed transports**: requesting `transport-http`/`transport-uds` without
  the feature errors at startup.
- The `--smoke` path is fully offline.

## Integration points

- `tdw-backend::server` — `load_config`, `run_daemon`, `resolve_profile`,
  `spawn_transport` (the shared serving glue).
- `tdw-service-api` — `AppState` (built inside `run_daemon`).
- `tdw-test-utils::smoke` — the `--smoke` end-to-end check.
- `tdw-app-server` — the transports `spawn_transport` selects.
