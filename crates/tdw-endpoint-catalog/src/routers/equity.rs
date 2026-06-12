//! `equity/*` catalog routes.

use schemars::{Schema, schema_for};
use tdw_core::query_params::StandardParams;
use tdw_domain::{
    CalendarEvent, CompanyProfile, CorporateAction, EquityHistoricalData, Estimate,
    FinancialStatement, Instrument, KeyMetrics, OwnershipRecord, PricePerformance, QuoteSnapshot,
    Ratios, ScreenerRow,
};

use crate::{CatalogEntry, EndpointKind, ProviderCandidate};

/// Candidate order for `equity/price/historical`.
///
/// The two offline fixture fetchers lead so an unkeyed default build resolves
/// network-free; the keyed HTTP providers follow. This is byte-for-byte the
/// order the legacy `provider_resolve::logical_endpoint_table` declared.
const EQUITY_PRICE_HISTORICAL: &[ProviderCandidate] = &[
    ProviderCandidate::new("fileset", "equity_historical"),
    ProviderCandidate::new("yahoo", "equity_historical"),
    ProviderCandidate::new("fmp", "equity_historical"),
    ProviderCandidate::new("tiingo", "historical"),
    ProviderCandidate::new("polygon", "aggregates"),
    ProviderCandidate::new("alpaca", "stock_bars"),
    ProviderCandidate::new("alpha_vantage", "market_data"),
    ProviderCandidate::new("databento", "timeseries"),
    ProviderCandidate::new("akshare", "hist"),
];

/// Keyless SEC candidate for the Form 13F-HR institutional-holdings index
/// (gap-matrix item **L2.6**).
const EQUITY_OWNERSHIP_FORM_13F: &[ProviderCandidate] =
    &[ProviderCandidate::new("sec", "form_13f")];

/// Keyless SEC candidate for fails-to-deliver records (gap-matrix item **L2.6**).
const EQUITY_SHORTS_FAILS_TO_DELIVER: &[ProviderCandidate] =
    &[ProviderCandidate::new("sec", "fails_to_deliver")];
// Keyless Yahoo expansion (gap-matrix item L2.4). Each route's single candidate
// endpoint key matches the Yahoo fetcher's `ENDPOINT` const, which is also the
// runtime fetch/ingest dispatch-table key; a conformance test in tdw-service-api
// keeps these rows and that table in sync.
/// Multi-provider `equity/profile` candidates: keyless Yahoo leads (resolves
/// network-free in the offline default build), then the keyed FMP `profile`
/// fetcher (gap-matrix item L2.x fmp). Order is keyless/offline-first, then keyed.
const EQUITY_PROFILE: &[ProviderCandidate] = &[
    ProviderCandidate::new("yahoo", "equity_profile"),
    ProviderCandidate::new("fmp", "profile"),
];

// FMP fundamentals breadth (gap-matrix item L2.x fmp). The three statement
// routes share one FMP `financial_statement` fetcher (the statement
// discriminator is injected per dispatch binding), so each route gets a distinct
// dispatch endpoint key — `financial_statement_<kind>` — to avoid a dispatch-key
// collision (the NASDAQ-calendar pattern). Ratios and key-metrics are their own
// FMP fetchers keyed by their `ENDPOINT` const. All keyed (FMP only).
const EQUITY_FUNDAMENTAL_INCOME: &[ProviderCandidate] =
    &[ProviderCandidate::new("fmp", "financial_statement_income")];
const EQUITY_FUNDAMENTAL_BALANCE: &[ProviderCandidate] =
    &[ProviderCandidate::new("fmp", "financial_statement_balance")];
const EQUITY_FUNDAMENTAL_CASH: &[ProviderCandidate] =
    &[ProviderCandidate::new("fmp", "financial_statement_cash")];
const EQUITY_FUNDAMENTAL_RATIOS: &[ProviderCandidate] = &[ProviderCandidate::new("fmp", "ratios")];
const EQUITY_FUNDAMENTAL_METRICS: &[ProviderCandidate] =
    &[ProviderCandidate::new("fmp", "key_metrics")];
const EQUITY_PRICE_QUOTE: &[ProviderCandidate] = &[ProviderCandidate::new("yahoo", "equity_quote")];
const EQUITY_PRICE_PERFORMANCE: &[ProviderCandidate] =
    &[ProviderCandidate::new("yahoo", "price_performance")];
