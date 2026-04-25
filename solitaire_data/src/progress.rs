//! Player progression — XP, level, unlocks, daily/weekly progress.
//!
//! Persisted to `progress.json` next to `stats.json` and `achievements.json`.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Datelike, Duration, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

const APP_DIR_NAME: &str = "solitaire_quest";
const FILE_NAME: &str = "progress.json";

/// XP-to-level lookup. Matches ARCHITECTURE.md §13.
///
/// Levels 1–10: `level = floor(total_xp / 500)`
/// Levels 11+:  `level = 10 + floor((total_xp - 5_000) / 1_000)`
pub fn level_for_xp(xp: u64) -> u32 {
    if xp < 5_000 {
        (xp / 500) as u32
    } else {
        10 + ((xp - 5_000) / 1_000) as u32
    }
}

/// Deterministic seed derived from a date, identical for all players globally.
/// Used as the RNG seed for the daily-challenge deal.
pub fn daily_seed_for(date: NaiveDate) -> u64 {
    let y = date.year() as u64;
    let m = date.month() as u64;
    let d = date.day() as u64;
    y * 10_000 + m * 100 + d
}

/// XP awarded for winning a game.
///
/// Base 50 + scaled fast-win bonus (10..=50 for sub-2-minute wins) + 25 if
/// the player did not use undo.
pub fn xp_for_win(time_seconds: u64, used_undo: bool) -> u64 {
    let base: u64 = 50;
    let speed_bonus: u64 = if time_seconds >= 120 {
        0
    } else {
        // Linearly scale 50 → 10 across 0..=120 seconds.
        // 0s → 50, 60s → 30, 120s → 10.
        let scaled = 50_u64.saturating_sub(time_seconds.saturating_mul(40) / 120);
        scaled.max(10)
    };
    let no_undo_bonus: u64 = if used_undo { 0 } else { 25 };
    base + speed_bonus + no_undo_bonus
}

/// Persisted player progression state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayerProgress {
    pub total_xp: u64,
    pub level: u32,
    pub daily_challenge_last_completed: Option<NaiveDate>,
    pub daily_challenge_streak: u32,
    pub weekly_goal_progress: HashMap<String, u32>,
    /// ISO week key (e.g. `"2026-W17"`) the current `weekly_goal_progress`
    /// counters belong to. When the engine sees a different week it clears
    /// progress and updates this field.
    #[serde(default)]
    pub weekly_goal_week_iso: Option<String>,
    pub unlocked_card_backs: Vec<usize>,
    pub unlocked_backgrounds: Vec<usize>,
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
            unlocked_card_backs: vec![0],   // back #0 always available
            unlocked_backgrounds: vec![0],  // background #0 always available
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
    /// Returns `true` if this call brought the counter from below `target`
    /// to at-or-above `target` (i.e. just completed the goal).
    pub fn record_weekly_progress(&mut self, goal_id: &str, target: u32) -> bool {
        let entry = self.weekly_goal_progress.entry(goal_id.to_string()).or_insert(0);
        if *entry >= target {
            // Already complete — do not over-count.
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
    /// - Same day as the previous: no-op (idempotent — a player can't double-count).
    ///
    /// Returns `true` if this call recorded a fresh completion (i.e. it wasn't
    /// the same-day no-op case).
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

/// Platform-specific default path for `progress.json`.
pub fn progress_file_path() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join(APP_DIR_NAME).join(FILE_NAME))
}

/// Load progress from an explicit path. Returns `default()` if missing/corrupt.
pub fn load_progress_from(path: &Path) -> PlayerProgress {
    let Ok(data) = fs::read(path) else {
        return PlayerProgress::default();
    };
    serde_json::from_slice(&data).unwrap_or_default()
}

