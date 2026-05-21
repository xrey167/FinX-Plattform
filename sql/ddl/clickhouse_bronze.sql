create table if not exists raw.market_data_bar (
  symbol String,
  venue String,
  granularity String,
  ts DateTime64(3, 'UTC'),
  open Float64,
  high Float64,
  low Float64,
  close Float64,
  volume Float64,
  source String
) engine = MergeTree
order by (symbol, ts);
