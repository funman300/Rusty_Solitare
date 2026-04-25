//! Player progression — XP, level, unlocks, daily/weekly progress.
//!
//! Persisted to `progress.json` next to `stats.json` and `achievements.json`.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use chrono::{DateTime, NaiveDate, Utc};
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
}
