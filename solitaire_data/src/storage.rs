//! Atomic file I/O for persisted game data.
//!
//! All saves go through `filename.json.tmp` → `rename()` so a crash or power
//! loss during a write never corrupts the saved data.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use solitaire_core::game_state::{GameState, GAME_STATE_SCHEMA_VERSION};

use crate::stats::StatsSnapshot;

const APP_DIR_NAME: &str = "solitaire_quest";
const STATS_FILE_NAME: &str = "stats.json";
const GAME_STATE_FILE_NAME: &str = "game_state.json";
const TIME_ATTACK_SESSION_FILE_NAME: &str = "time_attack_session.json";

/// Returns the platform-specific path to `stats.json`, or `None` if
/// `dirs::data_dir()` is unavailable (e.g. minimal Linux containers).
pub fn stats_file_path() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join(APP_DIR_NAME).join(STATS_FILE_NAME))
}

/// Load stats from an explicit path. Returns `StatsSnapshot::default()` if
/// the file is missing or cannot be deserialized (corrupt/truncated).
pub fn load_stats_from(path: &Path) -> StatsSnapshot {
    let Ok(data) = fs::read(path) else {
        return StatsSnapshot::default();
    };
    serde_json::from_slice(&data).unwrap_or_default()
}

/// Save stats to an explicit path using an atomic write (`.tmp` → rename).
pub fn save_stats_to(path: &Path, stats: &StatsSnapshot) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let json = serde_json::to_string_pretty(stats).map_err(io::Error::other)?;

    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, json.as_bytes())?;
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

/// Save stats to the platform default path. Returns an error if the platform
/// data dir is unavailable or the write fails.
pub fn save_stats(stats: &StatsSnapshot) -> io::Result<()> {
    let path = stats_file_path().ok_or_else(|| {
        io::Error::new(io::ErrorKind::NotFound, "platform data dir unavailable")
    })?;
    save_stats_to(&path, stats)
}

// ---------------------------------------------------------------------------
// In-progress game state
// ---------------------------------------------------------------------------

/// Returns the platform-specific path to `game_state.json`, or `None` if
/// `dirs::data_dir()` is unavailable.
pub fn game_state_file_path() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join(APP_DIR_NAME).join(GAME_STATE_FILE_NAME))
}

/// Load an in-progress `GameState` from `path`. Returns `None` if the file is
/// missing, corrupt, represents a finished game, or carries a save-schema
/// version other than [`GAME_STATE_SCHEMA_VERSION`].
///
/// Schema mismatch is treated as "no save" so a player upgrading across an
/// incompatible game-state format change starts fresh instead of seeing a
/// half-loaded game (or a deserialiser error). v1 saves with the old
/// `Foundation(Suit)` key shape will fail to parse outright; any v1 saves
/// that happen to round-trip but report `schema_version: 1` are also rejected
/// here.
pub fn load_game_state_from(path: &Path) -> Option<GameState> {
    let data = fs::read(path).ok()?;
    let gs: GameState = serde_json::from_slice(&data).ok()?;
    if gs.schema_version != GAME_STATE_SCHEMA_VERSION {
        return None;
    }
    if gs.is_won {
        None
    } else {
        Some(gs)
    }
}

/// Save an in-progress `GameState` atomically. Skips the write if `gs.is_won`
/// because a completed game should not be resumed.
pub fn save_game_state_to(path: &Path, gs: &GameState) -> io::Result<()> {
    if gs.is_won {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(gs).map_err(io::Error::other)?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, json.as_bytes())?;
    fs::rename(&tmp, path)?;
    Ok(())
}

