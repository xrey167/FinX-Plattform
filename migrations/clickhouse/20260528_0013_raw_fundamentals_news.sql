-- ClickHouse-native bronze fundamentals & news-sentiment fact tables, plus an
-- always-fresh daily news-sentiment rollup. These mirror the Postgres bronze
-- shapes (migrations/postgres/20260521_0002_bronze_market_data.sql:
-- raw.fundamental_metric, raw.news_sentiment) and the tdw_domain row structs
-- (tdw_domain::FundamentalMetric, tdw_domain::NewsSentiment) so the two stores
-- agree on column shape.
--
-- Typing / codec / dedup rationale matches raw.tick/raw.trade/raw.quote
-- (see 20260528_0002): symbol plain String (high-cardinality leading key),
-- LowCardinality for the handful-of-values columns (metric / source / currency),
-- timestamps as DateTime64(9,'UTC') codec(DoubleDelta, ZSTD), Float64 value /
-- score codec(ZSTD), ingested_at default now64(3). The non-idempotent
-- ingested_at DEFAULT means content-hash dedup is defeated, so an explicit
-- insert_deduplication_token is supplied per batch by the ingest path; the
-- non_replicated_deduplication_window below lets that token take effect on these
-- non-replicated MergeTree tables (swap to ReplicatedMergeTree in production).
-- TTL thresholds are tunable retention defaults.

-- raw.fundamental_metric mirrors tdw_domain::FundamentalMetric and the Postgres
-- raw.fundamental_metric: (symbol, fiscal_period, metric, value, currency,
-- reported_at). currency is nullable in both sources -> Nullable here. ORDER BY
-- (symbol, metric, reported_at) keeps a symbol's metric history contiguous;
-- partition by reported_at month.
create table if not exists raw.fundamental_metric (
  symbol String,
  fiscal_period LowCardinality(String),
  metric LowCardinality(String),
  value Float64 codec(ZSTD),
  currency LowCardinality(Nullable(String)),
  reported_at DateTime64(9, 'UTC') codec(DoubleDelta, ZSTD),
  ingested_at DateTime64(3, 'UTC') default now64(3) codec(DoubleDelta, ZSTD)
) engine = MergeTree
partition by toYYYYMM(reported_at)
order by (symbol, metric, reported_at)
ttl toDateTime(reported_at) + interval 5 year -- tunable: fundamentals retention
settings non_replicated_deduplication_window = 1000;

-- raw.news_sentiment mirrors tdw_domain::NewsSentiment and the Postgres
-- raw.news_sentiment. The domain struct carries a symbols Vec, but news rows are
-- modelled per-symbol here (symbol String leading key) so the per-(symbol, day)
-- rollup below is a clean GROUP BY; the ingest path fans a multi-symbol article
-- out into one row per symbol. body / headline are kept verbatim; published_at
-- is the event time. ORDER BY (symbol, published_at); partition by month.
create table if not exists raw.news_sentiment (
  id String,
  symbol String,
  headline String,
  body String,
  published_at DateTime64(9, 'UTC') codec(DoubleDelta, ZSTD),
  sentiment_score Float64 codec(ZSTD),
  source LowCardinality(String),
  ingested_at DateTime64(3, 'UTC') default now64(3) codec(DoubleDelta, ZSTD)
) engine = MergeTree
partition by toYYYYMM(published_at)
order by (symbol, published_at)
ttl toDateTime(published_at) + interval 2 year -- tunable: news retention
settings non_replicated_deduplication_window = 1000;

-- Always-fresh daily news-sentiment rollup per (symbol, day). avgState folds the
-- mean sentiment and countState folds the article count incrementally on every
-- insert into raw.news_sentiment; the reader view merges those partial states
-- into avg_sentiment + article_count. Same AggregatingMergeTree pattern as the
-- OHLC analytics tables (see 20260528_0003). day is a Date (toStartOfDay on the
-- DateTime64 published_at, cast to Date), partition by month, tunable TTL.
create table if not exists analytics.news_sentiment_daily (
  symbol String,
  day Date,
  avg_sentiment_state AggregateFunction(avg, Float64),
  article_count_state AggregateFunction(count)
) engine = AggregatingMergeTree
partition by toYYYYMM(day)
order by (symbol, day)
ttl day + interval 2 year -- tunable: daily news-sentiment rollup retention
settings non_replicated_deduplication_window = 1000;

create materialized view if not exists analytics.news_sentiment_daily_mv
to analytics.news_sentiment_daily
as select
  symbol,
  toDate(published_at) as day,
  avgState(sentiment_score) as avg_sentiment_state,
  countState() as article_count_state
from raw.news_sentiment
group by symbol, day;

-- Reader view: merge the partial states (avgMerge / countMerge) into plain
-- numbers -- the daily mean sentiment and the article count per (symbol, day).
create view if not exists analytics.news_sentiment_daily_v
as select
  symbol,
  day,
  avgMerge(avg_sentiment_state) as avg_sentiment,
  countMerge(article_count_state) as article_count
from analytics.news_sentiment_daily
group by symbol, day;
