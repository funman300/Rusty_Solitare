//! Player statistics — persisted to `stats.json` between sessions.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use solitaire_core::game_state::DrawMode;

/// Cumulative game statistics. Stored as `stats.json` in the platform data dir.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatsSnapshot {
    pub games_played: u32,
    pub games_won: u32,
    pub games_lost: u32,
    pub win_streak_current: u32,
    pub win_streak_best: u32,
    /// Rolling average of win times in seconds.
    pub avg_time_seconds: u64,
    /// Fastest win time. `u64::MAX` means no wins yet.
    pub fastest_win_seconds: u64,
    /// Sum of all winning scores.
    pub lifetime_score: u64,
    pub best_single_score: u32,
    pub draw_one_wins: u32,
    pub draw_three_wins: u32,
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
    /// Record a completed win. Updates all relevant counters and rolling averages.
    pub fn update_on_win(&mut self, score: i32, time_seconds: u64, draw_mode: &DrawMode) {
        let prev_wins = self.games_won;
        self.games_played += 1;
        self.games_won += 1;
        self.win_streak_current += 1;
        if self.win_streak_current > self.win_streak_best {
            self.win_streak_best = self.win_streak_current;
        }

        let score_u32 = score.max(0) as u32;
        self.lifetime_score = self.lifetime_score.saturating_add(score_u32 as u64);
        if score_u32 > self.best_single_score {
            self.best_single_score = score_u32;
        }

        if time_seconds < self.fastest_win_seconds {
            self.fastest_win_seconds = time_seconds;
        }

        self.avg_time_seconds = if prev_wins == 0 {
            time_seconds
        } else {
            ((self.avg_time_seconds as u128 * prev_wins as u128 + time_seconds as u128)
                / self.games_won as u128) as u64
        };

        match draw_mode {
            DrawMode::DrawOne => self.draw_one_wins += 1,
            DrawMode::DrawThree => self.draw_three_wins += 1,
        }

        self.last_modified = Utc::now();
    }

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
    fn default_stats_are_all_zero() {
        let s = StatsSnapshot::default();
        assert_eq!(s.games_played, 0);
        assert_eq!(s.games_won, 0);
        assert_eq!(s.win_streak_current, 0);
        assert_eq!(s.win_streak_best, 0);
        assert_eq!(s.lifetime_score, 0);
        assert_eq!(s.best_single_score, 0);
        assert_eq!(s.fastest_win_seconds, u64::MAX);
    }

    #[test]
    fn first_win_sets_all_fields() {
        let mut s = StatsSnapshot::default();
        s.update_on_win(1500, 120, &DrawMode::DrawOne);
        assert_eq!(s.games_played, 1);
        assert_eq!(s.games_won, 1);
        assert_eq!(s.win_streak_current, 1);
        assert_eq!(s.win_streak_best, 1);
        assert_eq!(s.lifetime_score, 1500);
        assert_eq!(s.best_single_score, 1500);
        assert_eq!(s.fastest_win_seconds, 120);
        assert_eq!(s.avg_time_seconds, 120);
        assert_eq!(s.draw_one_wins, 1);
        assert_eq!(s.draw_three_wins, 0);
    }

    #[test]
    fn streak_tracks_across_wins() {
        let mut s = StatsSnapshot::default();
        for _ in 0..3 {
            s.update_on_win(100, 60, &DrawMode::DrawOne);
        }
        assert_eq!(s.win_streak_current, 3);
        assert_eq!(s.win_streak_best, 3);
    }

    #[test]
    fn record_abandoned_resets_streak_and_increments_played() {
        let mut s = StatsSnapshot::default();
        s.update_on_win(100, 60, &DrawMode::DrawOne);
        s.update_on_win(100, 60, &DrawMode::DrawOne);
        assert_eq!(s.win_streak_current, 2);
        s.record_abandoned();
        assert_eq!(s.games_played, 3);
        assert_eq!(s.games_lost, 1);
        assert_eq!(s.win_streak_current, 0);
        assert_eq!(s.win_streak_best, 2);
    }

    #[test]
    fn fastest_win_takes_minimum() {
        let mut s = StatsSnapshot::default();
        s.update_on_win(100, 300, &DrawMode::DrawOne);
        s.update_on_win(100, 120, &DrawMode::DrawOne);
        s.update_on_win(100, 500, &DrawMode::DrawOne);
        assert_eq!(s.fastest_win_seconds, 120);
    }

    #[test]
    fn avg_time_is_correct_rolling_average() {
        let mut s = StatsSnapshot::default();
        s.update_on_win(100, 100, &DrawMode::DrawOne);
        s.update_on_win(100, 200, &DrawMode::DrawOne);
        s.update_on_win(100, 300, &DrawMode::DrawOne);
        assert_eq!(s.avg_time_seconds, 200);
    }

    #[test]
    fn best_score_updates_only_on_higher_score() {
        let mut s = StatsSnapshot::default();
        s.update_on_win(500, 60, &DrawMode::DrawOne);
        s.update_on_win(300, 60, &DrawMode::DrawOne);
        assert_eq!(s.best_single_score, 500);
        s.update_on_win(800, 60, &DrawMode::DrawOne);
        assert_eq!(s.best_single_score, 800);
    }

    #[test]
    fn negative_score_treated_as_zero() {
        let mut s = StatsSnapshot::default();
        s.update_on_win(-50, 60, &DrawMode::DrawOne);
        assert_eq!(s.best_single_score, 0);
        assert_eq!(s.lifetime_score, 0);
    }

    #[test]
    fn draw_three_wins_tracked_separately() {
        let mut s = StatsSnapshot::default();
        s.update_on_win(100, 60, &DrawMode::DrawOne);
        s.update_on_win(100, 60, &DrawMode::DrawThree);
        assert_eq!(s.draw_one_wins, 1);
        assert_eq!(s.draw_three_wins, 1);
    }
}
