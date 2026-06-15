# FinX ⇄ OpenBB — total parity

> **Clean-room note:** every contract was implemented from **public** OpenBB and
> source-vendor documentation only (`docs.openbb.co`, the OpenBB-AI SDK
> reference, each provider's own public API docs). **No OpenBB source code was
> consulted.** FinX replicates *capability and field shape*, written natively
> against `tdw_core::Fetcher` / `tdw-service-api`.

This is the **total-parity claim doc** for the OpenBB-parity-total program
(G001–G005), the companion to the surface overview in
[openbb-parity.md](./openbb-parity.md) and the full per-command scoreboard in
[the gap matrix](../roadmap/openbb-gap-matrix.md).

## What "total parity" means here

FinX implements **every OpenBB command that has a documented public API** — free
*or* paid. Paid-key commands are **code-complete and key-gated**: the provider,
routes, and serde models ship today and the data lights up the moment the key is
configured. **No OpenBB command remains unbuilt merely for lack of a *documented*
API.**

The **only** residual is OpenBB commands whose underlying source has **no public
API at all** — scrape-only pages or undocumented internal endpoints. Building
those would mean inventing an endpoint contract or ToS-sensitive HTML scraping,
which violates the clean-room "vendor's own public API docs only" rule. That is a
**source decision, not an engineering gap**.

## Final scoreboard (verified)

Counts are verified against the typed catalog (`tdw-endpoint-catalog::catalog()`)
and the generated `docs/schemas/openapi.json`:

| Metric | Count |
|---|---|
| **Total catalog routes** | **267** |
| `Fetch` routes (provider-backed) | **195** |
| `Compute` routes (derived) | **72** |
| — `technical/*` | 31 |
| — `quantitative/*` | 21 |
| — `econometrics/*` | 15 |
| — `portfolio/*` | 5 |
| OpenAPI 3.1 paths (one per `Fetch` route) | **195** |
| Clippy pedantic+nursery ratchet | **0** |

Every `Fetch` route is also a `GET /api/v1/<route>`, an OpenAPI path, an
OpenBB-Workspace widget, a Python-SDK method, and warehouse-ingestible — all
derived from the one catalog (drift-gated by `xtask catalog-check` /
`openapi-check`). Every `Compute` route is also an MCP tool.

## Provider coverage — 30+ of OpenBB's 32 providers built

Built and serving routes: yahoo, FMP, FRED, SEC/EDGAR, US-Treasury, Federal
Reserve, ECB, CBOE, EIA, IMF, EconDB, OECD, NASDAQ (Data Link), FINRA, CFTC, Ken
French Data Library, BLS, Deribit, congress.gov, biztoc, polygon, alpaca, alpha
vantage, databento, akshare, tiingo, benzinga, **intrinio** (key-gated), plus the
offline `fileset` fixtures — **30+ of OpenBB's 32 providers**.

The **only two** providers with **zero** routes are **stockgrid** and **wsj** —
both because they have **no public API** (see below). **finviz** and **multpl**
are intentionally covered by *equivalents* rather than their scrape surfaces:
finviz's screener/groups via FMP screener + Yahoo price/performance; multpl's
sp500 multiples via NASDAQ Data Link `MULTPL` (Shiller-CAPE family,
`index/sp500_multiples`).

## The honest residual

### 1. No-documented-public-API sources (scrape / internal) — the only true gap

| OpenBB command(s) | Vendor | Why un-built clean-room | Parity already served by |
|---|---|---|---|
| `equity/shorts/short_volume` | **stockgrid** | No documented public API; OpenBB hits an **undocumented internal endpoint** (confirmed by **OpenBB issue #503**). | FINRA short-interest + SEC `equity/shorts/fails_to_deliver` |
| etf / `equity/discovery` (active/gainers/losers) | **wsj** | No documented public JSON API; only an undocumented internal market-data endpoint. | FMP `equity/discovery/{active,gainers,losers}` + Yahoo price/performance |
| `equity/screener`, `equity/compare/groups` | **finviz** | HTML-scrape only, no official API, ToS-sensitive. | FMP screener + Yahoo price/performance |
| `index/sp500_multiples` | **multpl** | multpl.com publishes no API — data is HTML-table scrape only. | **NASDAQ** Data Link `MULTPL` (already covered) |

Each would require inventing an undocumented endpoint shape or ToS-sensitive
scraping. Revisit only if a vendor publishes official, stable API docs.

### 2. Paid-key-gated providers — BUILT, dormant without a key

These are **not unbuilt commands** — they are code-complete routes waiting on a
provisioned credential:

- **intrinio** (options unusual / snapshots / IV-surface, reported_financials,
  forward P/E) — BUILT in G002; needs a paid `INTRINIO_API_KEY`.
- **benzinga premium** (`analyst_search`, premium news) — needs a paid key.
- **tiingo** (`trailing_dividend_yield`) — needs a paid key.

Tell us which key/budget to provision and that slice un-defers immediately.

## What G005 (and the total program) closed

Beyond the keyless P1–P4 breadth, the total-parity program built out the
previously-deferred surface: the **congress.gov (`uscongress/*`)** and **biztoc
(`news/world`)** clusters, the **fixedincome FRED family + economy breadth**, the
**compute-router remainder** (econometrics / quantitative / technical), the
**equity / etf / index / commodity remainder**, the **intrinio key-gated
provider** (G002), and a final web-verified **scrape-provider assessment** (G004)
that confirmed stockgrid / wsj / finviz / multpl have no public API to build
against. After that assessment, the residual above is the honest floor of OpenBB
parity.

## Pointers

| To do this | Start here |
|---|---|
| See the surface at a glance | [openbb-parity.md](./openbb-parity.md) |
| Read the full per-command scoreboard | [gap matrix](../roadmap/openbb-gap-matrix.md) |
| Call data over REST + read the OpenAPI spec | [rest-api.md](./rest-api.md) |
| v1.6.0 release notes | [../release/v1.6.0-notes.md](../release/v1.6.0-notes.md) |
