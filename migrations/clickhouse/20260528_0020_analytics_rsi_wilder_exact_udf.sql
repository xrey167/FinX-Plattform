-- EXACT recursive Wilder RSI (14-bar) via ClickHouse SQL UDFs, VALIDATED on live
-- ClickHouse 26.6 (clickhouse local).
--
-- Supersedes the halflife-EMA APPROXIMATION in 20260528_0017. That view used
-- exponentialMovingAverage(9.35) to approximate Wilder's running moving average
-- (RMA); it tracks Wilder once warmed up but never reproduces the exact recursive
-- series (which seeds with an N-period SMA and then smooths recursively). The
-- comment in 0017 flagged the exact form as "a follow-up (app-side or a UDF)" —
-- this migration delivers it.
--
-- ClickHouse has no recursive-RMA window primitive, but arrayFold (left fold over
-- an array with a stateful accumulator, signature
-- `arrayFold((acc, x) -> acc, array, initial)`) expresses the recurrence exactly.
-- We carry a tuple accumulator (avgArr, seedSum):
--   * i < N  (warm-up)      -> append nan; keep summing the first N values.
--   * i = N  (seed)         -> append the N-period SMA  (seedSum + x) / N.
--   * i > N  (recursive RMA)-> append (prev * (N-1) + x) / N.
-- Verified for N=3 on [1,2,3,4,5] -> [nan, nan, 2, 2.6667, 3.4444].

-- RMA(values, period): exact recursive Wilder running moving average. The first
-- (period-1) entries are nan (warm-up); entry `period` is the SMA seed; the rest
-- are the recursive smoothing.
create function if not exists tdw_wilder_rma as (values, period) ->
  arrayFold(
    (acc, x) -> (
      arrayPushBack(
        acc.1,
        multiIf(
          length(acc.1) + 1 < period, nan,
          length(acc.1) + 1 = period, (acc.2 + x) / period,
          (acc.1[-1] * (period - 1) + x) / period
        )
      ),
      acc.2 + x
    ),
    values,
    CAST(([], CAST(0, 'Float64')), 'Tuple(Array(Float64), Float64)')
  ).1;

-- RSI(gains, losses, period): exact Wilder RSI series from per-bar gain/loss
-- arrays. Elementwise over the two RMAs: nan while either RMA is warming up;
-- 50 for a flat series (both averages 0); 100 when only losses are 0; otherwise
-- 100 - 100 / (1 + avgGain/avgLoss).
create function if not exists tdw_rsi_wilder as (gains, losses, period) ->
  arrayMap(
    (g, l) -> multiIf(
      isNaN(g) or isNaN(l), nan,
      l = 0 and g = 0, CAST(50, 'Float64'),
      l = 0, CAST(100, 'Float64'),
      100 - 100 / (1 + g / l)
    ),
    tdw_wilder_rma(gains, period),
    tdw_wilder_rma(losses, period)
  );

-- Exact Wilder RSI-14 reader view, replacing the 0017 approximation in place
-- (same name + primary columns: symbol, window_start, close, rsi_14_wilder) so
-- existing callers keep working. Per symbol we order bars, take close-to-close
-- changes (bars 2..M), fold them into the exact RSI series, and re-expand to one
-- row per bar via ARRAY JOIN. Warm-up bars (nan) surface as NULL. Bar 1 has no
-- prior close, so it produces no RSI row.
create or replace view analytics.rsi_14_wilder_v as
with per_symbol as (
  select
    symbol,
    arraySort(p -> p.1, groupArray((window_start, close))) as pairs
  from analytics.ohlc_1d_consolidated_v
  group by symbol
),
series as (
  select
    symbol,
    arrayMap(p -> p.1, pairs) as ws_all,
    arrayMap(p -> p.2, pairs) as closes_all,
    length(pairs) as n,
    -- close-to-close changes for bars 2..n (the later bar carries each change).
    arraySlice(closes_all, 2) as closes_tail,
    arraySlice(closes_all, 1, n - 1) as closes_head,
    arrayMap((a, b) -> greatest(a - b, 0), closes_tail, closes_head) as gains,
    arrayMap((a, b) -> greatest(b - a, 0), closes_tail, closes_head) as losses,
    arraySlice(ws_all, 2) as ws_change,
    arraySlice(closes_all, 2) as close_change,
    tdw_rsi_wilder(gains, losses, 14) as rsi_arr
  from per_symbol
  where n >= 2
)
select
  symbol,
  bar.1 as window_start,
  bar.2 as close,
  if(isNaN(bar.3), NULL, bar.3) as rsi_14_wilder
from series
array join arrayZip(ws_change, close_change, rsi_arr) as bar;
