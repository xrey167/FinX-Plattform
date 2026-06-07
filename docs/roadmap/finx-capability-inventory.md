# FinX-Plattform Capability Inventory

**Date:** June 2024  
**Scope:** Current state of FinX-Plattform (origin/main) with provider docs from worktree branches.

---

## 1. DATA COVERAGE

### 1.1 Provider Ecosystem (34 Providers)

| Provider | Vendor | Endpoint Type | Auth Mode | HTTP Feature | Status |
|----------|--------|---------------|-----------|--------------|--------|
| **alpaca** | Alpaca Markets | Historical bars (stock) | API Key (APCA_API_KEY_ID) | `http` | Live |
| **akshare** | AkShare (China) | OHLCV historical (A-shares, HK) | None (public) | `http` | Live |
| **alpha-vantage** | Alpha Vantage | DAILY/GLOBAL_QUOTE | API Key (free tier: 25 req/day) | `http` | Live |
| **binance** | Binance | Ticker price, Trade stream | None (public) | `http`, `ws` | Live |
| **bls** | Bureau of Labor Statistics | Time-series economic data | Optional API key | `http` | Live |
| **cboe** | Cboe Global Markets | Options chain, US-index quotes | None (public CDN) | `http` | Live |
| **coingecko** | CoinGecko | OHLC (crypto) | Optional Demo key | `http` | Live |
| **ccdata** | CCData (CryptoCompare) | Daily OHLCV, asset metadata | API Key | `http` | Live |
| **databento** | Databento | Historical timeseries (OHLCV), metadata | API Key (HTTP Basic) | `http` | Live |
| **deribit** | Deribit | Instruments, order book, funding history | None (public) | `http` | Live |
| **ecb** | European Central Bank | SDW statistical data | None (public) | `http` | Live |
| **eia** | U.S. Energy Information Admin | Spot prices (petroleum), natural gas | API Key | `http` | Live |
| **fileset** | Fixture/synthetic | Equity historical (deterministic) | None | Built-in | Offline-safe |
| **finra** | FINRA | Short interest, OTC weekly summary | None (public) | `http` | Live |
| **fmp** | Financial Modeling Prep | 50+ endpoints (financials, ratios) | API Key | `http` | Live |
| **fred** | Federal Reserve (St. Louis) | Series observations (economic) | API Key (FRED_API_KEY) | `http` | Live |
| **geckoterminal** | GeckoTerminal | DEX pools (on-chain liquidity) | None (public) | `http` | Live |
| **glassnode** | Glassnode | Crypto on-chain metrics | API Key | `http` | Live |
| **huggingface** | Hugging Face | Text generation (inference API) | Bearer token | `http` | Live |
| **nasdaq** | NASDAQ Data Link | Datasets (multi-source) | API Key | `http` | Live |
| **oecd** | OECD | SDMX-JSON statistics | None (public) | `http` | Live |
| **polygon** | Polygon.io | Daily aggregates (stocks) | API Key | `http` | Live |
| **sec** | SEC EDGAR | Filings, company facts | None (public) | `http` | Live |
| **seeking-alpha** | Seeking Alpha | News, earnings, ratings | Varies | `http` | Live |
| **tiingo** | Tiingo | Bars, news, fundamentals | API Key | `http` | Live |
| **tmx** | TMX Group | Canadian equity data | Varies | `http` | Live |
| **tradier** | Tradier | Account/order, market data | Bearer token | `http` | Live |
| **trading-economics** | Trading Economics | Macro indicators, forecasts | API Key | `http` | Live |
| **velodata** | VeloData | Equity research, fundamentals | API Key | `http` | Live |
| **adanos** | Adanos | Sentiment (stocks), trending, Polymarket | API Key | `http` | Live |
| **benzinga** | Benzinga | Company news, earnings calendar | API Key | `http` | Live |
| **ws** | WebSocket provider | Generic websocket stream | Provider-specific | `ws` | Live |
| **ws-mock** | Mock WebSocket | Test/offline fixture | None | Built-in | Offline-safe |

**Data shapes returned:**
- **Market data:** `tdw_domain::MarketDataBar` (OHLCV canonical)
- **Ticks/trades:** `tdw_domain::Tick`
- **Crypto/special:** Provider-specific models
- **News/fundamentals:** Provider-specific models

**Endpoint count:** ~70 distinct fetch operations across all 34 providers.

---

## 2. QUERY & ANALYSIS SURFACE

### 2.1 Daemon Operations (tdw-protocol Op enum)

