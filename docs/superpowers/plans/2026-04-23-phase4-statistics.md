# Phase 4 — Statistics Persistence & Stats Screen

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Persist game statistics to disk and display them in a toggleable bevy_ui overlay.

**Architecture:** `StatsSnapshot` is defined and serialized in `solitaire_data`; `StatsPlugin` in `solitaire_engine` loads it on startup, updates it on game events, and saves it atomically. A lightweight bevy_ui overlay (toggled with `S`) shows the player's stats.

**Tech Stack:** `solitaire_data` (stats type + file I/O), `solitaire_engine` (Bevy plugin + UI), `serde_json` (serialization), `dirs` (platform data dir), `chrono` (timestamps), `bevy::ui` (overlay screen).

---

## File Map

| File | Action | Responsibility |
|---|---|---|
| `solitaire_data/src/stats.rs` | **Create** | `StatsSnapshot` struct + `update_on_win` + `record_abandoned` |
| `solitaire_data/src/storage.rs` | **Create** | `stats_file_path`, `load_stats_from`, `save_stats_to`, public wrappers |
| `solitaire_data/src/lib.rs` | **Modify** | Re-export `stats` and `storage` modules |
| `solitaire_engine/src/stats_plugin.rs` | **Create** | `StatsResource`, `StatsPlugin` (load/update/save + UI toggle) |
| `solitaire_engine/src/lib.rs` | **Modify** | Export `StatsPlugin`, `StatsResource` |
| `solitaire_app/src/main.rs` | **Modify** | Register `StatsPlugin` |

---

## Task 1 — `StatsSnapshot` in `solitaire_data`

**Files:**
- Create: `solitaire_data/src/stats.rs`
- Modify: `solitaire_data/src/lib.rs`

### Step 1: Write failing tests

Add to a new file `solitaire_data/src/stats.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use solitaire_core::game_state::DrawMode;

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
        s.update_on_win(100, 60, &DrawMode::DrawOne);
        s.update_on_win(100, 60, &DrawMode::DrawOne);
        s.update_on_win(100, 60, &DrawMode::DrawOne);
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
        assert_eq!(s.win_streak_best, 2, "best streak must not drop");
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
        // (100 + 200 + 300) / 3 = 200
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
```

- [ ] **Step 2: Verify tests fail**

```bash
cargo test -p solitaire_data 2>&1 | tail -5
```

Expected: compile error — `stats.rs` does not exist.

- [ ] **Step 3: Implement `StatsSnapshot`**

Create `solitaire_data/src/stats.rs` with the full struct and methods:

```rust
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
        let prev_wins = self.games_won; // capture BEFORE increment
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

        // Rolling average using u128 to avoid overflow on the intermediate product.
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
    /// Increments `games_played` and `games_lost`, resets `win_streak_current`.
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
    // (test code from Step 1 goes here)
    use super::*;
    use solitaire_core::game_state::DrawMode;

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
        s.update_on_win(100, 60, &DrawMode::DrawOne);
        s.update_on_win(100, 60, &DrawMode::DrawOne);
        s.update_on_win(100, 60, &DrawMode::DrawOne);
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
        assert_eq!(s.win_streak_best, 2, "best streak must not drop");
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
```

- [ ] **Step 4: Expose the module from `solitaire_data/src/lib.rs`**

Append to the existing `lib.rs` (after the `SyncProvider` trait):

```rust
pub mod stats;
pub use stats::StatsSnapshot;
```

- [ ] **Step 5: Run tests and verify they pass**

```bash
cargo test -p solitaire_data 2>&1 | tail -10
```

Expected output:
```
test stats::tests::avg_time_is_correct_rolling_average ... ok
test stats::tests::best_score_updates_only_on_higher_score ... ok
test stats::tests::default_stats_are_all_zero ... ok
test stats::tests::draw_three_wins_tracked_separately ... ok
test stats::tests::fastest_win_takes_minimum ... ok
test stats::tests::first_win_sets_all_fields ... ok
test stats::tests::negative_score_treated_as_zero ... ok
test stats::tests::record_abandoned_resets_streak_and_increments_played ... ok
test stats::tests::streak_tracks_across_wins ... ok
test result: ok. 9 passed; 0 failed; ...
```

- [ ] **Step 6: Clippy**

```bash
cargo clippy -p solitaire_data -- -D warnings 2>&1 | tail -5
```

