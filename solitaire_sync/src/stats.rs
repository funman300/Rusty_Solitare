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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn win_rate_is_none_before_any_game() {
        let s = StatsSnapshot::default();
        assert!(s.win_rate().is_none());
    }

    #[test]
    fn win_rate_100_when_all_games_won() {
        let s = StatsSnapshot {
            games_played: 5,
            games_won: 5,
            ..StatsSnapshot::default()
        };
        let rate = s.win_rate().expect("should have a rate");
        assert!((rate - 100.0).abs() < 0.01, "expected 100.0, got {rate}");
    }

    #[test]
    fn win_rate_50_when_half_won() {
        let s = StatsSnapshot {
            games_played: 10,
            games_won: 5,
            ..StatsSnapshot::default()
        };
        let rate = s.win_rate().expect("should have a rate");
        assert!((rate - 50.0).abs() < 0.01, "expected 50.0, got {rate}");
    }

    #[test]
    fn win_rate_0_when_no_wins() {
        let s = StatsSnapshot {
            games_played: 3,
            games_won: 0,
            ..StatsSnapshot::default()
        };
        let rate = s.win_rate().expect("should have a rate");
        assert!((rate - 0.0).abs() < 0.01, "expected 0.0, got {rate}");
    }

    #[test]
    fn fastest_win_seconds_defaults_to_max() {
        let s = StatsSnapshot::default();
        assert_eq!(s.fastest_win_seconds, u64::MAX);
    }

    #[test]
    fn record_abandoned_increments_played_and_lost() {
        let mut s = StatsSnapshot::default();
        s.record_abandoned();
        assert_eq!(s.games_played, 1);
        assert_eq!(s.games_lost, 1);
        assert_eq!(s.games_won, 0);
    }

    #[test]
    fn record_abandoned_resets_win_streak() {
        let mut s = StatsSnapshot::default();
        s.win_streak_current = 5;
        s.record_abandoned();
        assert_eq!(s.win_streak_current, 0, "abandoned game must break the win streak");
    }

    #[test]
    fn record_abandoned_preserves_best_streak() {
        let mut s = StatsSnapshot::default();
        s.win_streak_best = 7;
        s.win_streak_current = 7;
        s.record_abandoned();
        assert_eq!(s.win_streak_best, 7, "best streak must not be reduced on abandon");
        assert_eq!(s.win_streak_current, 0);
    }
}
