create table if not exists system.event_archive (
  event_id text primary key,
  event_type text not null,
  schema_version text not null,
  actor jsonb not null,
  origin jsonb not null,
  trace jsonb not null,
  occurred_at timestamptz not null,
  payload jsonb not null,
  causation_id text,
  correlation_id text,
  depth integer not null default 0
);

create table if not exists system.event_outbox (
  sequence bigserial primary key,
  event_id text not null references system.event_archive(event_id),
  status text not null default 'pending',
  available_at timestamptz not null default now(),
  dispatched_at timestamptz
);

create table if not exists system.event_hook (
  hook_name text primary key,
  hook_order integer not null,
  transaction_mode text not null,
  enabled boolean not null default true,
  max_depth integer not null default 8
);

create table if not exists system.event_replay_run (
  replay_id text primary key,
  dry_run boolean not null,
  from_offset bigint not null,
  to_offset bigint,
  requested_at timestamptz not null default now()
);
