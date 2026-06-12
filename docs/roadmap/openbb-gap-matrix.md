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

## Part 1 — Gap Matrix (command-cluster level)

Status legend: **HAVE** (shippable today), **PARTIAL** (some surface, gaps noted),
**MISSING** (no FinX surface). "FinX crate" = where it lives / should live.

### 1. equity (79 cmds)

| OpenBB cluster | FinX status | FinX crate / what's missing | Status |
|---|---|---|---|
| price/historical (OHLCV) | HAVE | yahoo, polygon, alpaca, tiingo, fmp, cboe, tmx, alpha-vantage, databento | done |
| price/quote + price/performance | HAVE | quote + performance standardized via yahoo (keyless, L2.4); also quote via tradier/cboe/fmp | done |
| search / profile / market_snapshots / historical_market_cap | PARTIAL | `profile` standardized via yahoo (keyless, L2.4); sec/fmp search exist; `market_snapshots`, `historical_market_cap` MISSING | in-progress |
| screener | MISSING | no finviz/fmp screener endpoint | todo |
| fundamentals (balance/income/cash/ratios/metrics) | HAVE | fmp income/balance/cash → FinancialStatement, ratios → Ratios, metrics → KeyMetrics, projected into catalog routes equity/fundamental/{income,balance,cash,ratios,metrics} (G011); intrinio/polygon variants MISSING | done |
| fundamentals growth (balance/income/cash growth) | MISSING | fmp growth endpoints not wired | todo |
| fundamentals extras (dividends, splits, eps history, employees, esg, mgmt, transcript, segments) | PARTIAL | sec/fmp raw; standardized cluster MISSING | todo |
| estimates (price_target, consensus, forward_*) | PARTIAL | **`equity/estimates/consensus` HAVE** (standardized, G004); benzinga/seeking-alpha raw; price_target/forward_* still MISSING | in-progress |
| calendar (dividend/earnings/ipo/splits/events) | PARTIAL | dividend/earnings/ipo standardized via nasdaq (keyless public calendar API, G004p2); benzinga earnings raw; splits/events MISSING | in-progress |
| compare (peers/groups/company_facts) | MISSING | — | todo |
| discovery (active/gainers/losers/...) | MISSING | no fmp/yahoo discovery endpoints | todo |
| ownership (insider/institutional/13f/gov_trades/share_stats) | PARTIAL | **`equity/ownership/form_13f` + `equity/ownership/share_statistics` HAVE** (SEC, keyless, G003); insider/institutional/gov_trades still MISSING | in-progress |
| shorts (short_interest/short_volume/fails_to_deliver) | PARTIAL | finra short interest HAVE; **`equity/shorts/fails_to_deliver` HAVE** (SEC FTD, G003); stockgrid short_volume still MISSING | in-progress |
| darkpool/otc | HAVE | finra OTC weekly | done |

### 2. economy (46 cmds)

