-- Bronze landing tables for the generic websocket streaming endpoints
-- (provider "ws", endpoint "equity_ticks"). Rows are written verbatim from the
-- provider row shapes (tdw_domain::Tick / tdw_domain::Trade / tdw_domain::Quote)
-- via INSERT ... FORMAT JSONEachRow by the streaming ingest loop
-- (tdw_service_api::run_stream_ingest). Normalization into raw.market_data_bar
-- is a downstream (silver) concern.
--
-- `ts` is a typed DateTime64(9, 'UTC'). The Rust provider row shapes still carry
-- an ISO-8601 timestamp *string* (tdw_domain::{Tick,Trade,Quote}.ts: String);
-- JSONEachRow parses an ISO-8601 string -- including fractional seconds -- DIRECTLY
-- into DateTime64(9) on insert, so the ingest path is UNCHANGED: the provider keeps
-- sending strings and ClickHouse does the parse. This lets the monthly partition
-- key, OHLC bucketing and argMin/argMax run directly on the typed column with no
-- parseDateTimeBestEffort wrapping (see 20260528_0003/0004).
--
-- `symbol` is plain String: the symbol universe is high-cardinality (>10k
-- distinct symbols) and symbol is the leading sort key, so LowCardinality is the
-- wrong encoding here -- its dictionary loses its edge past ~100k distinct
-- values and adds overhead on the dominant key.
-- venue / side are LowCardinality(String) (handful of distinct values).
--
-- Codecs: ts uses codec(DoubleDelta, ZSTD) -- DoubleDelta is ideal for
-- monotonically increasing timestamps. Float columns use codec(ZSTD)
-- (codec(Gorilla, ZSTD) is an alternative well-suited to slowly-varying prices).
-- ingested_at keeps DateTime64(3,'UTC') default now64(3) codec(DoubleDelta, ZSTD).
--
-- `ingested_at DEFAULT now64(3)` is a deliberately non-idempotent column: it
-- makes every retried row hash uniquely, which would defeat ClickHouse's
-- content-hash insert deduplication. The ingest path therefore supplies an
-- explicit `insert_deduplication_token` per batch so retries are still dropped.
-- For that token to take effect on these non-replicated MergeTree tables we
-- enable the deduplication window below. In production, swap the engine for
-- ReplicatedMergeTree (block deduplication is on by default there) and drop the
-- non_replicated_deduplication_window setting.
--
-- ORDER BY is (symbol, ts): symbol is the dominant filter (prioritize-filters);
-- venue is low-cardinality and rarely the sole filter, so it stays off the key
-- to preserve per-symbol time contiguity. venue remains a plain column.
--
-- TTL thresholds below are TUNABLE retention defaults. A tiered
-- `TTL ... TO VOLUME 'cold'` / `TO DISK 'sN'` storage policy (hot SSD -> cold
-- object store) is the storage-tier follow-up.
create table if not exists raw.tick (
  symbol String,
  venue LowCardinality(String),
  ts DateTime64(9, 'UTC') codec(DoubleDelta, ZSTD),
  price Float64 codec(ZSTD),
  size Float64 codec(ZSTD),
  ingested_at DateTime64(3, 'UTC') default now64(3) codec(DoubleDelta, ZSTD)
) engine = MergeTree
partition by toYYYYMM(ts)
order by (symbol, ts)
ttl toDateTime(ts) + interval 90 day -- tunable: raw tick retention
settings non_replicated_deduplication_window = 1000;

create table if not exists raw.trade (
  symbol String,
  venue LowCardinality(String),
  ts DateTime64(9, 'UTC') codec(DoubleDelta, ZSTD),
  price Float64 codec(ZSTD),
  qty Float64 codec(ZSTD),
  side LowCardinality(String),
  ingested_at DateTime64(3, 'UTC') default now64(3) codec(DoubleDelta, ZSTD)
) engine = MergeTree
partition by toYYYYMM(ts)
order by (symbol, ts)
ttl toDateTime(ts) + interval 1 year -- tunable: raw trade retention
settings non_replicated_deduplication_window = 1000;

-- Top-of-book quotes (bid/ask + sizes) for the streaming endpoints. Same typing,
-- codec, ordering and dedup rationale as raw.tick/raw.trade above. Rows are
-- written verbatim from tdw_domain::Quote via JSONEachRow (ts as an ISO-8601
-- string parsed directly into DateTime64(9)).
create table if not exists raw.quote (
  symbol String,
  venue LowCardinality(String),
  ts DateTime64(9, 'UTC') codec(DoubleDelta, ZSTD),
  bid Float64 codec(ZSTD),
  ask Float64 codec(ZSTD),
  bid_size Float64 codec(ZSTD),
  ask_size Float64 codec(ZSTD),
  ingested_at DateTime64(3, 'UTC') default now64(3) codec(DoubleDelta, ZSTD)
) engine = MergeTree
partition by toYYYYMM(ts)
order by (symbol, ts)
ttl toDateTime(ts) + interval 90 day -- tunable: raw quote retention
settings non_replicated_deduplication_window = 1000;