Expected: `Finished ... 0 warnings`

- [ ] **Step 7: Commit**

```bash
git add solitaire_data/src/stats.rs solitaire_data/src/lib.rs
git commit -m "feat(data): add StatsSnapshot with update_on_win and record_abandoned"
```

---

## Task 2 — File Persistence in `solitaire_data`

**Files:**
- Create: `solitaire_data/src/storage.rs`
- Modify: `solitaire_data/src/lib.rs`

- [ ] **Step 1: Write failing tests**

Add to bottom of `solitaire_data/src/storage.rs` (new file, just the test module first):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::stats::StatsSnapshot;
    use solitaire_core::game_state::DrawMode;
    use std::env;

    fn tmp_path(name: &str) -> std::path::PathBuf {
        env::temp_dir().join(format!("solitaire_test_{name}.json"))
    }

    #[test]
    fn round_trip_save_and_load() {
        let path = tmp_path("round_trip");
        let _ = std::fs::remove_file(&path); // clean up from prior runs

        let mut stats = StatsSnapshot::default();
        stats.update_on_win(1000, 180, &DrawMode::DrawOne);
        save_stats_to(&path, &stats).expect("save");

        let loaded = load_stats_from(&path);
        assert_eq!(loaded.games_won, 1);
        assert_eq!(loaded.best_single_score, 1000);
        assert_eq!(loaded.fastest_win_seconds, 180);
    }

    #[test]
    fn load_from_missing_file_returns_default() {
        let path = tmp_path("missing_file_abc123");
        let _ = std::fs::remove_file(&path);
        let stats = load_stats_from(&path);
        assert_eq!(stats, StatsSnapshot::default());
    }

    #[test]
    fn save_is_atomic_no_half_written_file() {
        let path = tmp_path("atomic_write");
        let stats = StatsSnapshot::default();
        save_stats_to(&path, &stats).expect("save");

        // Verify the .tmp file was cleaned up after the rename.
        let tmp_path = path.with_extension("json.tmp");
        assert!(
            !tmp_path.exists(),
            ".tmp file should not exist after successful save"
        );
    }

    #[test]
    fn load_from_corrupt_file_returns_default() {
        let path = tmp_path("corrupt");
        std::fs::write(&path, b"not valid json!!!").expect("write corrupt");
        let stats = load_stats_from(&path);
        assert_eq!(stats, StatsSnapshot::default());
    }
}
```

- [ ] **Step 2: Verify tests fail**

```bash
cargo test -p solitaire_data storage 2>&1 | tail -5
```

Expected: compile error — `storage.rs` not found.

- [ ] **Step 3: Implement `storage.rs`**

Create `solitaire_data/src/storage.rs`:

```rust
//! Atomic file I/O for `StatsSnapshot` persistence.
//!
//! All saves go through `filename.json.tmp` → `rename()` so a crash or power
//! loss during a write never corrupts the saved data.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::stats::StatsSnapshot;

const APP_DIR_NAME: &str = "solitaire_quest";
const STATS_FILE_NAME: &str = "stats.json";

/// Returns the platform-specific path to `stats.json`, or `None` if
/// `dirs::data_dir()` is unavailable (e.g. minimal Linux containers).
pub fn stats_file_path() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join(APP_DIR_NAME).join(STATS_FILE_NAME))
}

/// Load stats from an explicit path. Returns `StatsSnapshot::default()` if
/// the file is missing or cannot be deserialized (corrupt/truncated).
pub fn load_stats_from(path: &Path) -> StatsSnapshot {
    let data = match fs::read(path) {
        Ok(d) => d,
        Err(_) => return StatsSnapshot::default(),
    };
    serde_json::from_slice(&data).unwrap_or_default()
}

/// Save stats to an explicit path using an atomic write (`.tmp` → rename).
pub fn save_stats_to(path: &Path, stats: &StatsSnapshot) -> io::Result<()> {
    // Ensure the parent directory exists.
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let json = serde_json::to_string_pretty(stats)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

    // Write to a temporary file alongside the target.
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, json.as_bytes())?;

    // Atomic rename — on POSIX this is guaranteed atomic.
    fs::rename(&tmp, path)?;
    Ok(())
}

/// Load stats from the platform default path. Returns default if the path
/// is unavailable or the file is missing/corrupt.
pub fn load_stats() -> StatsSnapshot {
    stats_file_path()
        .map(|p| load_stats_from(&p))
        .unwrap_or_default()
}

