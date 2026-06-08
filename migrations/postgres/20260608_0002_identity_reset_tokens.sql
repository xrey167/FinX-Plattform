CREATE TABLE IF NOT EXISTS system.identity_reset_tokens (
    token         text PRIMARY KEY,
    user_id       text NOT NULL,
    created_at_ms bigint NOT NULL CHECK (created_at_ms >= 0),
    expires_at_ms bigint NOT NULL CHECK (expires_at_ms >= 0)
);
CREATE INDEX IF NOT EXISTS idx_identity_reset_tokens_user ON system.identity_reset_tokens (user_id);
CREATE INDEX IF NOT EXISTS idx_identity_reset_tokens_expiry ON system.identity_reset_tokens (expires_at_ms);
