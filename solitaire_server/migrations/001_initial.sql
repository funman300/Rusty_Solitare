-- Migration 001: initial schema
-- Creates the core tables required by the Solitaire Quest sync server.

CREATE TABLE IF NOT EXISTS users (
    id                  TEXT PRIMARY KEY,           -- UUID v4
    username            TEXT UNIQUE NOT NULL,
    password_hash       TEXT NOT NULL,              -- bcrypt, cost 12
    created_at          TEXT NOT NULL,              -- ISO 8601
    leaderboard_opt_in  INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS sync_state (
    user_id             TEXT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    stats_json          TEXT NOT NULL,
    achievements_json   TEXT NOT NULL,
    progress_json       TEXT NOT NULL,
    last_modified       TEXT NOT NULL               -- ISO 8601
);

CREATE TABLE IF NOT EXISTS daily_challenges (
    date                TEXT PRIMARY KEY,           -- "YYYY-MM-DD"
    seed                INTEGER NOT NULL,
    goal_json           TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS leaderboard (
    user_id             TEXT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    display_name        TEXT NOT NULL,
    best_time_secs      INTEGER,
    best_score          INTEGER,
    recorded_at         TEXT NOT NULL               -- ISO 8601
);
