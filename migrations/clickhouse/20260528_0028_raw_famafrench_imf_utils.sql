-- Bronze landing tables for the Ken French portfolio-formation breadth and the
-- IMF SDMX discovery helpers (openbb-parity P4W9). Rows are written verbatim
-- from the provider row shapes via INSERT ... FORMAT JSONEachRow by the ingest
-- dispatcher.
--
-- Three new domain models introduced by P4W9 get fresh tables here:
--   * tdw_domain::PortfolioReturn    -> raw.portfolio_return
--   * tdw_domain::Breakpoint         -> raw.breakpoint
--   * tdw_domain::ImfDiscoveryRecord -> raw.imf_discovery_record
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

-- raw.portfolio_return mirrors tdw_domain::PortfolioReturn: one (date,
-- portfolio) cell from a Ken French portfolio-formation wide table, long-format.
-- value is a decimal return (the source percent is converted to a fraction by
-- the fetcher).
create table if not exists raw.portfolio_return (
  date String,
  portfolio String,
  value Nullable(Float64) codec(ZSTD),
  ingested_at DateTime64(3, 'UTC') default now64(3) codec(DoubleDelta, ZSTD)
) engine = MergeTree
partition by toYYYYMM(toDate(ingested_at))
order by (date, portfolio)
settings non_replicated_deduplication_window = 1000;

-- raw.breakpoint mirrors tdw_domain::Breakpoint: one (date, breakpoint) cell
-- from a Ken French portfolio-formation breakpoint table, long-format. value is
-- the source level (carried through unscaled).
create table if not exists raw.breakpoint (
  date String,
  breakpoint String,
  value Nullable(Float64) codec(ZSTD),
  ingested_at DateTime64(3, 'UTC') default now64(3) codec(DoubleDelta, ZSTD)
) engine = MergeTree
partition by toYYYYMM(toDate(ingested_at))
order by (date, breakpoint)
settings non_replicated_deduplication_window = 1000;

-- raw.imf_discovery_record mirrors tdw_domain::ImfDiscoveryRecord: one IMF SDMX
-- discovery row (dataflow / table / dimension / presentation cell). The kind
-- column discriminates the variant; descriptor fields are Nullable since each
-- imf_utils helper reports a different subset.
create table if not exists raw.imf_discovery_record (
  kind LowCardinality(String),
  id String,
  name Nullable(String),
  dataflow LowCardinality(Nullable(String)),
  structure LowCardinality(Nullable(String)),
  position Nullable(UInt32) codec(ZSTD),
  value Nullable(String),
  ingested_at DateTime64(3, 'UTC') default now64(3) codec(DoubleDelta, ZSTD)
) engine = MergeTree
partition by toYYYYMM(toDate(ingested_at))
order by (kind, id)
settings non_replicated_deduplication_window = 1000;
