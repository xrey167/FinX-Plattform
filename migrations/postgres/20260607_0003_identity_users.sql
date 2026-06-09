create schema if not exists system;

create table if not exists system.identity_users (
  -- Stable caller-supplied identifier (ULID or UUID string).
  id text primary key,
  -- Normalized (trimmed, ASCII-lowercased) email address.
  email text not null unique,
  -- Argon2id PHC string (e.g. $argon2id$v=19$...). Never returned to callers.
  password_hash text not null,
  -- Human-readable display name.
  display_name text not null,
  -- Unix-epoch ms of last user activity. Null until first session recorded.
  -- App field for rank-19 dormant-user projection.
  last_active_at_ms bigint,
  -- Unix-epoch ms of last re-engagement email. Null until first send.
  -- App field for future dormant-user re-engagement workflows.
  last_reengagement_sent_at_ms bigint,
  -- Unix-epoch ms when the record was created.
  created_at_ms bigint not null,
  -- Unix-epoch ms of last record mutation.
  updated_at_ms bigint not null
);
