-- FX spot rates and a latest-rate reader view, plus an optional dictionary for
-- dictGet-based currency conversion. raw.fx_rate is the bronze landing table for
-- (base, quote) exchange-rate ticks; analytics.fx_rate_latest_v exposes the most
-- recent rate per currency pair.
--
-- Typing / codec / dedup rationale matches the other bronze tables
-- (see 20260528_0002 / 0013): LowCardinality for base_currency / quote_currency /
-- source (small fixed universe of ISO currency codes / feed names); ts as
-- DateTime64(9,'UTC') codec(DoubleDelta, ZSTD); rate Float64 codec(ZSTD);
-- ingested_at default now64(3). The non-idempotent ingested_at DEFAULT defeats
-- content-hash dedup, so the ingest path supplies an explicit
-- insert_deduplication_token per batch; the non_replicated_deduplication_window
-- below lets that token take effect on this non-replicated MergeTree table
-- (swap to ReplicatedMergeTree in production). TTL is a tunable retention default.
--
-- ORDER BY (base_currency, quote_currency, ts) keeps each pair's rate history
-- contiguous; partition by ts month.
create table if not exists raw.fx_rate (
  base_currency LowCardinality(String),
  quote_currency LowCardinality(String),
  ts DateTime64(9, 'UTC') codec(DoubleDelta, ZSTD),
  rate Float64 codec(ZSTD),
  source LowCardinality(String) default '',
  ingested_at DateTime64(3, 'UTC') default now64(3) codec(DoubleDelta, ZSTD)
) engine = MergeTree
partition by toYYYYMM(ts)
order by (base_currency, quote_currency, ts)
ttl toDateTime(ts) + interval 5 year -- tunable: fx-rate retention
settings non_replicated_deduplication_window = 1000;

-- Latest rate per (base_currency, quote_currency): the rate of the row with the
-- maximum ts, via argMax(rate, ts). latest_ts is the max ts itself.
--
-- CERTAINTY:
--   * argMax(rate, ts): CERTAIN -- standard ClickHouse aggregate returning the
--     `rate` value associated with the maximum `ts`. Used the same way as
--     argMax/argMin already appear in 20260528_0003.
--   * max(ts): CERTAIN -- standard aggregate.
create view if not exists analytics.fx_rate_latest_v
as select
  base_currency,
  quote_currency,
  argMax(rate, ts) as rate,
  max(ts) as latest_ts
from raw.fx_rate
group by base_currency, quote_currency;

-- Optional dictionary for dictGet conversion, sourced from the CH-native
-- analytics.fx_rate_latest_v view (clickhouse source, mirroring dict_symbol_info
-- in 20260528_0009). The key is COMPOSITE -- (base_currency, quote_currency) --
-- so layout(complex_key_hashed()) is the correct layout (the single-scalar-key
-- hashed() layout used for dict_symbol_info does not apply to a two-column key).
--
-- CERTAINTY:
--   * clickhouse source + complex_key_hashed layout for a composite key: CERTAIN
--     of the form. The clickhouse-source dict syntax is exercised in 20260528_0009
--     (dict_symbol_info), and complex_key_hashed() is the documented layout for
--     multi-column keys. lifetime(min 300 max 600) matches the existing dicts.
--
-- dictGet usage:
--   select dictGet('analytics.dict_fx_latest', 'rate',
--                  tuple(base_currency, quote_currency));
create dictionary if not exists analytics.dict_fx_latest (
  base_currency String,
  quote_currency String,
  rate Float64
)
primary key base_currency, quote_currency
source(clickhouse(
  db 'analytics'
  table 'fx_rate_latest_v'
))
layout(complex_key_hashed())
lifetime(min 300 max 600);
