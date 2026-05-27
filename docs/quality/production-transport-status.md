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
| `tdw-storage-clickhouse` | `OlapEngine` | `ClickHouseRecordingEngine` | `ClickHouseHttpEngine` (reqwest HTTP) | ✅ shipped (PR #17 + #26 fix) |
| `tdw-storage-qdrant` | `VectorEngine` | `InMemoryVectorEngine` | `QdrantHttpEngine` (reqwest HTTP) | ✅ shipped (PR #18 + #26 fix) |
| `tdw-storage-meilisearch` | `LexicalEngine` | `InMemoryLexicalEngine` | `MeilisearchHttpEngine` (reqwest HTTP) | ✅ shipped (PR #19 / #26) |
| `tdw-storage-parquet` | — | (utility, not an engine) | — | n/a |
| `tdw-storage-router` | — | (router/dispatcher) | — | n/a |
| **CI containers + env wiring** | — | — | — | ✅ shipped (PR #15 + #21 + #26) |

Pattern reference: `docs/quality/production-storage-transports.md`.

**G010 is complete.** GitHub CI on `main` commit `04d07e3`
passed the real-backend integration job for Postgres, MinIO/S3,
ClickHouse, Qdrant, and Meilisearch.

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
| `tdw-provider-yahoo` | none | `equity_historical` (`YahooHttpEquityHistoricalFetcher`) | real HTTP via Yahoo v8 chart API behind `--features http`; cassette tests + `TDW_YAHOO_LIVE=1` live opt-in | ✅ landed |
| `tdw-provider-fred` | API key via `FRED_API_KEY` | `series_observations` (`FredHttpSeriesObservationsFetcher`) | real HTTP via FRED `/series/observations` behind `--features http`; cassette tests + `TDW_FRED_LIVE=1` live opt-in | ✅ landed |
| `tdw-provider-alpaca` | API key + secret via `APCA_API_KEY_ID` / `APCA_API_SECRET_KEY` | `stock_bars` (`AlpacaHttpStockBarsFetcher`) | real HTTP via Alpaca `/v2/stocks/bars` behind `--features http`; cassette tests + `TDW_ALPACA_LIVE=1` live opt-in | ✅ landed |
| `tdw-provider-binance` | none for public ticker price | `ticker_price` (`BinanceHttpTickerPriceFetcher`) | real HTTP via Binance `/api/v3/ticker/price` behind `--features http`; cassette tests + `TDW_BINANCE_LIVE=1` live opt-in | ✅ landed |
| `tdw-provider-polygon` | API key via `POLYGON_API_KEY` | `aggregates` (`PolygonHttpAggregatesFetcher`) | real HTTP via Polygon `/v2/aggs/ticker/.../range/1/day/.../...` behind `--features http`; cassette tests + `TDW_POLYGON_LIVE=1` live opt-in | ✅ landed |
| `tdw-provider-huggingface` | API token via `HF_TOKEN` / `HUGGINGFACE_API_TOKEN` / `HF_API_TOKEN` | `text_generation` (`HuggingFaceHttpTextGenerationFetcher`) | real HTTP via HuggingFace `/models/{model_id}` behind `--features http`; cassette tests + `TDW_HUGGINGFACE_LIVE=1` live opt-in | ✅ landed |

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
| `tdw-llm-anthropic` | Anthropic Messages API | yes (SSE) | real batch HTTP + SSE streaming via Anthropic `/v1/messages` behind `--features http`; cassette tests + `TDW_ANTHROPIC_LIVE=1` live opt-in | ✅ landed |
| `tdw-llm-openai-compat` | OpenAI / compat | yes (SSE) | real batch HTTP + SSE streaming via OpenAI-compatible `/v1/chat/completions` behind `--features http`; cassette tests + `TDW_OPENAI_COMPAT_LIVE=1` live opt-in | ✅ landed |
| `tdw-embed-openai` | OpenAI embeddings | no | real HTTP via OpenAI `/v1/embeddings` behind `--features http`; cassette tests + `TDW_OPENAI_EMBEDDING_LIVE=1` live opt-in | ✅ landed |
| `tdw-embed-google` | Google embeddings | no | real HTTP via Gemini `/v1beta/models/{model}:embedContent` behind `--features http`; cassette tests + `TDW_GOOGLE_EMBEDDING_LIVE=1` live opt-in | ✅ landed |
| `tdw-embed-local` | hash-based | no | real (in-process, hash) | ✅ already real |
| `tdw-llm` | trait crate | — | trait definitions only | n/a |
| `tdw-embed` | trait crate | — | trait definitions only | n/a |

**G012 is complete.** LLM batch HTTP, LLM SSE streaming, OpenAI
embeddings, and Google embeddings all have feature-gated production
transports with deterministic cassette coverage and explicit live
test gates.

## G013 durable persistence backends

The trait crates that need *persistent* (not just real-storage)
backends. Built on top of G010 transports.

| Crate | Trait | Current backend | Production backend | Status |
|---|---|---|---|---|
| `tdw-outbox` | `OutboxStore` | `InMemoryOutbox` | Postgres-backed (`PgOutboxStore`) | ✅ complete |
| `tdw-session` | `SessionStore` | `SqliteSessionStore` (real disk) | Postgres-backed alternative (`PgSessionStore`) | ✅ complete |
| `tdw-bus` | `EventBus` | in-memory | Postgres-backed (`PgEventBus`) | ✅ complete |
| `tdw-snapshot` | `SnapshotStore` | in-memory | Postgres-backed (`PgSnapshotStore`) | ✅ complete |
| `tdw-rollout` | `JsonlRollout` | filesystem JSONL | locked + synced filesystem JSONL | ✅ complete |

The Postgres-backed stores chain on top of G010's `PgEngine`. `JsonlRollout`
remains filesystem-backed, but G013 now serializes writers with file locking
and calls `sync_all` after each append. The cross-store smoke at
`crates/tdw-session/tests/g013_durable_cross_store.rs` verifies that outbox,
bus, session, snapshot, and rollout persistence can be exercised together.

**G013 is complete.** Remaining production work moves to packaging,
policy-enforcement binding, and the aggregate production-functional gate.

## Suggested next-session order

The pattern is now established; the work is mechanical from here.
Recommended sequence:

1. **G014 packaging**: Dockerfiles + docker-compose orchestration + release workflow.
2. **G015 policy enforcement binding**: wire auth/sandbox/mask onto the request path.
3. **G016 aggregate gate**: final verification across G009–G015.

Realistic total: **8–12 focused sessions** to land G009–G016 end to end.

## Companions

- `docs/quality/end-to-end-smoke.md` — G009 smoke recipe (the baseline every later goal must keep green).
- `docs/quality/production-storage-transports.md` — complete G010 status + per-backend recipes.
- `.omx/ultragoal/goals.json` — authoritative active goal pointer + per-goal objectives.
- `.omx/ultragoal/ledger.jsonl` — append-only history of goal progress + evidence.
