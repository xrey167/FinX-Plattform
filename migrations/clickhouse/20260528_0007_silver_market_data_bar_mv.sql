-- Bronze -> silver normalization. The bronze landing table raw.equity_historical
-- (daily bars from the yahoo / fileset providers, keyed by a `Date`) is folded
-- incrementally into the canonical silver bar shape raw.market_data_bar by this
-- materialized view -- one normalized row per inserted bronze row, on every
-- insert, with no scheduled batch job.
--
-- Mapping:
--   date  Date            -> ts = toDateTime64(date, 9, 'UTC')   (silver ts grain)
--   granularity (constant) -> '1d'                               (equity history is daily)
--   source      (constant) -> 'equity_historical'                (provenance tag)
--   venue       (enriched)  -> dictGet('analytics.dict_instrument_ref','mic', tuple(symbol))
--   open/high/low/close/volume -> mapped across (volume UInt64 -> Float64 silver)
--
-- `venue` is enriched from the instrument reference dictionary
-- (analytics.dict_instrument_ref, created in 20260528_0005_reference_dictionaries.sql)
-- keyed by symbol; dictGet is resolved at insert time. Referencing the dictionary
-- by name is fine -- it is resolved at runtime, so the apply order of the two
-- migrations does not matter as long as the dictionary exists before rows flow.
create materialized view if not exists analytics.equity_historical_to_bar_mv
to raw.market_data_bar
as select
  symbol,
  dictGet('analytics.dict_instrument_ref', 'mic', tuple(symbol)) as venue,
  '1d' as granularity,
  toDateTime64(date, 9, 'UTC') as ts,
  open,
  high,
  low,
  close,
  toFloat64(volume) as volume,
  'equity_historical' as source
from raw.equity_historical;
