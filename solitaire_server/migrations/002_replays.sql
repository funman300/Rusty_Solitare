-- Migration 002: winning-replay storage
--
-- One row per winning replay uploaded via POST /api/replays. The replay
-- itself is stored as the canonical JSON the desktop client wrote — it
-- already carries a schema_version field, so the server doesn't need to
-- shape-validate the payload beyond ensuring it parses as JSON.
--
-- The handful of denormalised columns (final_score, time_seconds,
-- recorded_at) are projected out of the JSON at insert time so list
-- endpoints (e.g. recent / per-user / leaderboard-style sorts) can be
-- served via a covering query without touching every row's blob.

CREATE TABLE IF NOT EXISTS replays (
    id              TEXT PRIMARY KEY,                      -- UUID v4 minted server-side
    user_id         TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    seed            INTEGER NOT NULL,                       -- replay's deal seed
    draw_mode       TEXT NOT NULL,                          -- "DrawOne" | "DrawThree"
    mode            TEXT NOT NULL,                          -- "Classic" | "Zen" | "Challenge" | "TimeAttack"
    time_seconds    INTEGER NOT NULL,                       -- duration of the win
    final_score     INTEGER NOT NULL,                       -- final score at the win
    recorded_at     TEXT NOT NULL,                          -- replay-side date (YYYY-MM-DD)
    received_at     TEXT NOT NULL,                          -- server insert timestamp (ISO 8601)
    replay_json     TEXT NOT NULL                           -- full Replay serialisation
);

-- Recent-replays list endpoint sorts by received_at DESC; the index
-- keeps that scan cheap on a populated table.
CREATE INDEX IF NOT EXISTS replays_received_at_idx
    ON replays(received_at DESC);

-- Lookups by user (e.g. "my replays" view) are common too.
CREATE INDEX IF NOT EXISTS replays_user_id_idx
    ON replays(user_id);
