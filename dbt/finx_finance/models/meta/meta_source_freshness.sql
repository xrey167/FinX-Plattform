{{ config(materialized='view') }}

select
  provider as source_name,
  max(fetched_at) as last_fetched_at
from {{ ref('bronze_ohlcv') }}
group by provider