/// Delete the game state file (called on win, loss, or new-game start).
/// Silently ignores `NotFound` errors.
pub fn delete_game_state_at(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// Remove any leftover `*.json.tmp` files in the app data directory.
///
/// These can be left behind if the process crashes between the write and rename
/// in an atomic save. Safe to call on startup; missing or unreadable entries
/// are silently skipped.
pub fn cleanup_orphaned_tmp_files() -> io::Result<()> {
    let dir = match dirs::data_dir() {
        Some(d) => d.join(APP_DIR_NAME),
        None => return Ok(()),
    };

    if !dir.exists() {
        return Ok(());
    }

    cleanup_tmp_files_in(&dir);
    Ok(())
}

// ---------------------------------------------------------------------------
// Time Attack session (mode-specific sibling of game_state.json)
// ---------------------------------------------------------------------------
//
// `GameState` carries `mode: GameMode`, so an in-progress Zen / Challenge /
// Classic / TimeAttack deal is already round-tripped through `game_state.json`
// — closing the window mid-deal in any of those modes restores the deal on
// next launch. Time Attack adds a 10-minute session window and a per-session
// win counter that live OUTSIDE `GameState` (in `TimeAttackResource` on the
// engine side), so they are NOT covered by the game-state save/load. This
// sibling file persists just that extra session-level state.
//
// The Bevy plugin layer (`solitaire_engine::time_attack_plugin`) is the only
// caller. The file lives next to `game_state.json` in the same data dir and
// is written using the same `.tmp` → rename atomic-write contract that the
// rest of `storage.rs` uses.

/// Persisted state for an in-progress Time Attack session.
///
/// Fields mirror the live `TimeAttackResource` minus the `active` flag (the
/// presence of the file *is* the active flag — a missing file means no
/// session in progress).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimeAttackSession {
    /// Seconds remaining in the 10-minute window when the save was written.
    pub remaining_secs: f32,
    /// Wins accumulated during the session so far.
    pub wins: u32,
    /// Wall-clock instant the save was written, as unix seconds. Used at
    /// load time to detect whether the session window expired in real
    /// time while the app was closed and to decrement `remaining_secs`
    /// by the real elapsed time so the resumed session reflects how
    /// long the window has actually been running.
    pub saved_at_unix_secs: u64,
}

/// Returns the platform-specific path to `time_attack_session.json`, or
/// `None` if `dirs::data_dir()` is unavailable.
pub fn time_attack_session_path() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join(APP_DIR_NAME).join(TIME_ATTACK_SESSION_FILE_NAME))
}

/// Save a Time Attack session atomically. Mirrors `save_game_state_to`'s
/// `.tmp` → rename contract.
pub fn save_time_attack_session_to(path: &Path, session: &TimeAttackSession) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(session).map_err(io::Error::other)?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, json.as_bytes())?;
    fs::rename(&tmp, path)?;
    Ok(())
}

/// Load a Time Attack session from `path`, decrementing `remaining_secs`
/// by the wall-clock time elapsed between the save and now.
///
/// Returns `None` when:
/// - the file is missing or unreadable,
/// - the JSON is corrupt / malformed, or
/// - the session window expired during the time the app was closed
///   (`saved_at_unix_secs + remaining_secs <= now_unix_secs`).
///
/// The `now_unix_secs` parameter is injectable so unit tests can simulate
/// arbitrary wall-clock gaps without touching the real system clock. The
/// public companion [`load_time_attack_session_from`] resolves "now" from
/// `SystemTime::now()`.
pub fn load_time_attack_session_from_at(
    path: &Path,
    now_unix_secs: u64,
) -> Option<TimeAttackSession> {
    let data = fs::read(path).ok()?;
    let session: TimeAttackSession = serde_json::from_slice(&data).ok()?;
    // Compute wall-clock elapsed seconds since the save was written.
    // Saturating subtraction guards against a clock that moved backwards
    // (rare, but possible across NTP corrections or VM clock drift).
    let elapsed = now_unix_secs.saturating_sub(session.saved_at_unix_secs);
    let remaining = session.remaining_secs - elapsed as f32;
    if remaining <= 0.0 {
        return None;
    }
    Some(TimeAttackSession {
        remaining_secs: remaining,
        wins: session.wins,
        saved_at_unix_secs: session.saved_at_unix_secs,
    })
}

/// Load a Time Attack session from `path`, using `SystemTime::now()` as
/// the reference for the wall-clock-elapsed adjustment.
///
/// See [`load_time_attack_session_from_at`] for the rules under which
/// the call returns `None` (missing file, corrupt JSON, expired window).
pub fn load_time_attack_session_from(path: &Path) -> Option<TimeAttackSession> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    load_time_attack_session_from_at(path, now)
}

