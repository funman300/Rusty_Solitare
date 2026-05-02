//! User settings (persistent).
//!
//! Tracks draw mode, volumes, animation speed, visual theme, sync backend, and
//! the first-run flag. All fields use `#[serde(default)]` so settings files
//! written by older versions of the game still deserialize correctly.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use solitaire_core::game_state::DrawMode;

const APP_DIR_NAME: &str = "solitaire_quest";
const SETTINGS_FILE_NAME: &str = "settings.json";

/// Animation playback speed for card transitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum AnimSpeed {
    /// Standard animation timing (default).
    #[default]
    Normal,
    /// Roughly 2× faster than Normal.
    Fast,
    /// Skip animations entirely — cards teleport to their destinations.
    Instant,
}

/// Visual theme applied to the table background and UI chrome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Theme {
    /// Classic green felt (default).
    #[default]
    Green,
    /// Blue felt variant.
    Blue,
    /// Dark / night-mode variant.
    Dark,
}

/// Which sync backend the player has configured.
///
/// `Local` keeps all progress on-device. `SolitaireServer` syncs via the
/// self-hosted server. JWT tokens are stored in the OS keychain via
/// `solitaire_data::auth_tokens` — **never** in this struct.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub enum SyncBackend {
    /// No sync — all progress stays on the local device (default).
    #[default]
    #[serde(rename = "local")]
    Local,
    /// Sync with a self-hosted Solitaire Quest server.
    #[serde(rename = "solitaire_server")]
    SolitaireServer {
        /// Base URL of the server, e.g. `"https://solitaire.example.com"`.
        url: String,
        /// The player's username on that server.
        username: String,
        // JWT tokens are stored in the OS keychain — not here.
    },

}

/// Persisted window size (in logical pixels) and screen position
/// (top-left corner, in physical pixels) — restored on next launch.
///
/// Stored inside [`Settings::window_geometry`]. `None` on `Settings`
/// means "use platform defaults"; a populated value is written every
/// time the player resizes or moves the window so the next launch
/// reopens at the same geometry.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WindowGeometry {
    /// Logical width of the window in pixels.
    pub width: u32,
    /// Logical height of the window in pixels.
    pub height: u32,
    /// X coordinate of the window's top-left corner, in physical pixels.
    pub x: i32,
    /// Y coordinate of the window's top-left corner, in physical pixels.
    pub y: i32,
}

/// Persistent user settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    /// Draw mode selected for new games.
    #[serde(default = "default_draw_mode")]
    pub draw_mode: DrawMode,
    /// Linear SFX volume in `[0.0, 1.0]`. Applied to kira's SFX channel gain.
    #[serde(default = "default_sfx_volume")]
    pub sfx_volume: f32,
    /// Linear music volume in `[0.0, 1.0]`. Applied to kira's music channel gain.
    #[serde(default = "default_music_volume")]
    pub music_volume: f32,
    /// Speed at which card animations play.
    #[serde(default)]
    pub animation_speed: AnimSpeed,
    /// Visual theme for the table and UI.
    #[serde(default)]
    pub theme: Theme,
    /// Which sync backend is active.
    #[serde(default)]
    pub sync_backend: SyncBackend,
    /// Index of the card-back design currently in use (0 = default).
    /// Only indices present in `PlayerProgress::unlocked_card_backs` are valid.
    #[serde(default)]
    pub selected_card_back: usize,
    /// Index of the background design currently in use (0 = default).
    /// Only indices present in `PlayerProgress::unlocked_backgrounds` are valid.
    #[serde(default)]
    pub selected_background: usize,
    /// Set to `true` once the player has dismissed the first-run banner.
    #[serde(default)]
    pub first_run_complete: bool,
    /// When `true`, red-suit card faces use a blue tint instead of the default
    /// cream so they are distinguishable from black-suit cards without relying
    /// solely on colour.
    #[serde(default)]
    pub color_blind_mode: bool,
    /// Window size and screen position to restore on next launch. `None`
    /// means "use platform defaults" — set on first run, then populated
    /// as the player resizes / moves the window. Older `settings.json`
    /// files written before this field existed deserialize cleanly to
    /// `None` thanks to `#[serde(default)]`.
    #[serde(default)]
    pub window_geometry: Option<WindowGeometry>,
    /// Identifier of the active card-art theme. Matches `meta.id` from
    /// the theme's `theme.ron` manifest. `"default"` is the bundled
    /// theme and is always present in the registry; user-supplied
    /// themes register under their own ids when they're imported.
    /// Older `settings.json` files default cleanly to `"default"` via
    /// `#[serde(default = ...)]`.
    #[serde(default = "default_theme_id")]
    pub selected_theme_id: String,
    /// Set to `true` once the achievement-onboarding info-toast has been
    /// shown to the player after their very first win. Acts as a
    /// one-shot teach: subsequent wins must not re-fire the cue. Older
    /// `settings.json` files written before this field existed
    /// deserialize cleanly to `false` thanks to `#[serde(default)]` —
    /// players who already had wins recorded before this field was
    /// introduced are guarded by the post-condition `games_won == 1`
    /// checked by `achievement_plugin::fire_achievement_onboarding_toast`,
    /// so the toast still does not fire for them.
    #[serde(default)]
    pub shown_achievement_onboarding: bool,
}

