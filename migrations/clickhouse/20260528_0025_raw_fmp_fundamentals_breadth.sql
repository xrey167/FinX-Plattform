-- Bronze landing tables for the FMP keyed-provider fundamentals breadth wave
-- (openbb-parity G011 / gap-matrix item L2.x fmp). Rows are written verbatim
-- from the provider row shapes via INSERT ... FORMAT JSONEachRow by the ingest
-- dispatcher, mirroring the tdw_domain row structs (FinancialStatement,
-- KeyMetrics, Ratios) so the column shapes agree byte-for-byte.
--
-- Only the three NEW domain models projected by this wave get fresh tables:
--   * tdw_domain::FinancialStatement -> raw.financial_statement
--   * tdw_domain::KeyMetrics         -> raw.key_metrics
--   * tdw_domain::Ratios             -> raw.ratios
-- The other FMP G011 routes reuse tables created by earlier migrations:
--   * FMP profile (equity/profile) -> raw.company_profile (see 20260528_0023)
--   * Polygon currency/crypto/index aggregates -> raw.market_data_bar (0008/0009)
--
-- Typing / codec / dedup rationale matches the FRED and Yahoo clusters (see
-- 20260528_0021 / 20260528_0023): high-cardinality identifiers (symbol) lead the
-- ORDER BY as plain String; low-cardinality descriptive columns use
-- LowCardinality; optional numeric columns are Nullable(Float64). The
-- provider-variable statement/metric/ratio lines land in a Map(String, Float64)
-- mirroring the domain BTreeMap bags. The non-idempotent ingested_at DEFAULT
-- defeats content-hash dedup, so the ingest path supplies an explicit
-- insert_deduplication_token per batch; the non_replicated_deduplication_window
-- below lets that token take effect on these non-replicated MergeTree tables
-- (swap to ReplicatedMergeTree in production). Statement/metric/ratio rows have a
-- natural (symbol, period, date) grain, so they ORDER BY that grain.

-- raw.financial_statement mirrors tdw_domain::FinancialStatement: one period row
-- per (symbol, statement, period, date). The statement column discriminates
-- income / balance / cash; the many provider-variable statement lines land in
-- the line_items Map keyed by a normalized snake_case name.
create table if not exists raw.financial_statement (
  symbol String,
  statement LowCardinality(String),
  period LowCardinality(String),
  fiscal_year Nullable(Int32) codec(ZSTD),
  fiscal_period LowCardinality(Nullable(String)),
  -- Non-nullable: `date` is part of the sorting key and ClickHouse rejects
  -- nullable key columns; absent dates arrive as '' (input_format_null_as_default).
  date String default '',
  filing_date Nullable(String),
  currency LowCardinality(Nullable(String)),
  line_items Map(String, Float64) codec(ZSTD),
  ingested_at DateTime64(3, 'UTC') default now64(3) codec(DoubleDelta, ZSTD)
) engine = MergeTree
partition by toYYYYMM(toDate(ingested_at))
order by (symbol, statement, period, date)
settings non_replicated_deduplication_window = 1000;

-- raw.key_metrics mirrors tdw_domain::KeyMetrics: per-share & valuation metrics
-- for a (symbol, period, date). Typed headline metrics are Nullable(Float64);
-- the remaining provider-specific metrics land in the extra_metrics Map.
create table if not exists raw.key_metrics (
  symbol String,
  period LowCardinality(Nullable(String)),
  -- Non-nullable: `date` is part of the sorting key and ClickHouse rejects
  -- nullable key columns; absent dates arrive as '' (input_format_null_as_default).
  date String default '',
  market_cap Nullable(Float64) codec(ZSTD),
  pe_ratio Nullable(Float64) codec(ZSTD),
  price_to_sales Nullable(Float64) codec(ZSTD),
  price_to_book Nullable(Float64) codec(ZSTD),
  enterprise_value Nullable(Float64) codec(ZSTD),
  ev_to_ebitda Nullable(Float64) codec(ZSTD),
  earnings_per_share Nullable(Float64) codec(ZSTD),
  revenue_per_share Nullable(Float64) codec(ZSTD),
  book_value_per_share Nullable(Float64) codec(ZSTD),
  free_cash_flow_per_share Nullable(Float64) codec(ZSTD),
  dividend_yield Nullable(Float64) codec(ZSTD),
  extra_metrics Map(String, Float64) codec(ZSTD),
  ingested_at DateTime64(3, 'UTC') default now64(3) codec(DoubleDelta, ZSTD)
) engine = MergeTree
partition by toYYYYMM(toDate(ingested_at))
order by (symbol, date)
settings non_replicated_deduplication_window = 1000;

-- raw.ratios mirrors tdw_domain::Ratios: liquidity / profitability / leverage /
-- efficiency ratios for a (symbol, period, date). Typed headline ratios are
-- Nullable(Float64); the remaining provider-specific ratios land in extra_ratios.
create table if not exists raw.ratios (
  symbol String,
  period LowCardinality(Nullable(String)),
  -- Non-nullable: `date` is part of the sorting key and ClickHouse rejects
  -- nullable key columns; absent dates arrive as '' (input_format_null_as_default).
  date String default '',
  current_ratio Nullable(Float64) codec(ZSTD),
  quick_ratio Nullable(Float64) codec(ZSTD),
  gross_margin Nullable(Float64) codec(ZSTD),
  operating_margin Nullable(Float64) codec(ZSTD),
  net_profit_margin Nullable(Float64) codec(ZSTD),
  return_on_assets Nullable(Float64) codec(ZSTD),
  return_on_equity Nullable(Float64) codec(ZSTD),
  debt_to_equity Nullable(Float64) codec(ZSTD),
  interest_coverage Nullable(Float64) codec(ZSTD),
  asset_turnover Nullable(Float64) codec(ZSTD),
  extra_ratios Map(String, Float64) codec(ZSTD),
  ingested_at DateTime64(3, 'UTC') default now64(3) codec(DoubleDelta, ZSTD)
) engine = MergeTree
partition by toYYYYMM(toDate(ingested_at))
order by (symbol, date)
settings non_replicated_deduplication_window = 1000;
