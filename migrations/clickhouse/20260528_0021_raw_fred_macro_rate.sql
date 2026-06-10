-- Bronze landing tables for the FRED macro / rate / fixedincome cluster
-- (gap-matrix item L2.3). Rows are written verbatim from the provider row shapes
-- via INSERT ... FORMAT JSONEachRow by the ingest dispatcher, mirroring the
-- tdw_domain row structs (tdw_domain::MacroSeries, RateObservation,
-- YieldCurvePoint, SeriesSearchResult) so the column shapes agree byte-for-byte.
--
-- Typing / codec / dedup rationale matches raw.fundamental_metric
-- (see 20260528_0013): the high-cardinality identifier (series_id / rate_id /
-- curve_id) is a plain String leading key; the low-cardinality descriptive
-- columns (frequency / unit / maturity / currency) use LowCardinality; `value`
-- is Nullable(Float64) because FRED reports missing observations. The
-- non-idempotent ingested_at DEFAULT defeats content-hash dedup, so the ingest
-- path supplies an explicit insert_deduplication_token per batch; the
-- non_replicated_deduplication_window below lets that token take effect on these
-- non-replicated MergeTree tables (swap to ReplicatedMergeTree in production).
-- `date` is a Date (these are daily/weekly/monthly observation series; day
-- resolution is exact and the natural partition grain). TTL thresholds are
-- tunable retention defaults.

-- raw.macro_series mirrors tdw_domain::MacroSeries: one (series_id, date)
-- macroeconomic observation. ORDER BY (series_id, date) keeps a series'
-- history contiguous; partition by observation month.
create table if not exists raw.macro_series (
  series_id String,
  title Nullable(String),
  date Date codec(DoubleDelta, ZSTD),
  value Nullable(Float64) codec(ZSTD),
  country LowCardinality(Nullable(String)),
  frequency LowCardinality(Nullable(String)),
  unit LowCardinality(Nullable(String)),
  transform LowCardinality(Nullable(String)),
  ingested_at DateTime64(3, 'UTC') default now64(3) codec(DoubleDelta, ZSTD)
) engine = MergeTree
partition by toYYYYMM(date)
order by (series_id, date)
ttl date + interval 10 year -- tunable: macro history retention
settings non_replicated_deduplication_window = 1000;

-- raw.rate_observation mirrors tdw_domain::RateObservation: one (rate_id, date)
-- interest-rate / spread observation, optionally tagged with a maturity tenor.
create table if not exists raw.rate_observation (
  rate_id String,
  date Date codec(DoubleDelta, ZSTD),
  value Nullable(Float64) codec(ZSTD),
  maturity LowCardinality(Nullable(String)),
  currency LowCardinality(Nullable(String)),
  ingested_at DateTime64(3, 'UTC') default now64(3) codec(DoubleDelta, ZSTD)
) engine = MergeTree
partition by toYYYYMM(date)
order by (rate_id, date)
ttl date + interval 10 year -- tunable: rate history retention
settings non_replicated_deduplication_window = 1000;

-- raw.yield_curve_point mirrors tdw_domain::YieldCurvePoint: one (curve_id,
-- date, maturity) constant-maturity yield. ORDER BY (curve_id, date, maturity)
-- keeps a single day's curve contiguous; partition by observation month.
create table if not exists raw.yield_curve_point (
  curve_id String,
  date Date codec(DoubleDelta, ZSTD),
  maturity LowCardinality(String),
  value Nullable(Float64) codec(ZSTD),
  currency LowCardinality(Nullable(String)),
  ingested_at DateTime64(3, 'UTC') default now64(3) codec(DoubleDelta, ZSTD)
) engine = MergeTree
partition by toYYYYMM(date)
order by (curve_id, date, maturity)
ttl date + interval 10 year -- tunable: yield-curve history retention
settings non_replicated_deduplication_window = 1000;

-- raw.series_search_result mirrors tdw_domain::SeriesSearchResult: a FRED
-- series-search discovery row (metadata, no observations). No event date, so
-- ORDER BY (series_id); ingested_at is the only time column. No TTL by default
-- (discovery rows are small and useful as a durable series directory).
create table if not exists raw.series_search_result (
  series_id String,
  title Nullable(String),
  frequency LowCardinality(Nullable(String)),
  units Nullable(String),
  popularity Nullable(Int64) codec(ZSTD),
  ingested_at DateTime64(3, 'UTC') default now64(3) codec(DoubleDelta, ZSTD)
) engine = MergeTree
order by (series_id)
settings non_replicated_deduplication_window = 1000;