/// `equity/fundamental/dividends` candidates: keyless Yahoo leads (resolves
/// network-free in the offline default build), then the keyed FMP `dividends`
/// fetcher (P2W2 FMP fundamentals completion).
const EQUITY_FUNDAMENTAL_DIVIDENDS: &[ProviderCandidate] = &[
    ProviderCandidate::new("yahoo", "dividends"),
    ProviderCandidate::new("fmp", "dividends"),
];

// FMP fundamentals completion (P2W2). Corporate-action splits, historical EPS,
// and company peers are each their own FMP fetcher keyed by its `ENDPOINT`
// const. The three discovery routes share one FMP movers fetcher (the
// `direction` discriminator is injected per dispatch binding), so each gets a
// distinct dispatch endpoint key — `discovery_<direction>` — to avoid a
// dispatch-key collision (the NASDAQ-calendar pattern). All keyed (FMP only).
const EQUITY_FUNDAMENTAL_SPLITS: &[ProviderCandidate] = &[ProviderCandidate::new("fmp", "splits")];
const EQUITY_ESTIMATES_HISTORICAL_EPS: &[ProviderCandidate] =
    &[ProviderCandidate::new("fmp", "historical_eps")];
// Analyst-estimate breadth (openbb-parity P3W5, gap-matrix D7). FMP-keyed:
// price-target consensus and forward analyst estimates, each its own FMP
// fetcher keyed by its `ENDPOINT` const.
const EQUITY_ESTIMATES_PRICE_TARGET: &[ProviderCandidate] =
    &[ProviderCandidate::new("fmp", "price_target")];
const EQUITY_ESTIMATES_FORWARD: &[ProviderCandidate] =
    &[ProviderCandidate::new("fmp", "analyst_estimates")];
const EQUITY_COMPARE_PEERS: &[ProviderCandidate] = &[ProviderCandidate::new("fmp", "peers")];
const EQUITY_DISCOVERY_GAINERS: &[ProviderCandidate] =
    &[ProviderCandidate::new("fmp", "discovery_gainers")];
const EQUITY_DISCOVERY_LOSERS: &[ProviderCandidate] =
    &[ProviderCandidate::new("fmp", "discovery_losers")];
const EQUITY_DISCOVERY_ACTIVE: &[ProviderCandidate] =
    &[ProviderCandidate::new("fmp", "discovery_active")];
const EQUITY_SCREENER: &[ProviderCandidate] = &[ProviderCandidate::new("fmp", "screener")];
const EQUITY_OWNERSHIP_SHARE_STATISTICS: &[ProviderCandidate] =
    &[ProviderCandidate::new("yahoo", "share_statistics")];
const EQUITY_ESTIMATES_CONSENSUS: &[ProviderCandidate] =
    &[ProviderCandidate::new("yahoo", "analyst_consensus")];

// NASDAQ market calendars (gap-matrix item L2.10). The three calendars share one
// NASDAQ `calendar` fetcher (the calendar discriminator is injected per dispatch
// binding), so each route gets a distinct dispatch endpoint key — the route with
// each `'/'` replaced by `'_'`, matching `endpoint_key_for_route` — to avoid a
// dispatch-key collision. Served by the public, keyless NASDAQ calendar API.
const EQUITY_CALENDAR_DIVIDENDS: &[ProviderCandidate] = &[ProviderCandidate::new(
    "nasdaq",
    "equity_calendar_dividends",
)];
const EQUITY_CALENDAR_EARNINGS: &[ProviderCandidate] =
    &[ProviderCandidate::new("nasdaq", "equity_calendar_earnings")];
const EQUITY_CALENDAR_IPO: &[ProviderCandidate] =
    &[ProviderCandidate::new("nasdaq", "equity_calendar_ipo")];

fn params_schema() -> Schema {
    schema_for!(StandardParams)
}

fn model_schema() -> Schema {
    schema_for!(EquityHistoricalData)
}

fn company_profile() -> Schema {
    schema_for!(CompanyProfile)
}

fn price_quote() -> Schema {
    schema_for!(QuoteSnapshot)
}

fn price_performance() -> Schema {
    schema_for!(PricePerformance)
}

fn corporate_action() -> Schema {
    schema_for!(CorporateAction)
}