/// Save progress to an explicit path using an atomic write.
pub fn save_progress_to(path: &Path, progress: &PlayerProgress) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(progress).map_err(io::Error::other)?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, json.as_bytes())?;
    fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    fn tmp_path(name: &str) -> PathBuf {
        env::temp_dir().join(format!("solitaire_progress_test_{name}.json"))
    }

    // --- Level formula ---

    #[test]
    fn level_for_xp_at_breakpoints() {
        assert_eq!(level_for_xp(0), 0);
        assert_eq!(level_for_xp(499), 0);
        assert_eq!(level_for_xp(500), 1);
        assert_eq!(level_for_xp(4_999), 9);
        assert_eq!(level_for_xp(5_000), 10);
        assert_eq!(level_for_xp(5_999), 10);
        assert_eq!(level_for_xp(6_000), 11);
        assert_eq!(level_for_xp(15_000), 20);
    }

    // --- XP-for-win formula ---

    #[test]
    fn xp_for_slow_win_with_undo_is_just_base() {
        assert_eq!(xp_for_win(300, true), 50);
    }

    #[test]
    fn xp_for_no_undo_win_adds_25() {
        assert_eq!(xp_for_win(300, false), 75);
    }

    #[test]
    fn xp_for_instant_win_includes_max_speed_bonus() {
        // base 50 + speed 50 = 100 with undo, +25 without
        assert_eq!(xp_for_win(0, true), 100);
        assert_eq!(xp_for_win(0, false), 125);
    }

    #[test]
    fn xp_speed_bonus_scales_linearly_to_120s() {
        // At 60s: 50 - (60*40/120) = 50 - 20 = 30
        assert_eq!(xp_for_win(60, true), 50 + 30);
        // At 119s: 50 - (119*40/120) = 50 - 39 = 11, but floored at 10
        assert!(xp_for_win(119, true) >= 60);
    }

    #[test]
    fn xp_no_speed_bonus_at_or_above_120s() {
        assert_eq!(xp_for_win(120, true), 50);
        assert_eq!(xp_for_win(180, true), 50);
    }

    // --- PlayerProgress.add_xp ---

    #[test]
    fn add_xp_returns_previous_level_and_recomputes() {
        let mut p = PlayerProgress::default();
        let prev = p.add_xp(500);
        assert_eq!(prev, 0);
        assert_eq!(p.total_xp, 500);
        assert_eq!(p.level, 1);
    }

    #[test]
    fn level_up_detection_works() {
        let mut p = PlayerProgress::default();
        let prev = p.add_xp(450);
        assert!(!p.leveled_up_from(prev), "no level change at 450 xp");
        let prev = p.add_xp(60);
        assert!(p.leveled_up_from(prev), "0 → 1 at 510 xp");
    }

    #[test]
    fn add_xp_saturates_on_overflow() {
        let mut p = PlayerProgress::default();
        p.total_xp = u64::MAX - 5;
        p.add_xp(100);
        assert_eq!(p.total_xp, u64::MAX);
    }

    #[test]
    fn default_unlocks_include_first_card_back_and_background() {
        let p = PlayerProgress::default();
        assert!(p.unlocked_card_backs.contains(&0));
        assert!(p.unlocked_backgrounds.contains(&0));
    }

    // --- Persistence ---

    #[test]
    fn round_trip_save_and_load() {
        let path = tmp_path("round_trip");
        let _ = fs::remove_file(&path);

        let mut p = PlayerProgress::default();
        p.add_xp(1234);
        p.unlocked_card_backs.push(2);
        save_progress_to(&path, &p).expect("save");
        let loaded = load_progress_from(&path);
        assert_eq!(loaded.total_xp, 1234);
        assert_eq!(loaded.level, p.level);
        assert!(loaded.unlocked_card_backs.contains(&2));
    }

    #[test]
    fn load_from_missing_file_returns_default() {
        let path = tmp_path("missing_xyz");
        let _ = fs::remove_file(&path);
        let p = load_progress_from(&path);
        assert_eq!(p, PlayerProgress::default());
    }

    #[test]
    fn load_from_corrupt_file_returns_default() {
        let path = tmp_path("corrupt");
        fs::write(&path, b"garbage").expect("write");
        let p = load_progress_from(&path);
        assert_eq!(p, PlayerProgress::default());
    }

    #[test]
    fn save_cleans_up_tmp_file() {
        let path = tmp_path("atomic");
        save_progress_to(&path, &PlayerProgress::default()).expect("save");
        assert!(!path.with_extension("json.tmp").exists());
    }

    // --- Daily challenge ---

    #[test]
    fn daily_seed_is_deterministic_per_date() {
        let d = NaiveDate::from_ymd_opt(2026, 4, 24).unwrap();
        assert_eq!(daily_seed_for(d), daily_seed_for(d));
    }

    #[test]
    fn daily_seed_differs_across_dates() {
        let a = NaiveDate::from_ymd_opt(2026, 4, 24).unwrap();
        let b = NaiveDate::from_ymd_opt(2026, 4, 25).unwrap();
        assert_ne!(daily_seed_for(a), daily_seed_for(b));
    }

    #[test]
    fn first_daily_completion_starts_streak_at_1() {
        let mut p = PlayerProgress::default();
        let d = NaiveDate::from_ymd_opt(2026, 4, 24).unwrap();
        let recorded = p.record_daily_completion(d);
        assert!(recorded);
        assert_eq!(p.daily_challenge_streak, 1);
        assert_eq!(p.daily_challenge_last_completed, Some(d));
    }

    #[test]
    fn consecutive_days_increment_streak() {
        let mut p = PlayerProgress::default();
        let d1 = NaiveDate::from_ymd_opt(2026, 4, 24).unwrap();
        let d2 = d1 + Duration::days(1);
        let d3 = d2 + Duration::days(1);
        p.record_daily_completion(d1);
        p.record_daily_completion(d2);
        p.record_daily_completion(d3);
        assert_eq!(p.daily_challenge_streak, 3);
    }

    #[test]
    fn skipped_day_resets_streak_to_1() {
        let mut p = PlayerProgress::default();
        let d1 = NaiveDate::from_ymd_opt(2026, 4, 24).unwrap();
        let d3 = d1 + Duration::days(2); // skipped d2
        p.record_daily_completion(d1);
        p.record_daily_completion(d3);
        assert_eq!(p.daily_challenge_streak, 1);
    }

    // --- Weekly goals ---

    #[test]
    fn first_week_roll_initializes_key_and_returns_true() {
        let mut p = PlayerProgress::default();
        let rolled = p.roll_weekly_goals_if_new_week("2026-W17");
        assert!(rolled);
        assert_eq!(p.weekly_goal_week_iso.as_deref(), Some("2026-W17"));
    }

    #[test]
    fn same_week_roll_is_noop() {
        let mut p = PlayerProgress::default();
        p.roll_weekly_goals_if_new_week("2026-W17");
        p.weekly_goal_progress.insert("g1".into(), 3);
        let rolled = p.roll_weekly_goals_if_new_week("2026-W17");
        assert!(!rolled);
        assert_eq!(p.weekly_goal_progress.get("g1"), Some(&3));
    }

    #[test]
    fn new_week_roll_clears_progress_and_updates_key() {
        let mut p = PlayerProgress::default();
        p.roll_weekly_goals_if_new_week("2026-W17");
        p.weekly_goal_progress.insert("g1".into(), 3);
        let rolled = p.roll_weekly_goals_if_new_week("2026-W18");
        assert!(rolled);
        assert!(p.weekly_goal_progress.is_empty());
        assert_eq!(p.weekly_goal_week_iso.as_deref(), Some("2026-W18"));
    }

    #[test]
    fn record_weekly_progress_returns_true_only_on_completion_step() {
        let mut p = PlayerProgress::default();
        assert!(!p.record_weekly_progress("g1", 3));
        assert!(!p.record_weekly_progress("g1", 3));
        assert!(p.record_weekly_progress("g1", 3), "third tick completes");
        // Further ticks should not re-fire completion.
        assert!(!p.record_weekly_progress("g1", 3));
        assert_eq!(p.weekly_goal_progress.get("g1"), Some(&3));
    }

    #[test]
    fn same_day_completion_is_idempotent() {
        let mut p = PlayerProgress::default();
        let d = NaiveDate::from_ymd_opt(2026, 4, 24).unwrap();
        p.record_daily_completion(d);
        let recorded_again = p.record_daily_completion(d);
        assert!(!recorded_again, "same-day completion must report no-op");
        assert_eq!(p.daily_challenge_streak, 1);
    }
}
