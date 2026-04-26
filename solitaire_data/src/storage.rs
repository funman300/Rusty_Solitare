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
                .map(|n| n.ends_with(".json.tmp"))
                .unwrap_or(false)
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
}
