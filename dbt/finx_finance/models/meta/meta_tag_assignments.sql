{{ config(materialized='view', tags=['lineage', 'domain:tags']) }}

select
  'instrument:AAPL' as entity_id,
  'asset:equity' as tag_id,
  'manual:seed' as provenance,
  cast('2026-05-21' as date) as assigned_at
