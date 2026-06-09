create schema if not exists system;

create table if not exists system.price_alerts (
  id text primary key,
  owner_id text not null,
  symbol text not null,
  target_price double precision not null,
  condition text not null check (condition in ('Above', 'Below')),
  active boolean not null default true,
  triggered boolean not null default false,
  expires_at_ms bigint not null check (expires_at_ms >= 0),
  created_at_ms bigint not null,
  updated_at_ms bigint not null
);

create index if not exists idx_price_alerts_owner
  on system.price_alerts (owner_id, created_at_ms desc);

create index if not exists idx_price_alerts_eligible
  on system.price_alerts (expires_at_ms)
  where active and not triggered;
