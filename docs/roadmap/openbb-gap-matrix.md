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
| fundamentals (balance/income/cash/ratios/metrics) | PARTIAL | fmp has many endpoints wired but not normalized to the 4 statements + ratios cluster; intrinio/polygon variants MISSING | todo |
| fundamentals growth (balance/income/cash growth) | MISSING | fmp growth endpoints not wired | todo |
| fundamentals extras (dividends, splits, eps history, employees, esg, mgmt, transcript, segments) | PARTIAL | sec/fmp raw; standardized cluster MISSING | todo |
| estimates (price_target, consensus, forward_*) | PARTIAL | benzinga/seeking-alpha raw; standardized estimates cluster MISSING | todo |
| calendar (dividend/earnings/ipo/splits/events) | PARTIAL | benzinga earnings raw; fmp/nasdaq calendars MISSING | todo |
| compare (peers/groups/company_facts) | MISSING | — | todo |
| discovery (active/gainers/losers/...) | MISSING | no fmp/yahoo discovery endpoints | todo |
| ownership (insider/institutional/13f/gov_trades/share_stats) | PARTIAL | sec facts raw; standardized ownership cluster MISSING | todo |
| shorts (short_interest/short_volume/fails_to_deliver) | PARTIAL | finra short interest HAVE; stockgrid short_volume + sec FTD MISSING | todo |
| darkpool/otc | HAVE | finra OTC weekly | done |

### 2. economy (46 cmds)

