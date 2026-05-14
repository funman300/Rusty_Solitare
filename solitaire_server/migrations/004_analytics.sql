-- Analytics event store.
-- Events are write-only; the server never modifies rows after insert.
-- `INSERT OR IGNORE` on `id` makes submissions idempotent.
CREATE TABLE IF NOT EXISTS analytics_events (
    id          TEXT    PRIMARY KEY NOT NULL,   -- UUID v4 minted by the client
    user_id     TEXT,                           -- optional username; NULL = anonymous
    session_id  TEXT    NOT NULL,               -- UUID v4, one per app launch
    event_type  TEXT    NOT NULL,               -- e.g. "game_won", "game_start"
    payload     TEXT    NOT NULL DEFAULT '{}',  -- JSON blob, event-specific fields
    client_time TEXT    NOT NULL,               -- ISO-8601, from the client clock
    received_at TEXT    NOT NULL                -- ISO-8601, server clock at ingest
);
CREATE INDEX IF NOT EXISTS idx_analytics_event_type  ON analytics_events(event_type);
CREATE INDEX IF NOT EXISTS idx_analytics_received_at ON analytics_events(received_at);
CREATE INDEX IF NOT EXISTS idx_analytics_user_id     ON analytics_events(user_id);
