-- Always-fresh "FlowField" market statistics, derived incrementally from the
-- raw tick/trade/quote flow and from the 1d OHLC candles defined in
-- 20260528_0003_analytics_ohlc_mv.sql.
--
-- FlowField/SIFT analogy: the raw streams (raw.trade, raw.tick, raw.quote) are
-- the unstructured "flow". Incremental materialized views fold each inserted
-- block into partial aggregate states held in AggregatingMergeTree targets (the
-- "field"). Because the MVs fire on every insert, these statistics stay fresh
-- continuously with no scheduled batch job. The reader VIEWs (..._v) merge the
-- partial states into plain numbers (and, for derived stats, compose over the
-- 1d reader view) so callers select clean values.
--
-- Note on `ts`: raw.tick/raw.trade/raw.quote.ts are now typed DateTime64(9,'UTC')
-- (see migration 0002), so toStartOfInterval(ts, ...) and argMax(.., ts) run
-- directly on the column with no parseDateTimeBestEffort wrapping.
-- symbol is plain String (high-cardinality universe, matching the raw sources);
-- venue stays LowCardinality(String). The per-venue stats keep
-- order by (symbol, venue, window_start); high_low_52w keeps order by (symbol).
--
-- Objects defined here:
--   analytics.vwap_1m / _mv / _v       -- per (symbol,venue,minute) VWAP from raw.trade
--   analytics.high_low_52w_v           -- trailing 52-week high/low per symbol (read-time)
--   analytics.quote_stats_1m / _mv / _v -- per (symbol,venue,minute) mid/spread/last from raw.quote
--   analytics.daily_return_v           -- per-day return from the 1d reader view
--   analytics.top_movers_v             -- daily_return ordered by return_pct
--   analytics.rolling_vol_30d_v        -- trailing 30d stddevPop of daily returns per symbol

-- ===================== VWAP per (symbol, venue, minute) =====================
-- VWAP = sum(price*qty) / sum(qty) over the minute bucket. We keep the
-- numerator (pv_state) and denominator (qty_state) as separate sum states so
-- the ratio can be merged correctly across parts at read time.
create table if not exists analytics.vwap_1m (
  symbol String,
  venue LowCardinality(String),
  window_start DateTime,
  pv_state AggregateFunction(sum, Float64),
  qty_state AggregateFunction(sum, Float64)
) engine = AggregatingMergeTree
partition by toYYYYMM(window_start)
order by (symbol, venue, window_start)
ttl window_start + interval 180 day -- tunable: VWAP retention
settings non_replicated_deduplication_window = 1000;

create materialized view if not exists analytics.vwap_1m_mv
to analytics.vwap_1m
as select
  symbol,
  venue,
  toStartOfInterval(ts, INTERVAL 1 MINUTE) as window_start,
  sumState(price * qty) as pv_state,
  sumState(qty) as qty_state
from raw.trade
group by symbol, venue, window_start;

create view if not exists analytics.vwap_1m_v
as select
  symbol,
  venue,
  window_start,
  sumMerge(pv_state) / nullIf(sumMerge(qty_state), 0) as vwap
from analytics.vwap_1m
group by symbol, venue, window_start;

-- ===================== Trailing 52-week high / low per symbol =====================
-- Trailing 52-week (~364d) high/low computed on read from the always-fresh 1d
-- candles -- correct trailing window, no accumulation pruning problem. A plain
-- incremental AggregatingMergeTree MV could only express an ALL-TIME max/min
-- (its states accumulate and are never pruned), so the trailing window is done
-- as a read-time aggregate over analytics.ohlc_1d_v (itself derived
-- incrementally and always fresh) filtered to the trailing 364-day window.
create view if not exists analytics.high_low_52w_v
as select
  symbol,
  max(high) as high_52w,
  min(low) as low_52w
from analytics.ohlc_1d_v
where window_start >= toStartOfDay(now()) - interval 364 day
group by symbol;

-- ===================== Quote stats per (symbol, venue, minute) =====================
-- Per-minute top-of-book stats from raw.quote: average mid price, average
-- bid/ask spread, and the last mid by event time within the minute. mid and
-- spread are kept as separate avg states; last_mid uses an argMax state keyed by
-- the typed event time `ts` (DateTime64(9)) so it merges correctly across parts.
create table if not exists analytics.quote_stats_1m (
  symbol String,
  venue LowCardinality(String),
  window_start DateTime,
  mid_state AggregateFunction(avg, Float64),
  spread_state AggregateFunction(avg, Float64),
  last_mid_state AggregateFunction(argMax, Float64, DateTime64(9))
) engine = AggregatingMergeTree
partition by toYYYYMM(window_start)
order by (symbol, venue, window_start)
ttl window_start + interval 180 day -- tunable: quote-stats retention
settings non_replicated_deduplication_window = 1000;

create materialized view if not exists analytics.quote_stats_1m_mv
to analytics.quote_stats_1m
as select
  symbol,
  venue,
  toStartOfInterval(ts, INTERVAL 1 MINUTE) as window_start,
  avgState((bid + ask) / 2) as mid_state,
  avgState(ask - bid) as spread_state,
  argMaxState((bid + ask) / 2, ts) as last_mid_state
from raw.quote
group by symbol, venue, window_start;

create view if not exists analytics.quote_stats_1m_v
as select
  symbol,
  venue,
  window_start,
  avgMerge(mid_state) as mid,
  avgMerge(spread_state) as spread,
  argMaxMerge(last_mid_state) as last_mid
from analytics.quote_stats_1m
group by symbol, venue, window_start;

-- ===================== Daily return / top movers =====================
-- Plain reader views composed over the always-fresh 1d OHLC reader view; no
-- new MV is needed because analytics.ohlc_1d_v is itself derived incrementally.
create view if not exists analytics.daily_return_v
as select
  symbol,
  venue,
  window_start,
  open,
  close,
  (close - open) / nullIf(open, 0) as return_pct
from analytics.ohlc_1d_v;

create view if not exists analytics.top_movers_v
as select
  symbol,
  venue,
  window_start,
  open,
  close,
  return_pct
from analytics.daily_return_v
order by return_pct desc;

-- ===================== Rolling volatility per symbol (trailing 30d) =====================
-- Trailing 30-day population stddev of daily returns; note other windows
-- (e.g. 90d) can be added as sibling views. Computed on read over the
-- always-fresh daily-return view filtered to the trailing 30 calendar days.
create view if not exists analytics.rolling_vol_30d_v
as select
  symbol,
  stddevPop(return_pct) as volatility,
  count() as sample_days
from analytics.daily_return_v
where window_start >= toStartOfDay(now()) - interval 30 day
group by symbol;
