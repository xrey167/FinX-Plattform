-- CH-native symbol-info dimension: descriptive listing metadata (name, currency,
-- asset class, lot size, status, primary-listing flag) keyed by symbol. This is
-- kept SEPARATE from the lean, symbol-keyed fact tables (raw.tick / raw.trade /
-- raw.quote / raw.market_data_bar) so those stay narrow and fast; descriptive
-- attributes live here and are joined/enriched on demand (e.g. via the
-- analytics.dict_symbol_info dictionary in 20260528_0009).
--
-- ReplacingMergeTree(updated_at): the version column is updated_at, so on merge
-- the row with the greatest updated_at wins (latest-wins per (symbol, mic)).
-- Because background merges are asynchronous, callers MUST read the
-- reference.symbol_info_current_v view below (which applies FINAL) rather than
-- the raw table, to always see the current de-duplicated snapshot.
create database if not exists reference;

create table if not exists reference.symbol_info (
  symbol String,
  mic LowCardinality(String),
  instrument_id String,
  name String,
  listing_currency LowCardinality(String),
  asset_class LowCardinality(String),
  lot_size UInt32 default 1,
  status LowCardinality(String) default 'active',
  is_primary UInt8 default 1,
  updated_at DateTime64(3, 'UTC') default now64(3)
) engine = ReplacingMergeTree(updated_at)
order by (symbol, mic);

-- Current-snapshot reader view. FINAL collapses the ReplacingMergeTree versions
-- to the latest row per (symbol, mic) at query time. This dimension is small, so
-- the FINAL cost is acceptable; callers should read THIS view, not the raw
-- reference.symbol_info table, to avoid seeing pre-merge duplicates.
create view if not exists reference.symbol_info_current_v as
select
  symbol,
  mic,
  instrument_id,
  name,
  listing_currency,
  asset_class,
  lot_size,
  status,
  is_primary,
  updated_at
from reference.symbol_info final;
