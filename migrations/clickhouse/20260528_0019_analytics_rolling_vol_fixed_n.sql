-- Fixed-N trailing volatility (last N=30 TRADING days), VALIDATED on live ClickHouse.
-- Replaces the now()-relative CALENDAR-window analytics.rolling_vol_30d_v defined
-- in 20260528_0004 with a true fixed-N trailing window: the population standard
-- deviation of a symbol's last 30 daily returns, regardless of calendar gaps
-- (weekends, holidays, halts). Two properties the calendar view lacked:
--   * Fixed sample size — always the last 30 available trading days, not "however
--     many days happened to fall in the trailing 30 calendar days" (which thins
--     out for illiquid names and over holiday stretches).
--   * Reproducible — no now() dependency, so the value is a pure function of the
--     return history and a backtest re-running over the same bars gets the same
--     number.
--
-- Pipeline:
--   daily    : collapse analytics.daily_return_v (which is per symbol+venue+day)
--              to ONE return per (symbol, day) by averaging across venues, matching
--              the prior view's per-symbol, venue-agnostic semantics.
--   windowed : stddevPop over a fixed ROWS frame of the trailing 30 day-rows per
--              symbol (29 PRECEDING .. CURRENT ROW). count() over the same frame
--              exposes how many of the 30 days were actually available (a fresh
--              listing with < 30 days of history reports its true sample size).
--   final    : the most-recent row per symbol (argMax by window_start) is the
--              current trailing-30-trading-day volatility.
--
-- Output columns are a superset of the prior view (symbol, volatility, sample_days)
-- plus as_of, so existing callers keep working.

create or replace view analytics.rolling_vol_30d_v as
with daily as (
  select
    symbol,
    window_start,
    avg(return_pct) as return_pct
  from analytics.daily_return_v
  group by symbol, window_start
),
windowed as (
  select
    symbol,
    window_start,
    stddevPop(return_pct) over (
      partition by symbol
      order by window_start
      rows between 29 preceding and current row
    ) as volatility,
    count() over (
      partition by symbol
      order by window_start
      rows between 29 preceding and current row
    ) as sample_days
  from daily
)
select
  symbol,
  argMax(volatility, window_start) as volatility,
  argMax(sample_days, window_start) as sample_days,
  max(window_start) as as_of
from windowed
group by symbol;
