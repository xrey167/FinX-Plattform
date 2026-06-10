-- Bronze landing tables for the keyless Yahoo expansion cluster (gap-matrix
-- item L2.4). Rows are written verbatim from the provider row shapes via
-- INSERT ... FORMAT JSONEachRow by the ingest dispatcher, mirroring the
-- tdw_domain row structs (CompanyProfile, QuoteSnapshot, PricePerformance,
-- OwnershipRecord, Estimate, OptionContract, FuturesCurvePoint) so the column
-- shapes agree byte-for-byte.
--
-- Typing / codec / dedup rationale matches the FRED cluster (see
-- 20260528_0021): high-cardinality identifiers (symbol / underlying) lead the
-- ORDER BY as plain String; low-cardinality descriptive columns use
-- LowCardinality; optional numeric columns are Nullable(Float64). The
-- non-idempotent ingested_at DEFAULT defeats content-hash dedup, so the ingest
-- path supplies an explicit insert_deduplication_token per batch; the
-- non_replicated_deduplication_window below lets that token take effect on
-- these non-replicated MergeTree tables (swap to ReplicatedMergeTree in
-- production). Snapshot-style rows (profile / quote / performance / ownership /
-- estimate) have no natural event date, so they ORDER BY (symbol) and partition
-- by the ingest day; observation-style rows (option chains, futures curve) sort
-- by their natural grain.

-- raw.company_profile mirrors tdw_domain::CompanyProfile: one profile row per
-- ticker (latest write wins on re-ingest via the dedup token).
create table if not exists raw.company_profile (
  ticker String,
  name String,
  currency LowCardinality(String) default '',
  exchange LowCardinality(String) default '',
  logo_url String default '',
  market_cap_millions Float64 codec(ZSTD),
  ingested_at DateTime64(3, 'UTC') default now64(3) codec(DoubleDelta, ZSTD)
) engine = MergeTree
partition by toYYYYMM(toDate(ingested_at))
order by (ticker)
settings non_replicated_deduplication_window = 1000;

-- raw.price_quote mirrors tdw_domain::QuoteSnapshot: one last-price snapshot per
-- (symbol, ts_ms). Distinct from raw.quote (an order-book bid/ask table); this
-- is the no-cache read-path snapshot used by the price-alert engine.
create table if not exists raw.price_quote (
  symbol String,
  current_price Float64 codec(ZSTD),
  change Float64 codec(ZSTD),
  change_percent Float64 codec(ZSTD),
  prev_close Float64 codec(ZSTD),
  ts_ms Int64 codec(DoubleDelta, ZSTD),
  ingested_at DateTime64(3, 'UTC') default now64(3) codec(DoubleDelta, ZSTD)
) engine = MergeTree
partition by toYYYYMM(toDate(ingested_at))
order by (symbol, ts_ms)
ttl toDate(ingested_at) + interval 90 day -- tunable: snapshot retention
settings non_replicated_deduplication_window = 1000;

-- raw.price_performance mirrors tdw_domain::PricePerformance: period total
-- returns (one-day through one-year) for a symbol; one row per ingest.
create table if not exists raw.price_performance (
  symbol String,
  price Nullable(Float64) codec(ZSTD),
  one_day Nullable(Float64) codec(ZSTD),
  one_week Nullable(Float64) codec(ZSTD),
  one_month Nullable(Float64) codec(ZSTD),
  three_month Nullable(Float64) codec(ZSTD),
  ytd Nullable(Float64) codec(ZSTD),
  one_year Nullable(Float64) codec(ZSTD),
  ingested_at DateTime64(3, 'UTC') default now64(3) codec(DoubleDelta, ZSTD)
) engine = MergeTree
partition by toYYYYMM(toDate(ingested_at))
order by (symbol)
settings non_replicated_deduplication_window = 1000;

-- raw.ownership_record mirrors tdw_domain::OwnershipRecord: an ownership /
-- insider / institutional holding row. The kind column discriminates the
-- record variant (e.g. share_statistics).
create table if not exists raw.ownership_record (
  symbol String,
  kind LowCardinality(String),
  holder Nullable(String),
  relationship Nullable(String),
  date Nullable(String),
  transaction_type LowCardinality(Nullable(String)),
  shares Nullable(Float64) codec(ZSTD),
  value Nullable(Float64) codec(ZSTD),
  percentage Nullable(Float64) codec(ZSTD),
  ingested_at DateTime64(3, 'UTC') default now64(3) codec(DoubleDelta, ZSTD)
) engine = MergeTree
partition by toYYYYMM(toDate(ingested_at))
order by (symbol, kind)
settings non_replicated_deduplication_window = 1000;

-- raw.estimate mirrors tdw_domain::Estimate: an analyst estimate / price-target
-- / consensus row. The kind column discriminates the estimate variant.
create table if not exists raw.estimate (
  symbol String,
  kind LowCardinality(String),
  fiscal_period Nullable(String),
  date Nullable(String),
  analyst Nullable(String),
  recommendation LowCardinality(Nullable(String)),
  value Nullable(Float64) codec(ZSTD),
  low Nullable(Float64) codec(ZSTD),
  high Nullable(Float64) codec(ZSTD),
  mean Nullable(Float64) codec(ZSTD),
  number_of_analysts Nullable(UInt32) codec(ZSTD),
  currency LowCardinality(Nullable(String)),
  ingested_at DateTime64(3, 'UTC') default now64(3) codec(DoubleDelta, ZSTD)
) engine = MergeTree
partition by toYYYYMM(toDate(ingested_at))
order by (symbol, kind)
settings non_replicated_deduplication_window = 1000;

-- raw.option_contract mirrors tdw_domain::OptionContract: one row per contract
-- in a chain, keyed by (underlying_symbol, expiration, strike, option_type).
-- Greeks and quote columns are Nullable since provider coverage varies.
create table if not exists raw.option_contract (
  underlying_symbol String,
  contract_symbol Nullable(String),
  expiration String,
  strike Float64 codec(ZSTD),
  option_type LowCardinality(String),
  bid Nullable(Float64) codec(ZSTD),
  ask Nullable(Float64) codec(ZSTD),
  last_price Nullable(Float64) codec(ZSTD),
  volume Nullable(UInt64) codec(ZSTD),
  open_interest Nullable(UInt64) codec(ZSTD),
  implied_volatility Nullable(Float64) codec(ZSTD),
  delta Nullable(Float64) codec(ZSTD),
  gamma Nullable(Float64) codec(ZSTD),
  theta Nullable(Float64) codec(ZSTD),
  vega Nullable(Float64) codec(ZSTD),
  rho Nullable(Float64) codec(ZSTD),
  ingested_at DateTime64(3, 'UTC') default now64(3) codec(DoubleDelta, ZSTD)
) engine = MergeTree
partition by toYYYYMM(toDate(ingested_at))
order by (underlying_symbol, expiration, strike, option_type)
settings non_replicated_deduplication_window = 1000;

-- raw.futures_curve_point mirrors tdw_domain::FuturesCurvePoint: one row per
-- expiry along a root's forward curve, keyed by (underlying, contract_symbol).
create table if not exists raw.futures_curve_point (
  underlying String,
  contract_symbol String,
  price Nullable(Float64) codec(ZSTD),
  expiration Nullable(String),
  ingested_at DateTime64(3, 'UTC') default now64(3) codec(DoubleDelta, ZSTD)
) engine = MergeTree
partition by toYYYYMM(toDate(ingested_at))
order by (underlying, contract_symbol)
settings non_replicated_deduplication_window = 1000;
