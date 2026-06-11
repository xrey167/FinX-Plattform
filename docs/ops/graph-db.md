# Graph Database Operations Runbook

The FinX daemon uses Memgraph (or any Bolt-protocol-compatible graph database,
e.g. Neo4j) as the production graph backend for the knowledge system. This
runbook covers deployment, configuration, backup, upgrade, and troubleshooting.

## Quick reference

| Item | Value |
|---|---|
| Default Bolt URI | `bolt://127.0.0.1:7687` |
| Config section | `[knowledge.graph]` in daemon TOML / `TDW_CONFIG` |
| Password env var | `TDW_GRAPH_PASSWORD` (default: empty, Memgraph default) |
| Bolt feature flag | `bolt` (Cargo feature on `tdw-backend` / `tdw-storage-graph`) |
| Conformance test env | `TDW_BOLT_TEST_URL` |
| Docker Compose service | `memgraph` (port 7687) |

## Configuration

Add to your daemon TOML (`TDW_CONFIG`):

```toml
[knowledge.graph]
backend = "bolt"
bolt_uri = "bolt://127.0.0.1:7687"
bolt_user = ""
bolt_password_env = "TDW_GRAPH_PASSWORD"
```

Set the password at runtime (not in the TOML file):

```bash
export TDW_GRAPH_PASSWORD="<your-password>"
```

For dev/test (no Memgraph), use the in-memory engine:

```toml
[knowledge.graph]
backend = "in-memory"
```

**No silent fallback**: if `backend = "bolt"` and Memgraph is unreachable, the
daemon refuses to start with a hard `Init` error. Correct the URI or the
network path before retrying.

## Deployment

### Docker Compose (development / CI)

`docker-compose.yml` in the repo root ships a `memgraph` service:

```yaml
memgraph:
  image: memgraph/memgraph:2.22.1
  ports:
    - "7687:7687"
```

Start it alongside the other services:

```bash
docker compose up -d memgraph
```

### Production (bare-metal / Kubernetes)

1. Follow the [Memgraph production deployment guide](https://memgraph.com/docs/deployment).
2. Bind Bolt to a non-loopback address only if necessary; prefer a loopback
   bind fronted by an mTLS/VPN tunnel for service-to-service communication.
3. Set `TDW_GRAPH_PASSWORD` from your secrets manager (Vault, AWS SSM, etc.).
4. The daemon binary must be built with `--features bolt`:
   ```bash
   cargo build -p tdw-backend --features bolt --release --target-dir target
   ```

## Backup and restore

Memgraph persists data to its configured `data-dir` (default `/var/lib/memgraph`
in the official Docker image). Back it up with the Memgraph snapshot/WAL
mechanism:

```bash
# Trigger a manual snapshot (Memgraph Cypher):
CALL mg.create_snapshot();
```

Then copy the snapshot files from the `data-dir`. Restore by placing the
snapshot files back and restarting Memgraph.

For a full logical backup use `mgconsole` to dump Cypher:

```bash
mgconsole --host 127.0.0.1 --port 7687 --output-format=cypher \
  --query="MATCH (n) RETURN n" > nodes.cypher
```

## Upgrade

1. Stop the daemon.
2. Create a snapshot before upgrading Memgraph.
3. Update the `memgraph/memgraph` image tag in `docker-compose.yml` (or the
   Kubernetes manifest).
4. Start Memgraph and verify it replays the snapshot cleanly.
5. Restart the daemon.

## Conformance tests

Set `TDW_BOLT_TEST_URL` to run the `tdw-storage-graph` Bolt conformance suite
against a live Memgraph instance:

```bash
export TDW_BOLT_TEST_URL="bolt://127.0.0.1:7687"
cargo test -p tdw-storage-graph --features bolt --test conformance \
  --target-dir target
```

In CI the conformance step starts a Memgraph container on port 7687, sets
`TDW_BOLT_TEST_URL`, and runs the test. See `.github/workflows/ci.yml`.

## Troubleshooting

### Daemon refuses to start: "bolt graph connect" error

- Verify Memgraph is running: `docker compose ps memgraph`
- Verify the Bolt URI is reachable: `nc -z 127.0.0.1 7687`
- Check `TDW_GRAPH_PASSWORD` is set correctly.
- Check the daemon was built with `--features bolt`; a non-bolt build will
  give the error "bolt feature required" at startup.

### Wrong `backend` value in config

The daemon will print:

```
unknown knowledge.graph.backend "foo"; valid values: bolt | in-memory
```

Correct the `[knowledge.graph] backend` value in the TOML.

### In-memory engine data loss on restart

`backend = "in-memory"` is intentionally ephemeral — all graph state is lost
on restart. Use `backend = "bolt"` for any environment where persistence
matters.

### Memgraph memory limits

Memgraph is an in-memory graph database. Set `--memory-limit` in the
Memgraph container/process config to cap RSS usage. Monitor with
`/var/log/memgraph/memgraph.log`.
