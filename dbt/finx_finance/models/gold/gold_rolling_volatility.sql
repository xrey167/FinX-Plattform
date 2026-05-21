{{ config(materialized='view', tags=['domain:market_data']) }}

select
  symbol,
  session_date,
  stddev_pop(absolute_return) over (
    partition by symbol order by session_date rows between 20 preceding and current row
  ) as rolling_volatility_21d
from {{ ref('gold_daily_returns') }}
