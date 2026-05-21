{{ config(materialized='view', tags=['domain:market_data']) }}

select
  symbol,
  cast(ts as date) as session_date,
  close,
  close - lag(close) over (partition by symbol order by ts) as absolute_return
from {{ ref('silver_ohlcv_adjusted') }}
