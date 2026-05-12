-- Migration 003: refresh token rotation table
--
-- One row per live refresh token. Issued at login/register and rotated
-- (old row deleted, new row inserted) on every POST /api/auth/refresh call.
-- Cascade on user deletion means no manual cleanup is needed when an
-- account is removed.

CREATE TABLE IF NOT EXISTS refresh_tokens (
    jti        TEXT PRIMARY KEY,                           -- UUID v4 embedded in the JWT
    user_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    expires_at TEXT NOT NULL                               -- ISO 8601, mirrors the JWT exp claim
);

-- Expired-row pruning (done inline in the refresh handler) uses this index
-- to avoid a full table scan on every refresh call.
CREATE INDEX IF NOT EXISTS refresh_tokens_expires_at_idx
    ON refresh_tokens(expires_at);
