{{ config(materialized='view', tags=['domain:agent']) }}

select
  run_id,
  agent_id,
  dataset_id,
  metric_name,
  metric_value,
  started_at,
  finished_at
from {{ source('raw', 'eval_run') }}
