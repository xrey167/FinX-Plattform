# OpenBB → FinX Gap-Closure Plan (clean-room "adapt the missing, don't copy")

> **Clean-room rule (applies to every item below):** implement from docs-level specs only
> (OpenBB public docs, the source vendor's own public API docs, the standard-model field
> lists in `docs/roadmap/openbb-surface-domains.md`). **Never read or copy OpenBB source
> code.** Replicate *capability and field shape*, written natively against FinX's
> `tdw_core::Fetcher` / `tdw-service-api` patterns.

**Inputs synthesized:** `openbb-surface-domains.md` (17 routers / 306 commands),
`openbb-surface-infra.md` (12 infra areas), `finx-capability-inventory.md`
(34 providers / ~70 endpoints). **Baseline:** main @ 88eb9ec; `tdw-symbology` = PR #173
(in flight); HTTP+SSE service = PR #161 (in flight).

**How to resume:** every table has a **Status** column (`todo` / `in-progress` / `done`).
Each L1–L5 item is a `/batch`-able task: **crate · scope · gates · done-when**. Edit Status
in place as work lands.

---

## P1 roll-up — 2026-06-11 (OpenBB-parity phase 1 closed)

> **Scoreboard snapshot.** The endpoint catalog (`tdw-endpoint-catalog`) now exposes
> **80 routes / 71 provider candidates** (`xtask catalog-check` green): **60 `Fetch`
> routes** (each a `GET /api/v1/<route>` and an OpenBB-Workspace widget) + **20
> `technical/*` `Compute` routes**. The generated OpenAPI 3.1 document
> (`docs/schemas/openapi.json`) carries **60 paths / 21 schemas**, drift-gated by
> `xtask openapi-check`. Single entry point: [`docs/products/openbb-parity.md`](../products/openbb-parity.md).

**P1 phases landed (verified in-tree):**

- **G001 catalog spine** — `tdw-endpoint-catalog` (`CatalogEntry`, `Op::FetchData`, ordered
  provider candidates with runtime fallback); `provider_resolve.rs` delegates to it.
- **G002 FRED** — macro / rate / spread / fixedincome cluster (cpi/pce/gdp/unemployment,
  sofr/effr/estr/ecb/sonia/iorb, tcm spreads, yield curve, bond + mortgage indices).
- **G003 SEC / Treasury / Fed** — `etf/holdings` (N-PORT), `equity/ownership/form_13f`,
  `equity/shorts/fails_to_deliver`, `regulators/sec/cik_map`, `government/treasury_prices`,
  `government/treasury_auctions`, `regulators/fed/fomc_documents`,
  `fixedincome/government/dealer_stats` (`tdw-provider-government-us` + `-federal-reserve`).
- **G004 keyless wave** — Yahoo (quote/profile/performance/calendars/estimates) + ECB
  `currency/reference_rates` + CBOE `index/snapshots` + EIA `commodity/*` + multi-provider
  `derivatives/options/chains`, `derivatives/futures/{curve,historical}`.
- **G005 REST + OpenAPI** — catalog-derived `GET /api/v1/*`, generated `GET /openapi.json`,
  `xtask openapi-sync`/`openapi-check` drift gate. (L5.1, L5.2 → **done**.)
- **G006 technical analytics** — 20 `technical/*` `Compute` routes + `technical.*` MCP tools
  (`tdw-analytics-technical`).
- **G007 widgets backend** — `tdw-widgets` + `/widgets.json` `/apps.json` `/widget-data`
  (`docs/products/openbb-workspace-backend.md`). (L5.8 → **done**.)
- **G008 copilot bridge** — `tdw-openbb-agent` + `/agents.json` `/v1/query` SSE
  (`docs/products/openbb-workspace-agent.md`). (L5.9 → **done**.)
- **G009 MCP alignment** — `TDW_MCP_ALLOWED_ORIGINS`, read-only widget-catalog tools,
  widget-citation contract test. (L5.10 → **done**.)
- **G016 LLM streaming** — `StreamingLanguageModel` driving the copilot SSE stream.

**Still open after P1 (P2/P3):** keyed providers (FMP fundamentals/screener, Polygon,
Benzinga/Tiingo/Intrinio), Python SDK (L5.3-adjacent), CLI + routines + export
(L5.3/L5.4), charting (L5.5), MCP dynamic exposure (L5.6), full credential-registry
migration (L5.7 — only FRED + EIA wired), quant + econometrics analytics (L4.2/L4.3),
and the examples suite (WS-B4).

---

## P4 roll-up — 2026-06-14 (OpenBB-parity phase 4: data-breadth closeout + scoreboard truth-up)

> **Scoreboard snapshot.** The endpoint catalog now exposes **216 routes / 185
> provider candidates** (`xtask catalog-check` green), up from the P3 close of
> 131/98: **169 `Fetch` routes** + **47 `Compute` routes** (technical 25 /
> quantitative 12 / econometrics 5 / portfolio 5). The generated OpenAPI 3.1
> document carries **169 paths / 56 schemas** (`xtask openapi-check` green); the
> Python SDK + Workspace widgets derive from the same catalog (drift-gated).
> P4 ran 11 waves (PRs #419–#427 + the W10 FINRA wave + this W11 cutover), each
> clean-room + 3-lens reviewed + drift-gated.
>
> **Fetch routes: 84 (P3 close) → 169 (P4 close).** Per-router Fetch before/after:
>
> | Router | P3 close | P4 close | What P4 added |
> |---|---|---|---|
> | equity | 22 | 53 | FMP fundamental breadth (growth / management / compensation / segments / transcript / esg / employee_count / filings), discovery breadth (6 screens + filings + latest_financial_reports), estimates (price_target / forward), ownership (insider / institutional / government_trades), calendar/splits, compare/company_facts, historical_market_cap |
> | economy | 14 | 36 | OECD SDMX (CLI / house_price_index / share_price_index / retail_prices), Fed/BLS surveys (sloos + 4 regional Fed surveys), central_bank_holdings, primary_dealer_*, gdp/forecast, Ken French portfolio-formation (breakpoints + 4 portfolio/index-return tables) |
> | fixedincome | 18 | 35 | Svensson yield curve (2y/5y/10y), HQM corporate spot (2y/5y/30y), rate fill (ameribor / dpcredit / overnight_bank_funding + EFFR forecast), tcm_effr spreads |
> | etf | 3 | 9 | search / info / sectors / countries / equity_exposure / nport_disclosure (FMP + SEC N-PORT) |
> | derivatives | 3 | 5 | futures/instruments + futures/info |
> | regulators | 2 | 10 | SEC utils (symbol_map / filing_headers / institutions_search / sic_search / schema_files / rss_litigation), CFTC cot + cot_search |
> | index | 4 | 6 | **W11: constituents (FMP), sp500_multiples (NASDAQ MULTPL / Shiller CAPE)** |
> | currency | 3 | 4 | **W11: snapshots (FMP /fx forex snapshot)** |
> | crypto / commodity / news / imf_utils | 2 / 3 / 2 / 4 | 2 / 3 / 2 / 4 | (imf_utils SDMX discovery landed P4W9) |
>
> **W11 (this wave) — index/currency remainder.** Three routes added (the W6
> deferrals built migration-and-model-first): `index/constituents` (FMP
> `/{index}_constituent`, sp500/nasdaq/dowjones → `IndexConstituent`),
> `index/sp500_multiples` (NASDAQ Data Link `MULTPL` Shiller-CAPE family →
> `Sp500Multiple`), `currency/snapshots` (FMP `/fx` forex snapshot →
> `CurrencySnapshot`). New bronze tables `raw.index_constituent` /
> `raw.sp500_multiple` / `raw.currency_snapshot` (migration `20260528_0029`).
> Gemini #427 MEDIUM folded (famafrench `value - -99.99` double-negative → named
> `KEN_FRENCH_MISSING_*` sentinel consts).
>
> **W11 deferred (one route, with reason):**
> - `index/sectors` (TMX) — **DEFERRED.** TMX Money's public REST API (the one
>   `tdw-provider-tmx` is specced against) publishes equity quotes, not an
>   index-sector-weight endpoint; no vendor-published TMX index-sector API exists
>   in the clean-room source set, so building it would mean inventing an endpoint
>   shape. The standardized sector-weight surface is already served for funds via
>   `etf/sectors` (FMP/SEC N-PORT → `EtfSectorWeight`). Revisit if TMX publishes
>   an official index-sector API.
> - `index/constituents` extra providers (CBOE/TMX) — the route ships with FMP as
>   the single verifiable keyed candidate (the model+one-provider pattern, like
>   `currency/search`); CBOE/TMX constituent feeds are not specced from a
>   vendor-published API in the clean-room set.
>
> **Net parity read (P4 close).** Full parity on the **keyless / free-key**
> surface (Yahoo, FRED, SEC, US-Treasury, Federal-Reserve, ECB, CBOE, EIA, IMF,
> EconDB, OECD, NASDAQ-calendar, FINRA, CFTC, Ken-French, plus FMP on a free key).
> The remaining gap is **paid-key or no-public-API** providers — a business
> decision, not an engineering gap. The standing deferred list (each with a
> documented decision, none blocking) is: **paid-key** intrinio (options
> unusual/snapshots/surface, reported_financials) and the premium tiers of
> benzinga/tiingo; **no-public-API** stockgrid (short_volume), wsj (etf
> discovery), finviz (screener/groups), biztoc (news world — keyed RapidAPI
> proxy), and TMX index-sectors; **new-keyed-crate-not-built** uscongress /
> congress.gov; **USDA-FAS-separate-provider** commodity/psd_* (PSD); and
> **Parquet** export (heavy-dep, vs the zero-heavy-dep posture). See the per-row
> Status column and the deferred tables (D1–D8) for the full accounting.