fn default_draw_mode() -> DrawMode {
    DrawMode::DrawOne
}

fn default_sfx_volume() -> f32 {
    0.8
}

fn default_music_volume() -> f32 {
    0.5
}

fn default_theme_id() -> String {
    "default".to_string()
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            draw_mode: DrawMode::DrawOne,
            sfx_volume: default_sfx_volume(),
            music_volume: default_music_volume(),
            animation_speed: AnimSpeed::Normal,
            theme: Theme::Green,
            sync_backend: SyncBackend::Local,
            selected_card_back: 0,
            selected_background: 0,
            first_run_complete: false,
            color_blind_mode: false,
            window_geometry: None,
            selected_theme_id: default_theme_id(),
            shown_achievement_onboarding: false,
        }
    }
}

impl Settings {
    /// Clamps both `sfx_volume` and `music_volume` into `[0.0, 1.0]` after
    /// deserialization or hand-editing of `settings.json`.
    pub fn sanitized(self) -> Self {
        Self {
            sfx_volume: self.sfx_volume.clamp(0.0, 1.0),
            music_volume: self.music_volume.clamp(0.0, 1.0),
            ..self
        }
    }

    /// Adjust SFX volume by `delta`, clamped to `[0.0, 1.0]`. Returns the new value.
    pub fn adjust_sfx_volume(&mut self, delta: f32) -> f32 {
        self.sfx_volume = (self.sfx_volume + delta).clamp(0.0, 1.0);
        self.sfx_volume
    }

    /// Adjust music volume by `delta`, clamped to `[0.0, 1.0]`. Returns the new value.
    pub fn adjust_music_volume(&mut self, delta: f32) -> f32 {
        self.music_volume = (self.music_volume + delta).clamp(0.0, 1.0);
        self.music_volume
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
        assert!((s.music_volume - 0.5).abs() < 1e-6);
        assert!(!s.first_run_complete);
        assert_eq!(s.draw_mode, DrawMode::DrawOne);
        assert_eq!(s.animation_speed, AnimSpeed::Normal);
        assert_eq!(s.theme, Theme::Green);
        assert_eq!(s.sync_backend, SyncBackend::Local);
    }

    #[test]
    fn adjust_sfx_volume_clamps() {
        let mut s = Settings { sfx_volume: 0.5, ..Default::default() };
        assert!((s.adjust_sfx_volume(0.3) - 0.8).abs() < 1e-6);
        assert!((s.adjust_sfx_volume(0.5) - 1.0).abs() < 1e-6);
        assert!((s.adjust_sfx_volume(-2.0) - 0.0).abs() < 1e-6);
        assert!((s.adjust_sfx_volume(-1.0) - 0.0).abs() < 1e-6);
    }

    #[test]
    fn adjust_music_volume_clamps() {
        let mut s = Settings { music_volume: 0.5, ..Default::default() };
        assert!((s.adjust_music_volume(0.3) - 0.8).abs() < 1e-6);
        assert!((s.adjust_music_volume(0.5) - 1.0).abs() < 1e-6);
        assert!((s.adjust_music_volume(-2.0) - 0.0).abs() < 1e-6);
        assert!((s.adjust_music_volume(-1.0) - 0.0).abs() < 1e-6);
    }

    #[test]
    fn sanitized_clamps_out_of_range_volume() {
        let s = Settings {
            sfx_volume: 5.0,
            music_volume: -1.5,
            first_run_complete: true,
            ..Settings::default()
        }
        .sanitized();
        assert_eq!(s.sfx_volume, 1.0);
        assert_eq!(s.music_volume, 0.0);
        assert!(s.first_run_complete);
    }

