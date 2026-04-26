//! Player statistics — persisted to `stats.json` between sessions.
//!
//! [`StatsSnapshot`] is defined in `solitaire_sync` and re-exported here.
//! This module adds the [`StatsExt`] extension trait, which supplies the
//! `update_on_win` method that depends on [`DrawMode`] from `solitaire_core`.

use chrono::Utc;
use solitaire_core::game_state::DrawMode;

pub use solitaire_sync::StatsSnapshot;

/// Extension trait providing game-logic mutation helpers for [`StatsSnapshot`].
///
/// Import this trait alongside `StatsSnapshot` to use `update_on_win`.
pub trait StatsExt {
    /// Record a completed win. Updates all relevant counters and rolling averages.
    fn update_on_win(&mut self, score: i32, time_seconds: u64, draw_mode: &DrawMode);
}

impl StatsExt for StatsSnapshot {
    fn update_on_win(&mut self, score: i32, time_seconds: u64, draw_mode: &DrawMode) {
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