/// Save stats to the platform default path. Logs a warning if the path is
/// unavailable or the write fails — never panics.
pub fn save_stats(stats: &StatsSnapshot) -> io::Result<()> {
    let path = stats_file_path().ok_or_else(|| {
        io::Error::new(io::ErrorKind::NotFound, "platform data dir unavailable")
    })?;
    save_stats_to(&path, stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stats::StatsSnapshot;
    use solitaire_core::game_state::DrawMode;
    use std::env;

    fn tmp_path(name: &str) -> PathBuf {
        env::temp_dir().join(format!("solitaire_test_{name}.json"))
    }

    #[test]
    fn round_trip_save_and_load() {
        let path = tmp_path("round_trip");
        let _ = fs::remove_file(&path);

        let mut stats = StatsSnapshot::default();
        stats.update_on_win(1000, 180, &DrawMode::DrawOne);
        save_stats_to(&path, &stats).expect("save");

        let loaded = load_stats_from(&path);
        assert_eq!(loaded.games_won, 1);
        assert_eq!(loaded.best_single_score, 1000);
        assert_eq!(loaded.fastest_win_seconds, 180);
    }

    #[test]
    fn load_from_missing_file_returns_default() {
        let path = tmp_path("missing_file_abc123");
        let _ = fs::remove_file(&path);
        let stats = load_stats_from(&path);
        assert_eq!(stats, StatsSnapshot::default());
    }

    #[test]
    fn save_is_atomic_no_half_written_file() {
        let path = tmp_path("atomic_write");
        let stats = StatsSnapshot::default();
        save_stats_to(&path, &stats).expect("save");

        let tmp = path.with_extension("json.tmp");
        assert!(!tmp.exists(), ".tmp file must be cleaned up after rename");
    }

    #[test]
    fn load_from_corrupt_file_returns_default() {
        let path = tmp_path("corrupt");
        fs::write(&path, b"not valid json!!!").expect("write corrupt");
        let stats = load_stats_from(&path);
        assert_eq!(stats, StatsSnapshot::default());
    }
}
```

- [ ] **Step 4: Update `solitaire_data/src/lib.rs`**

Add storage module and re-exports after the stats module lines:

```rust
pub mod storage;
pub use storage::{load_stats, save_stats, stats_file_path};
```

The full `solitaire_data/src/lib.rs` should now be:

```rust
use async_trait::async_trait;
use solitaire_sync::{SyncPayload, SyncResponse};
use thiserror::Error;

/// All errors that can arise during sync operations.
#[derive(Debug, Error)]
pub enum SyncError {
    #[error("unsupported platform for this sync backend")]
    UnsupportedPlatform,
    #[error("network error: {0}")]
    Network(String),
    #[error("authentication error: {0}")]
    Auth(String),
    #[error("serialization error: {0}")]
    Serialization(String),
}

/// Every sync backend implements this trait. The SyncPlugin only calls these
/// methods — it never matches on a backend enum variant.
#[async_trait]
pub trait SyncProvider: Send + Sync {
    /// Fetch the remote sync payload. Returns the latest server state for merging.
    async fn pull(&self) -> Result<SyncPayload, SyncError>;
    /// Push the local payload to the backend. Returns the merged server response.
    async fn push(&self, payload: &SyncPayload) -> Result<SyncResponse, SyncError>;
    /// Human-readable name of this backend, used in settings UI and logs.
    fn backend_name(&self) -> &'static str;
    /// Returns true if the user is currently authenticated with this backend.
    fn is_authenticated(&self) -> bool;
    /// Mirror an achievement unlock to this backend (no-op for most backends).
    async fn mirror_achievement(&self, _id: &str) -> Result<(), SyncError> {
        Ok(())
    }
}

pub mod stats;
pub use stats::StatsSnapshot;

pub mod storage;
pub use storage::{load_stats, save_stats, stats_file_path};
```

- [ ] **Step 5: Run tests and verify they pass**

```bash
cargo test -p solitaire_data 2>&1 | tail -10
```

Expected: 13 tests all passing (9 stats + 4 storage).

- [ ] **Step 6: Clippy**

```bash
cargo clippy -p solitaire_data -- -D warnings 2>&1 | tail -5
```

Expected: 0 warnings.

- [ ] **Step 7: Commit**

```bash
git add solitaire_data/src/storage.rs solitaire_data/src/lib.rs
git commit -m "feat(data): add atomic stats persistence (load_stats_from, save_stats_to)"
```

---

## Task 3 — `StatsPlugin` in `solitaire_engine`

**Files:**
- Create: `solitaire_engine/src/stats_plugin.rs`
- Modify: `solitaire_engine/src/lib.rs`

- [ ] **Step 1: Write failing tests**

Write the test module at the bottom of the (not-yet-existing) `solitaire_engine/src/stats_plugin.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::game_plugin::GamePlugin;
    use crate::table_plugin::TablePlugin;
    use solitaire_data::StatsSnapshot;

    fn headless_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(GamePlugin)
            .add_plugins(TablePlugin)
            .add_plugins(StatsPlugin);
        app.update();
        app
    }

    #[test]
    fn stats_resource_exists_after_startup() {
        let app = headless_app();
        assert!(app.world().get_resource::<StatsResource>().is_some());
    }

    #[test]
    fn win_event_increments_games_won() {
        let mut app = headless_app();
        assert_eq!(
            app.world().resource::<StatsResource>().0.games_won,
            0
        );
        app.world_mut().send_event(GameWonEvent {
            score: 1000,
            time_seconds: 120,
        });
        // Override draw_mode so handle_move picks DrawOne (default is DrawOne).
        app.update();
        assert_eq!(
            app.world().resource::<StatsResource>().0.games_won,
            1
        );
        assert_eq!(
            app.world().resource::<StatsResource>().0.games_played,
            1
        );
    }

    #[test]
    fn new_game_after_moves_records_abandoned() {
        let mut app = headless_app();

        // Simulate move_count > 0 by directly mutating the resource.
        app.world_mut()
            .resource_mut::<crate::resources::GameStateResource>()
            .0
            .move_count = 3;

        app.world_mut()
            .send_event(NewGameRequestEvent { seed: Some(999) });
        app.update();

        let stats = &app.world().resource::<StatsResource>().0;
        assert_eq!(stats.games_played, 1, "abandoned game counted as played");
        assert_eq!(stats.games_lost, 1);
        assert_eq!(stats.win_streak_current, 0);
    }

    #[test]
    fn new_game_without_moves_does_not_record_abandoned() {
        let mut app = headless_app();
        // move_count is 0 by default after new game
        app.world_mut()
            .send_event(NewGameRequestEvent { seed: Some(42) });
        app.update();

        let stats = &app.world().resource::<StatsResource>().0;
        assert_eq!(stats.games_played, 0, "no moves = no abandoned game");
    }
}
```

- [ ] **Step 2: Verify tests fail**

```bash
cargo test -p solitaire_engine stats_plugin 2>&1 | tail -5
```

Expected: compile error — `stats_plugin` module not found.

- [ ] **Step 3: Implement `stats_plugin.rs`**

Create `solitaire_engine/src/stats_plugin.rs`:

```rust
//! Loads, updates, and persists `StatsSnapshot` in response to game events.
//!
//! Stats are loaded from disk in `Startup` and saved after every event that
//! modifies them. File I/O is synchronous (stats.json is tiny, <1 KB).

