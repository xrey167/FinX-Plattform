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

1. **G011 providers next slice**: Yahoo, FRED, Polygon, Alpaca, and Binance are landed; continue with HuggingFace.
2. **G012 remaining LLM/embedding adapters**: Anthropic HTTP has landed; OpenAI-compatible, OpenAI embeddings, and Google embeddings remain.
3. **G013 durable persistence remainder**: outbox, snapshot, bus, and session Postgres slices have landed; rollout persistence and cross-store verification remain.
4. **G014 packaging**: Dockerfiles + docker-compose orchestration + release workflow.
5. **G015 policy enforcement binding**: wire auth/sandbox/mask onto the request path.
6. **G016 aggregate gate**: final verification across G009–G015.

Realistic total: **8–12 focused sessions** to land G009–G016 end to end.

## Companions

- `docs/quality/end-to-end-smoke.md` — G009 smoke recipe (the baseline every later goal must keep green).
- `docs/quality/production-storage-transports.md` — complete G010 status + per-backend recipes.
- `.omx/ultragoal/goals.json` — authoritative active goal pointer + per-goal objectives.
- `.omx/ultragoal/ledger.jsonl` — append-only history of goal progress + evidence.
