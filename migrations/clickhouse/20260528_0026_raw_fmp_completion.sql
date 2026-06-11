-- Bronze landing tables for the FMP fundamentals-completion wave (openbb-parity
-- P2W2 / gap-matrix L2.x fmp). Rows are written verbatim from the provider row
-- shapes via INSERT ... FORMAT JSONEachRow by the ingest dispatcher
-- (tdw_service_api::dispatcher::insert_fmp_completion_ingest_bindings), mirroring
-- the tdw_domain row structs (Instrument, ScreenerRow) so the column shapes agree
-- byte-for-byte. Without these tables the equity/compare/peers and equity/screener
-- ingest bindings (already registered on main) would fail at INSERT time.
--
-- Only the two NEW domain models projected by this wave get fresh tables:
--   * tdw_domain::Instrument  -> raw.instrument   (equity/compare/peers)
--   * tdw_domain::ScreenerRow -> raw.screener_row (equity/screener)
-- The other P2W2 FMP routes reuse tables created by earlier migrations:
--   * FMP dividends / splits (CorporateAction) -> raw.corporate_action (0014)
--   * FMP historical EPS (Estimate)            -> raw.estimate (0023)
--   * FMP discovery movers (QuoteSnapshot)     -> raw.price_quote (0023)
--
-- Typing / codec / dedup rationale matches the FRED and FMP clusters
-- (20260528_0021 / 20260528_0025): the high-cardinality identifier (symbol)
-- leads the ORDER BY as a plain String; low-cardinality descriptive columns use
-- LowCardinality; optional numeric columns are Nullable(Float64); the two boolean
-- flags use Nullable(Bool) (ClickHouse parses JSON true/false natively for Bool,
-- not for bare UInt8). The non-idempotent ingested_at DEFAULT defeats
-- content-hash dedup, so the ingest path supplies an explicit
-- insert_deduplication_token per batch; non_replicated_deduplication_window lets
-- that token take effect on these non-replicated MergeTree tables (swap to
-- ReplicatedMergeTree in production). Both row shapes are snapshot-style (no
-- natural event date), so they ORDER BY (symbol[, venue]) and partition by the
-- ingest day.

-- raw.instrument mirrors tdw_domain::Instrument: one comparable peer ticker per
-- (symbol, venue). Used by equity/compare/peers; venue is the provider tag.
create table if not exists raw.instrument (
  symbol String,
  name String,
  venue LowCardinality(String) default '',
  ingested_at DateTime64(3, 'UTC') default now64(3) codec(DoubleDelta, ZSTD)
) engine = MergeTree
partition by toYYYYMM(toDate(ingested_at))
order by (symbol, venue)
settings non_replicated_deduplication_window = 1000;

-- raw.screener_row mirrors tdw_domain::ScreenerRow: one screener result row per
-- symbol. Descriptive low-cardinality columns are LowCardinality; optional
-- numerics are Nullable(Float64); the ETF / actively-trading flags are
-- Nullable(Bool).
create table if not exists raw.screener_row (
  symbol String,
  company_name Nullable(String) codec(ZSTD),
  market_cap Nullable(Float64) codec(ZSTD),
  sector LowCardinality(Nullable(String)),
  industry LowCardinality(Nullable(String)),
  beta Nullable(Float64) codec(ZSTD),
  price Nullable(Float64) codec(ZSTD),
  last_annual_dividend Nullable(Float64) codec(ZSTD),
  volume Nullable(Float64) codec(ZSTD),
  exchange LowCardinality(Nullable(String)),
  exchange_short_name LowCardinality(Nullable(String)),
  country LowCardinality(Nullable(String)),
  is_etf Nullable(Bool),
  is_actively_trading Nullable(Bool),
  ingested_at DateTime64(3, 'UTC') default now64(3) codec(DoubleDelta, ZSTD)
) engine = MergeTree
partition by toYYYYMM(toDate(ingested_at))
order by (symbol)
settings non_replicated_deduplication_window = 1000;
