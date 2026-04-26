//! User settings (persistent).
//!
//! Currently tracks SFX volume and the first-run flag. Other fields from
//! ARCHITECTURE.md §9 (`draw_mode`, `music_volume`, `theme`, `sync_backend`)
//! will land alongside the systems that need them.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const APP_DIR_NAME: &str = "solitaire_quest";
const SETTINGS_FILE_NAME: &str = "settings.json";

/// Persistent user settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    /// Linear SFX volume in `[0.0, 1.0]`. Applied to kira's main track gain.
    pub sfx_volume: f32,
    /// Set to `true` once the player has dismissed the first-run banner.
    pub first_run_complete: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            sfx_volume: 0.8,
            first_run_complete: false,
        }
    }
}

impl Settings {
    /// Clamps `sfx_volume` into `[0.0, 1.0]` after deserialization or
    /// hand-editing of `settings.json`.
    pub fn sanitized(self) -> Self {
        Self {
            sfx_volume: self.sfx_volume.clamp(0.0, 1.0),
            ..self
        }
    }

    /// Adjust SFX volume by `delta`, clamped to `[0.0, 1.0]`. Returns the new value.
    pub fn adjust_sfx_volume(&mut self, delta: f32) -> f32 {
        self.sfx_volume = (self.sfx_volume + delta).clamp(0.0, 1.0);
        self.sfx_volume
    }
}

/// Returns the platform-specific path to `settings.json`, or `None` if
/// `dirs::data_dir()` is unavailable.
pub fn settings_file_path() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join(APP_DIR_NAME).join(SETTINGS_FILE_NAME))
}

/// Load settings from an explicit path. Returns `Settings::default()` if the
/// file is missing or cannot be deserialized.
pub fn load_settings_from(path: &Path) -> Settings {
    let Ok(data) = fs::read(path) else {
        return Settings::default();
    };
    serde_json::from_slice::<Settings>(&data)
        .unwrap_or_default()
        .sanitized()
}

/// Save settings to an explicit path using an atomic write (`.tmp` → rename).
pub fn save_settings_to(path: &Path, settings: &Settings) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(settings).map_err(io::Error::other)?;
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
        env::temp_dir().join(format!("solitaire_settings_test_{name}.json"))
    }

    #[test]
    fn defaults_are_reasonable() {
        let s = Settings::default();
        assert!((s.sfx_volume - 0.8).abs() < 1e-6);
        assert!(!s.first_run_complete);
    }

    #[test]
    fn adjust_sfx_volume_clamps() {
        let mut s = Settings::default();
        s.sfx_volume = 0.5;
        assert!((s.adjust_sfx_volume(0.3) - 0.8).abs() < 1e-6);
        assert!((s.adjust_sfx_volume(0.5) - 1.0).abs() < 1e-6);
        assert!((s.adjust_sfx_volume(-2.0) - 0.0).abs() < 1e-6);
        assert!((s.adjust_sfx_volume(-1.0) - 0.0).abs() < 1e-6);
    }

    #[test]
    fn sanitized_clamps_out_of_range_volume() {
        let s = Settings {
            sfx_volume: 5.0,
            first_run_complete: true,
        }
        .sanitized();
        assert_eq!(s.sfx_volume, 1.0);
        assert!(s.first_run_complete);
    }

    #[test]
    fn round_trip_save_and_load() {
        let path = tmp_path("round_trip");
        let _ = fs::remove_file(&path);
        let s = Settings {
            sfx_volume: 0.42,
            first_run_complete: true,
        };
        save_settings_to(&path, &s).expect("save");
        let loaded = load_settings_from(&path);
        assert_eq!(loaded, s);
    }

    #[test]
    fn load_from_missing_file_returns_default() {
        let path = tmp_path("missing_xyz");
        let _ = fs::remove_file(&path);
        let s = load_settings_from(&path);
        assert_eq!(s, Settings::default());
    }

    #[test]
    fn load_from_corrupt_file_returns_default() {
        let path = tmp_path("corrupt");
        fs::write(&path, b"definitely not json").expect("write");
        let s = load_settings_from(&path);
        assert_eq!(s, Settings::default());
    }
}
