-- Always-fresh L2 order-book depth analytics, derived incrementally from
-- raw.book_level (the full-depth fan-out table from migration 0010). One row per
-- (symbol, venue, minute) of pre-aggregated partial states, maintained by an
-- incremental materialized view so the stats stay fresh on every insert without
-- any scheduled batch job. The reader VIEW (..._v) merges those partial states
-- back into plain numbers at query time, yielding best bid/ask, cumulative
-- bid/ask depth, the spread, and the order-book imbalance.
--
-- Conditional -If combinators are the heart of this MV. raw.book_level carries
-- every price level for both sides in one stream (side = 'bid'/'ask',
-- level = 1..N); the per-minute stats need to slice that stream by side/level
-- INSIDE the aggregate, so we use the -IfState combinators. ClickHouse applies
-- -If BEFORE -State, so the state spelling is <func>IfState (e.g. sumIfState),
-- NOT <func>StateIf; the stored type uses the conditional function (sumIf /
-- argMaxIf) and the reader merges with the paired <func>IfMerge:
--   * best_bid/best_ask: argMaxIfState(price, ts, <predicate>) -- the level-1
--     price as of the latest event time ts within the minute, restricted to the
--     matching side. The -If predicate is the LAST argument of argMaxIfState, so
--     the AggregateFunction signature is argMaxIf(Float64, DateTime64(9), UInt8):
--     value type, argMax tiebreak type, then the UInt8 predicate type LAST.
--   * bid_depth/ask_depth: sumIfState(size, <predicate>) -- cumulative size over
--     all levels of the matching side, so the AggregateFunction signature is
--     sumIf(Float64, UInt8): value type then the UInt8 predicate type.
-- The reader view merges with the matching -IfMerge functions
-- (argMaxIfMerge / sumIfMerge); state and merge spellings are paired so the
-- stored column types and the merge calls stay internally consistent.
--
-- Typing / partition / ttl rationale matches the OHLC analytics tables
-- (see 20260528_0003): symbol plain String, venue LowCardinality(String),
-- window_start DateTime (toStartOfInterval on a DateTime64 returns DateTime),
-- partition by month, order by (symbol, venue, window_start) -- depth stats are
-- per-venue, so venue is part of row identity. TTL is a tunable retention
-- default.
create table if not exists analytics.book_stats_1m (
  symbol String,
  venue LowCardinality(String),
  window_start DateTime,
  -- -If state signatures: predicate (UInt8) is the LAST argument.
  best_bid_state AggregateFunction(argMaxIf, Float64, DateTime64(9), UInt8),
  best_ask_state AggregateFunction(argMaxIf, Float64, DateTime64(9), UInt8),
  bid_depth_state AggregateFunction(sumIf, Float64, UInt8),
  ask_depth_state AggregateFunction(sumIf, Float64, UInt8)
) engine = AggregatingMergeTree
partition by toYYYYMM(window_start)
-- order by keeps venue: depth stats are per-venue, so venue is part of row identity.
order by (symbol, venue, window_start)
ttl window_start + interval 180 day -- tunable: 1m book-stats retention
settings non_replicated_deduplication_window = 1000;

create materialized view if not exists analytics.book_stats_1m_mv
to analytics.book_stats_1m
as select
  symbol,
  venue,
  toStartOfInterval(ts, INTERVAL 1 MINUTE) as window_start,
  argMaxIfState(price, ts, (side = 'bid') and (level = 1)) as best_bid_state,
  argMaxIfState(price, ts, (side = 'ask') and (level = 1)) as best_ask_state,
  sumIfState(size, side = 'bid') as bid_depth_state,
  sumIfState(size, side = 'ask') as ask_depth_state
from raw.book_level
group by symbol, venue, window_start;

-- Reader view: merge the partial states with the paired -IfMerge functions
-- (argMaxIfMerge / sumIfMerge). spread and imbalance are derived columns;
-- imbalance divides by total depth, so nullIf guards the zero-depth minute
-- (returns NULL rather than raising a divide-by-zero).
create view if not exists analytics.book_stats_1m_v
as select
  symbol,
  venue,
  window_start,
  argMaxIfMerge(best_bid_state) as best_bid,
  argMaxIfMerge(best_ask_state) as best_ask,
  sumIfMerge(bid_depth_state) as bid_depth,
  sumIfMerge(ask_depth_state) as ask_depth,
  best_ask - best_bid as spread,
  (bid_depth - ask_depth) / nullIf(bid_depth + ask_depth, 0) as imbalance
from analytics.book_stats_1m
group by symbol, venue, window_start;
