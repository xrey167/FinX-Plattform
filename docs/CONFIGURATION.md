# Configuration reference

The authoritative reference for every environment variable read by the
FinX-Plattform (`tdw-*`) binaries, plus the cargo feature matrix that selects
which subsystems are compiled into a given deployment image.

This is harvested directly from the source (`crates/**` `env::var` call sites),
[`.env.example`](../.env.example), [`docker-compose.yaml`](../docker-compose.yaml),
and the release runbooks under [`docs/release/`](release/). When the code and
this document disagree, the code wins — please open a fix.

## How configuration is layered

- **Environment variables** are the primary operator surface and the only thing
  documented here. They are read at process start.
- **`TDW_CONFIG`** optionally points the daemon at a TOML config file
  (`tdw-config`). Env-var overrides described below are applied on top of either
  the TOML file or the synthesized minimal default.
- **`.env`** is loaded by Docker Compose only (it is not auto-loaded by the
  Rust binaries). Copy it from `.env.example`; see
  [`scripts/compose-setup.ps1` / `.sh`](#first-run-setup-helper).
- **Profiles** (`TDW_PROFILE`) gate fail-closed production auth and select
  in-container defaults. See [Profiles](#profiles).

Defaults shown as `—` mean the variable is unset by default and the named
behavior only activates when you set it.

---

## Daemon (`tdw-service` / `tdw-service-daemon`)

| Variable | Purpose | Default | Read by | Example |
|----------|---------|---------|---------|---------|
| `TDW_CONFIG` | Path to a TOML config file merged as the base config layer. Unset = synthesized minimal in-memory default. | — | `tdw-service` daemon | `/etc/tdw/config.toml` |
| `TDW_PROFILE` | Effective runtime profile. `prod`/`production` enable fail-closed OIDC ingress auth; any other value attaches a local-default policy. Overrides the profile from the TOML file. | profile from config (else minimal default) | daemon, worker, MCP, CLI | `prod` |
| `TDW_DAEMON_TCP_BIND` | TCP listen address for the daemon transport. Use `0.0.0.0:7878` to reach it across a container network. The transport is unauthenticated plaintext — keep it internal-only. | `127.0.0.1:7878` | `tdw-service` daemon | `0.0.0.0:7878` |
| `TDW_DAEMON_OPEN_POLICY` | Set to `1` to attach the local-dev policy regardless of `TDW_PROFILE` (operator escape hatch before OIDC is wired). Without it, non-prod profiles still attach a local policy; `prod`/`production` stay fail-closed until `TDW_OIDC_*` is set. | — | `tdw-service-api` (`AppState`) | `1` |
| `TDW_DAEMON_PG_URL` | Postgres URL backing the daemon's own session + rollout stores (durable across restarts). Requires the `daemon-postgres` feature; falls back to `DATABASE_URL`. Unset = SQLite/JSONL defaults. | — | daemon (`daemon-postgres`) | `postgres://tdw:tdw@postgres:5432/tdw` |
| `DATABASE_URL` | Shared Postgres URL used as a fallback for `TDW_DAEMON_PG_URL` (daemon) and `TDW_WORKER_PG_URL` (worker). | — | daemon, worker (postgres features) | `postgres://tdw:tdw@localhost:5432/tdw` |

### Agent memory / consolidation (daemon)

| Variable | Purpose | Default | Read by | Example |
|----------|---------|---------|---------|---------|
| `TDW_MEMORY_DIR` | Directory of `*.json5` agent-memory files. The standalone daemon only runs consolidation when this names a usable directory; unset = in-memory-only store. | — | `tdw-backend` | `/var/lib/tdw/memory` |
| `TDW_CONSOLIDATION_TICK_SECS` | Memory-consolidation scheduler tick, in seconds. Zero/unparseable falls back to the default so the scheduler never busy-spins. | `3600` (hourly) | `tdw-backend` | `900` |

### Production ingress auth (OIDC)

Read **only** when `TDW_PROFILE` is `prod` or `production`; inert in any other
profile. Fail-closed by default: leave all unset to keep the daemon
fail-closed, or set all five required vars to attach an auth-backed policy.
Validation is **structural** (claim/JWKS consistency), not cryptographic
signature verification. Full guide:
[`docs/release/production-auth-oidc.md`](release/production-auth-oidc.md).

| Variable | Purpose | Default | Read by | Example |
|----------|---------|---------|---------|---------|
| `TDW_OIDC_ISSUER` | Expected token issuer (`iss`). Required to attach the policy. | — | daemon (prod) | `https://issuer.example` |
| `TDW_OIDC_AUDIENCE` | Expected audience (`aud`). Required. | — | daemon (prod) | `tdw-daemon` |
| `TDW_OIDC_JWKS` | Comma-separated `kid:alg` pairs. Allowed algs: `RS256`, `ES256`. Required. | — | daemon (prod) | `key1:RS256,key2:ES256` |
| `TDW_OIDC_SUBJECT` | Expected subject (`sub`). Required. | — | daemon (prod) | `svc:prod` |
| `TDW_OIDC_KID` | Key id selected from the JWKS set. Required. | — | daemon (prod) | `key1` |
| `TDW_OIDC_ROLES` | Comma-separated roles attached to the resolved principal. | — | daemon (prod) | `analyst,udf_runner` |

---

## Worker (`tdw-worker --serve`)

| Variable | Purpose | Default | Read by | Example |
|----------|---------|---------|---------|---------|
| `TDW_WORKER_PG_URL` | Postgres URL selecting the `PgWorkerQueue` backend. Requires the `postgres` feature; falls back to `DATABASE_URL`. Unset = SQLite. | — | `tdw-worker` (`postgres`) | `postgres://tdw:tdw@postgres:5432/tdw` |
| `TDW_WORKER_DB` | SQLite URL for the default (non-Postgres) worker queue. | `sqlite://tdw-worker.sqlite` | `tdw-worker` | `sqlite:///var/lib/tdw/worker.sqlite` |
| `TDW_WORKER_ID` | Worker identity used when leasing jobs. | `tdw-worker` | `tdw-worker` | `tdw-worker-live` |
| `TDW_WORKER_LEASE_TTL_MS` | Lease time-to-live, in milliseconds. | `30000` | `tdw-worker` | `60000` |
| `TDW_WORKER_POLL_MS` | Queue poll interval, in milliseconds. | `500` | `tdw-worker` | `1000` |
| `TDW_WORKER_CONCURRENCY` | Max jobs leased/run in parallel within the serve loop. Clamped to at least 1. | `4` | `tdw-worker` | `8` |
| `TDW_WORKER_DISPATCH` | Set to `daemon` to dispatch leased jobs to the daemon (instead of the offline ack handler). Also enabled implicitly if either of the next two is set. | — (ack handler) | `tdw-worker` | `daemon` |
| `TDW_WORKER_DAEMON_ADDR` | Daemon endpoint address for dispatch. | transport default (`127.0.0.1:7878` for TCP) | `tdw-worker` | `tdw-service-daemon:7878` |
| `TDW_WORKER_DAEMON_TRANSPORT` | Dispatch transport: `tcp`, `uds`/`unix`, or `http-sse`/`http`/`sse`. | `tcp` | `tdw-worker` | `tcp` |
| `TDW_WORKER_DAEMON_TIMEOUT_MS` | Dispatch request timeout, in milliseconds. | `2000` | `tdw-worker` | `5000` |

---

## MCP server (`tdw-mcp --streamable-http`)

| Variable | Purpose | Default | Read by | Example |
|----------|---------|---------|---------|---------|
| `TDW_MCP_HTTP_TOKEN` | Bearer token required for the Streamable-HTTP server. A non-loopback bind **refuses to start** without it. Use a strong random value (`openssl rand -hex 32`). | — | `tdw-mcp` | `<hex-32>` |
| `TDW_MCP_DAEMON_ADDR` | Daemon endpoint the MCP server routes daemon-backed tools to. Falls back to `TDW_DAEMON_TCP_BIND`. | — | `tdw-mcp` | `tdw-service-daemon:7878` |
| `TDW_MCP_DAEMON_TRANSPORT` | Transport for daemon routing (`tcp`, `uds`, `http-sse`). | `tcp` | `tdw-mcp` | `tcp` |
| `TDW_MCP_DAEMON_TIMEOUT_MS` | Daemon request timeout, in milliseconds. | transport default | `tdw-mcp` | `5000` |

The default Streamable-HTTP bind is `127.0.0.1:8788`. Front the server with a
TLS/OAuth reverse proxy for any non-local exposure — see
[`docs/release/mcp-remote-deployment.md`](release/mcp-remote-deployment.md) and
[`docs/release/secrets-and-tls.md`](release/secrets-and-tls.md).

---

## Storage backends

Bootstrap (`tdw-bootstrap`) and the daemon read these to point at live storage.
The `real-clickhouse` / `real-qdrant` / `daemon-postgres` features must be built
for the daemon to use the live engines (without them the in-memory defaults are
used regardless of these URLs).

### Postgres

| Variable | Purpose | Default | Read by | Example |
|----------|---------|---------|---------|---------|
| `TDW_POSTGRES_URL` | Postgres URL used by `tdw-bootstrap` to apply the durable-persistence schemas. | — | `tdw-bootstrap` | `postgres://tdw:tdw@postgres:5432/tdw` |
| `POSTGRES_URL` | Host-side Postgres URL for some integration tests. | — | tests | `postgres://tdw:tdw@localhost:5432/tdw` |
| `TDW_POSTGRES_TEST_URL` | Postgres URL gating the Postgres integration tests (`tdw-session`, `tdw-worker`, `tdw-storage-postgres`, …). | — | tests | `postgres://tdw:tdw@localhost:5432/tdw` |

### ClickHouse (OLAP)

| Variable | Purpose | Default | Read by | Example |
|----------|---------|---------|---------|---------|
| `TDW_CLICKHOUSE_URL` | ClickHouse HTTP endpoint. Read by `tdw-bootstrap` and, with `real-clickhouse`, by the daemon ingest/query path. | — | `tdw-bootstrap`, daemon (`real-clickhouse`) | `http://clickhouse:8123` |
| `TDW_CLICKHOUSE_USER` | ClickHouse username. | — | `tdw-bootstrap`, daemon | `tdw` |
| `TDW_CLICKHOUSE_PASSWORD` | ClickHouse password. | — | `tdw-bootstrap`, daemon | `tdw` |
| `TDW_CLICKHOUSE_TEST_URL` | ClickHouse endpoint gating ClickHouse integration tests. | — | tests | `http://localhost:8123` |
| `TDW_CLICKHOUSE_TEST_USER` | Test ClickHouse username. | — | tests | `tdw` |
| `TDW_CLICKHOUSE_TEST_PASSWORD` | Test ClickHouse password. | — | tests | `tdw` |

### Qdrant (vector)

| Variable | Purpose | Default | Read by | Example |
|----------|---------|---------|---------|---------|
| `TDW_QDRANT_URL` | Qdrant HTTP endpoint. Read by `tdw-bootstrap` and, with `real-qdrant`, by the daemon knowledge index. | — | `tdw-bootstrap`, daemon (`real-qdrant`) | `http://qdrant:6333` |
| `TDW_QDRANT_API_KEY` | Qdrant API key (optional). | — | `tdw-bootstrap`, daemon | `<key>` |
| `TDW_QDRANT_VECTOR_SIZE` | Vector dimension used when `tdw-bootstrap` creates the baseline collection. | provider default | `tdw-bootstrap` | `1536` |
| `TDW_QDRANT_TEST_URL` | Qdrant endpoint gating Qdrant integration tests. | — | tests | `http://localhost:6333` |
| `TDW_QDRANT_TEST_API_KEY` | Test Qdrant API key. | — | tests | `<key>` |

### Meilisearch (lexical)

| Variable | Purpose | Default | Read by | Example |
|----------|---------|---------|---------|---------|
| `TDW_MEILI_URL` | Meilisearch endpoint read by `tdw-bootstrap` to create the baseline index. | — | `tdw-bootstrap` | `http://meilisearch:7700` |
| `TDW_MEILI_API_KEY` | Meilisearch API key (optional). | — | `tdw-bootstrap` | `<key>` |
| `TDW_MEILISEARCH_TEST_URL` | Meilisearch endpoint gating Meilisearch integration tests. | — | tests | `http://localhost:7700` |
| `TDW_MEILISEARCH_TEST_API_KEY` | Test Meilisearch API key. | — | tests | `<key>` |

### S3 / MinIO (blob)

| Variable | Purpose | Default | Read by | Example |
|----------|---------|---------|---------|---------|
| `TDW_S3_ENDPOINT` | S3-compatible endpoint (MinIO in the local stack). | — | `tdw-bootstrap` | `http://minio:9000` |
| `TDW_S3_BUCKET` | Blob bucket name. | — | `tdw-bootstrap` | `tdw-default` |
| `TDW_S3_ACCESS_KEY` | S3 access key. | — | `tdw-bootstrap` | `minio` |
| `TDW_S3_SECRET_KEY` | S3 secret key. | — | `tdw-bootstrap` | `minio123` |
| `TDW_S3_REGION` | S3 region. | `us-east-1` | `tdw-bootstrap` | `us-east-1` |
| `MINIO_ENDPOINT` | Host-side MinIO endpoint for tests/tooling. | — | tests | `http://localhost:9002` |
| `TDW_S3_TEST_BUCKET` / `_ENDPOINT` / `_ACCESS_KEY` / `_SECRET_KEY` / `_REGION` | S3 integration-test settings (`tdw-storage-s3`). | — | tests | see `.env.example` |

---

## LLM and embeddings

The LLM HTTP clients (`tdw-llm-anthropic`, `tdw-llm-openai-compat`) take their
API key and base URL as constructor arguments; the conventional `ANTHROPIC_API_KEY`
/ `OPENAI_API_KEY` env vars are honored as the source for those credentials.
The embedding selector (`tdw-backend`) reads its env directly and falls back to
the offline deterministic hash embedder when a key is missing.

| Variable | Purpose | Default | Read by | Example |
|----------|---------|---------|---------|---------|
| `ANTHROPIC_API_KEY` | Anthropic API key for the Anthropic LLM transport. | — | `tdw-llm-anthropic` | `sk-ant-…` |
| `OPENAI_API_KEY` | OpenAI API key; also a fallback for the OpenAI embedder. | — | `tdw-llm-openai-compat`, embedder | `sk-…` |
| `TDW_OPENAI_COMPAT_API_KEY` | API key for an OpenAI-compatible endpoint. | — | `tdw-llm-openai-compat` | `<key>` |
| `TDW_EMBED_PROVIDER` | Embedding backend selector: `hash`/`local` (offline default), `openai` (needs `openai` feature), `google` (needs `google` feature). Unknown/unbuilt values degrade to the hash embedder. | `hash` | `tdw-backend` | `openai` |
| `TDW_EMBED_MODEL` | Embedding model id override for the selected provider. | provider default (`text-embedding-3-small` / `gemini-embedding-001`) | `tdw-backend` | `text-embedding-3-large` |
| `TDW_OPENAI_EMBEDDING_API_KEY` | OpenAI embedder API key (preferred over `OPENAI_API_KEY`). | — | `tdw-backend` (`openai`) | `<key>` |
| `TDW_OPENAI_EMBEDDING_BASE_URL` | Override the OpenAI embedder base URL. | OpenAI default | `tdw-backend` (`openai`) | `https://api.openai.com/v1` |
| `TDW_GOOGLE_EMBEDDING_API_KEY` | Google/Gemini embedder API key (preferred over `GOOGLE_API_KEY` / `GEMINI_API_KEY`). | — | `tdw-backend` (`google`) | `<key>` |
| `GOOGLE_API_KEY` / `GEMINI_API_KEY` | Fallback keys for the Google/Gemini embedder. | — | `tdw-backend` (`google`) | `<key>` |
| `TDW_GOOGLE_EMBEDDING_BASE_URL` | Override the Google embedder base URL. | Google default | `tdw-backend` (`google`) | `https://generativelanguage.googleapis.com` |

Live-network LLM/embedding tests are additionally gated by `TDW_ANTHROPIC_LIVE`,
`TDW_OPENAI_COMPAT_LIVE`, `TDW_OPENAI_EMBEDDING_LIVE`, `TDW_GOOGLE_EMBEDDING_LIVE`
(`=1`), plus per-test `*_BASE_URL` / `*_MODEL` overrides — see the crate test
files. These are not needed at runtime.

---

## Market-data providers

Each live HTTP fetcher is compiled in via a per-provider cargo feature on
`tdw-service-api` (or `all-http-providers`); the default offline build registers
only the three offline providers and ignores these keys. A provider's live
integration test is gated by its `TDW_<PROVIDER>_LIVE=1` flag (set only for
intentional network tests; never needed at runtime).

| Variable | Provider | Key required for live? | Example |
|----------|----------|------------------------|---------|
| `POLYGON_API_KEY` | Polygon | yes | `<key>` |
| `FRED_API_KEY` | FRED | yes | `<key>` |
| `APCA_API_KEY_ID` | Alpaca | yes | `<key>` |
| `APCA_API_SECRET_KEY` | Alpaca | yes | `<key>` |
| `COINGECKO_API_KEY` | CoinGecko | optional (pro tier) | `<key>` |
| `HF_TOKEN` / `HUGGINGFACE_API_TOKEN` / `HF_API_TOKEN` | HuggingFace (first set wins) | optional | `hf_…` |
| `TDW_ADANOS_API_KEY` | Adanos | yes | `<key>` |
| `TDW_ALPHA_VANTAGE_API_KEY` | Alpha Vantage (free tier: 25 req/day) | yes | `<key>` |
| `TDW_BENZINGA_API_KEY` | Benzinga | yes | `<key>` |
| `TDW_BLS_API_KEY` | US BLS | optional (raises rate limits) | `<key>` |
| `TDW_CCDATA_API_KEY` | CCData | yes | `<key>` |
| `TDW_DATABENTO_API_KEY` | Databento | yes | `<key>` |
| `TDW_EIA_API_KEY` | US EIA | yes | `<key>` |
| `TDW_FMP_API_KEY` | Financial Modeling Prep | yes | `<key>` |
| `TDW_GLASSNODE_API_KEY` | Glassnode | yes | `<key>` |
| `TDW_NASDAQ_API_KEY` | Nasdaq Data Link | yes | `<key>` |
| `TDW_SEEKING_ALPHA_API_KEY` | Seeking Alpha (RapidAPI) | yes | `<key>` |
| `TDW_TIINGO_API_KEY` | Tiingo | yes | `<key>` |
| `TDW_TRADING_ECONOMICS_API_KEY` | Trading Economics | yes | `<key>` |
| `TDW_TRADIER_API_KEY` | Tradier | yes | `<key>` |
| `TDW_VELODATA_API_KEY` | Velodata | yes | `<key>` |

Providers with **no API key** (public endpoints): Yahoo, CBOE, AkShare, Binance,
Deribit, ECB, FINRA, GeckoTerminal, OECD, SEC EDGAR, TMX. Their live tests are
still gated by the corresponding `TDW_<PROVIDER>_LIVE=1` flag.

---

## Profiles

`TDW_PROFILE` selects runtime behavior:

| Profile | Ingress auth | Notes |
|---------|--------------|-------|
| unset / `service` / `docker` / dev values | Local-default policy attached (dispatches resolve) | Default for the `live` compose stack (`docker`). |
| `prod` / `production` | Fail-closed; requires `TDW_OIDC_*` to attach an auth-backed policy | Until OIDC is configured, dispatched ops return `Failed`. |

`TDW_DAEMON_OPEN_POLICY=1` forces the local-default policy regardless of profile
(operator escape hatch before OIDC is wired).

---

## Feature matrix (per deployment model)

Which cargo features to build per deployment model. These mirror the Docker
`FEATURES` build-args (`docker/tdw-*.Dockerfile`, passed through compose
`build.args`). The default build is fully offline and deterministic.

| Deployment model | Crate / binary | Cargo features | Notes |
|------------------|----------------|----------------|-------|
| Offline default | any | *(none)* | In-memory engines, 3 offline providers, hash embedder, SQLite/JSONL stores. No network. |
| Postgres-backed worker | `tdw-worker` | `postgres` | Selects `PgWorkerQueue` when `TDW_WORKER_PG_URL`/`DATABASE_URL` is set. Compose: `FEATURES=postgres`. |
| Postgres-backed daemon | `tdw-service` | `daemon-postgres` | Daemon session + rollout stores on Postgres via `TDW_DAEMON_PG_URL`. Implies `real-postgres`. Compose: `FEATURES=daemon-postgres`. |
| Live ClickHouse | `tdw-service` / `tdw-service-api` | `real-clickhouse` | Swaps the OLAP engine for the real ClickHouse HTTP engine; reads `TDW_CLICKHOUSE_URL`. |
| Live Qdrant | `tdw-service` / `tdw-service-api` | `real-qdrant` | Swaps the vector engine for the real Qdrant HTTP engine; reads `TDW_QDRANT_URL`. |
| Local filesystem blobs | `tdw-service-api` | `storage-fs` | Uses `LocalFsBlobEngine` when `profile == "service"`. |
| UDF / WASM sandbox | `tdw-service-api` | `udf-wasm` | Enables the WASM UDF runtime (`tdw-sandbox/udf-wasm`). |
| Real LLM embeddings | `tdw-backend` | `openai` and/or `google` | Compiles the OpenAI/Google embedder selector arms; `TDW_EMBED_PROVIDER` chooses at runtime. |
| One live data provider | `tdw-service-api` | `provider-<name>` (e.g. `provider-polygon`) | Adds that provider's live HTTP fetcher to `default_registry()`. |
| All live data providers | `tdw-service-api` | `all-http-providers` | Aggregate of every `provider-*` feature plus `provider-binance-http`. |
| Live Binance trade ws | `tdw-service-api` | `ws` | Enables the live Binance trade websocket subscribe path. |

Example builds:

```bash
# Postgres-backed worker image (matches the live compose stack)
cargo build --release --bin tdw-worker --features postgres

# Daemon with Postgres-backed stores + live ClickHouse + Qdrant
cargo build --release --bin tdw-service \
  --features daemon-postgres,real-clickhouse,real-qdrant

# Service API with every live provider compiled in
cargo build --release -p tdw-service-api --features all-http-providers
```

---

## First-run setup helper

[`scripts/compose-setup.ps1`](../scripts/compose-setup.ps1) (PowerShell) and
[`scripts/compose-setup.sh`](../scripts/compose-setup.sh) (POSIX) are idempotent
first-run helpers: they copy `.env.example` to `.env` if it does not already
exist and fill `TDW_MCP_HTTP_TOKEN` with a securely random hex-32 value. Run
once before `docker compose --profile live up`:

```powershell
.\scripts\compose-setup.ps1
```

```bash
./scripts/compose-setup.sh
```

`.env` is gitignored — never commit it.

---

## See also

- [`docs/docker.md`](docker.md) — local compose profiles + WSL2 guidance.
- [`docs/release/data-backend-runbook.md`](release/data-backend-runbook.md) — live-stack bring-up.
- [`docs/release/secrets-and-tls.md`](release/secrets-and-tls.md) — systemd / Kubernetes secret injection, TLS, token rotation.
- [`docs/release/production-auth-oidc.md`](release/production-auth-oidc.md) — production ingress auth.
- [`docs/release/mcp-remote-deployment.md`](release/mcp-remote-deployment.md) — exposing the MCP HTTP server.