use bevy::prelude::*;
use solitaire_data::{load_stats, save_stats, StatsSnapshot};

use crate::events::{GameWonEvent, NewGameRequestEvent};
use crate::game_plugin::GameMutation;
use crate::resources::GameStateResource;

/// Bevy resource wrapping the current stats.
#[derive(Resource, Debug, Clone)]
pub struct StatsResource(pub StatsSnapshot);

/// Registers stats resources and the systems that keep them in sync.
pub struct StatsPlugin;

impl Plugin for StatsPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(StatsResource(load_stats()))
            .add_event::<GameWonEvent>()
            .add_event::<NewGameRequestEvent>()
            .add_systems(
                Update,
                (update_stats_on_win, update_stats_on_new_game).after(GameMutation),
            );
    }
}

fn update_stats_on_win(
    mut events: EventReader<GameWonEvent>,
    game: Res<GameStateResource>,
    mut stats: ResMut<StatsResource>,
) {
    for ev in events.read() {
        stats.0.update_on_win(ev.score, ev.time_seconds, &game.0.draw_mode);
        if let Err(e) = save_stats(&stats.0) {
            warn!("failed to save stats after win: {e}");
        }
    }
}

fn update_stats_on_new_game(
    mut events: EventReader<NewGameRequestEvent>,
    game: Res<GameStateResource>,
    mut stats: ResMut<StatsResource>,
) {
    for _ in events.read() {
        // Only count as abandoned if the player made at least one move and did
        // not win — a re-deal from a brand-new untouched game is not a loss.
        if game.0.move_count > 0 && !game.0.is_won {
            stats.0.record_abandoned();
            if let Err(e) = save_stats(&stats.0) {
                warn!("failed to save stats after abandoned game: {e}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::GameWonEvent;
    use crate::game_plugin::GamePlugin;
    use crate::table_plugin::TablePlugin;

    fn headless_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(GamePlugin)
            .add_plugins(TablePlugin)
            .add_plugins(StatsPlugin);
        app.update();
        app
    }

    #[test]
    fn stats_resource_exists_after_startup() {
        let app = headless_app();
        assert!(app.world().get_resource::<StatsResource>().is_some());
    }

    #[test]
    fn win_event_increments_games_won() {
        let mut app = headless_app();
        assert_eq!(app.world().resource::<StatsResource>().0.games_won, 0);

        app.world_mut()
            .send_event(GameWonEvent { score: 1000, time_seconds: 120 });
        app.update();

        assert_eq!(app.world().resource::<StatsResource>().0.games_won, 1);
        assert_eq!(app.world().resource::<StatsResource>().0.games_played, 1);
    }

    #[test]
    fn new_game_after_moves_records_abandoned() {
        let mut app = headless_app();

        app.world_mut()
            .resource_mut::<crate::resources::GameStateResource>()
            .0
            .move_count = 3;

        app.world_mut()
            .send_event(NewGameRequestEvent { seed: Some(999) });
        app.update();

        let stats = &app.world().resource::<StatsResource>().0;
        assert_eq!(stats.games_played, 1);
        assert_eq!(stats.games_lost, 1);
        assert_eq!(stats.win_streak_current, 0);
    }

    #[test]
    fn new_game_without_moves_does_not_record_abandoned() {
        let mut app = headless_app();
        app.world_mut()
            .send_event(NewGameRequestEvent { seed: Some(42) });
        app.update();

        let stats = &app.world().resource::<StatsResource>().0;
        assert_eq!(stats.games_played, 0);
    }
}
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p solitaire_engine stats_plugin 2>&1 | tail -10
```

Expected: 4 tests passing.

- [ ] **Step 5: Clippy**

```bash
cargo clippy -p solitaire_engine -- -D warnings 2>&1 | tail -5
```

Expected: 0 warnings.

- [ ] **Step 6: Commit**

```bash
git add solitaire_engine/src/stats_plugin.rs
git commit -m "feat(engine): add StatsPlugin with persistent StatsResource"
```

---

## Task 4 — Stats Screen (bevy_ui overlay)

**Files:**
- Modify: `solitaire_engine/src/stats_plugin.rs` — add UI toggle systems
- Modify: `solitaire_engine/src/lib.rs` — export `StatsPlugin`, `StatsResource`
- Modify: `solitaire_app/src/main.rs` — register `StatsPlugin`

The stats screen is a full-window overlay spawned on demand. It reuses `StatsPlugin` — no separate plugin needed.

- [ ] **Step 1: Write failing tests**

Add these tests to `stats_plugin.rs` (inside the existing `tests` module):

```rust
    #[test]
    fn pressing_s_spawns_stats_screen() {
        let mut app = headless_app();
        assert_eq!(
            app.world_mut().query::<&StatsScreen>().iter(app.world()).count(),
            0,
            "screen must not exist before toggle"
        );

        // Simulate pressing S.
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyS);
        app.update();

        assert_eq!(
            app.world_mut().query::<&StatsScreen>().iter(app.world()).count(),
            1,
            "screen must appear after first S press"
        );
    }

    #[test]
    fn pressing_s_twice_closes_stats_screen() {
        let mut app = headless_app();

        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyS);
        app.update();

        // Release and re-press so just_pressed fires again.
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .release(KeyCode::KeyS);
        app.update();

        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyS);
        app.update();

        assert_eq!(
            app.world_mut().query::<&StatsScreen>().iter(app.world()).count(),
            0,
            "screen must close after second S press"
        );
    }