| Operation | Purpose | Parameters | Notes |
|-----------|---------|-----------|-------|
| `RunQuery` | Execute SQL | `sql`, `plan_id`, `cost_hint` | ClickHouse, Postgres backends |
| `IngestBatch` | Fetch & persist data | `provider`, `endpoint`, `symbols[]`, `range?` | Fan-out per symbol |
| `StreamStart` | Open live WebSocket | `provider`, `symbol`, `table?` | Routes to provider bronze table |
| `StreamStop` | Close live stream | `stream_id` | Graceful shutdown |
| `ToolCall` | Agent tool invocation | `tool_name`, `arguments`, `permission_id?` | tdw-tool-exec dispatch |
| `AppendUserMessage` | LLM context append | `message` | For agentic sessions |
| `ApprovalResponse` | Approval gate response | `permission_id`, `decision`, `reason?` | AllowOnce/AlwaysAllow/Deny |
| `CompactContext` | Trim session tokens | `target_tokens` | Keeps model context bounded |
| `Cancel` | Abort in-flight op | `op_id` | Cascades to children |
| `Shutdown` | Graceful daemon stop | — | Drains queued ops |

### 2.2 SQL Surface (Relational/OLAP)

**Backends:**
- **ClickHouse** — Column-store OLAP, streaming ingest, aggregations
- **PostgreSQL** — OLTP fallback, transactional consistency
- **Parquet** — File-based columnar (batch export, archival)
- **S3** — Cloud blob store integration
- **Router** — Multi-backend dispatch
- **Filesystem** — Local disk (dev/test)

**Schema domains:**
1. Market data (OHLCV, ticks)
2. Orders & positions
3. News & sentiment
4. Fundamentals
5. Strategy & backtests
6. Risk metrics
7. Time/calendar
8. Operations/audit
9. Reference data (symbols, taxonomy)
10. Costs/fees
11. Economic indicators

### 2.3 UDF Runtimes

| Runtime | Status | Use Cases |
|---------|--------|-----------|
| **JavaScript** | Live | Real-time calculations, custom metrics |
| **Python** | Live | Heavy compute, ML, indicator libraries |
| **WebAssembly** | Live | High-performance, vectorized ops |
| **External** | Live | Multi-language, distributed compute |

### 2.4 Search & Vector

- **Lexical:** Meilisearch (full-text, typo tolerance)
- **Vector:** Qdrant (embeddings via tdw-embed-openai/google/local)

### 2.5 Knowledge Graph & Agent

- **tdw-kg** — RDF-like fact store, entity resolution
- **tdw-agent** — Agentic runtime, consolidation, facets, watch
- **Multi-model dispatch:** Anthropic, OpenAI-compat
- **Tool registry:** tdw-tool-exec with MCP binding

### 2.6 Technical Indicators & Portfolio Analytics

**Status:** NOT YET IMPLEMENTED as native crates.

**Workaround:** UDF runtimes (Python UDF can call TA-Lib, pandas, numpy; JS UDF can use lightweight libraries)

---

## 3. PLATFORM SURFACE

### 3.1 CLI (tdw-cli)

**Commands:**
- `tdw-cli --smoke [SYMBOL]` — Offline end-to-end smoke test
- `tdw-cli run-query [SQL]` — Submit SQL to daemon
- `tdw-cli` (default) — Connect & shutdown

**Transports:** TCP (default `127.0.0.1:7878`), UDS (Unix), HTTP planned.

### 3.2 MCP (Model Context Protocol)

**Implementation:** `tdw-mcp` crate + worker deployment.

**Capabilities:**
- **Tools:** Dynamic registry from `tdw-tool-exec`
- **Resources:** Provider metadata, symbol reference, exchange calendar
- **Streaming:** SSE support for long-running operations

### 3.3 REST/HTTP Surfaces

