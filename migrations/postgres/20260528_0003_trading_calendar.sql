-- Per-venue trading sessions / holiday calendar (the `ref` schema). One row per
-- (mic, session_date) records whether that venue trades that day and, when it
-- does, the local session open/close times and timezone. This enables
-- session-aware aggregation (align bars/stats to real trading sessions) and
-- "is the market open" checks, and is the system-of-record the ClickHouse
-- analytics.dict_trading_calendar dictionary pulls from
-- (see migrations/clickhouse/20260528_0011_trading_calendar_dict.sql).
create table if not exists ref.trading_calendar (
  mic char(4) not null references ref.exchange(mic),
  session_date date not null,
  is_trading_day boolean not null default true,
  open_time time,
  close_time time,
  tz text,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  primary key (mic, session_date)
);

-- Date-leading lookups (e.g. "which venues trade on date D") that do not pin a
-- single mic; the composite PK already covers (mic, session_date) point reads.
create index if not exists idx_trading_calendar_session_date
  on ref.trading_calendar (session_date);
