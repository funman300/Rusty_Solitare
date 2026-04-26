//! Shared `PlayerProgress` definition — used by both the game client and the
//! sync server.

use std::collections::HashMap;

use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

/// XP-to-level calculation per ARCHITECTURE.md §13.
///
/// - Levels 1–10:  `level = floor(total_xp / 500)`
/// - Levels 11+:   `level = 10 + floor((total_xp - 5_000) / 1_000)`
pub fn level_for_xp(xp: u64) -> u32 {
    if xp < 5_000 {
        (xp / 500) as u32
    } else {
        10 + ((xp - 5_000) / 1_000) as u32
    }
}

/// Persisted player progression state.
///
/// Mutation helpers such as `add_xp`, `record_daily_completion`, etc. are
/// defined as inherent methods directly on this type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayerProgress {
    /// Total XP accumulated across all games.
    pub total_xp: u64,
    /// Current player level, recomputed from `total_xp`.
    pub level: u32,
    /// Date of the last completed daily challenge, if any.
    pub daily_challenge_last_completed: Option<NaiveDate>,
    /// Current daily-challenge streak length.
    pub daily_challenge_streak: u32,
    /// Per-goal progress counters for the current ISO week.
    pub weekly_goal_progress: HashMap<String, u32>,
    /// ISO week key (e.g. `"2026-W17"`) the `weekly_goal_progress` counters
    /// belong to. Cleared when a new week begins.
    #[serde(default)]
    pub weekly_goal_week_iso: Option<String>,
    /// Indices of card-back designs the player has unlocked (index 0 is always unlocked).
    pub unlocked_card_backs: Vec<usize>,
    /// Indices of background designs the player has unlocked (index 0 is always unlocked).
    pub unlocked_backgrounds: Vec<usize>,
    /// Index of the next Challenge-mode seed to serve to this player.
    #[serde(default)]
    pub challenge_index: u32,
    /// Wall-clock time of the last modification (used for conflict detection).
    pub last_modified: DateTime<Utc>,
}

impl Default for PlayerProgress {
    fn default() -> Self {
        Self {
            total_xp: 0,
            level: 0,
            daily_challenge_last_completed: None,
            daily_challenge_streak: 0,
            weekly_goal_progress: HashMap::new(),
            weekly_goal_week_iso: None,
            unlocked_card_backs: vec![0],
            unlocked_backgrounds: vec![0],
            challenge_index: 0,
            last_modified: DateTime::UNIX_EPOCH,
        }
    }
}

impl PlayerProgress {
    /// Add XP and recompute level. Returns the previous level so callers can
    /// detect level-up events.
    pub fn add_xp(&mut self, amount: u64) -> u32 {
        let prev_level = self.level;
        self.total_xp = self.total_xp.saturating_add(amount);
        self.level = level_for_xp(self.total_xp);
        self.last_modified = Utc::now();
        prev_level
    }

    /// `true` if a level-up just occurred (current level > `prev_level`).
    pub fn leveled_up_from(&self, prev_level: u32) -> bool {
        self.level > prev_level
    }

    /// Reset weekly-goal progress when the ISO week has rolled over.
    /// No-op if the stored week key already matches `current`.
    pub fn roll_weekly_goals_if_new_week(&mut self, current: &str) -> bool {
        if self.weekly_goal_week_iso.as_deref() == Some(current) {
            return false;
        }
        self.weekly_goal_progress.clear();
        self.weekly_goal_week_iso = Some(current.to_string());
        self.last_modified = Utc::now();
        true
    }

    /// Increment progress for `goal_id` by 1, capped at `target`.
    ///
    /// Returns `true` if this call brought the counter from below `target`
    /// to at-or-above `target` (i.e. just completed the goal).
    pub fn record_weekly_progress(&mut self, goal_id: &str, target: u32) -> bool {
        let entry = self.weekly_goal_progress.entry(goal_id.to_string()).or_insert(0);
        if *entry >= target {
            return false;
        }
        *entry = entry.saturating_add(1);
        self.last_modified = Utc::now();
        *entry >= target
    }

    /// Record a daily-challenge completion for `date`.
    ///
    /// - First completion ever, or a gap of more than one day: streak resets to 1.
    /// - Completion the day after the previous: streak increments.
    /// - Same day as the previous: no-op (idempotent).
    ///
    /// Returns `true` if this call recorded a fresh completion.
    pub fn record_daily_completion(&mut self, date: NaiveDate) -> bool {
        match self.daily_challenge_last_completed {
            Some(last) if last == date => return false,
            Some(last) if last + Duration::days(1) == date => {
                self.daily_challenge_streak = self.daily_challenge_streak.saturating_add(1);
            }
            _ => {
                self.daily_challenge_streak = 1;
            }
        }
        self.daily_challenge_last_completed = Some(date);
        self.last_modified = Utc::now();
        true
    }
}