---

## Part 1 — Gap Matrix (command-cluster level)

Status legend: **HAVE** (shippable today), **PARTIAL** (some surface, gaps noted),
**MISSING** (no FinX surface). "FinX crate" = where it lives / should live.

### 1. equity (79 cmds)

| OpenBB cluster | FinX status | FinX crate / what's missing | Status |
|---|---|---|---|
| price/historical (OHLCV) | HAVE | yahoo, polygon, alpaca, tiingo, fmp, cboe, tmx, alpha-vantage, databento | done |
| price/quote + price/performance | HAVE | quote + performance standardized via yahoo (keyless, L2.4); also quote via tradier/cboe/fmp | done |
| search / profile / market_snapshots / historical_market_cap | HAVE (market_snapshots deferred) | `equity/search` (FMP) + `equity/profile` (Yahoo+FMP) + `equity/historical_market_cap` (FMP, P4W2) all standardized; `market_snapshots` needs polygon snapshot — DEFERRED (paid polygon tier) | done (market_snapshots deferred) |
| screener | HAVE | `equity/screener` FMP-backed (`ScreenerRow`, P2W2); finviz screener DEFERRED (no public API, D2) | done |
| fundamentals (balance/income/cash/ratios/metrics) | HAVE | fmp income/balance/cash → FinancialStatement, ratios → Ratios, metrics → KeyMetrics → `equity/fundamental/{income,balance,cash,ratios,metrics}` (G011); intrinio/polygon variants DEFERRED (paid) | done |
| fundamentals growth (balance/income/cash growth) | HAVE | `equity/fundamental/{balance_growth,income_growth,cash_growth}` FMP-backed (P4W1) | done |
| fundamentals extras (dividends, splits, eps history, employees, esg, mgmt, transcript, segments) | HAVE | `equity/fundamental/{dividends,splits,historical_eps,employee_count,esg_score,management,management_compensation,transcript,revenue_per_segment,revenue_per_geography,filings}` FMP-backed (P4W1/P4W2) | done |
| estimates (price_target, consensus, forward_*) | HAVE | **`equity/estimates/{consensus,historical_eps,price_target,forward}`** (consensus Yahoo G004, the rest FMP-keyed — P3W5); benzinga/seeking-alpha premium variants DEFERRED (paid) | done |
| calendar (dividend/earnings/ipo/splits/events) | HAVE (events deferred) | `equity/calendar/{dividends,earnings,ipo}` (NASDAQ keyless, G004p2) + `equity/calendar/splits` (FMP, P4W2); benzinga `events` DEFERRED (premium) | done (events deferred) |
| compare (peers/groups/company_facts) | HAVE (groups deferred) | `equity/compare/peers` (FMP, P2W2) + `equity/compare/company_facts` (SEC XBRL, P4W2); finviz `groups` DEFERRED (no public API, D2) | done (groups deferred) |
| discovery (active/gainers/losers/...) | HAVE | `equity/discovery/{active,gainers,losers}` + `equity/discovery/{aggressive_small_caps,growth_tech,undervalued_growth,undervalued_large_caps,filings,latest_financial_reports}` FMP/SEC-backed (P4W2) | done |
| ownership (insider/institutional/13f/gov_trades/share_stats) | HAVE | `equity/ownership/{form_13f,share_statistics}` (SEC keyless, G003) + `equity/ownership/{insider_trading,institutional,government_trades}` (FMP, P4W2) | done |
| shorts (short_interest/short_volume/fails_to_deliver) | HAVE (short_volume deferred) | `equity/shorts/short_interest` (FINRA, P4W10) + `equity/shorts/fails_to_deliver` (SEC FTD, G003); stockgrid `short_volume` DEFERRED (no vendor-published API, D2) | done (short_volume deferred) |
| darkpool/otc | HAVE | finra OTC weekly | done |

### 2. economy (46 cmds)

