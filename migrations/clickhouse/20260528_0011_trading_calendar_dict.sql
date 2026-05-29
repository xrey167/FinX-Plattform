-- ClickHouse dictionary over the Postgres per-venue trading calendar
-- (ref.trading_calendar, see migrations/postgres/20260528_0003_trading_calendar.sql),
-- added alongside the other analytics.dict_* dictionaries -- this file does NOT
-- edit 20260528_0005 / 20260528_0009.
--
-- The key is composite -- (mic, session_date) -- so the layout is
-- complex_key_hashed(). open_time/close_time are surfaced as String here
-- (ClickHouse dictionaries cannot type a Postgres `time` column directly);
-- callers parse them if needed. lifetime(min 3600 max 7200): the calendar
-- changes rarely, so ClickHouse re-reads the source on a randomized 1-2 hour
-- interval.
--
-- Connection params (host / port / db / user / password) are PLACEHOLDER values
-- resolved from config/env at deploy time (the same place the rest of the
-- ingest stack reads its Postgres DSN) -- do NOT commit real secrets.
create dictionary if not exists analytics.dict_trading_calendar (
  mic String,
  session_date Date,
  is_trading_day UInt8,
  open_time String,
  close_time String,
  tz String
)
primary key mic, session_date
source(postgresql(
  host 'PLACEHOLDER_PG_HOST'      -- from config/env, e.g. $TDW_PG_HOST
  port 5432
  db 'PLACEHOLDER_PG_DB'
  user 'PLACEHOLDER_PG_USER'
  password 'PLACEHOLDER_PG_PASSWORD'
  schema 'ref'
  table 'trading_calendar'
))
layout(complex_key_hashed())
lifetime(min 3600 max 7200);

-- dictGet example -- is the Nasdaq (XNAS) open today?
--
--   select dictGet('analytics.dict_trading_calendar', 'is_trading_day',
--                  tuple('XNAS', toDate(now())));
