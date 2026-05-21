{{ config(materialized='view', tags=['domain:market_data']) }}

select
  symbol,
  venue,
  ts,
  open,
  high,
  low,
  close,
  volume,
  provider,
  fetched_at
from {{ ref('bronze_ohlcv') }}
where close >= 0 and volume >= 0
