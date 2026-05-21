{{ config(materialized='view', tags=['domain:news']) }}

select
  id,
  headline,
  body,
  published_at,
  sentiment_score,
  source
from {{ source('raw', 'news_sentiment') }}
where sentiment_score between -1 and 1