```

- [ ] **Step 2: Verify tests fail**

```bash
cargo test -p solitaire_engine pressing_s 2>&1 | tail -5
```

Expected: compile error — `StatsScreen` not found.

- [ ] **Step 3: Implement stats screen toggle**

Add the following to `solitaire_engine/src/stats_plugin.rs` — insert after the `update_stats_on_new_game` function and before the `tests` module:

First add imports at the top of the file:
```rust
use bevy::input::ButtonInput;
use solitaire_data::{load_stats, save_stats, StatsSnapshot};
```
(replace the existing `use solitaire_data::{load_stats, save_stats, StatsSnapshot};` import)

Add the full import block at the top:
```rust
use bevy::input::ButtonInput;
use bevy::prelude::*;
use solitaire_data::{load_stats, save_stats, StatsSnapshot};

use crate::events::{GameWonEvent, NewGameRequestEvent};
use crate::game_plugin::GameMutation;
use crate::resources::GameStateResource;
```

Add the `StatsScreen` marker and `StatsPlugin::build` update:

```rust
/// Marker component on the stats overlay root node.
#[derive(Component, Debug)]
pub struct StatsScreen;
```

Update `StatsPlugin::build` to also register the UI system:

```rust
impl Plugin for StatsPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(StatsResource(load_stats()))
            .add_event::<GameWonEvent>()
            .add_event::<NewGameRequestEvent>()
            .add_systems(
                Update,
                (
                    update_stats_on_win,
                    update_stats_on_new_game,
                    toggle_stats_screen,
                )
                    .after(GameMutation),
            );
    }
}
```

Add the toggle and spawn/despawn functions after `update_stats_on_new_game`:

```rust
fn toggle_stats_screen(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    stats: Res<StatsResource>,
    screens: Query<Entity, With<StatsScreen>>,
) {
    if !keys.just_pressed(KeyCode::KeyS) {
        return;
    }
    if let Ok(entity) = screens.get_single() {
        commands.entity(entity).despawn_recursive();
    } else {
        spawn_stats_screen(&mut commands, &stats.0);
    }
}

