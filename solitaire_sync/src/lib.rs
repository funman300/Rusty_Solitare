//! Shared API types and merge logic for Solitaire Quest.
//!
//! This crate is the contract between the game client (`solitaire_data`) and
//! the sync server (`solitaire_server`). Changing any public type here is a
//! breaking change on both sides — version carefully.
//!
//! **No Bevy. No network. No file I/O.** Only `serde`, `uuid`, and `chrono`.

pub mod achievements;
pub mod merge;
pub mod progress;
pub mod stats;

pub use achievements::AchievementRecord;
pub use merge::merge;
pub use progress::{level_for_xp, PlayerProgress};
pub use stats::StatsSnapshot;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Sync wire types
// ---------------------------------------------------------------------------

/// Full sync payload sent from the client to the server and returned after
/// server-side merge. Contains all data needed to reconcile two instances.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncPayload {
    /// Identifies the owning player. Must match the authenticated user.
    pub user_id: Uuid,
    /// Cumulative game statistics.
    pub stats: StatsSnapshot,
    /// Per-achievement unlock records.
    pub achievements: Vec<AchievementRecord>,
    /// XP, level, cosmetic unlocks, and daily/weekly progress.
    pub progress: PlayerProgress,
    /// Wall-clock time of the last local modification.
    pub last_modified: DateTime<Utc>,
}

/// Response returned by the sync server after a pull or push operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncResponse {
    /// The merged payload that the client should save locally.
    pub merged: SyncPayload,
    /// The server's current wall-clock time (useful for clock-skew detection).
    pub server_time: DateTime<Utc>,
    /// Fields where local and remote values differed and could not be merged
    /// deterministically. Returned for display purposes — data is never
    /// silently discarded.
    pub conflicts: Vec<ConflictReport>,
}

/// Describes a single field where local and remote values diverged in a way
/// that the merge function could not resolve automatically.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConflictReport {
    /// Dot-separated field path, e.g. `"win_streak_current"`.
    pub field: String,
    /// Human-readable representation of the local value.
    pub local_value: String,
    /// Human-readable representation of the remote value.
    pub remote_value: String,
}

// ---------------------------------------------------------------------------
// Daily challenge / leaderboard types
// ---------------------------------------------------------------------------

/// Describes today's daily challenge, returned by `GET /api/daily-challenge`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChallengeGoal {
    /// Date this challenge applies to, formatted as `"YYYY-MM-DD"`.
    pub date: String,
    /// Deterministic RNG seed for this date's deal — identical for all players.
    pub seed: u64,
    /// Human-readable description of the goal, e.g. "Win in under 5 minutes".
    pub description: String,
    /// Optional target score required to complete the challenge.
    pub target_score: Option<i32>,
    /// Optional maximum allowed time in seconds to complete the challenge.
    pub max_time_secs: Option<u64>,
}

/// A single row from the server leaderboard, returned by `GET /api/leaderboard`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeaderboardEntry {
    /// Display name chosen by the player at opt-in time.
    pub display_name: String,
    /// The player's best single-game score.
    pub best_score: Option<i32>,
    /// The player's fastest win time in seconds.
    pub best_time_secs: Option<u64>,
    /// When this entry was last recorded.
    pub recorded_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

/// Errors returned by the sync server in `application/json` error bodies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
pub enum ApiError {
    /// The request could not be authenticated (missing or invalid JWT).
    #[error("unauthorized")]
    Unauthorized,
    /// The supplied credentials were incorrect.
    #[error("invalid credentials")]
    InvalidCredentials,
    /// A username that was requested for registration is already taken.
    #[error("username already taken")]
    UsernameTaken,
    /// The request payload was too large (> 1 MB).
    #[error("payload too large")]
    PayloadTooLarge,
    /// The request body could not be parsed.
    #[error("bad request: {0}")]
    BadRequest(String),
    /// An unexpected server-side error occurred.
    #[error("internal server error")]
    Internal,
}