fn ownership_record() -> Schema {
    schema_for!(OwnershipRecord)
}

fn estimate() -> Schema {
    schema_for!(Estimate)
}

fn calendar_event() -> Schema {
    schema_for!(CalendarEvent)
}

fn financial_statement() -> Schema {
    schema_for!(FinancialStatement)
}

fn key_metrics() -> Schema {
    schema_for!(KeyMetrics)
}

fn ratios() -> Schema {
    schema_for!(Ratios)
}

fn instrument() -> Schema {
    schema_for!(Instrument)
}

fn screener_row() -> Schema {
    schema_for!(ScreenerRow)
}

/// One non-chartable single-model `equity/*` Fetch entry (the common shape of
/// every route in this router except `equity/price/historical`).
fn flat_entry(
    route: &'static str,
    model: fn() -> Schema,
    candidates: &'static [ProviderCandidate],
    bronze_table: &'static str,
    doc: &'static str,
) -> CatalogEntry {
    CatalogEntry {
        route,
        kind: EndpointKind::Fetch,
        params_schema,
        model,
        candidates,
        bronze_table: Some(bronze_table),
        doc,
        chartable: false,
    }
}

/// FMP fundamentals breadth entries (gap-matrix item L2.x fmp): the three
/// financial-statement routes plus ratios and key-metrics. Split out of
/// [`entries`] to keep each function compact; appended in declaration order.
fn fundamental_entries() -> Vec<CatalogEntry> {
    vec![
        flat_entry(
            "equity/fundamental/income",
            financial_statement,
            EQUITY_FUNDAMENTAL_INCOME,
            "raw.financial_statement",
            "Income statement (revenue, margins, net income) per period, FMP-backed.",
        ),
        flat_entry(
            "equity/fundamental/balance",
            financial_statement,
            EQUITY_FUNDAMENTAL_BALANCE,
            "raw.financial_statement",
            "Balance sheet (assets, liabilities, equity) per period, FMP-backed.",
        ),
        flat_entry(
            "equity/fundamental/cash",
            financial_statement,
            EQUITY_FUNDAMENTAL_CASH,
            "raw.financial_statement",
            "Cash-flow statement (operating/investing/financing) per period, FMP-backed.",
        ),
        flat_entry(
            "equity/fundamental/ratios",
            ratios,
            EQUITY_FUNDAMENTAL_RATIOS,
            "raw.ratios",
            "Financial ratios (liquidity, profitability, leverage), FMP-backed.",
        ),
        flat_entry(
            "equity/fundamental/metrics",
            key_metrics,
            EQUITY_FUNDAMENTAL_METRICS,
            "raw.key_metrics",
            "Key metrics (per-share and valuation), FMP-backed.",
        ),
    ]
}