/// Delete the Time Attack session file (called on session end, on session
/// start, or on game completion). Silently ignores `NotFound` errors.
pub fn delete_time_attack_session_at(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// Convenience helper for callers that want to stamp a session with the
/// current wall-clock time. Equivalent to constructing the struct
/// manually and setting `saved_at_unix_secs` to `SystemTime::now()`.
pub fn time_attack_session_with_now(remaining_secs: f32, wins: u32) -> TimeAttackSession {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    TimeAttackSession {
        remaining_secs,
        wins,
        saved_at_unix_secs: now,
    }
}

/// Inner helper: delete `*.json.tmp` entries inside `dir`.
///
/// Per-file errors (already deleted, permission denied) are silently ignored.
fn cleanup_tmp_files_in(dir: &Path) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.ends_with(".json.tmp"))
            {
                let _ = fs::remove_file(&path);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stats::{StatsExt, StatsSnapshot};
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

    /// Test the core cleanup logic by creating `.json.tmp` files in a temporary
    /// directory, running the cleanup loop manually, and verifying removal.
    #[test]
    fn cleanup_removes_tmp_files() {
        let dir = env::temp_dir().join("solitaire_cleanup_test");
        fs::create_dir_all(&dir).expect("create test dir");

        // Create a pair of .json.tmp files and one regular file that must survive.
        let tmp1 = dir.join("stats.json.tmp");
        let tmp2 = dir.join("progress.json.tmp");
        let keep = dir.join("settings.json");
        fs::write(&tmp1, b"orphan1").expect("write tmp1");
        fs::write(&tmp2, b"orphan2").expect("write tmp2");
        fs::write(&keep, b"{}").expect("write keep");

        // Run the cleanup logic directly against our test directory.
        cleanup_tmp_files_in(&dir);

        assert!(!tmp1.exists(), "stats.json.tmp should have been removed");
        assert!(!tmp2.exists(), "progress.json.tmp should have been removed");
        assert!(keep.exists(), "settings.json must not be removed");

        // Tidy up.
        let _ = fs::remove_file(&keep);
        let _ = fs::remove_dir(&dir);
    }

    /// Calling `cleanup_orphaned_tmp_files` on a box with no app data dir is a
    /// no-op and must not return an error.
    #[test]
    fn cleanup_on_nonexistent_dir_is_ok() {
        // We can't control whether the real app dir exists in the test
        // environment, but the public function must at least not panic or
        // return an Err when the directory is absent.
        // The real implementation returns Ok(()) for missing dirs.
        let result = cleanup_orphaned_tmp_files();
        // The function is allowed to succeed whether or not the dir exists.
        assert!(result.is_ok());
    }

    // -----------------------------------------------------------------------
    // game_state persistence tests
    // -----------------------------------------------------------------------

    fn gs_path(name: &str) -> PathBuf {
        env::temp_dir().join(format!("solitaire_test_gs_{name}.json"))
    }

    #[test]
    fn game_state_round_trip() {
        use solitaire_core::game_state::{DrawMode, GameState};
        let path = gs_path("round_trip");
        let _ = fs::remove_file(&path);

        let gs = GameState::new(12345, DrawMode::DrawOne);
        save_game_state_to(&path, &gs).expect("save");

        let loaded = load_game_state_from(&path).expect("load");
        assert_eq!(loaded.seed, gs.seed);
        assert_eq!(loaded.draw_mode, gs.draw_mode);
        assert!(!loaded.is_won);
    }

    #[test]
    fn load_game_state_missing_file_returns_none() {
        let path = gs_path("missing_xyz");
        let _ = fs::remove_file(&path);
        assert!(load_game_state_from(&path).is_none());
    }

    #[test]
    fn load_game_state_corrupt_file_returns_none() {
        let path = gs_path("corrupt");
        fs::write(&path, b"not valid json!!!").expect("write");
        assert!(load_game_state_from(&path).is_none());
    }

    #[test]
    fn save_game_state_skips_won_games() {
        use solitaire_core::game_state::{DrawMode, GameState};
        let path = gs_path("won_skip");
        let _ = fs::remove_file(&path);

        let mut gs = GameState::new(99, DrawMode::DrawOne);
        gs.is_won = true;
        save_game_state_to(&path, &gs).expect("save should be no-op, not error");
        assert!(!path.exists(), "should not have written a file for a won game");
    }

    #[test]
    fn load_game_state_ignores_won_games() {
        use solitaire_core::game_state::{DrawMode, GameState};
        let path = gs_path("won_load");
        let _ = fs::remove_file(&path);

        // Write a won game directly (bypassing save_game_state_to's guard).
        let mut gs = GameState::new(77, DrawMode::DrawOne);
        gs.is_won = true;
        let json = serde_json::to_string_pretty(&gs).unwrap();
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, json.as_bytes()).unwrap();
        fs::rename(&tmp, &path).unwrap();

        assert!(load_game_state_from(&path).is_none());
    }

    #[test]
    fn delete_game_state_removes_file() {
        use solitaire_core::game_state::{DrawMode, GameState};
        let path = gs_path("delete");
        let gs = GameState::new(1, DrawMode::DrawOne);
        save_game_state_to(&path, &gs).expect("save");
        assert!(path.exists());
        delete_game_state_at(&path).expect("delete");
        assert!(!path.exists());
    }

    #[test]
    fn delete_game_state_missing_file_is_ok() {
        let path = gs_path("delete_missing");
        let _ = fs::remove_file(&path);
        assert!(delete_game_state_at(&path).is_ok());
    }

    #[test]
    fn save_game_state_is_atomic() {
        use solitaire_core::game_state::{DrawMode, GameState};
        let path = gs_path("atomic");
        let gs = GameState::new(55, DrawMode::DrawThree);
        save_game_state_to(&path, &gs).expect("save");
        let tmp = path.with_extension("json.tmp");
        assert!(!tmp.exists(), ".tmp must be cleaned up after rename");
    }

    /// Pre-v2 save files used `Foundation(Suit)` keys and either fail to
    /// parse outright or surface a `schema_version: 1`. Either path must
    /// produce `None` so the player launches into a fresh game.
    ///
    /// Sibling assertion: the stats round-trip path is unaffected — only
    /// the game-state schema bumped.
    #[test]
    fn save_format_v1_is_rejected() {
        let path = gs_path("schema_v1");
        let _ = fs::remove_file(&path);

        // A pared-down v1 JSON literal: foundation pile keys use the old
        // suit-tagged form and the file omits `schema_version` (so it
        // deserialises with the default of 1). Even if a future change
        // makes `Foundation(Suit)` parse-compatible, the schema-version
        // gate keeps this case rejected.
        let v1_json = r#"{
            "piles": [
                [{"Foundation": "Hearts"}, {"pile_type": {"Foundation": "Hearts"}, "cards": []}]
            ],
            "draw_mode": "DrawOne",
            "score": 0,
            "move_count": 0,
            "elapsed_seconds": 0,
            "seed": 42,
            "is_won": false,
            "is_auto_completable": false,
            "undo_count": 0,
            "undo_stack": []
        }"#;
        fs::write(&path, v1_json).expect("write v1 fixture");

        assert!(
            load_game_state_from(&path).is_none(),
            "v1 game_state.json must be rejected (parse failure or schema bump)",
        );

        // Sibling sanity: stats files are independent and still round-trip.
        let stats_path = tmp_path("schema_unrelated_stats");
        let _ = fs::remove_file(&stats_path);
        save_stats_to(&stats_path, &StatsSnapshot::default()).expect("save stats");
        let loaded = load_stats_from(&stats_path);
        assert_eq!(loaded, StatsSnapshot::default());
    }

    // -----------------------------------------------------------------------
    // Time Attack session persistence
    //
    // Documents the contract that closing the window mid-Time-Attack does
    // NOT lose the 10-minute window or the running win count. Classic /
    // Zen / Challenge are covered by `game_state.json` because their entire
    // mid-deal state lives in `GameState.mode` + `GameState.piles`; Time
    // Attack additionally needs the session timer + wins counter, both of
    // which live in `TimeAttackResource` on the engine side and are NOT
    // part of `GameState`. This sibling file persists exactly that.
    // -----------------------------------------------------------------------

    fn ta_path(name: &str) -> PathBuf {
        env::temp_dir().join(format!("solitaire_test_ta_{name}.json"))
    }

    /// Round-trip a session that was saved "just now" (zero wall-clock
    /// elapsed). All three persisted fields must come back unchanged.
    #[test]
    fn time_attack_session_round_trips_through_save_and_load() {
        let path = ta_path("round_trip");
        let _ = fs::remove_file(&path);

        // Use a fixed unix timestamp so the load step (which receives the
        // SAME timestamp as "now") sees zero wall-clock elapsed.
        let saved_at: u64 = 1_800_000_000;
        let session = TimeAttackSession {
            remaining_secs: 240.0,
            wins: 3,
            saved_at_unix_secs: saved_at,
        };
        save_time_attack_session_to(&path, &session).expect("save");

        let loaded = load_time_attack_session_from_at(&path, saved_at)
            .expect("session must load when not yet expired");
        assert!(
            (loaded.remaining_secs - 240.0).abs() < 0.01,
            "remaining_secs must be unchanged when no wall-clock time has passed; got {}",
            loaded.remaining_secs,
        );
        assert_eq!(loaded.wins, 3, "wins must round-trip");
        assert_eq!(loaded.saved_at_unix_secs, saved_at, "timestamp must round-trip");

        let _ = fs::remove_file(&path);
    }

    /// A session whose window expired entirely between launches must be
    /// discarded on load — the caller starts fresh rather than resuming a
    /// dead session.
    #[test]
    fn time_attack_session_discarded_when_expired_between_launches() {
        let path = ta_path("expired");
        let _ = fs::remove_file(&path);

        // Saved 20 minutes ago with 240 s remaining — long expired.
        let saved_at: u64 = 1_800_000_000;
        let session = TimeAttackSession {
            remaining_secs: 240.0,
            wins: 5,
            saved_at_unix_secs: saved_at,
        };
        save_time_attack_session_to(&path, &session).expect("save");

        // 20 minutes (1200 s) later → 240 - 1200 = -960 s remaining.
        let now = saved_at + 1200;
        assert!(
            load_time_attack_session_from_at(&path, now).is_none(),
            "an expired session must return None so the player starts fresh",
        );

        let _ = fs::remove_file(&path);
    }

    /// The `remaining_secs` returned at load time must be the persisted
    /// value minus the wall-clock seconds that elapsed while the app was
    /// closed.
    #[test]
    fn time_attack_session_remaining_secs_decremented_by_real_elapsed() {
        let path = ta_path("decremented");
        let _ = fs::remove_file(&path);

        let saved_at: u64 = 1_800_000_000;
        let session = TimeAttackSession {
            remaining_secs: 240.0,
            wins: 2,
            saved_at_unix_secs: saved_at,
        };
        save_time_attack_session_to(&path, &session).expect("save");

        // 60 s elapsed in real time → expect 180 s remaining.
        let now = saved_at + 60;
        let loaded = load_time_attack_session_from_at(&path, now)
            .expect("session must still load — 180 s left");
        assert!(
            (loaded.remaining_secs - 180.0).abs() < 5.0,
            "remaining_secs ≈ 180 ± 5 s after a 60 s wall-clock gap; got {}",
            loaded.remaining_secs,
        );
        assert_eq!(loaded.wins, 2, "wins must survive the elapsed adjustment");

        let _ = fs::remove_file(&path);
    }

    /// Atomic-write contract — `.tmp` must not be left behind after
    /// `save_time_attack_session_to` returns.
    #[test]
    fn time_attack_session_save_is_atomic() {
        let path = ta_path("atomic");
        let session = TimeAttackSession {
            remaining_secs: 100.0,
            wins: 0,
            saved_at_unix_secs: 1_800_000_000,
        };
        save_time_attack_session_to(&path, &session).expect("save");
        let tmp = path.with_extension("json.tmp");
        assert!(!tmp.exists(), ".tmp must be cleaned up after rename");
        let _ = fs::remove_file(&path);
    }

    /// Loading from a path that does not exist must return `None`, not
    /// panic.
    #[test]
    fn time_attack_session_missing_file_returns_none() {
        let path = ta_path("missing_xyz");
        let _ = fs::remove_file(&path);
        assert!(load_time_attack_session_from_at(&path, 0).is_none());
    }

    /// Loading from a corrupt / partially-written file must return `None`,
    /// not surface a deserialiser error.
    #[test]
    fn time_attack_session_corrupt_file_returns_none() {
        let path = ta_path("corrupt");
        fs::write(&path, b"not valid json!!!").expect("write");
        assert!(load_time_attack_session_from_at(&path, 0).is_none());
        let _ = fs::remove_file(&path);
    }

    /// `delete_time_attack_session_at` removes the file when it exists
    /// and returns `Ok(())` when it does not.
    #[test]
    fn time_attack_session_delete_handles_present_and_absent() {
        let path = ta_path("delete");
        let session = TimeAttackSession {
            remaining_secs: 50.0,
            wins: 0,
            saved_at_unix_secs: 1_800_000_000,
        };
        save_time_attack_session_to(&path, &session).expect("save");
        assert!(path.exists());
        delete_time_attack_session_at(&path).expect("delete");
        assert!(!path.exists());
        // Second delete on the now-absent file must succeed.
        delete_time_attack_session_at(&path).expect("missing-file delete is ok");
    }

    /// A session whose `saved_at_unix_secs` is in the future (e.g. the
    /// system clock moved backward across NTP correction) must NOT be
    /// rejected as expired. Saturating subtraction must clamp the
    /// "elapsed" value to zero.
    #[test]
    fn time_attack_session_handles_clock_running_backwards() {
        let path = ta_path("clock_backwards");
        let _ = fs::remove_file(&path);

        let saved_at: u64 = 1_800_000_000;
        let session = TimeAttackSession {
            remaining_secs: 60.0,
            wins: 1,
            saved_at_unix_secs: saved_at,
        };
        save_time_attack_session_to(&path, &session).expect("save");

        // "now" is BEFORE the saved time — should not crash, should not expire.
        let now_in_past = saved_at - 100;
        let loaded = load_time_attack_session_from_at(&path, now_in_past)
            .expect("clock-backwards must not discard the session");
        assert!(
            (loaded.remaining_secs - 60.0).abs() < 0.01,
            "remaining_secs must clamp elapsed to 0 when clock ran backwards; got {}",
            loaded.remaining_secs,
        );

        let _ = fs::remove_file(&path);
    }
}
