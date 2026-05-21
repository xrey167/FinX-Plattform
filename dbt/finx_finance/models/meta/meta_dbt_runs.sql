{{ config(materialized='view') }}

select
  '{{ invocation_id }}' as dbt_invocation_id,
  current_timestamp as observed_at,
  '{{ target.name }}' as target_name
