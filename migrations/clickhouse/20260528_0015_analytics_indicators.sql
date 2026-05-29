-- Technical indicators derived from the consolidated daily candle
-- (analytics.ohlc_1d_consolidated_v from 20260528_0003). These are pure
-- read-time views over the existing OHLC aggregates -- no new storage / MV.
--
-- analytics.sma_v exposes the 20 / 50 / 200-day Simple Moving Averages of the
-- consolidated daily close. Each SMA is a trailing window AVERAGE of `close` over
-- the last N daily bars (current bar inclusive), computed per symbol in
-- window_start order.
--
-- CERTAINTY:
--   * avg(close) OVER (PARTITION BY symbol ORDER BY window_start
--     ROWS BETWEEN <n> PRECEDING AND CURRENT ROW): CERTAIN. This is a standard
--     SQL windowed moving average and is well supported by ClickHouse. sma_20
--     uses 19 PRECEDING (19 prior + current = 20 rows), sma_50 uses 49 PRECEDING,
--     sma_200 uses 199 PRECEDING. Near the start of a symbol's history the window
--     simply contains fewer rows (a partial average), which is the expected SMA
--     behaviour.
--
-- NOTE: the trailing window counts BARS, not calendar days -- it assumes one
-- consolidated bar per trading day (which analytics.ohlc_1d_consolidated_v
-- provides), so a non-trading day is simply absent and does not dilute the
-- average.
create view if not exists analytics.sma_v
as select
  symbol,
  window_start,
  close,
  avg(close) OVER (
    PARTITION BY symbol ORDER BY window_start
    ROWS BETWEEN 19 PRECEDING AND CURRENT ROW
  ) as sma_20,
  avg(close) OVER (
    PARTITION BY symbol ORDER BY window_start
    ROWS BETWEEN 49 PRECEDING AND CURRENT ROW
  ) as sma_50,
  avg(close) OVER (
    PARTITION BY symbol ORDER BY window_start
    ROWS BETWEEN 199 PRECEDING AND CURRENT ROW
  ) as sma_200
from analytics.ohlc_1d_consolidated_v;

-- EMA (exponential moving average), VALIDATED on live ClickHouse 26.6.
-- ClickHouse's `exponentialMovingAverage(halflife)(value, timeunit)` works as a
-- window function: `(halflife)` is a parenthesised aggregate PARAMETER, `value` is
-- the close, and `timeunit` is an increasing time index the decay is measured
-- against -- here toRelativeDayNum(window_start) (whole days since the epoch).
-- The weighting is HALF-LIFE based: a value loses half its weight every
-- `halflife` days. The halflives (12 / 26 days) echo the classic MACD pair and
-- are tunable. NOTES: (1) this is half-life weighting, not the span-based
-- alpha=2/(N+1) EMA -- pick halflife to taste; (2) the function ramps from 0, so
-- the first bars of a symbol's history are a warm-up and converge to price once
-- enough history (>> halflife) has accumulated.
create view if not exists analytics.ema_v
as select
  symbol,
  window_start,
  close,
  exponentialMovingAverage(12)(close, toRelativeDayNum(window_start)) OVER (
    PARTITION BY symbol ORDER BY window_start
    ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW
  ) as ema_12,
  exponentialMovingAverage(26)(close, toRelativeDayNum(window_start)) OVER (
    PARTITION BY symbol ORDER BY window_start
    ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW
  ) as ema_26
from analytics.ohlc_1d_consolidated_v;

-- RSI (Relative Strength Index), 14-bar, VALIDATED on live ClickHouse 26.6.
-- This is "Cutler's RSI": it uses a SIMPLE moving average of gains/losses over the
-- trailing 14 bars rather than Wilder's recursive smoothing (which has no direct
-- ClickHouse primitive). Pipeline (inner -> outer):
--   1. prior close via lagInFrame over the per-symbol ordered series; split the
--      close-over-close delta into gain = greatest(delta,0) and loss = greatest(-delta,0).
--   2. avg gain / avg loss over the trailing 14 bars (13 PRECEDING + current).
--   3. rsi = 100 - 100 / (1 + avgGain/avgLoss); avgLoss is nullIf-guarded so an
--      all-gains window yields NULL (RSI undefined / -> 100).
-- WARM-UP: the first bar per symbol has no prior close (lagInFrame default 0), so
-- its gain is inflated -- treat the leading 14 bars as warm-up.
-- FOLLOW-UP: Wilder's smoothing (the "classic" RSI) is a documented refinement.
create view if not exists analytics.rsi_14_v
as select
  symbol,
  window_start,
  close,
  100 - 100 / (1 + avg_gain / nullIf(avg_loss, 0)) as rsi_14
from (
  select
    symbol,
    window_start,
    close,
    avg(gain) OVER (PARTITION BY symbol ORDER BY window_start ROWS BETWEEN 13 PRECEDING AND CURRENT ROW) as avg_gain,
    avg(loss) OVER (PARTITION BY symbol ORDER BY window_start ROWS BETWEEN 13 PRECEDING AND CURRENT ROW) as avg_loss
  from (
    select
      symbol,
      window_start,
      close,
      greatest(close - lagInFrame(close) OVER (PARTITION BY symbol ORDER BY window_start ROWS BETWEEN 1 PRECEDING AND CURRENT ROW), 0) as gain,
      greatest(lagInFrame(close) OVER (PARTITION BY symbol ORDER BY window_start ROWS BETWEEN 1 PRECEDING AND CURRENT ROW) - close, 0) as loss
    from analytics.ohlc_1d_consolidated_v
  )
);
