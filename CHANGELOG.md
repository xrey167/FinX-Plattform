# Changelog

All notable changes to FinX-Plattform are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
SemVer tags `vMAJOR.MINOR.PATCH` as defined in [`docs/release.md`](docs/release.md).

From `v1.0.0` onward the project follows standard SemVer: `MAJOR` for
backward-incompatible protocol/persistence/API/operator-contract changes,
`MINOR` for backward-compatible user-visible additions, and `PATCH` for
compatible fixes, docs, CI-only changes, and packaging repairs. The pre-1.0
history below used `MINOR` for any user-visible change while the major version
was `0`. The workspace `Cargo.toml` `version` field is intentionally not bumped
per release — releases are tag-driven (see [`docs/release.md`](docs/release.md)).

## [Unreleased]

## [1.2.0] - 2026-06-10

The "live data, for real" release: the MCP financial-data server now serves
live market data end to end, verified by driving it as a real MCP client. See
[`docs/release/v1.2.0-notes.md`](docs/release/v1.2.0-notes.md).

### Added

- **`live` feature on `tdw-mcp`** (#269). Swaps the offline Yahoo fixture for
  the real HTTP fetcher and registers every live HTTP provider (34 providers /
  51 fetcher endpoints vs 3 offline). GHCR images (#271) and tagged release
  binaries (#272) build with it, so distribution artifacts serve real data.
- **Generic provider dispatch: `tdw.provider.fetch`** (#274). Any compiled-in
  fetcher is callable by `(provider, endpoint)`; a drift-guard test pins
  dispatch completeness against the registry. Previously only yahoo + fileset
  were reachable from `tools/call`.
- **Yahoo cookie+crumb handshake** (#268). v10 `quoteSummary` / v7
  quote/options endpoints reject anonymous requests with 401 "Invalid Crumb";
  the fetcher now performs the browser handshake lazily on 401/403 only, so
  offline tests never touch the network.
- **Live-test coverage** (#267, #268): CoinGecko's documented live test now
  exists; Binance gained a live websocket subscribe test (first real BTCUSDT
  trade tick asserted).
- **MCP quickstart** (#271, #272): `docs/products/mcp-quickstart.md` — GHCR
  one-liner, from-source build, Claude Code/Desktop wiring, per-provider
  API-key table. Plus the tool-surface audit that drove this release
  (`docs/products/mcp-tool-surface-audit.md`).
- **Nightly live-smoke job** (#275): provider live tests + MCP E2E live bars.
- **Per-crate `pedantic`+`nursery` deny** in the 58 lint-clean crates (#257)
  and a **performance benchmark + regression ratchet** in `xtask` (#263).

### Fixed

- **SEC EDGAR live conformance** (#267): CIKs normalized from the zero-padded
  wire form; XBRL revenue extraction falls back through the post-ASC-606
  us-gaap concepts (`RevenueFromContractWithCustomerExcludingAssessedTax`,
  `Revenues`, `SalesRevenueNet`).
- **`block_on` reactor panic** (#269): the noop-waker busy-poll panicked with
  "there is no reactor running" the moment a live reqwest fetcher ran, and a
  naive runtime rebuild panicked inside `#[tokio::main]` callers; the helper
  is now runtime-context-aware.
- **Container Image CI timeouts** (#270): buildx GHA layer cache (scoped per
  binary, shared by the scan and push builds) + a 120-minute cold-cache
  backstop; main-push image builds previously died at the 60-minute job
  timeout building the workspace under QEMU.

### Changed

- **MCP catalog honesty** (#273, #274): `.sample` evidence tools are hidden
  from the default `tools/list`, and the server discloses fixture-vs-live
  data mode in `initialize` instructions and tool descriptions.
- **G005–G007 hardening reconciled to main** (#264, #265): bounded HTTP
  clients via the shared `build_client` (10s connect / 30s request timeouts),
  validator coverage for the new protocol ops, and the hooks deny→ask
  reconciliation.
- **Dependency hygiene** (#266): unused deps pruned (eval-runner `serde_json`;
  service `tokio-util`, `toml`) and a `needless_return` cleanup in `tdw-core`.

## [1.1.0] - 2026-06-08

Security, observability, feature-platform, and production-readiness release on
top of `v1.0.0`. Hardens authentication (cryptographic OIDC, constant-time token
comparison, loopback-default daemon bind), wires real storage/compute engines
into the `live` profile by default, completes registry-driven dispatch, lands
the worker dead-letter operator surface and full ops/health surface, and adds
the first OpenBB-gap-closure layer (standardized result envelope, cluster data
models, shared query params, logical-endpoint resolution, symbology). It also
builds out the application feature platform — alert engine, transactional and
broadcast email, news aggregation, a multi-step function/cron spine, first-party
identity/session/password stores, the Finnhub provider, an LLM fallback/router
with error classification, and tool-execution autonomy gating with a hash-chained
receipt log — on top of a workspace-wide documentation, self-improve, and
dependency-hygiene sweep.

### Added

- **Real engines by default in the `live` profile** (#157). `tdw-service-api`'s
  `AppState` now wires the real ClickHouse / Postgres / Qdrant / Meilisearch /
  S3 engines (behind feature gates) instead of in-memory stand-ins when the
  `live` Compose profile is selected, so the deployed stack exercises the
  production storage/compute paths. The default offline build is unchanged.
- **Registry-driven dispatch end to end** (#158). Ingest is now driven through
  the provider registry, a `ToolRegistry` routes tool/MCP calls, and the wasm
  UDF runtime is reachable from the daemon dispatch path — closing the gap
  between the registered provider/tool set and what the daemon can actually
  execute.
- **Worker dead-letter operator surface** (#153). `tdw-worker` gains
  `dead-letter list` / `dead-letter replay` CLI subcommands for inspecting and
  re-enqueueing dead-lettered jobs, plus a bounded-concurrency clamp so
  `TDW_WORKER_CONCURRENCY` cannot exceed a safe ceiling. Documented in
  `docs/release/worker-deployment.md`.
- **Cryptographic OIDC verification** in `tdw-auth-oidc` (#150). New
  `verify_jwt` / `verify_jwt_strict` verify a compact JWT's signature against
  supplied verifying keys (RS256/ES256, resolved by `kid`) and enforce
  `exp`/`nbf`/`iat` (60s clock-skew leeway), issuer, and audience — failing
  closed on any error. The `none` pseudo-algorithm and HMAC tokens are rejected
  (alg-confusion / `alg:none` defence). Built on `jsonwebtoken` (default `ring`
  backend, already a vetted transitive dependency). The existing structural
  claim/JWKS checks remain as a pre-filter. Remote JWKS fetch stays out of
  scope: verifying keys are supplied from the configured JWKS.
- **Ops/health surface + graceful drain** (#161, G002). `/health`, `/ready`,
  and `/metrics` endpoints plus coordinated graceful drain for the daemon,
  worker, and MCP server, so the deployed stack is probe- and shutdown-aware.
- **Price-alert engine** (#180, #187, #199). A `PriceAlert` domain model with a
  Postgres migration and alert stores (#180), a `tdw-alert-evaluator` price-alert
  evaluation function on a 5-minute cron (#187), and owner-scoped alert CRUD
  daemon ops in `tdw-service-api` (#199).
- **Function/cron spine** (#177, #185, #186). A `tdw-cron` recurring-trigger
  spine over the worker queue (#177), a multi-step `tdw-functions` registry with
  per-step memoization (#185), and cron/event triggers wired to worker-job
  execution (#186).
- **Transactional + broadcast email** (#183, #201). `tdw-email` transactional
  SMTP send with HTML template fill (#183), plus a marketing/broadcast client
  behind a `broadcast` feature (#201).
- **News aggregation policy layer** (#204). A new `tdw-news-compose` crate that
  aggregates and composes news under an explicit policy layer.
- **First-party identity stores** (#193, #205). A first-party user + password
  store using argon2 (#193) and a session store (#205) in `tdw-identity`.
- **Finnhub provider** (`tdw-provider-finnhub`, #192). Company profile and quote
  fetchers following the canonical provider pattern.
- **LLM fallback, router, and error classification** (#169, #194, #195).
  `tdw-llm` gains a `FallbackModel` primary→secondary provider wrapper (#169), a
  credential-aware provider router (#195), and retryable-vs-permanent error
  classification (#194).
- **Tool-execution autonomy gating + receipt log** (#196, #197, #198).
  `tdw-tool-exec` gates execution on `ToolEffect` risk via an opt-in
  `AutonomyLevel` (#196), keeps an opt-in hash-chained tool-receipt log (#197),
  and validates call arguments against an opt-in arg schema before dispatch
  (#198).
- **FunctionRegistry over HTTP** (#188). `tdw-app-server` exposes the
  `FunctionRegistry` over HTTP with HMAC request signing.
- **Live `QuoteSnapshot` read path** (#179). A `QuoteSnapshot` domain type plus
  an uncached live read path for quote data.
- **OpenBB-gap-closure layer 1** (#176, #173, #190, #191). The first layer of
  the OpenBB clean-room gap-closure plan (analysis + layered plan in #176): a
  standardized result envelope and cluster data models, shared query-param
  normalization with a yahoo/fred pilot (L1.3, #191), logical-endpoint provider
  resolution (L1.5, #190), and a pure ticker-symbology normalization crate
  `tdw-symbology` (#173).

### Changed

- **Constant-time bearer-token comparison** on the MCP Streamable HTTP layer
  (#150). `TDW_MCP_HTTP_TOKEN` validation now compares tokens via `subtle`'s
  `ConstantTimeEq` over fixed-width digests instead of `==`, removing the
  timing side channel (and not leaking token length).
- **Safe daemon TCP defaults** (#150). The daemon TCP transport already
  defaults to loopback (`127.0.0.1:7878`) when `TDW_DAEMON_TCP_BIND` is unset;
  it now logs a prominent `SECURITY WARNING` at startup when bound to a
  non-loopback address with no auth-backed policy attached. **Operator note:**
  deployments that previously relied on an implicit non-loopback bind must set
  `TDW_DAEMON_TCP_BIND` explicitly and attach an auth-backed policy.
- **Partial OIDC config is now a hard startup error** (#150). A
  partially-configured `prod`/`production` boot (some but not all `TDW_OIDC_*`
  set, or invalid JWKS/claims) makes the daemon **refuse to start**, with a
  diagnostic listing every missing variable. A fully-unset OIDC config keeps the
  existing fail-closed (starts, dispatches return `Failed`) behavior.
  `OidcPolicyError` gained `MissingEnvVars(Vec<&'static str>)` (replacing the
  single-var `MissingEnvVar`).
- **CI: live-stack + tools smoke jobs, aarch64 release leg, multiarch images**
  (#155). A new `live-stack` workflow brings up the Compose stack and runs the
  smoke path; the CI tools job covers the `tdw-cli`/`tdw-mcp` surface; the
  release workflow gains an `aarch64-unknown-linux-gnu` build leg; and container
  images are now built multiarch.
- **CI: concurrency groups** (#200) cancel superseded PR runs so only the latest
  push per branch consumes runners.
- **`jsonwebtoken` 9.3.1 → 10.4.0** on the `rust_crypto` backend (#163), keeping
  the OIDC verifier on a current, maintained JWT implementation.

### Security

- The OIDC, constant-time token comparison, and loopback-default daemon-bind
  changes (#150) collectively harden the production authentication and transport
  posture. See *Upgrade notes* in
  [`docs/release/v1.1.0-notes.md`](docs/release/v1.1.0-notes.md) for the
  breaking-for-exposed-deployments details.
- `cargo-deny` now ignores `RUSTSEC-2026-0173` (proc-macro-error2 unmaintained),
  a build-time-only transitive advisory with no runtime exposure (#202).

### Performance

- **Verification wall time halved** (#178). Test-target gating, a doctest-harness
  purge, and fixture shrinking cut workspace verification wall time by ~54% over
  three self-improve iterations, without reducing coverage.

### Docs

- **Consolidated `TDW_*` environment reference + operator setup** (#149). New
  [`docs/CONFIGURATION.md`](docs/CONFIGURATION.md) is the single source of truth
  for every `TDW_*` variable, with a rewritten `.env.example`, a
  `secrets-and-tls.md` runbook, and `compose-setup` helper scripts.
- **Comprehensive per-crate README/ARCHITECTURE/examples across all crates**
  (#164–#168, #170–#172). A workspace-wide documentation sweep adding a README,
  an ARCHITECTURE note, and runnable examples to every crate — provider crates
  (batches A/B/C, including the ws and proto crates), domain/data, storage and
  persistence, AI (llm/embed/agent/udf), service/binary, and core infra.
- **OpenBB clean-room gap analysis + layered closure plan** (#176). A roadmap
  document that scopes the OpenBB feature gap and the layered plan to close it.
- **Lint-debt sweep.** `missing_const_for_fn` resolved across 16 crates
  (#156, #160) and `too_long_first_doc_paragraph` across 4 crates (#162).
- **Release roster + 1.0 gap-audit closure.** Crate-readiness roster sync (#182)
  and the 1.0 gap-audit closure (#159).

### Internal

- **Self-improve campaigns.** Provider HTTP fetchers deduplicated via a shared
  core (−60% duplication, #175); workspace verification time reduced (−54%, #178,
  see *Performance*); line coverage raised across `tdw-backend`/`tdw-service-api`/
  `tdw-core` (#181) and daemon-serving paths covered (#203); and 17
  code-reference-free workspace dependency edges removed (351 → 334, #184).

## [1.0.0] - 2026-06-07

### Added

- Release 1.0 readiness hardening: Yahoo's real HTTP fetcher is now selectable
  through `tdw-service-api`'s provider feature set and included in
  `all-http-providers`.
- Deterministic coverage for the `tdw-bootstrap`, `tdw-cli`, and `tdw-proto`
  crates so the batch backlog no longer treats them as untested leaf binaries.

### Changed

- Updated release-facing README status text to reflect the existing tag history
  and the active `v1.0.0` readiness branch.

## [0.10.0] - 2026-06-06

Protobuf market-data types and full data-provider wiring: every standalone
`tdw-provider-*` crate is now registrable in the service dispatcher behind
per-provider feature gates, with the default build still fully offline.

### Added

- **`tdw-proto` crate** (#139). Protobuf bindings for the core market-data
  types — `OhlcvBar`, `Tick`, `PriceLevel`, `OrderBookSnapshot`, and the
  `MarketDataEnvelope` `oneof` wrapper — generated from
  `proto/market_data.proto`. The generated bindings are vendored
  (`src/finance.gen.rs`) so the crate builds with no system `protoc` and no
  build-time codegen; the runtime depends only on upstream `prost`.
- **All 30 standalone data providers wired into the dispatcher** (#140, #141).
  `tdw-service-api::default_registry()` can now register every
  `tdw-provider-*` crate behind a per-provider cargo feature
  (`provider-<name>`) plus an `all-http-providers` aggregate. With no features
  the default registry stays offline and registers exactly 3 providers
  (fileset, yahoo, mock-ws); `all-http-providers` registers 50 distinct
  `(provider, endpoint)` entries. Seven providers (adanos, benzinga, cboe,
  deribit, eia, glassnode, seeking-alpha) were converted to the canonical
  `tdw_core::Fetcher` trait. The CI lint job now clippy- and test-checks the
  `all-http-providers` feature so the wired registry cannot silently regress.
- **`TDW_DAEMON_OPEN_POLICY` escape hatch and a worker concurrency default of
  4** (#133).

### Changed

- Dependency bumps: `ratatui` 0.30.0 → 0.30.1 (#137), `aws-config`
  1.8.17 → 1.8.18 (#135), `aws-sdk-s3` 1.134.0 → 1.135.0 (#138), `chrono`
  0.4.44 → 0.4.45 (#136), and the `docker/build-push-action` GitHub Action
  6 → 7 (#134).

### Fixed

- `tdw-provider-tiingo` did not compile under `--features http`
  (`TiingoNewsArticle` lacked the `Serialize`/`Deserialize`/`JsonSchema`
  derives required by `DataModel`); fixed and now covered by the new
  `all-http-providers` CI checks (#141).

## [0.9.0] - 2026-05-31

Ten additional data providers completing the gap-analysis coverage sweep
(waves 4 and 5). All follow the canonical `tdw-provider-polygon` pattern:
offline `lib.rs` with validation and mock fetcher, feature-gated
`http_fetcher.rs` with real HTTP + serde deserialization, and cassette +
live integration tests gated by `TDW_*_LIVE=1`.

### Added

- **OECD provider** (`tdw-provider-oecd`, #130). SDMX-JSON endpoint for
  international economic statistics; no API key required.
- **Velodata provider** (`tdw-provider-velodata`, #130). Crypto derivatives
  analytics — funding rates, liquidations, and open interest across
  Binance/Bybit/OKEx/Hyperliquid (`TDW_VELODATA_API_KEY`).
- **ECB provider** (`tdw-provider-ecb`, #130). ECB Statistical Data Warehouse
  — EUR exchange rates and €STR interest rates; no API key required.
- **TMX provider** (`tdw-provider-tmx`, #130). Toronto Stock Exchange equity
  quotes and MX options chain; no API key required.
- **GeckoTerminal provider** (`tdw-provider-geckoterminal`, #130). DeFi/DEX
  on-chain pool data — OHLCV, liquidity, token metrics; no API key required.
- **CCData provider** (`tdw-provider-ccdata`, #131). CryptoCompare daily OHLCV
  and asset metadata (`TDW_CCDATA_API_KEY`).
- **Adanos provider** (`tdw-provider-adanos`, #131). Social sentiment aggregator
  covering Reddit, X, news, and Polymarket events (`TDW_ADANOS_API_KEY`).
- **FINRA provider** (`tdw-provider-finra`, #131). FINRA short interest and
  weekly OTC market summary; public API, no auth required.
- **Seeking Alpha provider** (`tdw-provider-seeking-alpha`, #131). Analyst
  articles and quant/author ratings via RapidAPI
  (`TDW_SEEKING_ALPHA_API_KEY`).
- **Deribit provider** (`tdw-provider-deribit`, #131). Crypto options and
  futures — instrument listing, order book with Greeks, and perpetual funding
  rate history; public endpoints, no auth required.

## [0.8.0] - 2026-05-31

Production auth, embeddable backend, agent learning, and the first fifteen
data providers (waves 1–3). Thirteen user-visible runtime/provider changes
since v0.7.0, so this is a `MINOR` release.

### Added

- **Production OIDC policy** (#116, #119). `TDW_OIDC_*` env vars wire an
  auth-backed policy when `TDW_PROFILE=prod`; observable via a `/healthz`-style
  endpoint. Validation is structural (claim/JWKS consistency), not
  cryptographic.
- **Postgres + Clickhouse MCP servers** (#120). Project-scoped `.mcp.json`
  wires `postgres-mcp` (Pro, read-only) and `mcp-clickhouse` via
  `uvx --python 3.13` so Claude can query live local backends directly.
- **Unified embeddable backend** (`tdw-backend`, #121). Library + binary facade
  over the full warehouse stack; dual sync/async API for embedding or running
  standalone.
- **Durable agent learning** (`tdw-agent-learning`, #122). Knowledge index,
  memory-consolidation loop, and eval feedback cycle with adaptivity gate.
- **Data providers — wave 1** (#123–#127). Databento (CME Globex futures tick
  data), FMP (fundamentals + OHLCV), SEC EDGAR (filings, XBRL), Tiingo (OHLCV
  + news), CoinGecko (crypto market cap / dominance).
- **Data providers — wave 2** (#128). Alpha Vantage, CBOE (options/VIX),
  Benzinga (news + earnings calendar), NASDAQ Data Link, AkShare (Chinese
  A-share + HK markets).
- **Data providers — wave 3** (#129). Tradier (equities + options chains), EIA
  (US energy spot prices), Glassnode (on-chain MVRV/LTH/NUPL), Trading
  Economics (global macro calendar), BLS (US CPI + employment).

## [0.7.0] - 2026-05-30

Daemon hardening and durability follow-ups. The commits in `v0.6.0..HEAD`
include four user-visible runtime/storage changes, so per the pre-1.0 policy
this is a `MINOR` release.

### Added

- **Per-request WASM limits** (#110). `UdfRequest` gains an optional
  `wasm_limits` (`WasmLimitsRequest { fuel, max_memory_bytes, max_memories }`)
  so a caller can give an untrusted UDF a smaller fuel/memory budget per call.
  Values can only **tighten** a limit — they are clamped to the runtime default
  ceiling, never raised above it — so this is a budget knob, not a DoS lever.
  The field is serde-default + skip, so existing `udf.run` payloads
  deserialize/serialize unchanged.
- **Postgres-backed daemon session + rollout stores** (#112). New
  `daemon-postgres` feature plus `SessionBackend` / `RolloutBackend` enums on
  `AppState` and a new `tdw_rollout::PgRollout`. With the feature built **and**
  `TDW_DAEMON_PG_URL` (or `DATABASE_URL`) set, the daemon's session/cost ledger
  and rollout archive persist to Postgres instead of SQLite + a JSONL file, so
  they survive container restarts. Wired into the `live` compose daemon (image
  built `--features daemon-postgres`); default builds are unchanged.
- **Worker concurrency** (#111). `ServeConfig.max_concurrent` +
  `TDW_WORKER_CONCURRENCY` let `tdw-worker --serve` drive up to N in-flight jobs
  at once via `FuturesUnordered` (no extra threads). Default `1` preserves
  strictly serial behavior; shutdown stops new leases and drains in-flight work.
  The `live` worker runs at concurrency 4.

### Changed

- **Daemon honors `TDW_PROFILE`** (#109). `tdw-service` `load_config` now
  applies the `TDW_PROFILE` env var (e.g. the `live` stack's `docker`), so the
  profile-driven local policy attaches as intended and live dispatches resolve
  instead of failing closed. The startup log reports the actual attached-policy
  state rather than a fixed "no policy" message.

### Internal

- Multi-session git guardrails: pre-push hook + house-rules doc (#107); removal
  of files accidentally committed in #106 (#108); clippy pedantic/nursery
  warning cleanup 301 → 14 (#106). CI lint now also compile-checks the
  `daemon-postgres` Postgres store paths.

## [0.6.0] - 2026-05-29

Live streaming ingest plus the full long-running deployment surface. The 5
commits in `v0.5.0..HEAD` are user-visible runtime/protocol/deployment work, so
per the pre-1.0 policy this is a `MINOR` release.

### Added

- **Postgres-backed worker `--serve`** (#101). `tdw-worker --serve` selects its
  durable backend from the environment: `PgWorkerQueue` when built
  `--features postgres` with `TDW_WORKER_PG_URL`/`DATABASE_URL`, otherwise the
  SQLite default. `run_serve` is now generic over `ServeQueue`.
- **Long-running services in the `live` compose profile** (#102). A long-running
  `tdw-service` daemon (`TDW_DAEMON_TCP_BIND` lets it bind `0.0.0.0:7878` for
  cross-container reach), a `tdw-mcp --streamable-http` server (daemon-routed),
  and a Postgres-backed `tdw-worker --serve` (worker image `FEATURES` build-arg).
- **End-to-end streaming ingest** (#100). `run_ws_ingest` + `spawn_stream_ingest`
  make a `Streamer` reachable as a cancellable background ingest task draining
  into the OLAP engine; restart-safe via the content-addressed dedup token
  (at-least-once, no materialized-view double-count).
- **Live Binance trade feed + indicators** (#104). `tdw-provider-binance`
  `BinanceTradeStreamer` (live `wss://stream.binance.com` behind a `ws` feature;
  deterministic offline `decode_trade_frame` seam), `Op::StreamStart`/`StreamStop`
  protocol ops with `tdw-acp` validation and dispatcher routing, plus fixed-N
  volatility and an exact Wilder RSI UDF.

### Fixed / Security

- **Hardened `live` MCP/daemon exposure** (#103). `TDW_MCP_HTTP_TOKEN` is now
  required (no weak default) so a host-published, non-loopback MCP bind is always
  authenticated; the `tdw-service` daemon is internal-only (host port publication
  dropped) since its transport is unauthenticated plaintext.

### Notes

- The `live` profile end-to-end run requires a Docker daemon. `tdw-service`
  boots with no policy attached, so dispatched operations return `Failed` until a
  policy is wired.
- `Cargo.toml` keeps `publish = false`; the workspace `version` field is not
  bumped per release (releases are tag-driven).

## [0.5.0] - 2026-05-29

Adds a streaming market-data warehouse on ClickHouse. The 9 commits in
`v0.4.0..HEAD` include a major new ingest/analytics feature, so per the pre-1.0
policy this is a `MINOR` release.

### Added

- **Streaming market-data warehouse on ClickHouse** (#87). Real streaming
  ingest that persists (`dispatch_ingest` now writes the fetched batch with an
  idempotent `async_insert` + dedup-token helper), a `tdw-provider-ws`
  tokio-tungstenite streamer, and raw tables (`raw.tick`/`trade`/`quote`/
  `book_level` with `DateTime64(9)`, DoubleDelta+ZSTD codecs, `LowCardinality`
  dims, monthly partitions, TTL, dedup windows). A tier of always-fresh
  "FlowField" incremental materialized views (`AggregatingMergeTree` + reader
  views): OHLC 1m/5m/1h/1d (per-venue + consolidated), VWAP, daily return, top
  movers, trailing 52w high/low and 30d volatility, quote mid/spread, book
  best-bid/ask + depth + imbalance, daily news sentiment, and technical
  indicators. A reference-entity model (Postgres master + ClickHouse
  dictionaries for `dictGet` enrichment: symbol info, trading calendar,
  corporate actions, FX rates). Optional `tdw-storage-broker` (feature-gated
  pure-Rust `rskafka` write sink + Kafka-engine consumer migrations). Validated
  against live ClickHouse 26.6 + PostgreSQL 18.

### Changed

- **CI** now lints and tests the opt-in UDF runtime features
  (`tdw-udf-wasm --features wasmi`, `tdw-sandbox`/`tdw-service-api --features
  udf-wasm`), closing a blind spot where those paths were never built in the
  default matrix; `dependabot.yml` ignores wasmi semver-major bumps (#97).
- **Dependency bumps**: `aws-sdk-s3` 1.133→1.134 (#96), `uuid` 1.23.1→1.23.2
  (#98), and GitHub Actions `setup-buildx`/`upload-artifact`/`download-artifact`/
  `login-action`/`attest-build-provenance` (#90–#94).

### Notes

- `Cargo.toml` keeps `publish = false`; the workspace `version` field is not
  bumped per release (releases are tag-driven).

## [0.4.0] - 2026-05-29

Runtime follow-ups on top of `v0.3.0`: the worker now executes real work, and
the hardened UDF runtime is wired end to end. The 3 commits in `v0.3.0..HEAD`
are user-visible runtime changes, so per the pre-1.0 policy this is a `MINOR`
release.

### Added

- **Worker daemon dispatch.** `tdw-worker --serve` gains a `DaemonJobHandler`
  that submits each leased job's `OpEnvelope` to the configured TDW daemon via
  `tdw-app-client` and maps the terminal event onto the job contract
  (`Completed` -> complete; `Failed`/`Cancelled`/transport error -> retry then
  dead-letter). Selected by `TDW_WORKER_DISPATCH=daemon` / `TDW_WORKER_DAEMON_*`
  (TCP/UDS/HTTP-SSE); the offline `LoggingAckHandler` stays the default. (#85)
- **Wasm UDF string ABI.** `tdw-udf-wasm` adds `execute_wasm_string` (feature
  `wasmi`): a linear-memory string-in/string-out ABI (guest exports `memory` +
  `alloc(i32)->i32` + `<func>(in_ptr,in_len)->i64` returning packed
  `(out_ptr,out_len)`) under the existing fuel/memory/deny-imports hardening.
  All guest memory access uses wasmi's checked `Memory::read`/`write`, so a bad
  pointer/length or non-UTF-8 output yields `BadAbi`, never a host panic. (#86)
- **Sandbox routing to the hardened runtime.** `tdw-sandbox`'s `udf-wasm`
  feature now enables `tdw-udf-wasm/wasmi`; a `UdfRuntime::Wasm` request whose
  `source` base64-decodes to a real wasm module runs through
  `execute_wasm_string`, otherwise it falls back to the deterministic fixture.
  This completes the UDF runtime hardening scope (step #5). (#88)

### Notes

- Default `cargo test --workspace` stays offline: the worker daemon path and the
  `wasmi` UDF path are both opt-in (env / feature); without them the worker uses
  the ack handler and the sandbox uses the built-in dispatcher.
- `Cargo.toml` keeps `publish = false`; the workspace `version` field is not
  bumped per release (releases are tag-driven).

## [0.3.0] - 2026-05-29

Runtime, sandbox, and live-backend hardening on top of `v0.2.0`. The 12 commits
in `v0.2.0..HEAD` add a supervised worker process, a real sandboxed UDF engine,
and a fully-bootstrapped live data backend - user-visible runtime and storage
work, so per the pre-1.0 policy this is a `MINOR` release.

### Added

- **Supervised worker process.** `tdw-worker --serve` / `--serve-once` run a
  `WorkerRunner` lease loop over the durable queue: lease (with payload) →
  run a `JobHandler` → complete, or fail with retry/dead-letter at
  `max_attempts`. In-flight jobs always finish on shutdown (the stop signal is
  observed only between jobs). Tunables via `TDW_WORKER_DB` / `TDW_WORKER_ID` /
  `TDW_WORKER_LEASE_TTL_MS` / `TDW_WORKER_POLL_MS`. (#72)
- **Real `wasmi` UDF runtime.** `tdw-udf-wasm` gains a `wasmi`-backed engine
  behind the opt-in `wasmi` feature (`execute_wasm_i64`): fuel metering
  (`FuelExhausted`), `WasmLimits` memory caps (`MemoryLimitExceeded`), and
  deny-by-default host imports (empty `Linker`). The deterministic fixture path
  stays the default. (#79)
- **Live backend expansion.** The `live` compose profile now brings up
  ClickHouse, Qdrant, and Meilisearch; `tdw-bootstrap` creates baseline schemas
  in each (ClickHouse `tdw` DB + marker table, Qdrant `tdw-default` collection,
  Meilisearch `tdw-default` index) alongside the Postgres/S3 bootstrap; and a
  long-running `tdw-worker --serve` service starts after bootstrap succeeds.
  `QdrantHttpEngine::ensure_collection` is now public and
  `MeilisearchHttpEngine::ensure_index` was added. (#81)
- **Test-policy hardening.** Mutation tooling (#71), the first loom concurrency
  model on `tdw-app-server` (#73), stable corpus-replay fuzz harnesses for six
  parser surfaces (#74), nightly `cargo-fuzz` targets with a CI smoke job
  (#75, #78), and an `xtask` pre-release fuzz+loom check recipe (#76) - closing
  TEST-POLICY-001 through 005.

### Changed

- Reduced clippy pedantic/nursery warnings across the workspace (#80, #83).

### Docs

- Added the full deployed-stack runbook (#77) and updated the data-backend
  runbook + transport-status matrix for the expanded `live` profile.

### Notes

- `Cargo.toml` keeps `publish = false`; the workspace `version` field is not
  bumped per release (releases are tag-driven).

## [0.2.0] - 2026-05-29

First substantial release after the early `v0.1.0`/`v0.1.1` packaging tags
(both cut on 2026-05-28). The 15 commits in `v0.1.1..HEAD` land the daemon
runtime, the MCP server surface, durable worker schedulers, and the live data
backend - user-visible runtime and protocol work, so per the pre-1.0 policy
this is a `MINOR` release.

### Added

- **Daemon runtime (ADR-0012).** Completed the P0-P8 daemon integration cycle
  and the stateful `tdw-mcp` stdio JSON-RPC MCP server (`initialize`,
  `tools/*`, `resources/*`, `prompts/*`, progress notifications, cancellation,
  and error paths).
- **MCP Streamable HTTP transport.** `tdw-mcp --streamable-http [bind]` serves
  the same MCP protocol over a local-first HTTP endpoint at `/mcp`, with Origin
  validation, `MCP-Protocol-Version` checks, header/body size bounds, and
  optional bearer auth via `TDW_MCP_HTTP_TOKEN` (#63).
- **Daemon-backed MCP tools.** `tdw.daemon.triage` and
  `tdw.daemon.query.submit` build `OpEnvelope` operations and route them
  through `tdw-app-client` to a live daemon, returning event evidence as
  structured MCP output; deterministic offline tools continue to run without a
  daemon.
- **MCP daemon-client transport expansion.** `tdw-app-client` submits daemon
  operations over TCP, `cfg(unix)` Unix domain sockets, and plain HTTP/SSE
  (`POST /op` + `GET /events`). Selection is configurable through
  `TDW_MCP_DAEMON_TRANSPORT`, `TDW_MCP_DAEMON_ADDR` (or `TDW_DAEMON_TCP_BIND`),
  and `TDW_MCP_DAEMON_TIMEOUT_MS`; unsupported endpoints (Windows UDS, HTTPS
  HTTP/SSE) fail closed.
- **Durable worker schedulers.** `tdw-worker` gains an embedded SQLite durable
  scheduler (priority leasing, lease expiry, retry/dead-letter, idempotent
  enqueue/complete, stats, `--durable-smoke`) and a distributed Postgres
  `PgWorkerQueue` behind the `postgres` feature that mirrors the same contract.
  The in-memory contract backend remains for offline tests.
- **G014 live data backend.** A `live` compose profile plus the `tdw-bootstrap`
  one-shot binary bring up Postgres + MinIO, apply the G013 Postgres schemas,
  and write/read back an S3 marker; documented in
  `docs/release/data-backend-runbook.md` (#47).
- **Protocol and integration test coverage.** Always-on `tdw-app-client`
  daemon-framing tests (length-delimited writes, empty/oversized frame
  rejection, terminal-event matching, HTTP/SSE submit-path derivation), an
  env-gated daemon-backed MCP integration test
  (`TDW_MCP_DAEMON_INTEGRATION_ADDR`), an env-gated durable Postgres worker test
  (`TDW_POSTGRES_TEST_URL`), and a CI worker Postgres-queue step.
- **Test-policy decisions encoded.** ADR `docs/adr/0014-test-policy-backlog.md`
  plus policy docs fix the mutation cadence (O24), the first loom model scope
  (O25), and the initial fuzz-target list (O26), with deferred enforcement
  tracked as `TEST-POLICY-001..005`.
- **Deployment guidance for the remaining product gaps.**
  [`docs/release/mcp-remote-deployment.md`](docs/release/mcp-remote-deployment.md)
  (remote MCP HTTP behind a TLS/OAuth reverse proxy) and
  [`docs/release/worker-deployment.md`](docs/release/worker-deployment.md)
  (`PgWorkerQueue` rollout, supervision, and lease/dead-letter monitoring).

### Changed

- **Bounded daemon connect.** `tdw-app-client` uses
  `TcpStream::connect_timeout` for validated TCP daemon endpoints so the
  configured timeout now covers connection establishment as well as read/write.
- **License.** Repository relicensed to dual `MIT OR Apache-2.0` (#68).
- **Reduced pedantic/nursery lint noise** across the workspace; tooling pins
  `--target-dir` on the clean-room audit and documents the WDAC gotcha (#66).

### Governance and tooling

- Agent rules tracked and worktree cleanup guarded (#48); patch-equivalent
  worktree branch deletion fixed (#49); `.mcp.json` gitignored (#61); the
  dormant `create-private-repo` bootstrap script removed (#64); the production
  functional gate documented (#46); and validated ULTRAQA characterization
  coverage salvaged.

### Notes

- `Cargo.toml` keeps `publish = false`; the workspace `version` field is not
  bumped per release (releases are tag-driven, as they were at `v0.1.1`).

## [0.1.1] - 2026-05

Packaging and fix release on top of the `v0.1.0` G014 release surface. See the
`v0.1.1` tag and its GitHub release for the packaged archives, checksums, and
attestations.

## [0.1.0] - 2026-05

Initial tagged release. G014 release-packaging surface for `tdw-service`,
`tdw-cli`, `tdw-mcp`, and `tdw-worker`: multi-target binary archives with
checksums and build-provenance attestations, plus scanned GHCR container
images. See `docs/release.md` for the full artifact and image policy.

[Unreleased]: https://github.com/xrey167/FinX-Plattform/compare/v1.1.0...HEAD
[1.1.0]: https://github.com/xrey167/FinX-Plattform/compare/v1.0.0...v1.1.0
[1.0.0]: https://github.com/xrey167/FinX-Plattform/compare/v0.10.0...v1.0.0
[0.10.0]: https://github.com/xrey167/FinX-Plattform/compare/v0.9.0...v0.10.0
[0.9.0]: https://github.com/xrey167/FinX-Plattform/compare/v0.8.0...v0.9.0
[0.8.0]: https://github.com/xrey167/FinX-Plattform/compare/v0.7.0...v0.8.0
[0.7.0]: https://github.com/xrey167/FinX-Plattform/compare/v0.6.0...v0.7.0
[0.6.0]: https://github.com/xrey167/FinX-Plattform/compare/v0.5.0...v0.6.0
[0.5.0]: https://github.com/xrey167/FinX-Plattform/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/xrey167/FinX-Plattform/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/xrey167/FinX-Plattform/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/xrey167/FinX-Plattform/compare/v0.1.1...v0.2.0
[0.1.1]: https://github.com/xrey167/FinX-Plattform/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/xrey167/FinX-Plattform/releases/tag/v0.1.0
