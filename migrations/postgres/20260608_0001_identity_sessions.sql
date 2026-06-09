create table if not exists system.identity_sessions (
  -- High-entropy session token: 32 random bytes hex-encoded (64 lowercase hex chars, 256 bits entropy).
  -- This is the bearer credential; treat like a password — never log.
  token text primary key,
  -- Stable user identifier matching system.identity_users.id.
  user_id text not null,
  -- Unix-epoch ms when the session was created.
  created_at_ms bigint not null,
  -- Unix-epoch ms after which the session is invalid.
  -- Checked server-side on every resolve; expired rows are deleted on access.
  -- Note: no sliding-window extension in this sub-PR; TTL is fixed at creation.
  expires_at_ms bigint not null check (expires_at_ms >= 0)
);

create index if not exists idx_identity_sessions_user
  on system.identity_sessions (user_id);

create index if not exists idx_identity_sessions_expiry
  on system.identity_sessions (expires_at_ms);