    #[test]
    fn sanitized_clamps_music_volume() {
        let s = Settings { music_volume: 2.0, ..Default::default() }.sanitized();
        assert_eq!(s.music_volume, 1.0);

        let s2 = Settings { music_volume: -0.5, ..Default::default() }.sanitized();
        assert_eq!(s2.music_volume, 0.0);
    }

    #[test]
    fn round_trip_save_and_load() {
        let path = tmp_path("round_trip");
        let _ = fs::remove_file(&path);
        let s = Settings {
            sfx_volume: 0.42,
            first_run_complete: true,
            ..Settings::default()
        };
        save_settings_to(&path, &s).expect("save");
        let loaded = load_settings_from(&path);
        assert_eq!(loaded, s);
    }

    #[test]
    fn round_trip_save_and_load_full_settings() {
        let path = tmp_path("round_trip_full");
        let _ = fs::remove_file(&path);
        let s = Settings {
            draw_mode: DrawMode::DrawThree,
            sfx_volume: 0.3,
            music_volume: 0.7,
            animation_speed: AnimSpeed::Fast,
            theme: Theme::Dark,
            sync_backend: SyncBackend::SolitaireServer {
                url: "https://example.com".to_string(),
                username: "testuser".to_string(),
            },
            selected_card_back: 0,
            selected_background: 0,
            first_run_complete: true,
            color_blind_mode: false,
            window_geometry: None,
            selected_theme_id: "default".to_string(),
            shown_achievement_onboarding: false,
        };
        save_settings_to(&path, &s).expect("save");
        let loaded = load_settings_from(&path);
        assert_eq!(loaded, s);
    }