| OpenBB cluster | FinX status | FinX crate / what's missing | Status |
|---|---|---|---|
| fred_series / fred_search / fred_release / fred_regional | PARTIAL | tdw-provider-fred (series obs) HAVE; **fred_search HAVE** (economy/fred_search dispatchable end-to-end); release_table/regional MISSING | todo |
| cpi / pce / gdp / unemployment / interest_rates | HAVE | fred-backed standardized macro cluster dispatchable end-to-end (economy/cpi, economy/pce, economy/gdp/{real,nominal}, economy/unemployment) | done |
| calendar (economic events) | PARTIAL | trading-economics raw; standardized calendar MISSING | todo |
| indicators / available_indicators / country_profile | MISSING | needs econdb + imf | todo |
| money_measures / central_bank_holdings / primary_dealer_* / fomc_documents | PARTIAL | **money_measures/{m1,m2} HAVE** (fred-backed, dispatchable end-to-end); central_bank_holdings/primary_dealer_*/fomc_documents need dedicated federal_reserve provider | todo |
| balance_of_payments / direction_of_trade / shipping/* | MISSING | needs imf provider | todo |
| survey/* (nonfarm, sloos, sentiment, regional Fed surveys) | PARTIAL | **survey/{nonfarm_payrolls,university_of_michigan,inflation_expectations} HAVE** (fred-backed, dispatchable end-to-end); sloos/regional Fed surveys MISSING | todo |
| survey/bls_search + bls_series | PARTIAL | tdw-provider-bls (series) HAVE; search MISSING | todo |

### 3. fixedincome (30 cmds)

| OpenBB cluster | FinX status | FinX crate / what's missing | Status |
|---|---|---|---|
| government/yield_curve + treasury_rates | HAVE | fred-backed end-to-end: government/yield_curve (3m/2y/10y/30y aggregate) + government/treasury_rates/{3m,2y,10y,30y} + government/tips_yields/10y | done |
| government/treasury_prices/auctions/tips/svensson | PARTIAL | **tips_yields/10y HAVE** (fred-backed); treasury_prices/auctions/svensson need government-us | todo |
| rate/* (sofr, effr, estr, ecb, sonia, ameribor, iorb, ...) | HAVE | fred-backed rate cluster dispatchable end-to-end: rate/{sofr,effr,estr,sonia,ecb,iorb,dpcredit,overnight_bank_funding} (ameribor MISSING) | done |
| spreads/* (tcm, tcm_effr, treasury_effr) | HAVE | fred-backed end-to-end: spreads/tcm/{10y2y,10y3m}, spreads/treasury_effr/3m | done |
| corporate/* (bond_prices, commercial_paper, spot/hqm) | PARTIAL | **corporate/spot_rates/10y (HQM) + corporate/commercial_paper/90d HAVE** (fred-backed); bond_prices needs tmx | todo |
| bond_indices / mortgage_indices | HAVE | fred-backed end-to-end: bond_indices/us_corporate_hy, mortgage_indices/30y_fixed | done |

### 4. derivatives (11 cmds)

| OpenBB cluster | FinX status | FinX crate / what's missing | Status |
|---|---|---|---|
| options/chains | HAVE | cboe, deribit | done |
| options/unusual / snapshots / surface | MISSING | needs intrinio (+ IV surface compute) | todo |
| futures/historical / curve / instruments / info | PARTIAL | deribit instruments HAVE; futures curve/historical cluster MISSING | todo |

### 5. etf (14 cmds)

| OpenBB cluster | FinX status | FinX crate / what's missing | Status |
|---|---|---|---|
| search / info / historical | PARTIAL | historical reuses price; etf search/info MISSING | todo |
| holdings / sectors / countries / equity_exposure / nport | MISSING | needs fmp + sec N-PORT | todo |
| price_performance / discovery (active/gainers/losers) | MISSING | needs fmp + wsj | todo |

### 6. index (9 cmds)

| OpenBB cluster | FinX status | FinX crate / what's missing | Status |
|---|---|---|---|
| price/historical | PARTIAL | cboe/polygon bars reusable; index symbology MISSING | todo |
| constituents / available / search / snapshots / sectors | MISSING | needs cboe/fmp/tmx index endpoints | todo |
| sp500_multiples (Shiller PE) | MISSING | needs nasdaq | todo |

### 7. crypto (4 cmds)

| OpenBB cluster | FinX status | FinX crate / what's missing | Status |
|---|---|---|---|
| price/historical | HAVE | coingecko, ccdata, binance, geckoterminal | done |
| search | PARTIAL | provider-specific; standardized crypto search MISSING | todo |

### 8. currency (6 cmds)

| OpenBB cluster | FinX status | FinX crate / what's missing | Status |
|---|---|---|---|
| price/historical / snapshots / search | MISSING | no FX OHLCV cluster (fmp/polygon/tiingo) | todo |
| reference_rates (ECB) | PARTIAL | tdw-provider-ecb HAVE (SDW); reference_rates shape MISSING | todo |

### 9. commodity (8 cmds)

| OpenBB cluster | FinX status | FinX crate / what's missing | Status |
|---|---|---|---|
| price/spot | PARTIAL | fred-backed; standardized spot cluster MISSING | todo |
| petroleum_status / energy_outlook / psd_* (EIA) | PARTIAL | tdw-provider-eia (spot) HAVE; report endpoints MISSING | todo |

### 10. news (3 cmds)

| OpenBB cluster | FinX status | FinX crate / what's missing | Status |
|---|---|---|---|
| company | PARTIAL | benzinga/tiingo/seeking-alpha raw; standardized news cluster MISSING | todo |
| world | PARTIAL | needs biztoc + standardized shape | todo |

### 11. regulators (13 cmds)

| OpenBB cluster | FinX status | FinX crate / what's missing | Status |
|---|---|---|---|
| sec/* (cik_map, symbol_map, filings utils, litigation) | PARTIAL | tdw-provider-sec (filings, facts) HAVE; cik/symbol map + utils MISSING | todo |
| cftc/cot + cot_search | MISSING | needs cftc provider | todo |

### 12–13. analysis routers (technical 28 / quantitative 23 / econometrics 16 / famafrench 7)

| OpenBB cluster | FinX status | FinX crate / what's missing | Status |
|---|---|---|---|
| technical/* (sma, ema, rsi, macd, bbands, atr, ...) | MISSING | no native crate (UDF workaround only) → see L4 | todo |
| quantitative/* (sharpe, sortino, capm, rolling stats) | MISSING | no native crate → see L4 | todo |
| econometrics/* (ols, panel, cointegration, causality) | MISSING | no native crate → see L4 | todo |
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
| L2.1 | **tdw-provider-fmp** · expand to fundamentals cluster (balance/income/cash + *_growth, metrics, ratios, dividends, splits, eps history, peers, profile). Highest leverage: fmp alone serves ~25 OpenBB cmds. Gates: fmt+clippy+live-gated. Done-when: 4 statements + ratios + peers normalized to L1.4. | L | todo |
| L2.2 | **tdw-provider-fmp** · discovery + calendar (active/gainers/losers, dividend/earnings/ipo/splits calendars, price/performance, screener). Gates: live-gated. Done-when: 8 endpoints return standardized lists. | M | todo |
| L2.3 | **tdw-provider-fred** · macro + rate + spread + fixedincome cluster (cpi/pce/gdp/unemployment via series IDs, sofr/effr/estr/ecb/sonia, tcm spreads, yield_curve, bond/mortgage indices, fred_search/release/regional). fred serves ~40 OpenBB cmds. Gates: live-gated. Done-when: ≥15 fred-backed endpoints standardized. | L | todo |
| L2.4 | **tdw-provider-yahoo** · profile, quote, discovery, dividends, share_statistics, consensus, futures/historical+curve, options/chains. yahoo (no key) serves ~15 cmds. Gates: live-gated. Done-when: ≥8 endpoints standardized. | M | todo |
| L2.5 | **tdw-provider-polygon** · fundamentals (balance/income/cash), market_snapshots, FX + crypto historical, index historical. Gates: live-gated. Done-when: ≥5 endpoints. | M | todo |
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
| L3.3 | **tdw-provider-imf** · IMF SDMX · no key | indicators, direction_of_trade, balance_of_payments, shipping/*, imf_utils dataflow discovery | M | todo |
| L3.4 | **tdw-provider-econdb** · EconDB · optional key | gdp/real+nominal, indicators, country_profile, export_destinations | M | todo |
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
| L4.1 | **tdw-analytics-technical** · 28 indicators (sma/ema/wma/hma/zlma, macd, rsi, stoch, cci, adx, aroon, bbands, kc, donchian, atr, obv, ad, adosc, vwap, fisher, cg, cones, clenow, demark, ichimoku, fib, relative_rotation). Gates: fmt+clippy+unit (golden vectors vs known values). Done-when: all 28 with numeric tests; callable as daemon op + UDF. | L | todo |
| L4.2 | **tdw-analytics-quant** · summary, normality, capm, unitroot, perf ratios (sharpe/sortino/omega), stats + rolling stats (mean/stdev/var/skew/kurtosis/quantile). Gates: unit (golden). Done-when: 19 functions with tests. | M | todo |
| L4.3 | **tdw-analytics-econometrics** · correlation_matrix, ols(+summary), autocorrelation, residual_autocorr, cointegration, causality, unit_root, vif, panel models (random/fixed/between/pooled/first_diff/fmac). Gates: unit (golden vs statsmodels reference values, hardcoded). Done-when: 15 functions. | L | todo |
| L4.4 | **tdw-analytics-portfolio** (beyond OpenBB parity) · returns, drawdown, allocation, attribution — uses L4.2. Gates: unit. Done-when: core portfolio metrics. | M | todo |
| L4.5 | **tdw-service-api** · wire L4.1–L4.3 as computation ops (`technical/*`, `quantitative/*`, `econometrics/*`) taking caller data (no provider). Gates: integ. Done-when: ops resolve through daemon. | M | todo |

### L5 — Platform surfaces (parity for how OpenBB is *used*)

| # | Task · scope · gates · done-when | Size | Status |
|---|---|---|---|
| L5.1 | **tdw-service-api** (PR #161) · land HTTP+SSE (`POST /ops`, `GET /events/{id}`, `/health`, `/metrics`) over the daemon. Gates: integ. Done-when: #161 merged, REST command callable. | M | in-progress |
| L5.2 | **tdw-service-api** · REST command-tree mapping logical OpenBB-style routes (`/equity/price/historical?provider=`) onto daemon ops + envelope; auto-OpenAPI. Gates: integ. Done-when: ≥10 routes documented at `/docs`-equiv. | L | todo |
| L5.3 | **tdw-cli** · command-tree parity (menu/command mirror of routers; `--provider`; `-h`). Gates: smoke. Done-when: `tdw equity price historical --symbol AAPL` works. | M | todo |
| L5.4 | **tdw-table-format** + **tdw-storage-parquet** · export polish (CSV/XLSX/JSON/Parquet from any envelope; `export_directory`-style config). Gates: unit. Done-when: 4 export formats from one result. | S | todo |
| L5.5 | **new tdw-charting** · server-side chart spec (Plotly-JSON / Vega) on envelope `chart` field; `chart=true` flag; candlestick + line + indicator overlays. Gates: unit (spec snapshot). Done-when: candlestick spec emitted for price/historical. | L | todo |
| L5.6 | **tdw-mcp** · dynamic tool discovery + per-route exposure config (mcp_config: expose/methods/exclude_args) over the new REST command tree. Gates: integ. Done-when: agent browses categories, activates subset. | M | todo |
| L5.7 | **tdw-config** · per-provider credential resolution table + preferences (`output_type`, dirs) mirroring user_settings sections; env + file. Gates: unit. Done-when: all L2/L3 providers resolve keys via one path. | S | todo |

---

## Part 3 — Top-10 Prioritized Items (value = OpenBB-user-visible capability ÷ effort)

| Rank | Item | Why (capability/effort) | Status |
|---|---|---|---|
| 1 | **L1.4** standard data models | Unblocks every fundamentals/estimates/macro endpoint; nothing standardizes without it | todo |
| 2 | **L2.3** fred macro/rate/fixedincome expansion | One no-cost-ish crate unlocks ~40 OpenBB cmds (entire fixedincome + economy core) | todo |
| 3 | **L2.1** fmp fundamentals cluster | One key unlocks ~25 equity-fundamental cmds (the headline OpenBB use case) | todo |
| 4 | **L1.1** result envelope (OBBject-equiv) | Cross-client consistency + LLM/df interop; prereq for charting/export/MCP | todo |
| 5 | **L4.1** tdw-analytics-technical | 28 indicators users expect; replaces fragile UDF workaround; pure compute (no keys) | todo |
| 6 | **L5.1** HTTP+SSE service (#161) | Turns the daemon into a usable REST surface; gates L5.2/L5.6 | in-progress |
| 7 | **L2.4** yahoo expansion | No-key provider → ~15 cmds (profile/quote/discovery/options/futures) free to users | todo |
| 8 | **L1.2** tdw-symbology (#173) | Symbol/exchange/FX/crypto normalization needed by nearly every endpoint | in-progress |
| 9 | **L4.2** tdw-analytics-quant | sharpe/sortino/capm/rolling stats; high demand, pure compute, golden-testable | todo |
| 10 | **L2.6** sec utils (cik/symbol map, 13f, FTD) | No-key; completes ownership + shorts + regulators clusters cheaply | done |

**Top-3 (act first):** L1.4 standard models → L2.3 fred expansion → L2.1 fmp fundamentals.

---

## Resume checklist

- [ ] L1 complete (envelope, symbology, params, models, provider-resolution) before mass L2.
- [ ] Each L2/L3 endpoint normalized to an L1.4 model (no raw provider shapes leaking).
- [ ] L4 analytics golden-tested against hardcoded reference values (clean-room, no OpenBB).
- [ ] L5 REST/CLI/charting layered on the envelope, not bypassing it.
- [ ] Clean-room rule honored: every PR cites vendor/textbook docs, never OpenBB source.
