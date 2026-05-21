{{ config(materialized='view', tags=['domain:agent']) }}

select
  agent_id,
  dataset_id,
  metric_name,
  max(metric_value) as best_metric_value
from {{ ref('silver_eval_runs') }}
group by agent_id, dataset_id, metric_name
