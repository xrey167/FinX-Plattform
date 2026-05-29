-- ClickHouse reference dictionaries for dictGet() enrichment.
--
-- These dictionaries pull reference-master rows from Postgres (the `ref` schema,
-- see migrations/postgres/20260528_0001_reference_master.sql) and cache them in
-- ClickHouse so analytics queries can enrich market-data rows (which only carry
-- symbol / venue) with mic, currency, issuer, sector, etc. via dictGet --
-- without a join back to Postgres on the hot path.
--
-- Connection params (host / port / db / user / password) MUST come from config
-- or environment at deploy time -- do NOT commit real secrets. The source()
-- blocks below use PLACEHOLDER values; substitute them during migration with
-- values resolved from config/env (the same place the rest of the ingest stack
-- reads its Postgres DSN).
--
-- lifetime(min 300 max 600): ClickHouse re-reads the source on a randomized
-- interval between 5 and 10 minutes, so reference edits in Postgres propagate
-- without a manual reload.

-- Instrument reference, keyed by symbol. Single scalar key -> layout(hashed()).
-- Sourced from the Postgres view ref.v_instrument_enriched (instrument joined to
-- instrument_classification) so that `sector_code` actually exists in the source
-- -- pointing this dictionary directly at ref.instrument would fail to load,
-- because ref.instrument has no sector_code column. See
-- migrations/postgres/20260528_0001_reference_master.sql (view definition).
create dictionary if not exists analytics.dict_instrument_ref (
  symbol String,
  mic String,
  currency String,
  issuer_id String,
  sector_code String
)
primary key symbol
source(postgresql(
  host 'PLACEHOLDER_PG_HOST'      -- from config/env, e.g. $TDW_PG_HOST
  port 5432
  db 'PLACEHOLDER_PG_DB'
  user 'PLACEHOLDER_PG_USER'
  password 'PLACEHOLDER_PG_PASSWORD'
  schema 'ref'
  table 'v_instrument_enriched'
))
layout(hashed())
lifetime(min 300 max 600);

-- Classification nodes, keyed by the composite (scheme, code) -> complex_key.
create dictionary if not exists analytics.dict_classification (
  scheme String,
  code String,
  name String,
  level UInt8,
  parent_code String
)
primary key scheme, code
source(postgresql(
  host 'PLACEHOLDER_PG_HOST'
  port 5432
  db 'PLACEHOLDER_PG_DB'
  user 'PLACEHOLDER_PG_USER'
  password 'PLACEHOLDER_PG_PASSWORD'
  schema 'ref'
  table 'classification_node'
))
layout(complex_key_hashed())
lifetime(min 300 max 600);

-- Exchange reference, keyed by mic. Single scalar key -> layout(hashed()).
create dictionary if not exists analytics.dict_exchange (
  mic String,
  name String,
  country_alpha2 String
)
primary key mic
source(postgresql(
  host 'PLACEHOLDER_PG_HOST'
  port 5432
  db 'PLACEHOLDER_PG_DB'
  user 'PLACEHOLDER_PG_USER'
  password 'PLACEHOLDER_PG_PASSWORD'
  schema 'ref'
  table 'exchange'
))
layout(hashed())
lifetime(min 300 max 600);

-- dictGet enrichment examples (run against the dictionaries above):
--
--   -- single scalar key (hashed layout): pass the key value directly
--   select dictGet('analytics.dict_instrument_ref', 'sector_code', tuple('AAPL'));
--   select dictGet('analytics.dict_exchange', 'name', tuple('XNAS'));
--
--   -- composite key (complex_key_hashed layout): pass a tuple of key columns
--   select dictGet('analytics.dict_classification', 'name', tuple('ICB', '101010'));
--
--   -- enrich a raw bar with its sector and exchange name:
--   select
--     b.symbol,
--     dictGet('analytics.dict_instrument_ref', 'mic', tuple(b.symbol))         as mic,
--     dictGet('analytics.dict_instrument_ref', 'sector_code', tuple(b.symbol)) as sector_code,
--     dictGet('analytics.dict_exchange', 'name',
--             tuple(dictGet('analytics.dict_instrument_ref', 'mic', tuple(b.symbol)))) as exchange_name
--   from raw.equity_historical as b;
