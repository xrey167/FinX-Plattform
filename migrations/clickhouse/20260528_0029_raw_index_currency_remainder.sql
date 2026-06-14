-- Bronze landing tables for the OpenBB-parity P4W11 index/currency remainder.
-- Rows are written verbatim from the provider row shapes via
-- INSERT ... FORMAT JSONEachRow by the ingest dispatcher.
--
-- Three new domain models introduced by P4W11 get fresh tables here:
--   * tdw_domain::IndexConstituent -> raw.index_constituent
--   * tdw_domain::Sp500Multiple    -> raw.sp500_multiple
--   * tdw_domain::CurrencySnapshot -> raw.currency_snapshot
--
-- Typing / codec / dedup rationale matches the FRED, Yahoo, and FMP clusters
-- (see 20260528_0021 / 20260528_0023 / 20260528_0025): high-cardinality
-- identifiers lead the ORDER BY as plain String; low-cardinality descriptive
-- columns use LowCardinality; optional numeric / text columns are Nullable. The
-- non-idempotent ingested_at DEFAULT defeats content-hash dedup, so the ingest
-- path supplies an explicit insert_deduplication_token per batch; the
-- non_replicated_deduplication_window below lets that token take effect on these
-- non-replicated MergeTree tables (swap to ReplicatedMergeTree in production).

-- raw.index_constituent mirrors tdw_domain::IndexConstituent: one (index,
-- member) pair from an FMP index-constituent list. The descriptive columns are
-- Nullable since FMP reports a variable subset per index.
create table if not exists raw.index_constituent (
  index LowCardinality(String),
  symbol String,
  name Nullable(String),
  sector LowCardinality(Nullable(String)),
  sub_industry LowCardinality(Nullable(String)),
  headquarters Nullable(String),
  date_added Nullable(String),
  ingested_at DateTime64(3, 'UTC') default now64(3) codec(DoubleDelta, ZSTD)
) engine = MergeTree
partition by toYYYYMM(toDate(ingested_at))
order by (index, symbol)
settings non_replicated_deduplication_window = 1000;

-- raw.sp500_multiple mirrors tdw_domain::Sp500Multiple: one (metric, date)
-- observation from a NASDAQ Data Link MULTPL dataset (Shiller CAPE et al.).
create table if not exists raw.sp500_multiple (
  metric LowCardinality(String),
  date String,
  value Nullable(Float64) codec(ZSTD),
  ingested_at DateTime64(3, 'UTC') default now64(3) codec(DoubleDelta, ZSTD)
) engine = MergeTree
partition by toYYYYMM(toDate(ingested_at))
order by (metric, date)
settings non_replicated_deduplication_window = 1000;

-- raw.currency_snapshot mirrors tdw_domain::CurrencySnapshot: one FX-pair quote
-- from the FMP /fx forex snapshot. Price / bid / ask / change columns are
-- Nullable since the snapshot reports a variable subset per pair.
create table if not exists raw.currency_snapshot (
  symbol String,
  base_currency LowCardinality(Nullable(String)),
  quote_currency LowCardinality(Nullable(String)),
  price Nullable(Float64) codec(ZSTD),
  bid Nullable(Float64) codec(ZSTD),
  ask Nullable(Float64) codec(ZSTD),
  change Nullable(Float64) codec(ZSTD),
  change_percent Nullable(Float64) codec(ZSTD),
  ts_ms Nullable(Int64) codec(ZSTD),
  ingested_at DateTime64(3, 'UTC') default now64(3) codec(DoubleDelta, ZSTD)
) engine = MergeTree
partition by toYYYYMM(toDate(ingested_at))
order by symbol
settings non_replicated_deduplication_window = 1000;
