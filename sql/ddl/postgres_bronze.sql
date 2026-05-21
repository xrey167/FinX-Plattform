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
