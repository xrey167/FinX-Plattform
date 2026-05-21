{{ config(materialized='view', tags=['domain:news']) }}

select
  cast(published_at as date) as session_date,
  source,
  avg(sentiment_score) as avg_sentiment_score,
  count(*) as article_count
from {{ ref('silver_news_normalized') }}
group by cast(published_at as date), source