/// The `equity` namespace's catalog entries, in declaration order.
pub fn entries() -> Vec<CatalogEntry> {
    let mut entries = vec![
        CatalogEntry {
            route: "equity/price/historical",
            kind: EndpointKind::Fetch,
            params_schema,
            model: model_schema,
            candidates: EQUITY_PRICE_HISTORICAL,
            bronze_table: Some("raw.equity_historical"),
            doc: "Historical end-of-day OHLCV bars for an equity symbol.",
            chartable: true,
        },
        flat_entry(
            "equity/ownership/form_13f",
            ownership_record,
            EQUITY_OWNERSHIP_FORM_13F,
            "raw.ownership_record",
            "SEC Form 13F-HR institutional-holdings filing index by CIK (keyless).",
        ),
        flat_entry(
            "equity/shorts/fails_to_deliver",
            ownership_record,
            EQUITY_SHORTS_FAILS_TO_DELIVER,
            "raw.ownership_record",
            "SEC fails-to-deliver records for an equity symbol (keyless).",
        ),
        flat_entry(
            "equity/profile",
            company_profile,
            EQUITY_PROFILE,
            "raw.company_profile",
            "Company profile (name, exchange, currency, market cap); Yahoo (keyless) \
             then FMP candidate.",
        ),
        flat_entry(
            "equity/price/quote",
            price_quote,
            EQUITY_PRICE_QUOTE,
            "raw.price_quote",
            "Current last-price quote snapshot for an equity symbol, Yahoo-backed.",
        ),
        flat_entry(
            "equity/price/performance",
            price_performance,
            EQUITY_PRICE_PERFORMANCE,
            "raw.price_performance",
            "Period total returns (one-day through one-year) for a symbol, Yahoo-backed.",
        ),
        flat_entry(
            "equity/fundamental/dividends",
            corporate_action,
            EQUITY_FUNDAMENTAL_DIVIDENDS,
            "raw.corporate_action",
            "Historical cash dividends for an equity symbol; Yahoo (keyless) then FMP candidate.",
        ),
        flat_entry(
            "equity/ownership/share_statistics",
            ownership_record,
            EQUITY_OWNERSHIP_SHARE_STATISTICS,
            "raw.ownership_record",
            "Share statistics (float, shares outstanding, ownership), Yahoo-backed.",
        ),
        flat_entry(
            "equity/estimates/consensus",
            estimate,
            EQUITY_ESTIMATES_CONSENSUS,
            "raw.estimate",
            "Analyst consensus and price targets for a symbol, Yahoo-backed.",
        ),
        flat_entry(
            "equity/calendar/dividends",
            calendar_event,
            EQUITY_CALENDAR_DIVIDENDS,
            "raw.calendar_event",
            "Upcoming dividend calendar (ex-date, amount, pay date), NASDAQ-backed.",
        ),
        flat_entry(
            "equity/calendar/earnings",
            calendar_event,
            EQUITY_CALENDAR_EARNINGS,
            "raw.calendar_event",
            "Upcoming earnings calendar (date, EPS estimate), NASDAQ-backed.",
        ),
        flat_entry(
            "equity/calendar/ipo",
            calendar_event,
            EQUITY_CALENDAR_IPO,
            "raw.calendar_event",
            "Upcoming IPO calendar (price, shares, exchange), NASDAQ-backed.",
        ),
    ];
    entries.extend(fundamental_entries());
    entries.extend(completion_entries());
    entries
}

/// FMP fundamentals-completion entries (openbb-parity P2W2): corporate-action
/// splits, historical EPS, company peers, the three market-movers discovery
/// routes, and the equity screener. Split out of [`entries`] to keep each
/// function compact; appended in declaration order.
fn completion_entries() -> Vec<CatalogEntry> {
    vec![
        flat_entry(
            "equity/fundamental/splits",
            corporate_action,
            EQUITY_FUNDAMENTAL_SPLITS,
            "raw.corporate_action",
            "Historical stock splits for an equity symbol, FMP-backed.",
        ),
        flat_entry(
            "equity/estimates/historical_eps",
            estimate,
            EQUITY_ESTIMATES_HISTORICAL_EPS,
            "raw.estimate",
            "Historical reported vs estimated EPS per period for a symbol, FMP-backed.",
        ),
        flat_entry(
            "equity/estimates/price_target",
            estimate,
            EQUITY_ESTIMATES_PRICE_TARGET,
            "raw.estimate",
            "Analyst price-target consensus (high/low/consensus) for a symbol, FMP-backed.",
        ),
        flat_entry(
            "equity/estimates/forward",
            estimate,
            EQUITY_ESTIMATES_FORWARD,
            "raw.estimate",
            "Forward analyst estimates per period (EPS, sales, EBITDA), FMP-backed.",
        ),
        flat_entry(
            "equity/compare/peers",
            instrument,
            EQUITY_COMPARE_PEERS,
            "raw.instrument",
            "Comparable peer tickers for an equity symbol, FMP-backed.",
        ),
        flat_entry(
            "equity/discovery/gainers",
            price_quote,
            EQUITY_DISCOVERY_GAINERS,
            "raw.price_quote",
            "Top market gainers snapshot (price, change, % change), FMP-backed.",
        ),
        flat_entry(
            "equity/discovery/losers",
            price_quote,
            EQUITY_DISCOVERY_LOSERS,
            "raw.price_quote",
            "Top market losers snapshot (price, change, % change), FMP-backed.",
        ),
        flat_entry(
            "equity/discovery/active",
            price_quote,
            EQUITY_DISCOVERY_ACTIVE,
            "raw.price_quote",
            "Most-active equities snapshot (price, change, % change), FMP-backed.",
        ),
        flat_entry(
            "equity/screener",
            screener_row,
            EQUITY_SCREENER,
            "raw.screener_row",
            "Equity screener results filtered by market cap, sector, industry, and exchange, \
             FMP-backed.",
        ),
    ]
}