**Service API:**
- Daemon: TCP (Op/Event length-delimited JSON)
- **Planned:** HTTP+SSE wrapper (PR #161)
  - `POST /ops` — Submit operation
  - `GET /events/{op_id}` — SSE stream
  - `GET /health` — Readiness probe
  - `GET /metrics` — Prometheus metrics

### 3.4 TUI (Text User Interface)

**tdw-tui crate:** Terminal dashboard foundation; charting/search TBD.

### 3.5 Charting & Export

**Export formats:**
- **Parquet** — Native columnar (via `tdw-storage-parquet`)
- **CSV** — Via `OutputChunk` events
- **JSON** — Protocol events are JSON-serializable

**Charting:** NOT YET IMPLEMENTED (export to Parquet/CSV, use external tools)

### 3.6 Credentials & Configuration

**Environment variables:**
- `TDW_<PROVIDER>_API_KEY` — Provider-specific keys
- `TDW_<PROVIDER>_LIVE` — Enable live integration tests
- `TDW_STORAGE_*` — Backend config
- `TDW_LLM_PROVIDER` — Select LLM (anthropic, openai-compat)
- `TDW_EMBED_PROVIDER` — Embedding model

**Config layers:**
- Workspace `.env.example`
- Per-session overrides
- Feature-gated provider enablement

### 3.7 Extension Story

**Feature-gated providers:**
- Each `tdw-provider-*` has optional `http` feature (default off)
- Compile-time opt-in; offline by default

**Custom providers:**
- Implement `tdw_core::Fetcher` (async, single endpoint)
- Implement `tdw_core::Streamer` (WebSocket)
- Register in `tdw-agent` registry

**Custom analysis:**
- UDF: Python/JS/WASM function
- dbt macro: Domain-specific transforms
- Agent tool: Implement `ToolDescriptor`, register in MCP

---

## 4. KEY FINDINGS

### Strengths

1. **Provider breadth:** 34 diverse vendors (equities, crypto, macro, derivatives, news, fundamentals)
2. **Multi-backend storage:** ClickHouse (OLAP), Postgres (OLTP), cloud-native (S3), file (Parquet)
3. **Polyglot UDF:** JS/Python/WASM/external runtimes
4. **Daemon protocol:** Clean Op/Event model with async, cancellation, approval gates, streaming
5. **MCP integration:** Tools & resources dynamically loaded; SSE streaming planned

### Gaps

1. **Technical indicators:** No native sma/ema/rsi/volatility library; use UDF workaround
2. **Portfolio analytics:** No standalone returns/sharpe/correlation; build with SQL + UDF
3. **Charting:** No built-in visualization; export & use external tools
4. **REST/HTTP API:** TCP/UDS only; HTTP+SSE wrapper in progress (PR #161)
5. **TUI completeness:** Basic scaffold; charting/search TBD
6. **Real-time streaming:** Only `ws` provider; not bidirectional yet
7. **Live test coverage:** Feature-gated; CI offline by default

---

## 5. PROVIDER METADATA SUMMARY

### Authentication Overview

| Type | Count | Examples |
|------|-------|----------|
| No auth (public) | 12 | binance, cboe, ecb, finra, oecd, coingecko demo |
| API Key required | 16 | alpaca, fmp, polygon, trading-economics |
| Bearer token | 2 | huggingface, tradier |
| HTTP Basic | 1 | databento |
| Custom header | 3 | Various |

### Data Categories

| Domain | Providers |
|--------|-----------|
| **Equities (US)** | alpaca, alpha-vantage, polygon, sec, tiingo, fmp, tradier |
| **Equities (Intl)** | akshare (China), tmx (Canada) |
| **Crypto** | binance, ccdata, coingecko, deribit, geckoterminal, glassnode |
| **Macro/Economics** | bls, ecb, eia, fred, oecd, trading-economics |
| **News & Sentiment** | adanos, benzinga, seeking-alpha |
| **Fundamentals** | fmp, sec, seeking-alpha, tiingo, velodata |
| **Options & Derivatives** | cboe, deribit |
| **Fixtures/Testing** | fileset, ws-mock |

---

## 6. CAPABILITY MATRIX

```
┌─────────────────┬────────────────┬───────────────────────────┐
│ Category        │ Coverage       │ Production Readiness      │
├─────────────────┼────────────────┼───────────────────────────┤
│ Data ingest     │ 34 providers   │ Stable (batch + stream)   │
│ Storage backend │ 6 engines      │ Live (CH, PG primary)     │
│ Query language  │ SQL (dbt+raw)  │ Stable                    │
│ Analytics       │ Limited*       │ Via UDF workaround        │
│ UDF compute     │ 4 runtimes     │ Stable                    │
│ Knowledge/KG    │ Basic RDF      │ Beta                      │
│ Agent toolkit   │ Tool dispatch  │ Beta                      │
│ MCP protocol    │ Yes            │ Emerging (SSE planned)    │
│ REST/HTTP API   │ TCP+UDS only   │ In progress (#161)        │
│ TUI/charting    │ Foundation     │ WIP                       │
│ Credentials mgmt│ Env-var based  │ Stable                    │
│ Multi-tenancy   │ Via tenant_id  │ Schema-based (emerging)   │
└─────────────────┴────────────────┴───────────────────────────┘
* sma/ema/rsi/volatility/portfolio metrics require custom UDF or SQL
```

---

**Document version:** 1.0  
**Generated:** 2024-06-07  
**Source:** FinX-Plattform main + docs-providers-{a,b,c} worktrees
