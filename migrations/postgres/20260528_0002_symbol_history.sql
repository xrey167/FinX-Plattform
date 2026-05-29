-- Effective-dated symbol -> instrument map (slowly-changing dimension, SCD type 2)
-- in the `ref` schema. This handles TICKER CHANGES over time -- e.g. FB -> META,
-- GOOG share-class shuffles, post-merger re-listings -- so that a historical bar
-- carrying a given symbol on a given date resolves to the instrument that
-- actually owned that symbol on that date, not whoever holds it today.
--
-- Each row is a closed-open validity interval [valid_from, valid_to) for a
-- (symbol, mic) listing pointing at an instrument_id. valid_to NULL means the
-- interval is still open (current). is_current is a convenience flag for the
-- open row. This is the Postgres system-of-record that the ClickHouse
-- range_hashed dictionary analytics.dict_symbol_history pulls from (via the
-- ranges view below), see migrations/clickhouse/20260528_0009_symbol_dictionaries.sql.
create table if not exists ref.symbol_history (
  symbol text not null,
  mic char(4) not null,
  instrument_id text not null references ref.instrument(instrument_id),
  valid_from date not null,
  valid_to date,
  is_current boolean not null default true,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  primary key (symbol, mic, valid_from)
);

-- Hot lookup paths: resolve by symbol (point-in-time as-of lookups) and by
-- instrument_id (reverse: all historical symbols for an instrument).
create index if not exists idx_symbol_history_symbol        on ref.symbol_history (symbol);
create index if not exists idx_symbol_history_instrument     on ref.symbol_history (instrument_id);

-- Source view for the ClickHouse range_hashed dictionary. The range_hashed
-- layout requires a closed [min, max] range per row, so open intervals
-- (valid_to NULL) are coalesced to a far-future sentinel (2999-12-31) here.
create or replace view ref.v_symbol_history_ranges as
select
  symbol,
  instrument_id,
  mic,
  valid_from,
  coalesce(valid_to, date '2999-12-31') as valid_to
from ref.symbol_history;
