create table if not exists raw.market_data_bar (
  symbol text not null,
  venue text not null,
  granularity text not null,
  ts timestamptz not null,
  open double precision not null,
  high double precision not null,
  low double precision not null,
  close double precision not null,
  volume double precision not null,
  source text not null
);

create table if not exists raw.fundamental_metric (
  symbol text not null,
  fiscal_period text not null,
  metric text not null,
  value double precision not null,
  currency text,
  reported_at timestamptz not null
);

create table if not exists raw.news_sentiment (
  id text primary key,
  headline text not null,
  body text not null,
  published_at timestamptz not null,
  sentiment_score double precision not null,
  source text not null
);
