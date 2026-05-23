# Production Transport Status Matrix (G010–G013)

Workspace-wide punch list of which crates have a real production
backend wired vs. which still ship a stub or request-builder only.
Cross-references the per-goal docs (see *Companions* below).

## Conventions

- **`in-memory`** — engine type exists, holds state in a `Mutex<BTreeMap>` or similar; suitable for offline tests, not for production.
- **`request-builder`** — crate builds `ProviderRequest` / `EmbeddingHttpRequest` / similar shapes but does not execute HTTP; some other layer is responsible for actually dispatching the request.
- **`real`** — production backend implemented, gated behind an opt-in feature flag; default workspace test set stays offline.
- **`local`** — production-ready offline implementation (filesystem, hashing); no network involved by design.
- **`✅ shipped`** — landed on main (or in an open PR — annotated).
- **`⏳ pending`** — work still required.

## G010 storage transports

| Crate | Trait | Default | Production | Status |
|---|---|---|---|---|
| `tdw-storage-fs` | `BlobEngine` | `LocalBlobEngine` (real disk) | — | ✅ already real |
| `tdw-storage-postgres` | `RelationalEngine` | `PostgresRecordingEngine` (in-memory) | `PgEngine` (sqlx 0.9 `PgPool`) | ✅ shipped (PR #13) |
| `tdw-storage-s3` | `BlobEngine` | `InMemoryS3BlobEngine` | `S3Engine` (aws-sdk-s3) | ✅ shipped (PR #14) |
| `tdw-storage-clickhouse` | `RelationalEngine` | in-memory | (pending) | ⏳ pending |
| `tdw-storage-qdrant` | `VectorEngine` | `InMemoryVectorEngine` | (pending) | ⏳ pending |
| `tdw-storage-meilisearch` | `LexicalEngine` | `InMemoryLexicalEngine` | (pending) | ⏳ pending |
| `tdw-storage-parquet` | — | (utility, not an engine) | — | n/a |
| `tdw-storage-router` | — | (router/dispatcher) | — | n/a |
| **CI containers + env wiring** | — | — | — | ✅ shipped (PR #15) |

Pattern reference: `docs/quality/production-storage-transports.md`.

## G011 provider transports

These crates currently expose **request-builders** (functions that
construct a `ProviderRequest` describing the HTTP call) but do not
themselves dispatch HTTP. G011 work = add an HTTP execution layer
that takes a `ProviderRequest` and runs it (probably via `reqwest`
behind an opt-in feature flag), then implement the `tdw_core::Fetcher`
trait against the live response. CI runs against recorded cassettes
for offline determinism.

| Crate | Auth | Endpoint count | Current state | Status |
|---|---|---|---|---|
| `tdw-provider-fileset` | none | 1 (`equity_historical`) | real (fixture rows) — used by G009 smoke | ✅ already real |
| `tdw-provider-ws-mock` | none | streamer | mock streamer (deterministic ticks) | ✅ already real |
| `tdw-provider-yahoo` | none | `equity_historical` (`YahooEquityHistoricalFetcher`) | partial — has a `Fetcher` impl; verify whether `extract_data` actually hits Yahoo | ⏳ pending (audit + cassette tests) |
| `tdw-provider-fred` | API key | `series_observations`, others | request-builder only | ⏳ pending |
| `tdw-provider-alpaca` | API key + secret | `stock_bars`, others | request-builder only | ⏳ pending |
| `tdw-provider-binance` | API key + secret | `ticker_price`, others | request-builder only | ⏳ pending |
| `tdw-provider-polygon` | API key | `aggregates`, others | request-builder only | ⏳ pending |
| `tdw-provider-huggingface` | API token | `text_generation`, others | request-builder only | ⏳ pending |

Suggested per-provider PR shape:
1. Add `reqwest = { workspace = true, optional = true }` workspace dep (one-time).
2. Add `http` feature on each provider crate gating an HTTP executor.
3. Implement `Fetcher` (or `Streamer`) trait for the existing request shape, executing via reqwest and parsing the JSON response.
4. Use [`wiremock`](https://docs.rs/wiremock) to record a small set of cassettes; integration test asserts against the recorded responses, no live network.
5. Update `tdw-test-utils::smoke` to optionally route through the real provider when its feature is enabled (extension of G009).

## G012 LLM + embedding transports

These crates also currently expose **request-builders** (the
adapter to the LLM/embedding endpoint shape). G012 = add the
HTTP execution layer behind feature flags, with streaming support
where the model offers it.

| Crate | Vendor | Streaming | Current state | Status |
|---|---|---|---|---|
| `tdw-llm-anthropic` | Anthropic Messages API | yes (SSE) | model adapter only | ⏳ pending |
| `tdw-llm-openai-compat` | OpenAI / compat | yes (SSE) | model adapter only | ⏳ pending |
| `tdw-embed-openai` | OpenAI embeddings | no | request-builder + decoder | ⏳ pending |
| `tdw-embed-google` | Google embeddings | no | request-builder + decoder | ⏳ pending |
| `tdw-embed-local` | hash-based | no | real (in-process, hash) | ✅ already real |
| `tdw-llm` | trait crate | — | trait definitions only | n/a |
| `tdw-embed` | trait crate | — | trait definitions only | n/a |

Per-vendor PR shape mirrors G011:
1. Add `reqwest` + `eventsource-stream` (for SSE) behind a feature flag.
2. Implement the LLM / embedding trait, executing real HTTP, parsing the streamed or batch response.
3. Integration test gated by env var (e.g. `TDW_ANTHROPIC_TEST_KEY`); skip if unset.
4. Optional: extend G009 smoke variant to include a tiny LLM round-trip when a key is available.

## G013 durable persistence backends

The trait crates that need *persistent* (not just real-storage)
backends. Built on top of G010 transports.

| Crate | Trait | Current backend | Production backend | Status |
|---|---|---|---|---|
| `tdw-outbox` | `OutboxStore` | `InMemoryOutbox` | Postgres-backed (depends on G010 postgres) | ⏳ pending |
| `tdw-session` | `SessionStore` | `SqliteSessionStore` (real disk) | Postgres-backed alternative | ⏳ pending (sqlite already real; Postgres is enhancement) |
| `tdw-bus` | `EventBus` | in-memory | Postgres LISTEN/NOTIFY or Kafka | ⏳ pending |
| `tdw-snapshot` | `SnapshotStore` | in-memory | Postgres + S3 (depends on G010) | ⏳ pending |
| `tdw-rollout` | `RolloutStore` | in-memory | Postgres-backed | ⏳ pending |

These all chain on top of G010's `PgEngine` and `S3Engine`. Per-store
PRs become possible once #13 + #14 are merged.

## Suggested next-session order

The pattern is now established; the work is mechanical from here.
Recommended sequence:

1. **Merge open G010 PRs first** (#12, #13, #14, #15) so the foundation lands. See `docs/quality/production-storage-transports.md` for the merge-order constraint.
2. **G010 remaining backends** (1–2 sessions): ClickHouse, then Qdrant, then Meilisearch — each one PR matching the shape of PRs #13/#14.
3. **G011 providers first slice** (1 session): add `reqwest` workspace dep + ship one provider end-to-end (recommend Yahoo first since it's already partial, then FRED because no auth).
4. **G012 LLM first slice** (1 session): ship one Anthropic adapter end-to-end including SSE streaming.
5. **G013 durable persistence** (1–2 sessions): Postgres-backed outbox + session, building on G010's PgEngine.
6. **G014 packaging** (1 session): Dockerfiles + docker-compose orchestration + release workflow.
7. **G015 policy enforcement binding** (1–2 sessions): wire auth/sandbox/mask onto the request path.
8. **G016 aggregate gate** (1 session): final verification across G009–G015.

Realistic total: **8–12 focused sessions** to land G009–G016 end to end.

## Companions

- `docs/quality/end-to-end-smoke.md` — G009 smoke recipe (the baseline every later goal must keep green).
- `docs/quality/production-storage-transports.md` — G010 status + per-backend recipes (Postgres + S3 currently).
- `.omx/ultragoal/goals.json` — authoritative active goal pointer + per-goal objectives.
- `.omx/ultragoal/ledger.jsonl` — append-only history of goal progress + evidence.
