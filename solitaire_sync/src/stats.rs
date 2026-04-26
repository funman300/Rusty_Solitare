//! Shared `StatsSnapshot` definition — used by both the game client and the
//! sync server to represent cumulative player statistics.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Cumulative game statistics that travel across the sync boundary.
///
/// Game-logic mutation helpers that depend on `solitaire_core` types (e.g.
/// `update_on_win`) are provided via the `StatsExt` extension trait in
/// `solitaire_data`. File I/O helpers also live in `solitaire_data::storage`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatsSnapshot {
    /// Total number of games started (won + lost + abandoned).
    pub games_played: u32,
    /// Number of games won.
    pub games_won: u32,
    /// Number of games lost or abandoned.
    pub games_lost: u32,
    /// Current win streak length.
    pub win_streak_current: u32,
    /// All-time best win streak.
    pub win_streak_best: u32,
    /// Rolling average of win times in seconds.
    pub avg_time_seconds: u64,
    /// Fastest single win time in seconds. `u64::MAX` when no wins recorded yet.
    pub fastest_win_seconds: u64,
    /// Sum of all winning scores.
    pub lifetime_score: u64,
    /// Highest score achieved in a single game.
    pub best_single_score: u32,
    /// Wins achieved in Draw-One mode.
    pub draw_one_wins: u32,
    /// Wins achieved in Draw-Three mode.
    pub draw_three_wins: u32,
    /// Wall-clock time of the last modification (used for conflict detection).
    pub last_modified: DateTime<Utc>,
}

impl Default for StatsSnapshot {
    fn default() -> Self {
        Self {
            games_played: 0,
            games_won: 0,
            games_lost: 0,
            win_streak_current: 0,
            win_streak_best: 0,
            avg_time_seconds: 0,
            fastest_win_seconds: u64::MAX,
            lifetime_score: 0,
            best_single_score: 0,
            draw_one_wins: 0,
            draw_three_wins: 0,
            last_modified: DateTime::UNIX_EPOCH,
        }
    }
}

impl StatsSnapshot {
    /// Record an abandoned game (player started a new game without winning).
    pub fn record_abandoned(&mut self) {
        self.games_played += 1;
        self.games_lost += 1;
        self.win_streak_current = 0;
        self.last_modified = Utc::now();
    }

    /// Win percentage as 0–100, or `None` if no games played.
    pub fn win_rate(&self) -> Option<f32> {
        if self.games_played == 0 {
            None
        } else {
            Some(self.games_won as f32 / self.games_played as f32 * 100.0)
        }
    }
}
