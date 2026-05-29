-- Silver canonical OHLC bar table. Bronze sources (raw.equity_historical, and in
-- future the streaming tick/trade flow) are normalized into this single shape.
-- See 20260528_0007_silver_market_data_bar_mv.sql for the bronze->silver MV.
--
-- `ts` is a typed DateTime64(9, 'UTC') codec(DoubleDelta, ZSTD): the silver MV
-- writes it via toDateTime64(date, 9, 'UTC'), and any JSONEachRow producer can
-- send an ISO-8601 string which ClickHouse parses directly into DateTime64(9).
-- `symbol` is plain String: the symbol universe is high-cardinality (>10k
-- distinct symbols) and symbol is the leading sort key, so LowCardinality is the
-- wrong encoding here (its dictionary loses its edge past ~100k distinct values
-- and adds overhead on the dominant key). venue / granularity / source stay
-- LowCardinality(String) (small distinct sets). Float columns use codec(ZSTD)
-- (codec(Gorilla, ZSTD) is an alternative for slowly-varying prices).
--
-- partition by toYYYYMM(ts) runs directly on the typed column. TTL is a TUNABLE
-- retention default; a tiered `TTL ... TO VOLUME/DISK` storage policy is the
-- storage-tier follow-up.
create table if not exists raw.market_data_bar (
  symbol String,
  venue LowCardinality(String),
  granularity LowCardinality(String),
  ts DateTime64(9, 'UTC') codec(DoubleDelta, ZSTD),
  open Float64 codec(ZSTD),
  high Float64 codec(ZSTD),
  low Float64 codec(ZSTD),
  close Float64 codec(ZSTD),
  volume Float64 codec(ZSTD),
  source LowCardinality(String)
) engine = MergeTree
partition by toYYYYMM(ts)
-- order by (symbol, ts): symbol is the dominant filter (prioritize-filters);
-- venue is low-cardinality and rarely the sole filter, so it stays off the key
-- to preserve per-symbol time contiguity (venue remains a plain column).
order by (symbol, ts)
ttl toDateTime(ts) + interval 5 year; -- tunable: silver bar retention
