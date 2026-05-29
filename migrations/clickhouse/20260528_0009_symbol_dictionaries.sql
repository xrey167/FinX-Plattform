-- Symbol-keyed ClickHouse dictionaries for dictGet() enrichment, added alongside
-- the reference-master dictionaries in 20260528_0005 (this file does NOT edit
-- that one). Two dictionaries:
--
--   analytics.dict_symbol_history -- POINT-IN-TIME symbol -> instrument_id map,
--       sourced from Postgres ref.v_symbol_history_ranges (effective-dated SCD).
--       range_hashed layout with range(min valid_from max valid_to) so a lookup
--       supplies the as-of date and gets the instrument that owned the symbol on
--       that date -- this is what makes a ticker change (e.g. FB -> META) resolve
--       historical bars to the correct instrument for their date.
--
--   analytics.dict_symbol_info    -- CURRENT descriptive listing metadata,
--       sourced from the CH-native reference.symbol_info_current_v view.
--
-- Connection params for the Postgres source (host / port / db / user / password)
-- are PLACEHOLDER values resolved from config/env at deploy time (the same place
-- the rest of the ingest stack reads its Postgres DSN) -- do NOT commit real
-- secrets. lifetime(min 300 max 600): ClickHouse re-reads the source on a
-- randomized 5-10 minute interval, so edits propagate without a manual reload.

-- Point-in-time symbol -> instrument map. Single scalar key `symbol` plus a
-- [valid_from, valid_to] validity range -> layout(range_hashed()). The source
-- view coalesces open intervals (valid_to NULL -> 2999-12-31) so every row has a
-- closed range the range_hashed layout can index.
create dictionary if not exists analytics.dict_symbol_history (
  symbol String,
  instrument_id String,
  mic String,
  valid_from Date,
  valid_to Date
)
primary key symbol
source(postgresql(
  host 'PLACEHOLDER_PG_HOST'      -- from config/env, e.g. $TDW_PG_HOST
  port 5432
  db 'PLACEHOLDER_PG_DB'
  user 'PLACEHOLDER_PG_USER'
  password 'PLACEHOLDER_PG_PASSWORD'
  schema 'ref'
  table 'v_symbol_history_ranges'
))
layout(range_hashed())
range(min valid_from max valid_to)
lifetime(min 300 max 600);

-- Current descriptive listing metadata, sourced from the CH-native
-- reference.symbol_info_current_v view (clickhouse source, not Postgres).
-- LAYOUT CHOICE: the primary key is a single scalar String `symbol`, so
-- layout(hashed()) is used -- it is the correct/cleaner layout for a single
-- key. (complex_key_hashed() is reserved for composite or tuple keys; using it
-- for one scalar key would be unnecessary.)
create dictionary if not exists analytics.dict_symbol_info (
  symbol String,
  mic String,
  instrument_id String,
  name String,
  listing_currency String,
  asset_class String,
  status String
)
primary key symbol
source(clickhouse(
  db 'reference'
  table 'symbol_info_current_v'
))
layout(hashed())
lifetime(min 300 max 600);

-- dictGet enrichment examples (run against the dictionaries above):
--
--   -- point-in-time: which instrument did `symbol` map to as of ts's date?
--   -- (range_hashed: pass the key, then the as-of date for the range match)
--   select dictGet('analytics.dict_symbol_history', 'instrument_id',
--                  tuple(symbol), toDate(ts));
--
--   -- current descriptive metadata (hashed, single scalar key):
--   select dictGet('analytics.dict_symbol_info', 'name', tuple(symbol));
--   select dictGet('analytics.dict_symbol_info', 'asset_class', tuple(symbol));
