-- Bronze landing table for the equity_historical provider endpoint
-- (yahoo / fileset). Rows are written verbatim from the provider row shape
-- (tdw_domain::EquityHistoricalData) via INSERT ... FORMAT JSONEachRow by the
-- ingest dispatcher. Normalization into raw.market_data_bar is a downstream
-- (silver) concern (see 20260528_0007_silver_market_data_bar_mv.sql).
--
-- `date` stays a `Date` (this is a DAILY bar series, so day resolution is exact
-- and the natural partition/order grain). `symbol` is plain String: the symbol
-- universe is high-cardinality (>10k distinct symbols) and symbol is the leading
-- sort key, so LowCardinality is the wrong encoding here. Float columns use
-- codec(ZSTD) (codec(Gorilla, ZSTD) is an alternative for slowly-varying
-- prices). `date` uses codec(DoubleDelta, ZSTD) (monotonic dates).
--
-- `ingested_at DEFAULT now64(3)` is a deliberately non-idempotent column: it
-- makes every retried row hash uniquely, which would defeat ClickHouse's
-- content-hash insert deduplication. The ingest path therefore supplies an
-- explicit `insert_deduplication_token` per batch so retries are still dropped.
-- For that token to take effect on this non-replicated MergeTree we enable the
-- deduplication window below. In production, swap the engine for
-- ReplicatedMergeTree (block deduplication is on by default there) and drop the
-- non_replicated_deduplication_window setting.
--
-- TTL threshold below is a TUNABLE retention default (equity history is keyed by
-- `date`, so the TTL is expressed on the Date column). A tiered
-- `TTL ... TO VOLUME/DISK` storage policy is the storage-tier follow-up.
create table if not exists raw.equity_historical (
  symbol String,
  date Date codec(DoubleDelta, ZSTD),
  open Float64 codec(ZSTD),
  high Float64 codec(ZSTD),
  low Float64 codec(ZSTD),
  close Float64 codec(ZSTD),
  volume UInt32 codec(ZSTD),
  ingested_at DateTime64(3, 'UTC') default now64(3) codec(DoubleDelta, ZSTD)
) engine = MergeTree
partition by toYYYYMM(date)
order by (symbol, date)
ttl date + interval 5 year -- tunable: equity history retention
settings non_replicated_deduplication_window = 1000;
