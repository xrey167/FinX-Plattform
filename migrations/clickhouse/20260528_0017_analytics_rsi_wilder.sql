-- Wilder-style RSI (14-bar), VALIDATED on live ClickHouse 26.6.
--
-- Complements the Cutler RSI in 20260528_0015 (which uses a SIMPLE moving
-- average of gains/losses). Wilder's RSI smooths gains/losses with a running
-- moving average (RMA) — an EMA with alpha = 1/N. ClickHouse has no recursive
-- RMA primitive, but exponentialMovingAverage(halflife)(value, timeunit) is an
-- EMA and approximates Wilder's RMA when the halflife matches alpha = 1/N:
--   alpha = 1/N  <=>  (1 - alpha) = 2^(-1/halflife)
--   => halflife = ln(2) / -ln(1 - 1/N)
-- For N = 14: halflife = 0.6931 / -ln(13/14) = 0.6931 / 0.07411 ≈ 9.35.
--
-- APPROXIMATION CAVEAT: this is a halflife-based EMA, not the exact recursive
-- Wilder RMA (which seeds the first value with an N-period SMA). The curve
-- tracks Wilder closely once warmed up, but the first ~N bars are a warm-up
-- (ramp from 0; avgLoss is nullIf-guarded so an all-gains window yields NULL).
-- The exact recursive RMA seed would require a windowed/recursive computation
-- not expressible as a single ClickHouse aggregate; that exactness is a
-- follow-up (app-side or a UDF).
--
-- timeunit is a per-symbol bar index (row_number) so the EMA decays one step per
-- bar regardless of calendar gaps.
create view if not exists analytics.rsi_14_wilder_v
as select
  symbol,
  window_start,
  close,
  100 - 100 / (1 + avg_gain / nullIf(avg_loss, 0)) as rsi_14_wilder
from (
  select
    symbol,
    window_start,
    close,
    exponentialMovingAverage(9.35)(gain, bar_idx) OVER (
      PARTITION BY symbol ORDER BY window_start
      ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW
    ) as avg_gain,
    exponentialMovingAverage(9.35)(loss, bar_idx) OVER (
      PARTITION BY symbol ORDER BY window_start
      ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW
    ) as avg_loss
  from (
    select
      symbol,
      window_start,
      close,
      row_number() OVER (PARTITION BY symbol ORDER BY window_start) as bar_idx,
      greatest(close - lagInFrame(close) OVER (PARTITION BY symbol ORDER BY window_start ROWS BETWEEN 1 PRECEDING AND CURRENT ROW), 0) as gain,
      greatest(lagInFrame(close) OVER (PARTITION BY symbol ORDER BY window_start ROWS BETWEEN 1 PRECEDING AND CURRENT ROW) - close, 0) as loss
    from analytics.ohlc_1d_consolidated_v
  )
);
