# Service operability — daemon + MCP ops endpoints

Operating guide for the `/health`, `/ready`, and `/metrics` endpoints and the
graceful-drain behaviour of the two long-running, non-worker services: the TDW
daemon (`tdw-service` / `tdw-backend` daemon surface) and the MCP Streamable
HTTP server (`tdw-mcp --streamable-http`). The worker (`tdw-worker --serve`) has
the same surface, documented in
[`worker-deployment.md`](worker-deployment.md#operability-endpoints).

All three reuse one hand-rolled Prometheus 0.0.4 text renderer
(`crates/tdw-app-server/src/ops.rs`); no metrics framework is pulled in.

## Endpoints

Each ops listener serves three `GET` routes (plus `404` for anything else):

| Endpoint | Meaning | Status |
|---|---|---|
| `/health` | Process liveness | `200` once serving |
| `/ready` | Dependencies reachable | `200` ready / `503` not ready |
| `/metrics` | Prometheus text exposition | `200` |

The ops listener is **off by default** on every service; it binds only when its
env var is set. It is a separate, token-free listener — keep it on an internal
port, never host-published.

| Service | Bind env var | Compose port | `/ready` checks |
|---|---|---|---|
| daemon | `TDW_DAEMON_HTTP_BIND` | `9102` | durable session/rollout stores reachable |
| MCP | `TDW_MCP_OPS_BIND` | `9103` | daemon reachable (when daemon-routed over TCP) |
| worker | `TDW_WORKER_HTTP_BIND` | `9101` | queue (DB) reachable |

## Metrics

**Daemon** — dispatch outcomes + in-flight gauge:

```text
# TYPE tdw_daemon_dispatch_total counter
tdw_daemon_dispatch_total{outcome="completed"} N
tdw_daemon_dispatch_total{outcome="failed"} N
tdw_daemon_dispatch_total{outcome="cancelled"} N
# TYPE tdw_daemon_dispatch_in_flight gauge
tdw_daemon_dispatch_in_flight N
```

A rising `outcome="failed"` is the primary daemon alarm; a persistently high
`in_flight` indicates dispatch back-pressure.

**MCP** — request count by JSON-RPC method:

```text
# TYPE tdw_mcp_requests_total counter
tdw_mcp_requests_total{method="initialize"} N
tdw_mcp_requests_total{method="tools/list"} N
tdw_mcp_requests_total{method="tools/call"} N
```

## Graceful drain

On `SIGTERM` (sent by `docker stop`, systemd, and Kubernetes at the end of the
termination grace period) or Ctrl-C, both services stop accepting new work,
finish in-flight requests, and exit `0`:

- **Daemon:** the serve loop stops the transport accept path, the relay and ops
  listener observe the same cancellation token, and in-flight dispatches drain
  before the daemon returns.
- **MCP:** the Streamable HTTP accept loop and the ops listener both poll a
  shared shutdown flag set by the signal handler, so neither accepts new
  connections after the signal; in-flight request threads finish.

Size the supervisor's stop timeout above your slowest in-flight request.

### systemd

```ini
[Service]
# daemon
Environment=TDW_DAEMON_TCP_BIND=127.0.0.1:7878
Environment=TDW_DAEMON_HTTP_BIND=127.0.0.1:9102
ExecStart=/usr/local/bin/tdw-service
Restart=always
RestartSec=5
# Allow in-flight dispatches to drain before SIGKILL.
TimeoutStopSec=30
```

### Kubernetes

```yaml
spec:
  # >= slowest in-flight request so the drain completes.
  terminationGracePeriodSeconds: 30
  containers:
    - name: tdw-service-daemon
      env:
        - { name: TDW_DAEMON_HTTP_BIND, value: "0.0.0.0:9102" }
      ports:
        - { containerPort: 9102, name: ops }
      livenessProbe:
        httpGet: { path: /health, port: ops }
        periodSeconds: 15
      readinessProbe:
        httpGet: { path: /ready, port: ops }
        periodSeconds: 15
```

The MCP `Deployment` is identical with `TDW_MCP_OPS_BIND`/port `9103`; probe the
token-free ops `/health`, never the bearer-gated `/mcp` surface.

## Compose

The `live` profile in `docker-compose.yaml` binds each ops listener and wires a
`curl`-based `healthcheck` against `/health` for the daemon, MCP, and worker.
The `tdw-mcp-serve` service waits on `tdw-service-daemon` being
`service_healthy` before starting.

## Endpoint aliases

The ops listener also accepts the Kubernetes-conventional aliases
`/healthz` (→ `/health`) and `/readyz` (→ `/ready`) on all three services.
These are identical in behaviour to the canonical paths and are provided so
Kubernetes liveness/readiness probes can use either convention without
reconfiguration.

Source: `crates/tdw-app-server/src/ops.rs` (`classify_route`).

## Backup, restore, and upgrade cross-reference

| Task | Runbook |
|---|---|
| Back up and restore named volumes | [`backup-restore-runbook.md`](backup-restore-runbook.md) |
| Upgrade TDW images and apply schema migrations | [`upgrade-runbook.md`](upgrade-runbook.md) |
| Manual post-upgrade acceptance smoke | [`live-stack-smoke.md`](live-stack-smoke.md) |

## See also

- [`worker-deployment.md`](worker-deployment.md) — the worker's ops surface,
  alert table, and supervision.
- [`mcp-remote-deployment.md`](mcp-remote-deployment.md) — securing the MCP
  Streamable HTTP `/mcp` surface (TLS/OAuth) for remote exposure.
- [`data-backend-runbook.md`](data-backend-runbook.md) — bringing the `live`
  stack up.
- `crates/tdw-app-server/src/ops.rs` — the shared Prometheus renderer and ops
  listener.
