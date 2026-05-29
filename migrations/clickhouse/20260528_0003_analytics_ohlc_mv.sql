-- Always-fresh "FlowField" OHLC candles, derived incrementally from raw.tick.
--
-- FlowField/SIFT analogy: raw ticks are the unstructured "flow"; the
-- AggregatingMergeTree targets are the "field" of pre-aggregated partial
-- states that ClickHouse maintains. An incremental materialized view runs on
-- every block inserted into raw.tick and folds that block's rows into partial
-- aggregate states (argMinState / argMaxState / maxState / minState / sumState)
-- in the target table -- so the candles stay fresh on every insert without any
-- scheduled batch job. The reader VIEW (..._v) merges those partial states back
-- into plain numbers at query time, giving callers a clean OHLC surface.
--
-- Note on `ts`: raw.tick.ts is now a typed DateTime64(9, 'UTC') (see migration
-- 0002), so it is bucketed directly with toStartOfInterval(ts, ...) -- no
-- parseDateTimeBestEffort wrapping. open/close use argMin/argMax over the event
-- time `ts`, so they are the first/last traded price within each window by
-- actual event time (not insertion order). The argMin/argMax secondary argument
-- is therefore DateTime64(9); the target AggregateFunction states are typed to
-- match. window_start stays DateTime (toStartOfInterval on a DateTime64 returns
-- DateTime). symbol is plain String (high-cardinality universe, matching the raw
-- source); venue stays LowCardinality(String). order by keeps (symbol, venue,
-- window_start): candles are per-venue, so venue is part of the row identity.
--
-- TTL per granularity is a TUNABLE retention default (longer bars are retained
-- longer; 1d is retained indefinitely). A tiered `TTL ... TO VOLUME/DISK`
-- storage policy is the storage-tier follow-up.
--
-- For each granularity g in {1m, 5m, 1h, 1d} this file defines three objects:
--   analytics.ohlc_<g>      -- AggregatingMergeTree target (partial states)
--   analytics.ohlc_<g>_mv   -- incremental MV: raw.tick -> target
--   analytics.ohlc_<g>_v    -- reader view: merges states into plain OHLC

-- ============================ 1 minute =============================
create table if not exists analytics.ohlc_1m (
  symbol String,
  venue LowCardinality(String),
  window_start DateTime,
  open_state AggregateFunction(argMin, Float64, DateTime64(9)),
  close_state AggregateFunction(argMax, Float64, DateTime64(9)),
  high_state AggregateFunction(max, Float64),
  low_state AggregateFunction(min, Float64),
  volume_state AggregateFunction(sum, Float64)
) engine = AggregatingMergeTree
partition by toYYYYMM(window_start)
-- order by keeps venue: candles are per-venue, so venue is part of row identity.
order by (symbol, venue, window_start)
ttl window_start + interval 180 day -- tunable: 1m OHLC retention
-- non_replicated_deduplication_window lets a retried ingest batch carrying the
-- same insert_deduplication_token (with deduplicate_blocks_in_dependent_
-- materialized_views=1 on the INSERT) dedup THROUGH this MV target, preventing
-- double-counted volume; production ReplicatedMergeTree has this on by default.
settings non_replicated_deduplication_window = 1000;

create materialized view if not exists analytics.ohlc_1m_mv
to analytics.ohlc_1m
as select
  symbol,
  venue,
  toStartOfInterval(ts, INTERVAL 1 MINUTE) as window_start,
  argMinState(price, ts) as open_state,
  argMaxState(price, ts) as close_state,
  maxState(price) as high_state,
  minState(price) as low_state,
  sumState(size) as volume_state
from raw.tick
group by symbol, venue, window_start;

create view if not exists analytics.ohlc_1m_v
as select
  symbol,
  venue,
  window_start,
  argMinMerge(open_state) as open,
  argMaxMerge(close_state) as close,
  maxMerge(high_state) as high,
  minMerge(low_state) as low,
  sumMerge(volume_state) as volume
from analytics.ohlc_1m
group by symbol, venue, window_start;

-- ============================ 5 minutes ============================
create table if not exists analytics.ohlc_5m (
  symbol String,
  venue LowCardinality(String),
  window_start DateTime,
  open_state AggregateFunction(argMin, Float64, DateTime64(9)),
  close_state AggregateFunction(argMax, Float64, DateTime64(9)),
  high_state AggregateFunction(max, Float64),
  low_state AggregateFunction(min, Float64),
  volume_state AggregateFunction(sum, Float64)
) engine = AggregatingMergeTree
partition by toYYYYMM(window_start)
-- order by keeps venue: candles are per-venue, so venue is part of row identity.
order by (symbol, venue, window_start)
ttl window_start + interval 1 year -- tunable: 5m OHLC retention
settings non_replicated_deduplication_window = 1000;

create materialized view if not exists analytics.ohlc_5m_mv
to analytics.ohlc_5m
as select
  symbol,
  venue,
  toStartOfInterval(ts, INTERVAL 5 MINUTE) as window_start,
  argMinState(price, ts) as open_state,
  argMaxState(price, ts) as close_state,
  maxState(price) as high_state,
  minState(price) as low_state,
  sumState(size) as volume_state
