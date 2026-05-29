# Live-Stack Smoke Checklist

Manual verification of the `live` Docker Compose profile, focused on the daemon
hardening / durability work landed for `v0.7.0`:

- daemon policy so live dispatches **resolve** (not `Failed`) — `TDW_PROFILE` honored (#109)
- Postgres-backed daemon **session + rollout** stores (#112)
- worker **concurrency** — parallel in-flight jobs (#111)
- per-request **WasmLimits** on `udf.run` (#110)

> Requires a Docker daemon. None of this is exercised by `cargo test` — the
> Postgres store paths are compile-checked only (like `real-postgres` /
> `real-clickhouse`). Run this once on a machine with Docker before trusting the
> live deployment.

## 0. Prerequisites

```bash
docker version                 # daemon reachable
cp .env.example .env 2>/dev/null || true
# Required for the MCP service (non-loopback bind must be authenticated):
echo "TDW_MCP_HTTP_TOKEN=$(openssl rand -hex 24)" >> .env
```

Validate the compose model without starting anything:

```bash
docker compose --profile live config >/dev/null && echo "compose model OK"
```

## 1. Bring up the live stack

```bash
docker compose --profile live up --build -d
docker compose --profile live ps
```

Expect `postgres`, `clickhouse`, `qdrant`, `meilisearch`, `minio`,
`tdw-bootstrap` (exits 0), `tdw-service-daemon`, `tdw-mcp-serve`, and
`tdw-worker-serve` to come up. `tdw-bootstrap` and `minio-init` should complete
successfully before the long-running services start.

## 2. Daemon policy — dispatches resolve (#109)

```bash
docker compose --profile live logs tdw-service-daemon | grep -i "policy"
```

- [ ] Log says **"daemon starting in 'docker' profile with a policy attached;
      dispatches will resolve"** — i.e. `TDW_PROFILE: docker` was honored and a
      local policy is attached. It must **not** say "no policy attached".

## 3. Postgres-backed session + rollout stores (#112)

The daemon was built `--features daemon-postgres` and given
`TDW_DAEMON_PG_URL`. After it has handled at least one op (the worker dispatches
jobs to it), the Postgres tables should exist and fill:

```bash
PSQL="docker compose --profile live exec -T postgres psql -U tdw -d tdw -c"
# Schemas created on connect:
$PSQL "\dt tdw_sessions*"      # tdw_sessions, _permission_state, _pending_approvals, _cost_ledger
$PSQL "\dt tdw_rollout"        # rollout table
# After traffic, rows accumulate (not a fresh empty file each restart):
$PSQL "SELECT count(*) FROM tdw_rollout;"
$PSQL "SELECT count(*) FROM tdw_sessions_cost_ledger;"
```

- [ ] `tdw_sessions*` and `tdw_rollout` tables exist.
- [ ] Row counts are **> 0** after the worker has dispatched ops, and **persist
      across a daemon restart**:

```bash
docker compose --profile live restart tdw-service-daemon
# wait a few seconds, then re-run the count queries — counts must not reset to 0
```

## 4. Worker concurrency (#111)

```bash
docker compose --profile live logs tdw-worker-serve | grep -iE "serving|backend=postgres"
docker compose --profile live exec -T tdw-worker-serve printenv TDW_WORKER_CONCURRENCY
```

- [ ] `TDW_WORKER_CONCURRENCY=4` is set.
- [ ] Worker logs show it serving against the Postgres backend.
- [ ] (Optional, load test) enqueue several jobs and confirm up to 4 are
      in-flight concurrently (e.g. overlapping "processing" log lines / daemon
      receiving multiple ops in the same window) rather than strictly one-at-a-time.

## 5. Per-request WasmLimits (#110)

Drive a `udf.run` tool call through the daemon (via the MCP server or a daemon
client) with a WASM UDF and an explicit tight limit:

```jsonc
// udf.run arguments
{
  "name": "echo",
  "runtime": "Wasm",
  "source": "<base64 of a wasm module exporting `alloc` + `echo`>",
  "input": "hello",
  "allow_network": false,
  "allow_filesystem": false,
  "wasm_limits": { "fuel": 1 }     // tiny budget
}
```

- [ ] With a normal/absent `wasm_limits`, the UDF returns its output.
- [ ] With `"wasm_limits": { "fuel": 1 }`, the same module **traps** (UDF error)
      — proving the per-request limit reaches execution.
- [ ] An over-large request (e.g. `"fuel": 999999999999`) behaves like the
      default ceiling, not a raised limit (tighten-only clamp).

> A minimal `echo` WAT module and the exact ABI are in
> `crates/tdw-sandbox/src/lib.rs` (`wasm_routing_tests::ECHO`).

## 6. Tear down

```bash
docker compose --profile live down            # keep volumes
docker compose --profile live down -v         # also drop postgres/clickhouse/... data
```

## Result

- [ ] §2 policy attached — dispatches resolve
- [ ] §3 PG session + rollout tables persist across restart
- [ ] §4 worker concurrency = 4, PG-backed
- [ ] §5 per-request WasmLimits tighten (and clamp) as expected

All four boxes ticked = the `v0.7.0` daemon/durability surface is verified live.
