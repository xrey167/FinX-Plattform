{{ config(materialized='view', tags=['domain:market_data']) }}

select
  {{ clean_symbol('symbol') }} as symbol,
  venue,
  ts,
  open,
  high,
  low,
  close,
  volume,
  source as provider,
  current_timestamp as fetched_at
from {{ source('raw', 'market_data_bar') }}
where {{ business_day_only('ts') }}