from raw.tick
group by symbol, venue, window_start;

create view if not exists analytics.ohlc_5m_v
as select
  symbol,
  venue,
  window_start,
  argMinMerge(open_state) as open,
  argMaxMerge(close_state) as close,
  maxMerge(high_state) as high,
  minMerge(low_state) as low,
  sumMerge(volume_state) as volume
from analytics.ohlc_5m
group by symbol, venue, window_start;

-- ============================== 1 hour ==============================
create table if not exists analytics.ohlc_1h (
  symbol String,
  venue LowCardinality(String),
  window_start DateTime,
  open_state AggregateFunction(argMin, Float64, DateTime64(9)),
  close_state AggregateFunction(argMax, Float64, DateTime64(9)),
  high_state AggregateFunction(max, Float64),
  low_state AggregateFunction(min, Float64),
  volume_state AggregateFunction(sum, Float64)
) engine = AggregatingMergeTree
partition by toYYYYMM(window_start)
-- order by keeps venue: candles are per-venue, so venue is part of row identity.
order by (symbol, venue, window_start)
ttl window_start + interval 2 year -- tunable: 1h OHLC retention
settings non_replicated_deduplication_window = 1000;

create materialized view if not exists analytics.ohlc_1h_mv
to analytics.ohlc_1h
as select
  symbol,
  venue,
  toStartOfInterval(ts, INTERVAL 1 HOUR) as window_start,
  argMinState(price, ts) as open_state,
  argMaxState(price, ts) as close_state,
  maxState(price) as high_state,
  minState(price) as low_state,
  sumState(size) as volume_state
from raw.tick
group by symbol, venue, window_start;

create view if not exists analytics.ohlc_1h_v
as select
  symbol,
  venue,
  window_start,
  argMinMerge(open_state) as open,
  argMaxMerge(close_state) as close,
  maxMerge(high_state) as high,
  minMerge(low_state) as low,
  sumMerge(volume_state) as volume
from analytics.ohlc_1h
group by symbol, venue, window_start;

-- ============================== 1 day ===============================
-- No TTL: daily candles are the long-horizon series and are retained.
create table if not exists analytics.ohlc_1d (
  symbol String,
  venue LowCardinality(String),
  window_start DateTime,
  open_state AggregateFunction(argMin, Float64, DateTime64(9)),
  close_state AggregateFunction(argMax, Float64, DateTime64(9)),
  high_state AggregateFunction(max, Float64),
  low_state AggregateFunction(min, Float64),
  volume_state AggregateFunction(sum, Float64)
) engine = AggregatingMergeTree
partition by toYYYYMM(window_start)
-- order by keeps venue: candles are per-venue, so venue is part of row identity.
order by (symbol, venue, window_start)
settings non_replicated_deduplication_window = 1000;

create materialized view if not exists analytics.ohlc_1d_mv
to analytics.ohlc_1d
as select
  symbol,
  venue,
  toStartOfInterval(ts, INTERVAL 1 DAY) as window_start,
  argMinState(price, ts) as open_state,
  argMaxState(price, ts) as close_state,
  maxState(price) as high_state,
  minState(price) as low_state,
  sumState(size) as volume_state
from raw.tick
group by symbol, venue, window_start;

create view if not exists analytics.ohlc_1d_v
as select
  symbol,
  venue,
  window_start,
  argMinMerge(open_state) as open,
  argMaxMerge(close_state) as close,
  maxMerge(high_state) as high,
  minMerge(low_state) as low,
  sumMerge(volume_state) as volume
from analytics.ohlc_1d
group by symbol, venue, window_start;

-- ===================== consolidated (cross-venue) =====================
-- Consolidated tape across venues: the whole-market candle for a symbol,
-- merging the SAME stored partial states as the per-venue _v views above but
-- grouped by (symbol, window_start) ONLY -- venue drops out of the GROUP BY, so
-- argMin/argMax/max/min/sum fold across every venue into one consolidated bar.
-- No new MV or storage is needed: this is a pure read-time re-aggregation of the
-- existing AggregatingMergeTree states. The per-venue _v views remain for
-- venue-specific analysis. Only 1m and 1d are materialized as named views here;
-- the other granularities (5m, 1h) follow the identical pattern if needed.
create view if not exists analytics.ohlc_1m_consolidated_v
as select
  symbol,
  window_start,
  argMinMerge(open_state) as open,
  argMaxMerge(close_state) as close,
  maxMerge(high_state) as high,
  minMerge(low_state) as low,
  sumMerge(volume_state) as volume
from analytics.ohlc_1m
group by symbol, window_start;

create view if not exists analytics.ohlc_1d_consolidated_v
as select
  symbol,
  window_start,
  argMinMerge(open_state) as open,
  argMaxMerge(close_state) as close,
  maxMerge(high_state) as high,
  minMerge(low_state) as low,
  sumMerge(volume_state) as volume
from analytics.ohlc_1d
group by symbol, window_start;
