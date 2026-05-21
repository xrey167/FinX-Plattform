{{ config(materialized='view', tags=['domain:fundamentals']) }}

select
  {{ clean_symbol('symbol') }} as symbol,
  fiscal_period,
  metric,
  value,
  currency,
  reported_at
from {{ source('raw', 'fundamental_metric') }}
