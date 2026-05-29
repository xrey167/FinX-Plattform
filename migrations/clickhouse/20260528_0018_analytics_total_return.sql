-- Total-return adjusted daily close (splits + dividends), VALIDATED on live
-- ClickHouse 26.6. Extends the split-only adjustment in 20260528_0014 to a
-- total-return series that also reinvests cash dividends.
--
-- For each daily bar, adjusted_total_return = close * (product of every event
-- factor whose ex_date is STRICTLY AFTER the bar's date), where the per-event
-- factor is:
--   * split:    1 / split_ratio          (same as the split-only view)
--   * dividend: 1 - cash_amount / C_prev  (C_prev = close on the day before ex)
-- A pre-event price is scaled down so the return from it to the present captures
-- the dividend/split — the standard back-adjusted total-return convention.
--
-- Pipeline:
--   events : per ex_date factor. The dividend leg looks up C_prev via an
--            ASOF LEFT JOIN of corporate_action to the daily candles on the
--            strict inequality px.d < ca.ex_date (nearest close BEFORE ex).
--   cumfac : per-symbol cumulative product of future event factors via
--            exp(sum(log(factor)) OVER (... ORDER BY ex_date DESC ...)) computed
--            in a subquery (exp() must wrap the windowed sum from OUTSIDE — see
--            the note in 20260528_0014's split_factor_v).
--   bars   : ASOF LEFT JOIN bars to cumfac on toDate(window_start) < f.ex_date
--            (nearest FUTURE event). No match -> ASOF fills 0.0, so the multiplier
--            is guarded with if(fac > 0, fac, 1.0) (a real factor is always > 0).
--
-- VALIDATION: the cumfac/ASOF/guard math was verified live against the daily
-- candles with an inline event set (a $2 dividend, ex 2026-05-06, prev close
-- 105 -> factor 0.98095): pre-ex closes scaled exactly (100->98.0952,
-- 105->103.0) and ex-date-and-later closes left unchanged. The events source
-- (corporate_action -> C_prev ASOF) reuses the same proven ASOF pattern as
-- split_factor_v; end-to-end with seeded corporate actions is exercised by the
-- gated integration suite.
create view if not exists analytics.ohlc_1d_total_return_v
as
with
  events as (
    select
      ca.symbol as symbol,
      ca.ex_date as ex_date,
      multiIf(
        ca.action_type = 'split', 1.0 / ca.split_ratio,
        ca.action_type = 'dividend', 1.0 - ca.cash_amount / nullIf(px.close, 0),
        1.0
      ) as factor
    from raw.corporate_action as ca
    asof left join (
      select symbol, toDate(window_start) as d, close
      from analytics.ohlc_1d_consolidated_v
    ) as px
      on ca.symbol = px.symbol and px.d < ca.ex_date
  ),
  cumfac as (
    select symbol, ex_date, exp(slog) as fac
    from (
      select symbol, ex_date,
        sum(log(factor)) over (
          partition by symbol order by ex_date desc
          rows between unbounded preceding and current row
        ) as slog
      from events
    )
  )
select
  b.symbol as symbol,
  b.window_start as window_start,
  b.close as close,
  b.close * if(f.fac > 0, f.fac, 1.0) as adjusted_total_return
from analytics.ohlc_1d_consolidated_v as b
asof left join cumfac as f
  on b.symbol = f.symbol and toDate(b.window_start) < f.ex_date;