| OpenBB cluster | FinX status | FinX crate / what's missing | Status |
|---|---|---|---|
| fred_series / fred_search / fred_release / fred_regional | PARTIAL | tdw-provider-fred (series obs) HAVE; **fred_search HAVE** (economy/fred_search dispatchable end-to-end); release_table/regional MISSING | todo |
| cpi / pce / gdp / unemployment / interest_rates | HAVE | fred-backed standardized macro cluster dispatchable end-to-end (economy/cpi, economy/pce, economy/gdp/{real,nominal}, economy/unemployment) | done |
| calendar (economic events) | PARTIAL | trading-economics raw; standardized calendar MISSING | todo |
| indicators / available_indicators / country_profile | MISSING | needs econdb + imf | todo |
| money_measures / central_bank_holdings / primary_dealer_* / fomc_documents | PARTIAL | **money_measures/{m1,m2} HAVE** (fred-backed); **`regulators/fed/fomc_documents` + `fixedincome/government/dealer_stats` HAVE** (tdw-provider-federal-reserve, keyless, G003); central_bank_holdings still needs federal_reserve | in-progress |
| balance_of_payments / direction_of_trade / shipping/* | MISSING | needs imf provider | todo |
| survey/* (nonfarm, sloos, sentiment, regional Fed surveys) | PARTIAL | **survey/{nonfarm_payrolls,university_of_michigan,inflation_expectations} HAVE** (fred-backed, dispatchable end-to-end); sloos/regional Fed surveys MISSING | todo |
| survey/bls_search + bls_series | PARTIAL | tdw-provider-bls (series) HAVE; search MISSING | todo |

### 3. fixedincome (30 cmds)

| OpenBB cluster | FinX status | FinX crate / what's missing | Status |
|---|---|---|---|
| government/yield_curve + treasury_rates | HAVE | fred-backed end-to-end: government/yield_curve (3m/2y/10y/30y aggregate) + government/treasury_rates/{3m,2y,10y,30y} + government/tips_yields/10y | done |
| government/treasury_prices/auctions/tips/svensson | PARTIAL | **`government/treasury_prices` + `government/treasury_auctions` HAVE** (tdw-provider-government-us, keyless, G003) and **tips_yields/10y HAVE** (fred-backed); svensson still MISSING | in-progress |
| rate/* (sofr, effr, estr, ecb, sonia, ameribor, iorb, ...) | HAVE | fred-backed rate cluster dispatchable end-to-end: rate/{sofr,effr,estr,sonia,ecb,iorb,dpcredit,overnight_bank_funding} (ameribor MISSING) | done |
| spreads/* (tcm, tcm_effr, treasury_effr) | HAVE | fred-backed end-to-end: spreads/tcm/{10y2y,10y3m}, spreads/treasury_effr/3m | done |
| corporate/* (bond_prices, commercial_paper, spot/hqm) | PARTIAL | **corporate/spot_rates/10y (HQM) + corporate/commercial_paper/90d HAVE** (fred-backed); bond_prices needs tmx | todo |
| bond_indices / mortgage_indices | HAVE | fred-backed end-to-end: bond_indices/us_corporate_hy, mortgage_indices/30y_fixed | done |

### 4. derivatives (11 cmds)

| OpenBB cluster | FinX status | FinX crate / what's missing | Status |
|---|---|---|---|
| options/chains | HAVE | cboe, deribit | done |
| options/unusual / snapshots / surface | MISSING | needs intrinio (+ IV surface compute) | todo |
| futures/historical / curve / instruments / info | HAVE | catalog routes `derivatives/futures/historical` + `derivatives/futures/curve` landed (G004 keyless wave); deribit instruments HAVE; instruments/info long-tail deferred | done |

### 5. etf (14 cmds)

| OpenBB cluster | FinX status | FinX crate / what's missing | Status |
|---|---|---|---|
| search / info / historical | PARTIAL | historical reuses price; etf search/info MISSING | todo |
| holdings / sectors / countries / equity_exposure / nport | PARTIAL | **`etf/holdings` HAVE** (keyless, SEC N-PORT `NPORT-P` disclosures, G003); sectors/countries/equity_exposure still need fmp | in-progress |
| price_performance / discovery (active/gainers/losers) | MISSING | needs fmp + wsj | todo |

### 6. index (9 cmds)

| OpenBB cluster | FinX status | FinX crate / what's missing | Status |
|---|---|---|---|
| price/historical | HAVE | catalog route `index/price/historical` landed (G004); cboe/polygon bars; polygon aggregates `I:` ticker-prefix candidate added (G011) | done |
| constituents / available / search / snapshots / sectors | PARTIAL | **`index/snapshots` HAVE** (CBOE, G004); constituents/available/search/sectors still need cboe/fmp/tmx index endpoints | in-progress |
| sp500_multiples (Shiller PE) | MISSING | needs nasdaq | todo |

### 7. crypto (4 cmds)

| OpenBB cluster | FinX status | FinX crate / what's missing | Status |
|---|---|---|---|
| price/historical | HAVE | coingecko, ccdata, binance, geckoterminal, polygon (`X:` ticker prefix, G011) | done |
| search | PARTIAL | provider-specific; standardized crypto search MISSING | todo |

### 8. currency (6 cmds)

| OpenBB cluster | FinX status | FinX crate / what's missing | Status |
|---|---|---|---|
| price/historical / snapshots / search | PARTIAL | `currency/price/historical` standardized via polygon aggregates (`C:` ticker prefix, G011); snapshots/search + fmp/tiingo FX MISSING | in-progress |
| reference_rates (ECB) | HAVE | catalog route `currency/reference_rates` landed (ECB-backed, keyless, G004); tdw-provider-ecb HAVE (SDW) | done |

### 9. commodity (8 cmds)

| OpenBB cluster | FinX status | FinX crate / what's missing | Status |
|---|---|---|---|
| price/spot | PARTIAL | fred-backed; standardized spot cluster MISSING | todo |
| petroleum_status / energy_outlook / psd_* (EIA) | PARTIAL | **`commodity/petroleum_status_report` + `commodity/short_term_energy_outlook` HAVE** (EIA report endpoints, G004); tdw-provider-eia (spot) HAVE; psd_* still MISSING | in-progress |

### 10. news (3 cmds)

| OpenBB cluster | FinX status | FinX crate / what's missing | Status |
|---|---|---|---|
| company | PARTIAL | benzinga/tiingo/seeking-alpha raw; standardized news cluster MISSING | todo |
| world | PARTIAL | needs biztoc + standardized shape | todo |

### 11. regulators (13 cmds)

| OpenBB cluster | FinX status | FinX crate / what's missing | Status |
|---|---|---|---|
| sec/* (cik_map, symbol_map, filings utils, litigation) | PARTIAL | **`regulators/sec/cik_map` HAVE** (keyless, G003); tdw-provider-sec (filings, facts) HAVE; symbol_map/utils/litigation still MISSING | in-progress |
| cftc/cot + cot_search | MISSING | needs cftc provider | todo |

### 12–13. analysis routers (technical 28 / quantitative 23 / econometrics 16 / famafrench 7)

| OpenBB cluster | FinX status | FinX crate / what's missing | Status |
|---|---|---|---|
| technical/* (sma, ema, rsi, macd, bbands, atr, ...) | NATIVE | tdw-analytics-technical: ~18 core indicators as `technical/*` Compute routes + `technical.*` MCP tools (L4.1 done; long-tail Ichimoku/cones/Clenow/Demark/fib/RRG deferred) | done |
| quantitative/* (sharpe, sortino, capm, rolling stats) | NATIVE | tdw-analytics-quant: 12 returns-based metrics (sharpe/sortino/omega, max_drawdown/calmar, volatility, skewness/kurtosis, value_at_risk/expected_shortfall, capm, jarque_bera) as `quantitative/*` Compute routes + `quantitative.*` MCP tools (L4.2 done; rolling-window variants + standalone unit-root deferred) | done |
| econometrics/* (ols, panel, cointegration, causality) | NATIVE | tdw-analytics-econometrics: 5 estimators (ols+summary, correlation_matrix, vif, granger_causality, cointegration) as `econometrics/*` Compute routes + `econometrics.*` MCP tools (L4.3 done; panel models + autocorrelation series + formal unit-root deferred; Granger/cointegration p-values documented as honest simplifications) | done |
| famafrench/* (factors, portfolio returns) | MISSING | needs French Data Library provider | todo |

### Roll-up

| Domain | OpenBB cmds | HAVE | PARTIAL | MISSING |
|---|---|---|---|---|
| equity | 79 | 2 | 7 | 5 |
| economy | 46 | 0 | 4 | 4 |
| fixedincome | 30 | 0 | 0 | 6 |
| derivatives | 11 | 1 | 1 | 1 |
| etf | 14 | 0 | 1 | 2 |
| index | 9 | 0 | 1 | 2 |
| crypto | 4 | 1 | 1 | 0 |
| currency | 6 | 0 | 1 | 1 |
| commodity | 8 | 0 | 2 | 0 |
| news | 3 | 0 | 2 | 0 |
| regulators | 13 | 0 | 1 | 1 |
| analysis (4 routers) | 74 | 0 | 0 | 4 |
| **Total clusters** | — | **7** | **22** | **26** |

**Honest read:** FinX's ~70 endpoints cover OHLCV/crypto/options breadth well, but the
*standardized fundamentals/estimates/macro/fixedincome/analytics* surface — OpenBB's bulk —
is mostly MISSING or only PARTIAL (raw, un-normalized).

---

## Part 2 — Implementation Layers (dependency-ordered, leaves first)

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
| L2.2 | **tdw-provider-fmp** · discovery + calendar (active/gainers/losers, dividend/earnings/ipo/splits calendars, price/performance, screener). Gates: live-gated. Done-when: 8 endpoints return standardized lists. | M | todo (G011 trim: screener needs a new ScreenerRow model + fetcher; discovery/calendar FMP fetchers not yet built — out of the keyed-projection scope) |
| L2.3 | **tdw-provider-fred** · macro + rate + spread + fixedincome cluster (cpi/pce/gdp/unemployment via series IDs, sofr/effr/estr/ecb/sonia, tcm spreads, yield_curve, bond/mortgage indices, fred_search/release/regional). fred serves ~40 OpenBB cmds. Gates: live-gated. Done-when: ≥15 fred-backed endpoints standardized. | L | todo |
| L2.4 | **tdw-provider-yahoo** · profile, quote, discovery, dividends, share_statistics, consensus, futures/historical+curve, options/chains. yahoo (no key) serves ~15 cmds. Gates: live-gated. Done-when: ≥8 endpoints standardized. | M | todo |
| L2.5 | **tdw-provider-polygon** · fundamentals (balance/income/cash), market_snapshots, FX + crypto historical, index historical. Gates: live-gated. Done-when: ≥5 endpoints. | M | partial (G011: the single polygon `aggregates` fetcher reused as a candidate on currency/price/historical [`C:` prefix], crypto/price/historical [`X:`], and index/price/historical [`I:`] — caller supplies the prefixed ticker; polygon fundamentals + market_snapshots still todo) |
| L2.6 | **tdw-provider-sec** · cik_map/symbol_map, filings index, form_13f, company_facts, fails_to_deliver, N-PORT, MD&A, latest_financial_reports. Gates: unit (public). Done-when: cik/symbol map + 13f + FTD standardized. | M | done |
| L2.7 | **tdw-provider-cboe** · index (price/constituents/available/search/snapshots), options/chains normalize, futures/curve. Gates: live-gated. Done-when: index cluster standardized. | M | todo |
| L2.8 | **tdw-provider-eia** · petroleum_status_report, short_term_energy_outlook, psd_data/psd_report. Gates: live-gated. Done-when: 3 report endpoints. | S | todo |
| L2.9 | **tdw-provider-ecb** · currency/reference_rates + rate/ecb + balance_of_payments shape. Gates: live-gated. Done-when: reference_rates standardized. | S | todo |
| L2.10 | **tdw-provider-nasdaq** · calendars (dividend/earnings/ipo), top_retail, sp500_multiples (Shiller PE). Gates: live-gated. Done-when: 4 endpoints. | S | todo |
| L2.11 | **tdw-provider-finra** + **stockgrid (new, see L3)** · shorts cluster (short_interest HAVE; add short_volume, FTD via sec). Gates: live-gated. Done-when: shorts cluster complete. | S | todo |
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
| L3.5 | **tdw-provider-intrinio** · Intrinio · paid key | options/unusual+snapshots+surface, reported_financials, forward_pe, data-tag attributes, ipo calendar | L | todo |
| L3.6 | **tdw-provider-finviz** · Finviz · no key | screener, compare/groups, price/performance, metrics, price_target | M | todo |
| L3.7 | **tdw-provider-cftc** · CFTC (Socrata) · app token | regulators/cftc/cot + cot_search | S | todo |
| L3.8 | **tdw-provider-stockgrid** · Stockgrid · no key | shorts/short_volume | S | todo |
| L3.9 | **tdw-provider-wsj** · WSJ market data · no key | etf/discovery (active/gainers/losers) | S | todo |
| L3.10 | **tdw-provider-biztoc** · Biztoc · free key | news/world | S | todo |
| L3.11 | **tdw-provider-famafrench** · Ken French Data Library · no key (academic) | famafrench/* factors + portfolio returns | M | todo |
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
| L5.4 | **tdw-table-format** + **tdw-storage-parquet** · export polish (CSV/XLSX/JSON/Parquet from any envelope; `export_directory`-style config). Gates: unit. Done-when: 4 export formats from one result. CSV + JSON are delivered for the CLI envelope path in G013 (`tdw-cli`'s `--export csv|json`, hand-rolled RFC-4180 escaping; no new export dep); XLSX + Parquet + `export_directory` config remain todo here. | S | in-progress |
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

## Deferred P2 tasks (descoped 2026-06-11, tracked for a future wave)

P2 delivered 8 waves (catalog 103→119: FMP completion, CFTC COT, MCP dynamic
route tools, warehouse landing tables, news cluster, technical long-tail). The
following were **explicitly descoped** at the P2 cutover (user decision) and are
tracked here as todos — they are NOT done. Each is a clean, well-scoped future
slice; none blocks the delivered surface.

| # | Deferred task | Why deferred | Status |
|---|---|---|---|
| D1 | `tdw-provider-famafrench` (Ken French factors) | — | **DONE** (P2W6, route `economy/factors/famafrench`; pure-Rust zip/miniz_oxide, no C). Portfolio-returns variant still deferred. |
| D2 | `tdw-provider-{stockgrid,wsj,biztoc}` (short_volume / etf-discovery / news-world) | Niche providers whose exact public API shapes cannot be verified clean-room without vendor-doc access — building blind risks invented endpoints | todo (deferred) |
| D3 | `tdw-provider-intrinio` (options unusual/snapshots/surface, reported_financials) | Paid key; lower priority than the keyless surface | todo (deferred) |
| D4 | `tdw-provider-{imf,econdb,finviz}` (W5 remainder: SDMX macro / gdp+indicators / screener+price_target) | Net-new niche crates; macro breadth already broad via FRED (32 routes) | **imf DONE** (P3W3, #367; `economy/imf/*` SDMX-JSON → MacroSeries). **econdb DONE** (P3W4; `economy/econdb/series` → MacroSeries, optional token). finviz screener/price_target still todo (deferred). |
| D5 | Parquet + XLSX export from any `ResultEnvelope` (L5.4) | Needs a heavy native dep (`arrow`/`parquet` + an xlsx writer); the platform has avoided native deps everywhere (charting/analytics used zero). CSV + JSON export already shipped (G013) | todo (deferred — needs a dep decision) |
| D6 | `tdw-analytics-portfolio` (returns/drawdown/allocation/attribution, L4.4) | Beyond OpenBB parity; pure-compute, can be added on the L4.2 quant base any time | todo (deferred) |
| D7 | Estimates breadth: `equity/estimates/price_target` + forward estimates (W7) | consensus + historical_eps shipped; price_target/forward need a keyed estimates provider | todo (deferred) |
| D8 | Per-provider credential path (L5.7) | partially layering-constrained | **MOSTLY DONE** (P3W2): credential reading is centralized via the shared `tdw_core::http_support::{read_required_key,read_optional_key}` helpers (env-based, uniform across all keyed providers); migrated the lone outlier (tdw-provider-cftc raw `std::env::var` → `read_optional_key`). Deferred delta: wiring the `tdw-config` registry config-file fallback into those helpers needs an injected resolver (tdw-core cannot depend on tdw-config) — a separate architectural task. |