fn spawn_stats_screen(commands: &mut Commands, stats: &StatsSnapshot) {
    let win_rate = stats
        .win_rate()
        .map_or("N/A".to_string(), |r| format!("{r:.1}%"));
    let fastest = if stats.fastest_win_seconds == u64::MAX {
        "N/A".to_string()
    } else {
        format_duration(stats.fastest_win_seconds)
    };
    let avg = if stats.games_won == 0 {
        "N/A".to_string()
    } else {
        format_duration(stats.avg_time_seconds)
    };

    let lines = vec![
        "=== Statistics ===".to_string(),
        format!("Games Played:  {}", stats.games_played),
        format!("Games Won:     {}", stats.games_won),
        format!("Win Rate:      {win_rate}"),
        format!("Win Streak:    {} (Best: {})", stats.win_streak_current, stats.win_streak_best),
        format!("Best Score:    {}", stats.best_single_score),
        format!("Fastest Win:   {fastest}"),
        format!("Avg Win Time:  {avg}"),
        String::new(),
        "Press S to close".to_string(),
    ];

    commands
        .spawn((
            StatsScreen,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Percent(0.0),
                top: Val::Percent(0.0),
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                row_gap: Val::Px(6.0),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.88)),
            ZIndex(200),
        ))
        .with_children(|b| {
            for line in lines {
                b.spawn((
                    Text::new(line),
                    TextFont { font_size: 24.0, ..default() },
                    TextColor(Color::srgb(0.95, 0.95, 0.90)),
                ));
            }
        });
}

fn format_duration(secs: u64) -> String {
    let m = secs / 60;
    let s = secs % 60;
    format!("{m}m {s:02}s")
}
```

The headless app needs `ButtonInput<KeyCode>` registered. Add to `headless_app()` in tests:

```rust
fn headless_app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_plugins(GamePlugin)
        .add_plugins(TablePlugin)
        .add_plugins(StatsPlugin);
    app.init_resource::<ButtonInput<KeyCode>>();
    app.update();
    app
}
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p solitaire_engine stats_plugin 2>&1 | tail -10
```

Expected: all 6 stats_plugin tests passing.

- [ ] **Step 5: Update `solitaire_engine/src/lib.rs`**

Add `stats_plugin` module and exports. The full updated section:

```rust
pub mod animation_plugin;
pub mod card_plugin;
pub mod events;
pub mod game_plugin;
pub mod input_plugin;
pub mod layout;
pub mod resources;
pub mod stats_plugin;
pub mod table_plugin;

