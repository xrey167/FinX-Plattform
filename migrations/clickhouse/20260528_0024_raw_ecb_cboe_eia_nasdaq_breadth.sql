-- Bronze landing tables for the ECB / CBOE / EIA / NASDAQ catalog projection
-- (openbb-parity G004 part 2). Rows are written verbatim from the provider row
-- shapes via INSERT ... FORMAT JSONEachRow by the ingest dispatcher.
--
-- Only the two NEW domain models introduced by part 2 get fresh tables here:
--   * tdw_domain::CommodityReportRow -> raw.commodity_report_row (EIA WPSR/STEO)
--   * tdw_domain::CalendarEvent      -> raw.calendar_event      (NASDAQ calendars)
-- The other part-2 routes reuse tables created by earlier migrations:
--   * ECB reference_rates  -> raw.macro_series      (see 20260528_0021)
--   * CBOE index_snapshots -> raw.price_quote       (see 20260528_0023)
--   * CBOE options_chains  -> raw.option_contract   (see 20260528_0023)
--
-- Typing / codec / dedup rationale matches the FRED and Yahoo clusters (see
-- 20260528_0021 / 20260528_0023): high-cardinality identifiers lead the
-- ORDER BY as plain String; low-cardinality descriptive columns use
-- LowCardinality; optional numeric columns are Nullable(Float64). The
-- non-idempotent ingested_at DEFAULT defeats content-hash dedup, so the ingest
-- path supplies an explicit insert_deduplication_token per batch; the
-- non_replicated_deduplication_window below lets that token take effect on
-- these non-replicated MergeTree tables (swap to ReplicatedMergeTree in
-- production).

-- raw.commodity_report_row mirrors tdw_domain::CommodityReportRow: one
-- (report, series, period) observation from an EIA statistical report. The
-- report column discriminates WPSR vs STEO; series_id is the EIA series.
create table if not exists raw.commodity_report_row (
  report LowCardinality(String),
  series_id String,
  period String,
  series_description Nullable(String),
  value Nullable(Float64) codec(ZSTD),
  units LowCardinality(Nullable(String)),
  ingested_at DateTime64(3, 'UTC') default now64(3) codec(DoubleDelta, ZSTD)
) engine = MergeTree
partition by toYYYYMM(toDate(ingested_at))
order by (report, series_id, period)
settings non_replicated_deduplication_window = 1000;

-- raw.calendar_event mirrors tdw_domain::CalendarEvent: one scheduled
-- corporate-calendar event (dividend / earnings / IPO). The kind column
-- discriminates the variant; variant-specific numeric/textual fields are
-- Nullable since each calendar reports a different subset.
create table if not exists raw.calendar_event (
  kind LowCardinality(String),
  symbol String,
  name Nullable(String),
  date Nullable(String),
  dividend Nullable(Float64) codec(ZSTD),
  payment_date Nullable(String),
  record_date Nullable(String),
  eps_estimate Nullable(Float64) codec(ZSTD),
  fiscal_period Nullable(String),
  price Nullable(Float64) codec(ZSTD),
  shares Nullable(Float64) codec(ZSTD),
  exchange LowCardinality(Nullable(String)),
  ingested_at DateTime64(3, 'UTC') default now64(3) codec(DoubleDelta, ZSTD)
) engine = MergeTree
partition by toYYYYMM(toDate(ingested_at))
order by (kind, symbol)
settings non_replicated_deduplication_window = 1000;