    #[test]
    fn round_trip_preserves_non_default_cosmetic_selections() {
        // selected_card_back and selected_background must survive save→load with
        // non-zero values — zero is the default and not a meaningful regression check.
        let path = tmp_path("cosmetic_selections");
        let _ = fs::remove_file(&path);
        let s = Settings {
            selected_card_back: 3,
            selected_background: 2,
            ..Settings::default()
        };
        save_settings_to(&path, &s).expect("save");
        let loaded = load_settings_from(&path);
        assert_eq!(loaded.selected_card_back, 3);
        assert_eq!(loaded.selected_background, 2);
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

    #[test]
    fn load_from_old_format_uses_defaults_for_new_fields() {
        // Simulate a settings.json written by an older version that only had
        // sfx_volume and first_run_complete.
        let path = tmp_path("old_format");
        fs::write(
            &path,
            br#"{ "sfx_volume": 0.6, "first_run_complete": true }"#,
        )
        .expect("write");
        let s = load_settings_from(&path);
        assert!((s.sfx_volume - 0.6).abs() < 1e-6);
        assert!(s.first_run_complete);
        // New fields should fall back to their defaults.
        assert!((s.music_volume - 0.5).abs() < 1e-6);
        assert_eq!(s.animation_speed, AnimSpeed::Normal);
        assert_eq!(s.theme, Theme::Green);
        assert_eq!(s.sync_backend, SyncBackend::Local);
        assert_eq!(s.draw_mode, DrawMode::DrawOne);
        assert_eq!(s.selected_card_back, 0, "cosmetic card-back must default to 0 on old format");
        assert_eq!(s.selected_background, 0, "cosmetic background must default to 0 on old format");
        assert!(!s.color_blind_mode, "color_blind_mode must default to false on old format");
    }

    #[test]
    fn color_blind_mode_defaults_to_false_when_field_absent() {
        // Simulate a JSON file that has no color_blind_mode field.
        let json = br#"{ "sfx_volume": 0.7 }"#;
        let s: Settings = serde_json::from_slice(json).unwrap_or_default();
        assert!(!s.color_blind_mode, "color_blind_mode must be false when absent from JSON");
    }

    #[test]
    fn color_blind_mode_round_trips() {
        let path = tmp_path("color_blind");
        let _ = std::fs::remove_file(&path);
        let s = Settings {
            color_blind_mode: true,
            ..Settings::default()
        };
        save_settings_to(&path, &s).expect("save");
        let loaded = load_settings_from(&path);
        assert!(loaded.color_blind_mode, "color_blind_mode must survive a save/load round-trip");
        let _ = std::fs::remove_file(&path);
    }

    // -----------------------------------------------------------------------
    // Task #62 — selected_card_back
    // -----------------------------------------------------------------------

    #[test]
    fn settings_card_back_default_is_zero() {
        assert_eq!(Settings::default().selected_card_back, 0);
    }

    #[test]
    fn settings_card_back_serializes_round_trip() {
        let path = tmp_path("card_back_round_trip");
        let _ = fs::remove_file(&path);
        let s = Settings {
            selected_card_back: 2,
            ..Settings::default()
        };
        save_settings_to(&path, &s).expect("save");
        let loaded = load_settings_from(&path);
        assert_eq!(loaded.selected_card_back, 2, "selected_card_back must survive serde round-trip");
        let _ = fs::remove_file(&path);
    }

    // -----------------------------------------------------------------------
    // Task #63 — selected_background
    // -----------------------------------------------------------------------

    #[test]
    fn settings_background_default_is_zero() {
        assert_eq!(Settings::default().selected_background, 0);
    }

    #[test]
    fn settings_background_serializes_round_trip() {
        let path = tmp_path("background_round_trip");
        let _ = fs::remove_file(&path);
        let s = Settings {
            selected_background: 3,
            ..Settings::default()
        };
        save_settings_to(&path, &s).expect("save");
        let loaded = load_settings_from(&path);
        assert_eq!(loaded.selected_background, 3, "selected_background must survive serde round-trip");
        let _ = fs::remove_file(&path);
    }

    // -----------------------------------------------------------------------
    // window_geometry — persisted window size/position
    // -----------------------------------------------------------------------

    #[test]
    fn settings_window_geometry_default_is_none() {
        assert!(
            Settings::default().window_geometry.is_none(),
            "default window_geometry must be None so first launch uses platform defaults"
        );
    }

    #[test]
    fn settings_with_window_geometry_round_trip() {
        let path = tmp_path("window_geometry_round_trip");
        let _ = fs::remove_file(&path);
        let geom = WindowGeometry {
            width: 1440,
            height: 900,
            x: 120,
            y: 80,
        };
        let s = Settings {
            window_geometry: Some(geom),
            ..Settings::default()
        };
        save_settings_to(&path, &s).expect("save");
        let loaded = load_settings_from(&path);
        assert_eq!(
            loaded.window_geometry,
            Some(geom),
            "window_geometry must survive serde round-trip"
        );
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn legacy_settings_without_window_geometry_deserializes_to_none() {
        // A settings.json written by an older version of the game will be
        // missing this field entirely. `#[serde(default)]` on the field
        // must yield `None` rather than failing the whole deserialise.
        let json = br#"{ "sfx_volume": 0.7, "first_run_complete": true }"#;
        let s: Settings = serde_json::from_slice(json).unwrap_or_default();
        assert!(
            s.window_geometry.is_none(),
            "legacy settings.json missing window_geometry must deserialize to None"
        );
    }

    #[test]
    fn window_geometry_explicit_null_deserializes_to_none() {
        // An explicit `"window_geometry": null` is also valid input that
        // must yield None — keeps tooling that hand-edits the file safe.
        let json = br#"{ "window_geometry": null }"#;
        let s: Settings = serde_json::from_slice(json).unwrap_or_default();
        assert!(s.window_geometry.is_none());
    }

    // -----------------------------------------------------------------------
    // shown_achievement_onboarding — first-win cue one-shot guard
    // -----------------------------------------------------------------------

    #[test]
    fn settings_shown_achievement_onboarding_default_is_false() {
        assert!(
            !Settings::default().shown_achievement_onboarding,
            "default shown_achievement_onboarding must be false so the cue fires once"
        );
    }

    #[test]
    fn settings_shown_achievement_onboarding_round_trip() {
        let path = tmp_path("achievement_onboarding_round_trip");
        let _ = fs::remove_file(&path);
        let s = Settings {
            shown_achievement_onboarding: true,
            ..Settings::default()
        };
        save_settings_to(&path, &s).expect("save");
        let loaded = load_settings_from(&path);
        assert!(
            loaded.shown_achievement_onboarding,
            "shown_achievement_onboarding must survive serde round-trip"
        );
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn legacy_settings_without_shown_achievement_onboarding_deserializes_to_false() {
        // A settings.json written by an older version of the game will be
        // missing this field entirely. `#[serde(default)]` on the field
        // must yield `false` — the cue then fires on the next win, but
        // only when stats.games_won == 1, so existing players who have
        // already won past their first game won't see the toast either.
        let json = br#"{ "sfx_volume": 0.7, "first_run_complete": true }"#;
        let s: Settings = serde_json::from_slice(json).unwrap_or_default();
        assert!(
            !s.shown_achievement_onboarding,
            "legacy settings.json missing shown_achievement_onboarding must deserialize to false"
        );
    }
}