pub use animation_plugin::{AnimationPlugin, CardAnim};
pub use card_plugin::{CardEntity, CardLabel, CardPlugin};
pub use events::{
    AchievementUnlockedEvent, CardFlippedEvent, DrawRequestEvent, GameWonEvent, MoveRequestEvent,
    NewGameRequestEvent, StateChangedEvent, UndoRequestEvent,
};
pub use game_plugin::{GameMutation, GamePlugin};
pub use input_plugin::InputPlugin;
pub use layout::{compute_layout, Layout, LayoutResource};
pub use resources::{DragState, GameStateResource, SyncStatus, SyncStatusResource};
pub use stats_plugin::{StatsPlugin, StatsResource, StatsScreen};
pub use table_plugin::{PileMarker, TableBackground, TablePlugin};
```

- [ ] **Step 6: Update `solitaire_app/src/main.rs`**

```rust
use bevy::prelude::*;
use solitaire_engine::{AnimationPlugin, CardPlugin, GamePlugin, InputPlugin, StatsPlugin, TablePlugin};

fn main() {
    App::new()
        .add_plugins(
            DefaultPlugins.set(WindowPlugin {
                primary_window: Some(Window {
                    title: "Solitaire Quest".into(),
                    resolution: (1280.0, 800.0).into(),
                    ..default()
                }),
                ..default()
            }),
        )
        .add_plugins(GamePlugin)
        .add_plugins(TablePlugin)
        .add_plugins(CardPlugin)
        .add_plugins(InputPlugin)
        .add_plugins(AnimationPlugin)
        .add_plugins(StatsPlugin)
        .run();
}
```

- [ ] **Step 7: Full workspace test + clippy**

```bash
cargo test --workspace 2>&1 | grep -E "FAILED|test result"
cargo clippy --workspace -- -D warnings 2>&1 | tail -5
```

Expected: all tests passing, 0 clippy warnings.

- [ ] **Step 8: Commit**

```bash
git add solitaire_engine/src/stats_plugin.rs solitaire_engine/src/lib.rs solitaire_app/src/main.rs
git commit -m "feat(engine): add stats screen overlay toggled with S key (Phase 4)"
```

---

## Task 5 — Final Gate

**Files:** none new — just verification.

- [ ] **Step 1: Full workspace test**

```bash
cargo test --workspace 2>&1 | grep -E "test result|FAILED"
```

Expected: all test results show `ok`, no `FAILED` lines. Total passing count should be ≥ 120 (110 existing + ~13 new).

- [ ] **Step 2: Clippy (zero warnings)**

```bash
cargo clippy --workspace -- -D warnings 2>&1 | tail -3
```

Expected: `Finished ... 0 warnings`

- [ ] **Step 3: Smoke-test the running game**

```bash
cargo run -p solitaire_app --features bevy/dynamic_linking
```

Verify manually:
- Game window opens and cards render
- Press `S` → stats overlay appears showing zeros (or loaded stats)
- Press `S` again → overlay closes
- Play a game to completion (drag cards, press D to draw, U to undo)
- Win detection triggers cascade animation
- Press `S` → games_played = 1, games_won = 1 displayed

- [ ] **Step 4: Update SESSION_HANDOFF.md**

Update `docs/SESSION_HANDOFF.md`:
- Mark Phase 4 complete in the commit history table
- Update "What Is Next" to point to Phase 5 (Achievements)
- Update the running test count

- [ ] **Step 5: Final commit (if anything changed during smoke test)**

```bash
git add -p  # review any fixes made during smoke test
git commit -m "chore: update session handoff for Phase 4 completion"
```

---

## Cross-Cutting Rules (reminder)

- `solitaire_core` and `solitaire_sync` must NOT gain new dependencies.
- `save_stats` / `load_stats` handle `dirs::data_dir() = None` without panicking.
- No `unwrap()` in new code — use `if let`, `unwrap_or_default()`, or `?`.
- `cargo clippy --workspace -- -D warnings` must pass after every task.
- `cargo test --workspace` must pass after every task.