| OpenBB cluster | FinX status | FinX crate / what's missing | Status |
|---|---|---|---|
| fred_series / fred_search / fred_release / fred_regional | HAVE (release/regional deferred) | `economy/fred_search` + FRED series obs HAVE; release_table/regional DEFERRED (niche FRED metadata endpoints, low value) | done (release/regional deferred) |
| cpi / pce / gdp / unemployment / interest_rates | HAVE | fred-backed `economy/{cpi,pce,gdp/{real,nominal,forecast},unemployment,interest_rates}` | done |
| calendar (economic events) | DEFERRED | trading-economics standardized calendar needs a paid TE tier (L3.13); no free public economic-calendar API in the clean-room set | deferred (paid) |
| indicators / available_indicators / country_profile | HAVE (country_profile deferred) | `economy/econdb/series` (EconDB, P3W4) + `economy/imf/*` (IMF SDMX, P3W3) + `economy/composite_leading_indicator` + OECD price indices (P4W4); econdb country_profile/export_destinations DEFERRED (niche) | done (country_profile deferred) |
| money_measures / central_bank_holdings / primary_dealer_* / fomc_documents | HAVE | `economy/money_measures/{m1,m2}` (FRED) + `economy/central_bank_holdings` + `economy/primary_dealer_{fails,positioning}` + `economy/fomc_documents` (Federal-Reserve, P4W4) | done |
| balance_of_payments / direction_of_trade / shipping/* | HAVE (shipping deferred) | `economy/imf/{balance_of_payments,direction_of_trade,international_financial_statistics}` (IMF SDMX, P3W3); shipping/* DEFERRED (niche IMF dataflow) | done (shipping deferred) |
| survey/* (nonfarm, sloos, sentiment, regional Fed surveys) | HAVE | `economy/survey/{nonfarm_payrolls,university_of_michigan,inflation_expectations,sloos,economic_conditions_chicago,manufacturing_outlook_ny,manufacturing_outlook_texas}` (FRED/Fed, P4W4) | done |
| survey/bls_search + bls_series | HAVE (search deferred) | tdw-provider-bls series HAVE; bls_search DEFERRED (BLS has no keyless search API; series-by-id is the supported path) | done (search deferred) |

### 3. fixedincome (30 cmds)

| OpenBB cluster | FinX status | FinX crate / what's missing | Status |
|---|---|---|---|
| government/yield_curve + treasury_rates | HAVE | fred-backed end-to-end: government/yield_curve (3m/2y/10y/30y aggregate) + government/treasury_rates/{3m,2y,10y,30y} + government/tips_yields/10y | done |
| government/treasury_prices/auctions/tips/svensson | HAVE | `government/{treasury_prices,treasury_auctions}` (US-Treasury keyless, G003), `tips_yields/10y` (FRED), `svensson_yield_curve/{2y,5y,10y}` (FRED, P4W5) | done |
| rate/* (sofr, effr, estr, ecb, sonia, ameribor, iorb, ...) | HAVE | fred-backed `rate/{sofr,effr,estr,sonia,ecb,iorb,dpcredit,overnight_bank_funding,ameribor}` + `rate/effr_forecast` (P4W5) | done |
| spreads/* (tcm, tcm_effr, treasury_effr) | HAVE | fred-backed `spreads/tcm/{10y2y,10y3m}`, `spreads/tcm_effr/{1y,10y}`, `spreads/treasury_effr/3m` (P4W5) | done |
| corporate/* (bond_prices, commercial_paper, spot/hqm) | HAVE (bond_prices deferred) | `corporate/spot_rates/10y` + `corporate/hqm/{2y,5y,30y}` + `corporate/commercial_paper/90d` (FRED, P4W5); bond_prices needs a paid TMX/FINRA TRACE feed — DEFERRED | done (bond_prices deferred) |
| bond_indices / mortgage_indices | HAVE | fred-backed end-to-end: bond_indices/us_corporate_hy, mortgage_indices/30y_fixed | done |

### 4. derivatives (11 cmds)

| OpenBB cluster | FinX status | FinX crate / what's missing | Status |
|---|---|---|---|
| options/chains | HAVE | cboe, deribit | done |
| options/unusual / snapshots / surface | DEFERRED | needs intrinio (paid key) + an IV-surface compute layer; cannot be free/live-verified (D3) | deferred (paid) |
| futures/historical / curve / instruments / info | HAVE | `derivatives/futures/{historical,curve}` (G004) + `derivatives/futures/{instruments,info}` (P4W7) | done |

### 5. etf (14 cmds)

| OpenBB cluster | FinX status | FinX crate / what's missing | Status |
|---|---|---|---|
| search / info / historical | HAVE | `etf/search` + `etf/info` (FMP, P4W3) + `etf/historical` (reuses price) | done |
| holdings / sectors / countries / equity_exposure / nport | HAVE | `etf/holdings` + `etf/nport_disclosure` (SEC N-PORT keyless, G003) + `etf/{sectors,countries,equity_exposure}` (FMP, P4W3) | done |
| price_performance / discovery (active/gainers/losers) | HAVE | `equity/price/performance` (Yahoo) + `equity/discovery/{active,gainers,losers}` (FMP); wsj etf-discovery DEFERRED (P3W7 — no vendor-published API, see D2) | done (wsj variant deferred) |

### 6. index (9 cmds)

| OpenBB cluster | FinX status | FinX crate / what's missing | Status |
|---|---|---|---|
| price/historical | HAVE | catalog route `index/price/historical` landed (G004); cboe/polygon bars; polygon aggregates `I:` ticker-prefix candidate added (G011) | done |
| constituents / available / search / snapshots / sectors | HAVE (sectors deferred) | `index/snapshots` + `index/available` + `index/search` (CBOE, G004/P4W6) + `index/constituents` (FMP, P4W11); `index/sectors` (TMX) DEFERRED — no vendor-published TMX index-sector API in the clean-room set (fund sector-weights already served by `etf/sectors`) | done (sectors deferred) |
| sp500_multiples (Shiller PE) | HAVE | `index/sp500_multiples` (NASDAQ Data Link `MULTPL` → `Sp500Multiple`, Shiller CAPE family, P4W11) | done |

### 7. crypto (4 cmds)

| OpenBB cluster | FinX status | FinX crate / what's missing | Status |
|---|---|---|---|
| price/historical | HAVE | coingecko, ccdata, binance, geckoterminal, polygon (`X:` ticker prefix, G011) | done |
| search | HAVE | `crypto/search` standardized (FMP symbol search spans crypto pairs) | done |

### 8. currency (6 cmds)

| OpenBB cluster | FinX status | FinX crate / what's missing | Status |
|---|---|---|---|
| price/historical / snapshots / search | HAVE | `currency/price/historical` (polygon `C:` prefix, G011), `currency/search` (FMP, P4W6), `currency/snapshots` (FMP `/fx` forex snapshot → `CurrencySnapshot`, P4W11) | done |
| reference_rates (ECB) | HAVE | `currency/reference_rates` (ECB SDW, keyless, G004) | done |

### 9. commodity (8 cmds)

| OpenBB cluster | FinX status | FinX crate / what's missing | Status |
|---|---|---|---|
| price/spot | HAVE | `commodity/price/spot` standardized (EIA/FRED-backed) | done |
| petroleum_status / energy_outlook / psd_* (EIA) | HAVE (psd deferred) | `commodity/petroleum_status_report` + `commodity/short_term_energy_outlook` (EIA report endpoints, G004); `psd_*` DEFERRED — the Production-Supply-Distribution data is a separate USDA-FAS provider, not EIA | done (psd deferred) |

### 10. news (3 cmds)

| OpenBB cluster | FinX status | FinX crate / what's missing | Status |
|---|---|---|---|
| company | HAVE | `news/company` standardized via benzinga → `NewsArticle` (P2W7) | done |
| world | HAVE (biztoc deferred) | `news/world` standardized via benzinga → `NewsArticle` (P2W7); biztoc variant DEFERRED (keyed RapidAPI proxy, no free public API, D2) | done (biztoc deferred) |

### 11. regulators (13 cmds)

| OpenBB cluster | FinX status | FinX crate / what's missing | Status |
|---|---|---|---|
| sec/* (cik_map, symbol_map, filings utils, litigation) | HAVE | `regulators/sec/{cik_map,symbol_map,filing_headers,institutions_search,sic_search,schema_files,rss_litigation}` (SEC keyless, G003/P4W8) + `regulators/fed/fomc_documents` | done |
| cftc/cot + cot_search | HAVE | `regulators/cftc/{cot,cot_search}` (CFTC Socrata Commitments of Traders → `CommitmentOfTraders`, P2W5) | done |

### 12–13. analysis routers (technical 28 / quantitative 23 / econometrics 16 / famafrench 7)

| OpenBB cluster | FinX status | FinX crate / what's missing | Status |
|---|---|---|---|
| technical/* (sma, ema, rsi, macd, bbands, atr, ...) | NATIVE | tdw-analytics-technical: ~18 core indicators as `technical/*` Compute routes + `technical.*` MCP tools (L4.1 done; long-tail Ichimoku/cones/Clenow/Demark/fib/RRG deferred) | done |
| quantitative/* (sharpe, sortino, capm, rolling stats) | NATIVE | tdw-analytics-quant: 12 returns-based metrics (sharpe/sortino/omega, max_drawdown/calmar, volatility, skewness/kurtosis, value_at_risk/expected_shortfall, capm, jarque_bera) as `quantitative/*` Compute routes + `quantitative.*` MCP tools (L4.2 done; rolling-window variants + standalone unit-root deferred) | done |
| econometrics/* (ols, panel, cointegration, causality) | NATIVE | tdw-analytics-econometrics: 5 estimators (ols+summary, correlation_matrix, vif, granger_causality, cointegration) as `econometrics/*` Compute routes + `econometrics.*` MCP tools (L4.3 done; panel models + autocorrelation series + formal unit-root deferred; Granger/cointegration p-values documented as honest simplifications) | done |
| famafrench/* (factors, portfolio returns) | HAVE | `economy/factors/famafrench` (research factors, P2W6) + `economy/factors/famafrench/{breakpoints,us_portfolio_returns,regional_portfolio_returns,country_portfolio_returns,international_index_returns}` (Ken French Data Library, pure-Rust zip, P4W9) | done |

### Roll-up (post-P4, command-cluster level)

> Status legend at cluster level: **HAVE** = the cluster's standardized surface
> ships (possibly with one sub-command deferred-by-decision); **DEFERRED** = the
> whole cluster is deferred for a documented reason (paid key / no public API).
> After P4 every cluster is **HAVE** except `derivatives/options-unusual` (paid
> intrinio) and `economy/calendar` (paid trading-economics) — see the per-row
> Status column above for the exact route and the deferred sub-command in each.

| Domain | OpenBB clusters | HAVE | DEFERRED (whole cluster) | Deferred sub-commands (reason) |
|---|---|---|---|---|
| equity | 13 | 13 | 0 | market_snapshots (paid polygon), finviz groups (no API), short_volume (no API), benzinga events (paid) |
| economy | 8 | 7 | 1 (calendar — paid TE) | release/regional (niche), shipping (niche), bls_search (no API), country_profile (niche) |
| fixedincome | 6 | 6 | 0 | corporate/bond_prices (paid TRACE/TMX) |
| derivatives | 3 | 2 | 1 (options unusual/snapshots/surface — paid intrinio) | — |
| etf | 3 | 3 | 0 | — |
| index | 3 | 3 | 0 | index/sectors (no public TMX index-sector API) |
| crypto | 2 | 2 | 0 | — |
| currency | 2 | 2 | 0 | — |
| commodity | 2 | 2 | 0 | psd_* (separate USDA-FAS provider) |
| news | 2 | 2 | 0 | biztoc world (keyed proxy) |
| regulators | 2 | 2 | 0 | — |
| analysis (4 routers) | 4 | 4 | 0 | long-tail indicators / panel models (documented, low value) |
| **Total clusters** | **50** | **48** | **2** | — |

**Honest read (post-P4):** the standardized fundamentals / estimates / macro /
fixedincome / analytics surface — OpenBB's bulk — is now **delivered**. The
catalog exposes **169 Fetch routes + 47 Compute routes** across all 17 routers,
all derived from one typed source (REST + OpenAPI + Python SDK + Workspace widgets
+ MCP tools + warehouse ingest stay in lockstep, drift-gated). Full parity holds
on the **keyless / free-key** surface; the only standing gaps are **paid-key**
(intrinio options, trading-economics calendar, benzinga/tiingo premium) or
**no-public-API** (stockgrid / wsj / finviz / biztoc / TMX index-sectors)
providers — a business decision, not an engineering gap.

---

## Part 2 — Implementation Layers (dependency-ordered, leaves first)

> **⚠️ Historical planning section — superseded by the P1–P4 roll-ups above.**
> The L1–L5 rows below are the *original* dependency-ordered work-breakdown
> authored before P1. They are retained as a planning record; their per-row
> `todo`/`done` Status column is **stale** and is **not** the authoritative
> coverage scoreboard. Most are now **done**: the L1 core abstractions
> (result envelope, standard `tdw-domain` models, `provider=` resolution),
> the L2 provider expansions (FMP fundamentals/discovery/estimates/ownership,
> FRED/Fed/OECD/BLS macro, yahoo discovery+ETF, cboe/nasdaq/eia/ecb breadth),
> the L3 new provider crates (cftc, famafrench, imf, econdb), and the L4/L5
> analytics + platform surfaces (technical/quantitative/econometrics Compute
> routes, REST/OpenAPI/MCP/CLI/Python-SDK/Workspace) all shipped across
> P1–P4. For the **true, current coverage** read the **Part 1 gap matrix**
> and the **P1/P2/P3/P4 roll-ups**, which are kept in sync with the catalog
> (`xtask catalog-check` / `openapi-check`). Genuinely-unbuilt rows here are
> only those matching the deferred-with-reason set (paid keys:
> intrinio/benzinga-premium/tiingo; no public API: stockgrid/finviz/wsj/biztoc;
> uscongress new keyed crate; USDA-FAS PSD).

Sizing: **S** ≈ ≤1 day, **M** ≈ 2–4 days, **L** ≈ 1–2 weeks. Each row is one `/batch` task.

### L1 — Core abstractions (must land first; everything else depends on these)

| # | Task (crate · scope · gates · done-when) | Size | Status |
|---|---|---|---|
| L1.1 | **tdw-domain** · add standardized result envelope (`id/results/provider/warnings/extra{route,timestamp,arguments}`) mirroring OBBject field shape; serde + `to_records`. Gates: fmt+clippy+unit. Done-when: every Fetcher result wraps in envelope, snapshot test passes. | M | todo |
| L1.2 | **tdw-symbology** (PR #173) · land symbol normalization (ticker↔CIK↔figi, exchange suffixes, FX pair `CUR1-CUR2`, crypto pairs, index symbols). Gates: fmt+clippy+unit. Done-when: #173 merged; providers resolve via it. | M | in-progress |
| L1.3 | **tdw-core** · standard QueryParams normalization (start_date/end_date/interval/period/limit defaults + validation) shared across Fetchers (TET "transform_query" stage). Gates: unit. Done-when: providers consume shared params struct, no per-provider date parsing. | M | todo |
| L1.4 | **tdw-domain** · standard data models for the big clusters: `FinancialStatement` (balance/income/cash), `KeyMetrics`/`Ratios`, `Estimate`, `MacroSeries`, `RateObservation`, `OptionContract`, `NewsArticle`, `OwnershipRecord`. Field names per surface-domains tables. Gates: unit. Done-when: models compile + serde round-trip tests. | L | todo |
| L1.5 | **tdw-service-api** · `provider=` resolution layer: one logical endpoint → many providers, pick by arg/availability. Gates: integ. Done-when: `equity/price/historical` dispatches across ≥3 providers by `provider=`. | M | todo |

### L2 — Provider endpoint EXPANSION (cheapest wins; existing crates gain endpoints)

Clean-room: add endpoints from each vendor's *own* public API docs, normalize to L1.4 models.

| # | Task | Size | Status |
|---|---|---|---|
| L2.1 | **tdw-provider-fmp** · expand to fundamentals cluster (balance/income/cash + *_growth, metrics, ratios, dividends, splits, eps history, peers, profile). Highest leverage: fmp alone serves ~25 OpenBB cmds. Gates: fmt+clippy+live-gated. Done-when: 4 statements + ratios + peers normalized to L1.4. | L | done (G011: fmp statement/ratios/metrics fetchers projected into catalog routes equity/fundamental/{income,balance,cash,ratios,metrics}; profile added as 2nd candidate on equity/profile; dividends/splits/eps/peers fetchers exist at provider level) |
| L2.2 | **tdw-provider-fmp** · discovery + calendar (active/gainers/losers, dividend/earnings/ipo/splits calendars, price/performance, screener). Gates: live-gated. Done-when: 8 endpoints return standardized lists. | M | **done** (P2W2 + P4W2: `equity/discovery/{active,gainers,losers}` + 6 screen variants, `equity/calendar/{dividends,earnings,ipo,splits}`, `equity/price/performance`, `equity/screener` via `ScreenerRow`) |
| L2.3 | **tdw-provider-fred** · macro + rate + spread + fixedincome cluster (cpi/pce/gdp/unemployment via series IDs, sofr/effr/estr/ecb/sonia, tcm spreads, yield_curve, bond/mortgage indices, fred_search/release/regional). fred serves ~40 OpenBB cmds. Gates: live-gated. Done-when: ≥15 fred-backed endpoints standardized. | L | todo |
| L2.4 | **tdw-provider-yahoo** · profile, quote, discovery, dividends, share_statistics, consensus, futures/historical+curve, options/chains. yahoo (no key) serves ~15 cmds. Gates: live-gated. Done-when: ≥8 endpoints standardized. | M | todo |
| L2.5 | **tdw-provider-polygon** · fundamentals (balance/income/cash), market_snapshots, FX + crypto historical, index historical. Gates: live-gated. Done-when: ≥5 endpoints. | M | partial (G011: the single polygon `aggregates` fetcher reused as a candidate on currency/price/historical [`C:` prefix], crypto/price/historical [`X:`], and index/price/historical [`I:`] — caller supplies the prefixed ticker; polygon fundamentals + market_snapshots still todo) |
| L2.6 | **tdw-provider-sec** · cik_map/symbol_map, filings index, form_13f, company_facts, fails_to_deliver, N-PORT, MD&A, latest_financial_reports. Gates: unit (public). Done-when: cik/symbol map + 13f + FTD standardized. | M | done |
| L2.7 | **tdw-provider-cboe** · index (price/constituents/available/search/snapshots), options/chains normalize, futures/curve. Gates: live-gated. Done-when: index cluster standardized. | M | **done** (G004/P4W6: `index/{snapshots,available,search}` CBOE-backed + `derivatives/options/chains` normalized; `index/price/historical` via polygon/databento; `index/constituents` ships via FMP — CBOE constituent feed not specced from a vendor-published API) |
| L2.8 | **tdw-provider-eia** · petroleum_status_report, short_term_energy_outlook, psd_data/psd_report. Gates: live-gated. Done-when: 3 report endpoints. | S | todo |
| L2.9 | **tdw-provider-ecb** · currency/reference_rates + rate/ecb + balance_of_payments shape. Gates: live-gated. Done-when: reference_rates standardized. | S | todo |
| L2.10 | **tdw-provider-nasdaq** · calendars (dividend/earnings/ipo), top_retail, sp500_multiples (Shiller PE). Gates: live-gated. Done-when: 4 endpoints. | S | **done (top_retail deferred)** (G004p2: `equity/calendar/{dividends,earnings,ipo}` keyless; P4W11: `index/sp500_multiples` via Data Link `MULTPL` → `Sp500Multiple`. `top_retail` DEFERRED — niche, no standardized OpenBB model demand) |
| L2.11 | **tdw-provider-finra** + **stockgrid (new, see L3)** · shorts cluster (short_interest HAVE; add short_volume, FTD via sec). Gates: live-gated. Done-when: shorts cluster complete. | S | mostly done (short_interest + SEC FTD HAVE; stockgrid `short_volume` deferred P3W7 — see D2/L3.8) |
| L2.12 | **tdw-provider-{benzinga,tiingo,seeking-alpha}** · standardized news/company + news/world + estimates (price_target/consensus/forward). Gates: live-gated. Done-when: news + estimates clusters normalized. | M | todo |
| L2.13 | **tdw-provider-deribit** · futures/instruments+info+curve, options/chains normalize. Gates: live-gated. Done-when: derivatives cluster for deribit standardized. | S | todo |
| L2.14 | **tdw-provider-tradier** · price/quote, options/chains. Gates: live-gated. Done-when: quote+chains standardized. | S | todo |

### L3 — NEW provider crates (sources FinX lacks entirely)

Clean-room: scaffold `crates/tdw-provider-<name>` against `tdw_core::Fetcher`; spec from
the vendor's public API docs.

| # | New crate · source · auth model | Serves | Size | Status |
|---|---|---|---|---|
| L3.1 | **tdw-provider-federal-reserve** · U.S. Fed data portal · no key | money_measures, central_bank_holdings, primary_dealer_*, fomc_documents, treasury_rates, overnight rates | M | done |
| L3.2 | **tdw-provider-government-us** · US Treasury Fiscal/Direct · no key | treasury_prices, treasury_auctions, treasury yield data | S | done |
| L3.3 | **tdw-provider-imf** · IMF SDMX · no key | indicators, direction_of_trade, balance_of_payments, shipping/*, imf_utils dataflow discovery | M | done (P3W3, #367; `economy/imf/{international_financial_statistics,direction_of_trade,balance_of_payments}`. shipping/imf_utils discovery deferred) |
| L3.4 | **tdw-provider-econdb** · EconDB · optional key | gdp/real+nominal, indicators, country_profile, export_destinations | M | done (P3W4; `economy/econdb/series` — series-by-ticker → MacroSeries, optional token. country_profile/export_destinations deferred) |
| L3.5 | **tdw-provider-intrinio** · Intrinio · paid key | options/unusual+snapshots+surface, reported_financials, forward_pe, data-tag attributes, ipo calendar | L | deferred (P3W7 — paid key, not free/live-verifiable; see D3) |
| L3.6 | **tdw-provider-finviz** · Finviz · no key | screener, compare/groups, price/performance, metrics, price_target | M | deferred (P3W7 — no official API; HTML-scrape only, see D2. price/performance already via Yahoo) |
| L3.7 | **tdw-provider-cftc** · CFTC (Socrata) · app token | regulators/cftc/cot + cot_search | S | todo |
| L3.8 | **tdw-provider-stockgrid** · Stockgrid · no key | shorts/short_volume | S | deferred (P3W7 — no vendor-published API; site-backing JSON only, see D2) |
| L3.9 | **tdw-provider-wsj** · WSJ market data · no key | etf/discovery (active/gainers/losers) | S | todo |
| L3.10 | **tdw-provider-biztoc** · Biztoc · free key | news/world | S | todo |
| L3.11 | **tdw-provider-famafrench** · Ken French Data Library · no key (academic) | famafrench/* factors + portfolio returns | M | **done** (P2W6 factors + P4W9 portfolio-formation: `economy/factors/famafrench` + `/{breakpoints,us_portfolio_returns,regional_portfolio_returns,country_portfolio_returns,international_index_returns}`; pure-Rust zip, no C) |
| L3.12 | **tdw-provider-congress-gov** · congress.gov · key | uscongress/* (bills, bill_info, gov_trades context) | S | todo |
| L3.13 | **tdw-provider-tradingeconomics** (exists as trading-economics) · expand · paid | economy/calendar standardized | S | todo |

### L4 — Analytics crates (no current FinX crate; UDF workaround today)

Clean-room: implement indicator/stat math from public formula definitions (textbook /
vendor docs), **not** OpenBB code. Operate on L1.1 envelopes / record sets.

| # | New crate · scope · gates · done-when | Size | Status |
|---|---|---|---|
| L4.1 | **tdw-analytics-technical** · ~18 core indicators delivered (sma/ema/wma/hma, macd, rsi, stoch, cci, adx[+di/-di], aroon, bbands, kc, donchian, atr, obv, ad, vwap, fisher, roc, momentum) as `technical/*` Compute catalog routes + `technical.*` MCP/ToolCall tools, wired in tdw-service-api (inline-data OR nested source fetch). Gates: fmt+clippy(pedantic/nursery zero)+unit (golden vectors). Done-when: core set with numeric tests; callable as daemon op + MCP tool. Long-tail (zlma, adosc, cg, cones, clenow, demark, ichimoku, fib, relative_rotation) deferred. | L | done (core) |
| L4.2 | **tdw-analytics-quant** · G015 delivered 12 returns-based metrics (sharpe/sortino/omega, max_drawdown+calmar, volatility, skewness, excess kurtosis, value_at_risk, expected_shortfall, capm alpha+beta, jarque_bera with a χ²₂ p-value) as `quantitative/*` Compute catalog routes + `quantitative.*` MCP/ToolCall tools, wired in tdw-service-api (inline-data returns OR nested source price fetch reduced to returns). Gates: fmt+clippy(pedantic/nursery zero)+unit (golden vectors hand-derived). Done-when: core metric set with numeric tests; callable as daemon op + MCP tool. Deferred: rolling-window stat variants, a standalone normality `summary`, a formal unit-root route. | M | done (core) |
| L4.3 | **tdw-analytics-econometrics** · G015 delivered 5 estimators (ols with std errors/t-stats/R²/adj-R²/F/Durbin-Watson, correlation_matrix, vif, granger_causality F-test, Engle-Granger cointegration step one + residual stationarity score) as `econometrics/*` Compute catalog routes + `econometrics.*` MCP/ToolCall tools. Hand-rolled OLS via the normal equations factored by a dependency-free Cholesky solve (NO nalgebra/faer/ndarray). Gates: fmt+clippy(pedantic/nursery zero)+unit (golden vs hand-derived worked examples; no Python statsmodels in this clean-room). Done-when: core estimator set with numeric tests. Honest simplifications documented: Granger reports F (no p-value); cointegration reports a Dickey-Fuller ρ score (no MacKinnon p-table). Deferred: panel models, autocorrelation series, formal unit-root route. | L | done (core) |
| L4.4 | **tdw-analytics-portfolio** (beyond OpenBB parity) · P3W1 delivered 5 pure-compute metrics (cumulative_returns, drawdown, max_drawdown with peak/trough indices, allocation normalize-to-weights, per-asset contribution weight·return) as `portfolio/*` Compute catalog routes + `portfolio.*` MCP/ToolCall tools, wired in tdw-service-api (inline params-only, like econometrics/*; reuses tdw-analytics-quant's prices_to_returns). Gates: fmt+clippy(pedantic/nursery zero)+unit (golden vectors hand-compounded). Done-when: core portfolio metrics with numeric tests; callable as daemon op + MCP tool. Deferred: full performance attribution, rolling-window drawdown, risk-parity weights. | M | done (core) |
| L4.5 | **tdw-service-api** · wire L4.1–L4.3 as computation ops (`technical/*`, `quantitative/*`, `econometrics/*`) taking caller data (no provider). Gates: integ. Done-when: ops resolve through daemon. | M | todo |

### L5 — Platform surfaces (parity for how OpenBB is *used*)

| # | Task · scope · gates · done-when | Size | Status |
|---|---|---|---|
| L5.1 | **tdw-service-api** (PR #161) · land HTTP+SSE (`POST /op`, `GET /events/{id}`, `/health`, `/metrics`) over the daemon. Gates: integ. Done-when: #161 merged, REST command callable — **done** (`POST /op` + `GET /events/{id}` + `/health` + `/metrics` served by `tdw-app-server/src/transport_http.rs`; the catalog REST family in `rest_route.rs` builds on it). | M | done |
| L5.2 | **tdw-app-server** + **tdw-service-api** (WS2, G005) · catalog-derived REST route family (`GET /api/v1/{route...}?provider=`) dispatching through the policy-guarded `Op::FetchData` path + the `ResultEnvelope`; generated OpenAPI 3.1 at `GET /openapi.json` (xtask `openapi-sync`/`openapi-check` drift gate). Gates: integ. Done-when: ≥10 routes documented — **done** (60 paths / 21 schemas in `docs/schemas/openapi.json`, drift-gated by `openapi-check`; e2e tests in `tdw-service-api/tests/rest_route_e2e.rs`). | L | done |
| L5.3 | **tdw-cli** (WS4, G013) · command-tree parity (menu/command mirror of routers; `--provider`; `-h`). Gates: smoke. Done-when: `tdw equity price historical --symbol AAPL` works — **done** (clap command tree built at runtime from `tdw-endpoint-catalog`: every catalog route — Fetch + Compute — becomes a nested subcommand path with schema-derived `--flag`s; submits `Op::FetchData` over the existing TCP `OpEnvelope` path; aligned plain-text table / `--json` / `--export csv|json`; `tdw routes` lists all 80; event-spine `routine record|run|list` over `.tdw/routines/<name>.jsonl`; legacy `run-query` / `--smoke` unchanged; quickstart in `docs/products/cli.md`). | M | done |
| L5.4 | **tdw-table-format** + **tdw-storage-parquet** · export polish (CSV/XLSX/JSON/Parquet from any envelope; `export_directory`-style config). Gates: unit. Done-when: 4 export formats from one result. CSV + JSON shipped in G013 (`tdw-cli`'s `--export csv\|json`, hand-rolled RFC-4180 escaping; no new export dep). **XLSX done (P3W6, `rust_xlsxwriter` pure-Rust)** — `tdw-cli --export xlsx` writes a single-sheet (`data`) workbook from any result envelope, reusing the CSV first-seen key-union column model with typed cells (number/bool/string, blank for null/missing, compact-JSON for nested); MIT, `default-features = false` keeps the tree C-free (`zip`→`flate2`/`miniz_oxide`), `cargo deny` green. **Parquet DEFERRED**: arrow-rs `parquet` is pure-Rust but a ~40-crate heavy tree contradicting the platform's zero-heavy-dep posture; the light alternatives (arrow2/parquet2) are archived/unmaintained; revisit if a light maintained pure-Rust Parquet writer emerges or the dep weight is explicitly accepted. `export_directory` config also remains todo. | S | in-progress |
| L5.5 | **new tdw-charting** (WS5, G014) · pure-Rust server-side chart spec as a **Plotly figure** (`{ "data": [...traces], "layout": {...} }`, plain serde_json, ZERO new native dep; plotly.js renders client-side) on the `ResultEnvelope.chart` slot (`#[serde(skip_serializing_if)]` so chart-less payloads stay byte-identical); `chart=true` flows through the dispatch fetch/compute paths and the REST query string. Builders: `candlestick` (+ volume subplot), `line`, `indicator_overlay`; deterministic key order + golden snapshots. Shape detection reuses the tolerant OHLCV parser (`technical_compute::parse_bars`): candlestick for OHLCV rows, line for date+value rows, else a `chart_unsupported` warning. Gates: unit (spec snapshot). Done-when: candlestick spec emitted for price/historical — **done** (golden snapshots in `crates/tdw-charting`; envelope round-trip in `tdw-domain`; dispatch shape-detection unit tests + REST `chart=true` e2e in `tdw-service-api/tests/rest_route_e2e.rs`). | L | in-progress |
| L5.6 | **tdw-mcp** · dynamic tool discovery + per-route exposure config (mcp_config: expose/methods/exclude_args) over the new REST command tree. Gates: integ. Done-when: agent browses categories, activates subset. | M | todo |
| L5.7 | **tdw-config** (WS2, G005) · per-provider credential registry (`provider → env var + optional config-file key`) with a lookup/`resolve_credential` fn, mirroring the `user_settings.json` `credentials` section. FRED + EIA wired; remaining provider crates still read their own env vars (full migration is follow-up). Preferences (`output_type`, dirs) remain todo. Gates: unit. Done-when: all L2/L3 providers resolve keys via one path. | S | in-progress |
| L5.8 | **new tdw-widgets** + **tdw-app-server** (WSB1, G007) · OpenBB Workspace bridge: serve `GET /widgets.json`, `GET /apps.json`, `GET /widget-data/{route...}` byte-compatible with the published backends-for-openbb contract, derived automatically from the endpoint catalog (60 Fetch widgets); CORS + optional `X-TDW-API-KEY` auth (fail-closed non-loopback). Env-gated on `TDW_WORKSPACE_BIND`. Compute routes excluded for v1 (follow-up). Gates: integ. Done-when: `widgets.json` parses in Workspace + a widget renders — **done** (derivation + serde round-trips + golden snapshot + e2e in `tdw-service-api/tests/workspace_route_e2e.rs`; manual Workspace interop checklist in `docs/products/openbb-workspace-backend.md`). | L | done |
| L5.9 | **new tdw-openbb-agent** + **tdw-app-server** (WSB2, G008) · OpenBB Workspace **agent protocol** bridge: make registry agents callable *from* Workspace as custom copilots. Serve `GET /agents.json` (one default copilot) + `POST /v1/query` (the openbb-ai SSE protocol — `reasoning_step` / `message_chunk` / `get_widget_data` / `citations` / `table` / `chart`), a thin transport over the pure `tdw-openbb-agent` mapping crate driving the G016 `StreamingLanguageModel` (offline `StubLanguageModel` by default). Stateless two-request widget-data pattern. CORS + optional `X-TDW-API-KEY` auth reused from the WSB1 family. Gates: integ. Done-when: `agents.json` parses + `POST /query` streams an answer + the two-request widget-data round trip closes — **done** (serde round-trips + golden SSE frames + two-request folding unit tests; e2e in `tdw-service-api/tests/agent_route_e2e.rs`; agent listener gated on `TDW_AGENT_BIND`, sharing the `TDW_WORKSPACE_*` CORS/key posture; manual Workspace copilot checklist in `docs/products/openbb-workspace-agent.md`). v1 exposes one copilot (registry projection is a follow-up). | L | done |
| L5.10 | **tdw-mcp** (WSB3, G009) · make `tdw-mcp` registrable as a Workspace app `mcp_server` and align the widget-citation contract. Configurable Streamable-HTTP origin allow-list via `TDW_MCP_ALLOWED_ORIGINS` (comma-separated exact origins, loopback-default unchanged, `https://pro.openbb.co` opt-in; bearer-token rule untouched); read-only widget-catalog tools `tdw.widgets.list` / `tdw.widgets.describe` / `tdw.apps.list` backed by tdw-widgets; a contract test asserting every widget `mcp_tool.tool_id` resolves to a real tdw-mcp tool. Gates: unit (origin allow-list, widget tools, citation contract). Done-when: `tdw-mcp` registerable as an app `mcp_server` and no widget can cite a nonexistent MCP tool — **done** (tests in `crates/tdw-mcp/src/lib.rs`; registration docs in `docs/products/openbb-workspace-backend.md`). | S | done |
| L5.11 | **new sdk/python (`finx-platform`)** (WS3, G012) · OpenBB Python-package parity: a thin, **generated** stdlib-only HTTP client over the catalog REST surface (`pip install finx-platform` → `finx.equity.price.historical(symbol="AAPL", provider="yahoo")` → `FinXObject` with `.to_dataframe()`/`.to_polars()`/`.to_dict()`). Generated by xtask `pysdk-sync`/`pysdk-check` from `tdw_endpoint_catalog::catalog()` (one module/class per router namespace, nested accessors, method per route; typed kwargs from `params_schema`; `provider`/`chart`/`**kwargs`); `Fetch` routes call `GET /api/v1/<route>`, `Compute` (`technical/*`) raise `NotImplementedError` (REST serves Fetch only). Explicitly **not** PyO3 — the daemon is the product, policy/credentials stay server-side. Gates: xtask determinism + drift (`pysdk-check` in CI), Python `unittest` (URL/kwarg/envelope/error mapping). Done-when: generated client imports and a route call round-trips against a local daemon — **done** (generator in `xtask/src/pysdk.rs`, 12 generated files covering 80 routes; runtime in `sdk/python/finx_platform/_client.py`; tests in `sdk/python/tests/`; quickstart in `docs/products/python-sdk.md`). | M | done |

---

## Part 3 — Top-10 Prioritized Items (value = OpenBB-user-visible capability ÷ effort)

| Rank | Item | Why (capability/effort) | Status |
|---|---|---|---|
| 1 | **L1.4** standard data models | Unblocks every fundamentals/estimates/macro endpoint; nothing standardizes without it | todo |
| 2 | **L2.3** fred macro/rate/fixedincome expansion | One no-cost-ish crate unlocks ~40 OpenBB cmds (entire fixedincome + economy core) | todo |
| 3 | **L2.1** fmp fundamentals cluster | One key unlocks ~25 equity-fundamental cmds (the headline OpenBB use case) | todo |
| 4 | **L1.1** result envelope (OBBject-equiv) | Cross-client consistency + LLM/df interop; prereq for charting/export/MCP | todo |
| 5 | **L4.1** tdw-analytics-technical | ~18 core indicators users expect; replaces fragile UDF workaround; pure compute (no keys). Core set delivered as `technical/*` ops + `technical.*` tools | done (core) |
| 6 | **L5.1** HTTP+SSE service (#161) | Turns the daemon into a usable REST surface; gates L5.2/L5.6 | in-progress |
| 7 | **L2.4** yahoo expansion | No-key provider → ~15 cmds (profile/quote/discovery/options/futures) free to users | todo |
| 8 | **L1.2** tdw-symbology (#173) | Symbol/exchange/FX/crypto normalization needed by nearly every endpoint | in-progress |
| 9 | **L4.2** tdw-analytics-quant | sharpe/sortino/capm/rolling stats; high demand, pure compute, golden-testable. Core set delivered as `quantitative/*` ops + `quantitative.*` tools (G015) | done (core) |
| 10 | **L2.6** sec utils (cik/symbol map, 13f, FTD) | No-key; completes ownership + shorts + regulators clusters cheaply | done |

**Top-3 (act first):** L1.4 standard models → L2.3 fred expansion → L2.1 fmp fundamentals.

---

## Resume checklist

- [ ] L1 complete (envelope, symbology, params, models, provider-resolution) before mass L2.
- [ ] Each L2/L3 endpoint normalized to an L1.4 model (no raw provider shapes leaking).
- [ ] L4 analytics golden-tested against hardcoded reference values (clean-room, no OpenBB).
- [ ] L5 REST/CLI/charting layered on the envelope, not bypassing it.
- [ ] Clean-room rule honored: every PR cites vendor/textbook docs, never OpenBB source.

---

## P2 roll-up — 2026-06-11 (OpenBB-parity phase 2: data-breadth + warehouse)

> **Scoreboard snapshot.** The endpoint catalog now exposes **119 routes / 91
> provider candidates** (`xtask catalog-check` green), up from the P1 baseline of
> 103/79. P2 added 8 waves, all clean-room + 3-lens reviewed:
> - **W2 FMP completion** — equity/fundamental/{splits,dividends+fmp}, estimates/historical_eps, compare/peers, discovery/{gainers,losers,active}, screener (new `ScreenerRow`).
> - **W5 CFTC** — `tdw-provider-cftc` Commitments of Traders (`CommitmentOfTraders`) → regulators/cftc/{cot,cot_search}.
> - **W7 News** — benzinga company + world news normalized to `NewsArticle` → news/{company,world}.
> - **W8 technical long-tail** — ichimoku, zlma, adosc, vortex, supertrend (Compute).
> - **W9 MCP dynamic route tools** — every Fetch route exposed as `tdw.route.*`, read from the catalog at call time.
> - **W10 warehouse** — bronze landing tables (raw.instrument, raw.screener_row, raw.commitment_of_traders; migrations 0026/0027) matching the ingest bindings.
> - **W1/W3/W4 verified already-delivered by P1** (symbology, FRED 32-route breadth, Yahoo breadth).
>
> P1 foundations reused throughout: `tdw-domain` standard models + `ResultEnvelope`,
> the `(provider,endpoint)` dispatch + ingest registries, OpenAPI 3.1 + Python SDK
> + tdw-widgets derivation (all drift-gated). Deferred items tracked below (D1–D8).

---

## P3 roll-up — 2026-06-12 (OpenBB-parity phase 3: deferred-backlog burn-down)

> **Scoreboard snapshot.** The endpoint catalog now exposes **131 routes / 98
> provider candidates** (`xtask catalog-check` green), up from the P2 close of
> 119/91. P3 burned down the P2 deferred backlog (D1–D8) across 8 waves, each
> clean-room + 3-lens reviewed + drift-gated, landed PR-by-PR (#361–#373):
> - **W1 analytics-portfolio** (#361, D6) — `tdw-analytics-portfolio`: portfolio/{cumulative_returns,drawdown,max_drawdown,allocation,contribution} (Compute, pure-Rust, hand-verified golden).
> - **W2 credential-registry** (#363, D8) — credential reads centralized on the shared `tdw_core::http_support::{read_required_key,read_optional_key}` helpers; migrated the lone `std::env::var` outlier (CFTC). Config-file-fallback delta deferred (layering: tdw-core cannot depend on tdw-config).
> - **W3 IMF** (#367, D4) — `tdw-provider-imf` SDMX-JSON `CompactData` → `MacroSeries`: economy/imf/{international_financial_statistics,direction_of_trade,balance_of_payments}. Defensive single-or-array parser; TLS + path-injection guard.
> - **W4 EconDB** (#369, D4) — `tdw-provider-econdb` → `MacroSeries`: economy/econdb/series (ticker-parameterized, optional token). Dual-shape (parallel-arrays / record-list) parser.
> - **W5 estimates breadth** (#371, D7) — FMP `tdw_domain::Estimate`: equity/estimates/{price_target (price-target-consensus), forward (analyst-estimates → forward_eps/sales/ebitda)}.
> - **W6 XLSX export** (#372, D5) — `tdw-cli --export xlsx` via `rust_xlsxwriter` (pure-Rust, MIT, `default-features=false`, `cargo deny` green). **Parquet deferred** (arrow-rs heavy tree vs the zero-heavy-dep posture; arrow2/parquet2 archived).
> - **W7 niche discovery/short** (#373, D2/D3) — assessed and **deferred** stockgrid/wsj/finviz/biztoc/intrinio (no vendor-published API / keyed / paid — would invent endpoints). The parity value is already served by FMP discovery + Yahoo price/performance + FINRA/SEC shorts; corrected 3 stale `MISSING` scoreboard rows.
> - **W8 cutover** — this scoreboard refresh + the full quality gate.
>
> Net deferred-after-P3 (each with a documented decision, none blocking): **Parquet**
> export (heavy-dep), the **niche providers** stockgrid/wsj/finviz/biztoc/intrinio
> (no verifiable public API), **intrinio** options (paid key), **stockgrid
> short_volume** (no API), and the **D8 config-file credential fallback** (a
> layering refactor). All other D1–D8 items are **DONE**.

---

## Deferred P2 tasks (descoped 2026-06-11, tracked for a future wave)

P2 delivered 8 waves (catalog 103→119: FMP completion, CFTC COT, MCP dynamic
route tools, warehouse landing tables, news cluster, technical long-tail). The
following were **explicitly descoped** at the P2 cutover (user decision) and are
tracked here as todos — they are NOT done. Each is a clean, well-scoped future
slice; none blocks the delivered surface.

| # | Deferred task | Why deferred | Status |
|---|---|---|---|
| D1 | `tdw-provider-famafrench` (Ken French factors) | — | **DONE** (P2W6 factors `economy/factors/famafrench`; P4W9 portfolio-formation breadth `economy/factors/famafrench/{breakpoints,us_portfolio_returns,regional_portfolio_returns,country_portfolio_returns,international_index_returns}`; pure-Rust zip/miniz_oxide, no C). |
| D2 | `tdw-provider-{stockgrid,wsj,biztoc,finviz}` (short_volume / etf-discovery / news-world / screener) | **DEFERRED (P3W7, assessed) — none has a verifiable *vendor-published* public API.** stockgrid / wsj / finviz expose only undocumented site-backing JSON endpoints (reverse-engineered, no official API docs); biztoc is a keyed RapidAPI proxy. Building any would mean inventing/guessing endpoint shapes, violating the clean-room "vendor's own public API docs only" rule (and no network to live-verify). Crucially the *parity value* these would add is already delivered by verifiable providers: **discovery** active/gainers/losers via FMP (`equity/discovery/{active,gainers,losers}`), **price/performance** via Yahoo (`equity/price/performance`), **shorts** via FINRA short-interest + SEC `equity/shorts/fails_to_deliver` (G003). Revisit only if a vendor publishes official API docs. | deferred (P3W7, documented) |
| D3 | `tdw-provider-intrinio` (options unusual/snapshots/surface, reported_financials) | **DEFERRED (P3W7, assessed):** paid key — cannot be free/live-verified, and its surface (options unusual/IV-surface) needs both the paid feed and a compute layer; lower priority than the keyless surface. Revisit if a paid Intrinio key is provisioned. | todo (deferred) |
| D4 | `tdw-provider-{imf,econdb,finviz}` (W5 remainder: SDMX macro / gdp+indicators / screener+price_target) | Net-new niche crates; macro breadth already broad via FRED (32 routes) | **imf DONE** (P3W3, #367; `economy/imf/*` SDMX-JSON → MacroSeries). **econdb DONE** (P3W4; `economy/econdb/series` → MacroSeries, optional token). finviz screener/price_target still todo (deferred). |
| D5 | Parquet + XLSX export from any `ResultEnvelope` (L5.4) | CSV + JSON shipped (G013). XLSX needed an export writer; the platform avoids native/C deps everywhere (charting/analytics used zero). | **XLSX DONE (P3W6, `rust_xlsxwriter` pure-Rust, MIT)** — `tdw-cli --export xlsx` from any envelope; `default-features = false` keeps the tree C-free (`zip`→`flate2`/`miniz_oxide`), `cargo deny` green; no `arrow`/`parquet`/C-binding dep added. **Parquet DEFERRED**: arrow-rs `parquet` is pure-Rust but a ~40-crate heavy tree contradicting the platform's zero-heavy-dep posture; the light alternatives (arrow2/parquet2) are archived/unmaintained; revisit if a light maintained pure-Rust Parquet writer emerges or the dep weight is explicitly accepted. |
| D6 | `tdw-analytics-portfolio` (returns/drawdown/allocation/attribution, L4.4) | Beyond OpenBB parity; pure-compute, can be added on the L4.2 quant base any time | **DONE** (P3W1, #361): `portfolio/{cumulative_returns,drawdown,max_drawdown,allocation,contribution}` Compute routes, pure-Rust with hand-verified golden tests. |
| D7 | Estimates breadth: `equity/estimates/price_target` + forward estimates (W7) | consensus + historical_eps shipped; price_target/forward need a keyed estimates provider | **DONE** (P3W5; FMP-keyed). New routes: `equity/estimates/price_target` (FMP `/v4/price-target-consensus` → Estimate kind=price_target) and `equity/estimates/forward` (FMP `/analyst-estimates?period=annual` → one Estimate per period per forward metric: forward_eps/forward_sales/forward_ebitda). |
| D8 | Per-provider credential path (L5.7) | partially layering-constrained | **MOSTLY DONE** (P3W2): credential reading is centralized via the shared `tdw_core::http_support::{read_required_key,read_optional_key}` helpers (env-based, uniform across all keyed providers); migrated the lone outlier (tdw-provider-cftc raw `std::env::var` → `read_optional_key`). Deferred delta: wiring the `tdw-config` registry config-file fallback into those helpers needs an injected resolver (tdw-core cannot depend on tdw-config) — a separate architectural task. |
