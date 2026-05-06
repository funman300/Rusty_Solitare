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
    /// Hover delay (seconds) before a tooltip appears. Range
    /// `[0.0, 1.5]`; default matches `MOTION_TOOLTIP_DELAY_SECS` (0.5 s).
    /// `0.0` means tooltips fire on the very next tick after hover —
    /// the "Instant" setting. Older `settings.json` files written before
    /// this field existed deserialize cleanly to the default via
    /// `#[serde(default = "default_tooltip_delay")]`.
    #[serde(default = "default_tooltip_delay")]
    pub tooltip_delay_secs: f32,
    /// Multiplier applied to the post-game time-bonus score component
    /// shown in the win-summary modal. Range
    /// `[TIME_BONUS_MULTIPLIER_MIN, TIME_BONUS_MULTIPLIER_MAX]`
    /// (`0.0`–`2.0`); default `1.0` keeps the existing behaviour.
    ///
    /// **COSMETIC ONLY** — this multiplier changes what the player
    /// sees in the win modal's score breakdown but does **not** affect
    /// achievement unlock thresholds, lifetime score totals, or
    /// leaderboard submissions, which all use the raw, unmultiplied
    /// score values produced by `solitaire_core`. Older
    /// `settings.json` files written before this field existed
    /// deserialize cleanly to `1.0` via
    /// `#[serde(default = "default_time_bonus_multiplier")]`.
    #[serde(default = "default_time_bonus_multiplier")]
    pub time_bonus_multiplier: f32,
    /// When `true`, the engine rejects new-game deals the
    /// [`solitaire_core::solver`] cannot prove winnable, retrying
    /// fresh seeds up to [`SOLVER_DEAL_RETRY_CAP`] attempts before
    /// giving up and using the last tried seed. Off by default —
    /// the solver adds a few hundred milliseconds of latency on the
    /// pathological deals that hit the budget cap, and not every
    /// player wants to wait. Older `settings.json` files written
    /// before this field existed deserialize cleanly to `false` via
    /// `#[serde(default)]`.
    ///
    /// Scope: only random-seed Classic-mode deals are filtered.
    /// Daily challenges, replays, and explicit-seed requests skip the
    /// solver retry loop — see `solitaire_engine::handle_new_game`.
    #[serde(default)]
    pub winnable_deals_only: bool,
    /// Per-move duration during replay playback, in seconds. Range
    /// `[REPLAY_MOVE_INTERVAL_MIN_SECS, REPLAY_MOVE_INTERVAL_MAX_SECS]`;
    /// default mirrors `solitaire_engine::replay_playback::REPLAY_MOVE_INTERVAL_SECS`
    /// (0.45 s/move) so existing playback behaviour is unchanged for
    /// players who never touch the slider. Smaller values scrub
    /// faster through the recorded move list. Older `settings.json`
    /// files written before this field existed deserialize cleanly to
    /// the default via
    /// `#[serde(default = "default_replay_move_interval_secs")]`.
    #[serde(default = "default_replay_move_interval_secs")]
    pub replay_move_interval_secs: f32,
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

/// Default tooltip-hover dwell delay in seconds. Mirrors
/// `solitaire_engine::ui_theme::MOTION_TOOLTIP_DELAY_SECS` so legacy
/// `settings.json` files load to the existing baseline. The constant
/// lives in the engine crate (which the data crate cannot depend on),
/// so the value is duplicated here — kept in sync by the
/// `settings_tooltip_delay_default_is_existing_baseline` test in
/// `solitaire_engine::settings_plugin`.
fn default_tooltip_delay() -> f32 {
    0.5
}

/// Lower bound of the player-tunable tooltip delay slider, in seconds.
pub const TOOLTIP_DELAY_MIN_SECS: f32 = 0.0;

/// Upper bound of the player-tunable tooltip delay slider, in seconds.
pub const TOOLTIP_DELAY_MAX_SECS: f32 = 1.5;

/// Increment applied by the tooltip-delay decrement / increment buttons.
pub const TOOLTIP_DELAY_STEP_SECS: f32 = 0.1;

/// Lower bound of the player-tunable time-bonus multiplier. `0.0`
/// disables the time-bonus row entirely (renders as "Off" in the UI).
pub const TIME_BONUS_MULTIPLIER_MIN: f32 = 0.0;

/// Upper bound of the player-tunable time-bonus multiplier. `2.0`
/// doubles the displayed time bonus.
pub const TIME_BONUS_MULTIPLIER_MAX: f32 = 2.0;

/// Increment applied by the time-bonus multiplier decrement /
/// increment buttons.
pub const TIME_BONUS_MULTIPLIER_STEP: f32 = 0.1;

/// Default value for [`Settings::time_bonus_multiplier`]. `1.0` keeps
/// the displayed time bonus identical to the raw value produced by
/// `solitaire_core::scoring::compute_time_bonus`.
fn default_time_bonus_multiplier() -> f32 {
    1.0
}

/// Default per-move duration during replay playback, in seconds.
/// Mirrors `solitaire_engine::replay_playback::REPLAY_MOVE_INTERVAL_SECS`
/// so legacy `settings.json` files load to the existing baseline and
/// playback feels identical for players who never touch the slider.
/// The constant is duplicated across the data and engine crates
/// because `solitaire_data` cannot depend on the engine crate — keep
/// the two values in sync when adjusting either.
fn default_replay_move_interval_secs() -> f32 {
    0.45
}

/// Lower bound of the player-tunable replay-playback per-move interval,
/// in seconds. Below this the cards barely register visually before
/// the next move fires; the cap keeps the playback legible.
pub const REPLAY_MOVE_INTERVAL_MIN_SECS: f32 = 0.10;

/// Upper bound of the player-tunable replay-playback per-move interval,
/// in seconds. One second per move is a comfortable upper limit for
/// players who want to study a recorded game frame by frame.
pub const REPLAY_MOVE_INTERVAL_MAX_SECS: f32 = 1.00;

/// Increment applied by the replay-playback decrement / increment
/// buttons. 0.05 s gives 19 stops between MIN and MAX — fine-grained
/// enough to land on any "round" speed (0.10 s, 0.25 s, 0.45 s, etc.)
/// without making the slider feel stuck on the same value.
pub const REPLAY_MOVE_INTERVAL_STEP_SECS: f32 = 0.05;

/// Maximum number of seed retries [`solitaire_engine::handle_new_game`]
/// is willing to attempt before giving up and accepting the latest
/// candidate seed when [`Settings::winnable_deals_only`] is on. If
/// every retry comes back [`SolverResult::Unwinnable`] (which would
/// be very unusual) we'd rather hand the player a possibly-unwinnable
/// deal than spin forever on the main thread.
///
/// 50 attempts × ~50 ms median per solve = ~2.5 s worst-case stall —
/// the upper bound on UI freeze when the toggle is on.
pub const SOLVER_DEAL_RETRY_CAP: u32 = 50;

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
            tooltip_delay_secs: default_tooltip_delay(),
            time_bonus_multiplier: default_time_bonus_multiplier(),
            winnable_deals_only: false,
            replay_move_interval_secs: default_replay_move_interval_secs(),
        }
    }
}

impl Settings {
    /// Clamps `sfx_volume`, `music_volume`, `tooltip_delay_secs`,
    /// `time_bonus_multiplier`, and `replay_move_interval_secs` into
    /// their respective ranges after deserialization or hand-editing of
    /// `settings.json`.
    pub fn sanitized(self) -> Self {
        Self {
            sfx_volume: self.sfx_volume.clamp(0.0, 1.0),
            music_volume: self.music_volume.clamp(0.0, 1.0),
            tooltip_delay_secs: self
                .tooltip_delay_secs
                .clamp(TOOLTIP_DELAY_MIN_SECS, TOOLTIP_DELAY_MAX_SECS),
            time_bonus_multiplier: self
                .time_bonus_multiplier
                .clamp(TIME_BONUS_MULTIPLIER_MIN, TIME_BONUS_MULTIPLIER_MAX),
            replay_move_interval_secs: self
                .replay_move_interval_secs
                .clamp(REPLAY_MOVE_INTERVAL_MIN_SECS, REPLAY_MOVE_INTERVAL_MAX_SECS),
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

    /// Adjust the tooltip-hover dwell delay by `delta` seconds, clamped
    /// to `[TOOLTIP_DELAY_MIN_SECS, TOOLTIP_DELAY_MAX_SECS]`. Returns the
    /// new value.
    pub fn adjust_tooltip_delay(&mut self, delta: f32) -> f32 {
        self.tooltip_delay_secs = (self.tooltip_delay_secs + delta)
            .clamp(TOOLTIP_DELAY_MIN_SECS, TOOLTIP_DELAY_MAX_SECS);
        self.tooltip_delay_secs
    }

    /// Adjust the time-bonus multiplier by `delta`, clamped to
    /// `[TIME_BONUS_MULTIPLIER_MIN, TIME_BONUS_MULTIPLIER_MAX]`. The
    /// result is rounded to one decimal place so the readout stays
    /// clean across repeated `±` clicks (avoids float drift like
    /// `0.30000004`). Returns the new value.
    pub fn adjust_time_bonus_multiplier(&mut self, delta: f32) -> f32 {
        let raw = (self.time_bonus_multiplier + delta)
            .clamp(TIME_BONUS_MULTIPLIER_MIN, TIME_BONUS_MULTIPLIER_MAX);
        // Round to 1 decimal place — the slider step is 0.1, so this
        // collapses any FP drift introduced by repeated additions.
        self.time_bonus_multiplier = (raw * 10.0).round() / 10.0;
        self.time_bonus_multiplier
    }

    /// Adjust the replay-playback per-move interval by `delta`
    /// seconds, clamped to
    /// `[REPLAY_MOVE_INTERVAL_MIN_SECS, REPLAY_MOVE_INTERVAL_MAX_SECS]`.
    /// The result is rounded to two decimal places so the readout
    /// stays clean across repeated `±` clicks at the 0.05 s step
    /// (avoids float drift like `0.45000003`). Returns the new value.
    pub fn adjust_replay_move_interval(&mut self, delta: f32) -> f32 {
        let raw = (self.replay_move_interval_secs + delta)
            .clamp(REPLAY_MOVE_INTERVAL_MIN_SECS, REPLAY_MOVE_INTERVAL_MAX_SECS);
        // Round to 2 decimal places — the slider step is 0.05, so this
        // collapses any FP drift introduced by repeated additions.
        self.replay_move_interval_secs = (raw * 100.0).round() / 100.0;
        self.replay_move_interval_secs
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
        assert!((s.tooltip_delay_secs - default_tooltip_delay()).abs() < 1e-6);
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
            tooltip_delay_secs: default_tooltip_delay(),
            time_bonus_multiplier: default_time_bonus_multiplier(),
            winnable_deals_only: false,
            replay_move_interval_secs: default_replay_move_interval_secs(),
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

    // -----------------------------------------------------------------------
    // tooltip_delay_secs — player-tunable tooltip hover delay
    // -----------------------------------------------------------------------

    #[test]
    fn settings_tooltip_delay_default_is_existing_baseline() {
        // The existing baseline pre-slider is 0.5 s, matching the
        // `MOTION_TOOLTIP_DELAY_SECS` constant in
        // `solitaire_engine::ui_theme`. The default must not regress.
        let s = Settings::default();
        assert!(
            (s.tooltip_delay_secs - 0.5).abs() < 1e-6,
            "tooltip_delay_secs default must be 0.5 (the pre-slider baseline), got {}",
            s.tooltip_delay_secs
        );
    }

    #[test]
    fn settings_tooltip_delay_round_trip() {
        let path = tmp_path("tooltip_delay_round_trip");
        let _ = fs::remove_file(&path);
        let s = Settings {
            tooltip_delay_secs: 1.2,
            ..Settings::default()
        };
        save_settings_to(&path, &s).expect("save");
        let loaded = load_settings_from(&path);
        assert!(
            (loaded.tooltip_delay_secs - 1.2).abs() < 1e-6,
            "tooltip_delay_secs must survive serde round-trip; got {}",
            loaded.tooltip_delay_secs
        );
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn legacy_settings_without_tooltip_delay_deserializes_to_default() {
        // A settings.json written before this field existed must
        // deserialize cleanly to the existing 0.5 s baseline rather
        // than failing the whole load or yielding a zero value.
        let json = br#"{ "sfx_volume": 0.7, "first_run_complete": true }"#;
        let s: Settings = serde_json::from_slice(json).unwrap_or_default();
        assert!(
            (s.tooltip_delay_secs - default_tooltip_delay()).abs() < 1e-6,
            "legacy settings.json missing tooltip_delay_secs must deserialize to default ({}), got {}",
            default_tooltip_delay(),
            s.tooltip_delay_secs
        );
    }

    #[test]
    fn adjust_tooltip_delay_clamps_to_range() {
        let mut s = Settings { tooltip_delay_secs: 0.5, ..Default::default() };
        // Step up to 0.6.
        assert!((s.adjust_tooltip_delay(0.1) - 0.6).abs() < 1e-6);
        // Big positive jump clamps to TOOLTIP_DELAY_MAX_SECS.
        assert!((s.adjust_tooltip_delay(5.0) - TOOLTIP_DELAY_MAX_SECS).abs() < 1e-6);
        // Big negative jump clamps to TOOLTIP_DELAY_MIN_SECS.
        assert!((s.adjust_tooltip_delay(-99.0) - TOOLTIP_DELAY_MIN_SECS).abs() < 1e-6);
        // Confirm the floor is exactly zero.
        assert_eq!(s.tooltip_delay_secs, 0.0);
    }

    #[test]
    fn sanitized_clamps_out_of_range_tooltip_delay() {
        // Negative or oversized values from a hand-edited file must be
        // clamped on load.
        let s = Settings {
            tooltip_delay_secs: -0.4,
            ..Settings::default()
        }
        .sanitized();
        assert_eq!(s.tooltip_delay_secs, TOOLTIP_DELAY_MIN_SECS);

        let s2 = Settings {
            tooltip_delay_secs: 99.0,
            ..Settings::default()
        }
        .sanitized();
        assert_eq!(s2.tooltip_delay_secs, TOOLTIP_DELAY_MAX_SECS);
    }

    // -----------------------------------------------------------------------
    // time_bonus_multiplier — cosmetic win-modal time-bonus weight
    // -----------------------------------------------------------------------

    #[test]
    fn settings_time_bonus_multiplier_default_is_one() {
        let s = Settings::default();
        assert!(
            (s.time_bonus_multiplier - 1.0).abs() < 1e-6,
            "default time_bonus_multiplier must be 1.0 (no change to displayed bonus), got {}",
            s.time_bonus_multiplier
        );
    }

    #[test]
    fn settings_time_bonus_multiplier_round_trip() {
        let path = tmp_path("time_bonus_multiplier_round_trip");
        let _ = fs::remove_file(&path);
        let s = Settings {
            time_bonus_multiplier: 1.5,
            ..Settings::default()
        };
        save_settings_to(&path, &s).expect("save");
        let loaded = load_settings_from(&path);
        assert!(
            (loaded.time_bonus_multiplier - 1.5).abs() < 1e-6,
            "time_bonus_multiplier must survive serde round-trip; got {}",
            loaded.time_bonus_multiplier
        );
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn legacy_settings_without_time_bonus_multiplier_deserializes_to_one() {
        // A settings.json written before this field existed must
        // deserialize cleanly to the existing 1.0 baseline so old
        // players see no change to their win-modal bonuses.
        let json = br#"{ "sfx_volume": 0.7, "first_run_complete": true }"#;
        let s: Settings = serde_json::from_slice(json).unwrap_or_default();
        assert!(
            (s.time_bonus_multiplier - 1.0).abs() < 1e-6,
            "legacy settings.json missing time_bonus_multiplier must deserialize to 1.0, got {}",
            s.time_bonus_multiplier
        );
    }

    #[test]
    fn settings_time_bonus_multiplier_clamps_to_range() {
        // Negative or oversized values from a hand-edited file must be
        // clamped on load.
        let s = Settings {
            time_bonus_multiplier: -0.5,
            ..Settings::default()
        }
        .sanitized();
        assert_eq!(s.time_bonus_multiplier, TIME_BONUS_MULTIPLIER_MIN);

        let s2 = Settings {
            time_bonus_multiplier: 99.0,
            ..Settings::default()
        }
        .sanitized();
        assert_eq!(s2.time_bonus_multiplier, TIME_BONUS_MULTIPLIER_MAX);
    }

    #[test]
    fn adjust_time_bonus_multiplier_clamps_and_rounds() {
        let mut s = Settings { time_bonus_multiplier: 1.0, ..Default::default() };
        // Step up to 1.1.
        assert!((s.adjust_time_bonus_multiplier(0.1) - 1.1).abs() < 1e-6);
        // Big positive jump clamps to TIME_BONUS_MULTIPLIER_MAX.
        assert!(
            (s.adjust_time_bonus_multiplier(99.0) - TIME_BONUS_MULTIPLIER_MAX).abs() < 1e-6
        );
        // Big negative jump clamps to TIME_BONUS_MULTIPLIER_MIN.
        assert!(
            (s.adjust_time_bonus_multiplier(-99.0) - TIME_BONUS_MULTIPLIER_MIN).abs() < 1e-6
        );
        assert_eq!(s.time_bonus_multiplier, 0.0);

        // Repeated incremental adds must not drift past the 0.1 grid.
        let mut s2 = Settings { time_bonus_multiplier: 0.0, ..Default::default() };
        for _ in 0..10 {
            s2.adjust_time_bonus_multiplier(0.1);
        }
        // After ten +0.1 steps, value should be exactly 1.0 (1 decimal).
        assert!(
            (s2.time_bonus_multiplier - 1.0).abs() < 1e-6,
            "rounding should pin repeated 0.1 steps to the decimal grid, got {}",
            s2.time_bonus_multiplier
        );
    }

    // -----------------------------------------------------------------------
    // winnable_deals_only — solver-backed deal filter toggle
    // -----------------------------------------------------------------------

    #[test]
    fn settings_winnable_deals_only_default_is_false() {
        // Off by default — the solver adds latency we shouldn't impose
        // on every player without their consent.
        assert!(
            !Settings::default().winnable_deals_only,
            "default winnable_deals_only must be false"
        );
    }

    #[test]
    fn settings_winnable_deals_only_round_trip() {
        let path = tmp_path("winnable_deals_only_round_trip");
        let _ = fs::remove_file(&path);
        let s = Settings {
            winnable_deals_only: true,
            ..Settings::default()
        };
        save_settings_to(&path, &s).expect("save");
        let loaded = load_settings_from(&path);
        assert!(
            loaded.winnable_deals_only,
            "winnable_deals_only must survive serde round-trip"
        );
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn legacy_settings_without_winnable_deals_only_deserializes_to_false() {
        // A settings.json written before this field existed must
        // deserialize cleanly to `false` (the default-off behaviour)
        // rather than failing the whole load or surprising the player
        // by switching the toggle on.
        let json = br#"{ "sfx_volume": 0.7, "first_run_complete": true }"#;
        let s: Settings = serde_json::from_slice(json).unwrap_or_default();
        assert!(
            !s.winnable_deals_only,
            "legacy settings.json missing winnable_deals_only must deserialize to false"
        );
    }

    // -----------------------------------------------------------------------
    // replay_move_interval_secs — player-tunable replay playback speed
    // -----------------------------------------------------------------------

    #[test]
    fn settings_replay_move_interval_default_is_zero_point_four_five() {
        // The pre-slider baseline is 0.45 s/move, matching
        // `solitaire_engine::replay_playback::REPLAY_MOVE_INTERVAL_SECS`.
        // The default must not regress for players who never touch
        // the slider.
        let s = Settings::default();
        assert!(
            (s.replay_move_interval_secs - 0.45).abs() < 1e-6,
            "replay_move_interval_secs default must be 0.45 (the pre-slider baseline), got {}",
            s.replay_move_interval_secs
        );
    }

    #[test]
    fn settings_replay_move_interval_round_trip() {
        let path = tmp_path("replay_move_interval_round_trip");
        let _ = fs::remove_file(&path);
        let s = Settings {
            replay_move_interval_secs: 0.20,
            ..Settings::default()
        };
        save_settings_to(&path, &s).expect("save");
        let loaded = load_settings_from(&path);
        assert!(
            (loaded.replay_move_interval_secs - 0.20).abs() < 1e-6,
            "replay_move_interval_secs must survive serde round-trip; got {}",
            loaded.replay_move_interval_secs
        );
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn legacy_settings_without_replay_move_interval_deserializes_to_default() {
        // A settings.json written before this field existed must
        // deserialize cleanly to the existing 0.45 s baseline so old
        // players see no change to replay playback speed.
        let json = br#"{ "sfx_volume": 0.7, "first_run_complete": true }"#;
        let s: Settings = serde_json::from_slice(json).unwrap_or_default();
        assert!(
            (s.replay_move_interval_secs - default_replay_move_interval_secs()).abs() < 1e-6,
            "legacy settings.json missing replay_move_interval_secs must deserialize to default ({}), got {}",
            default_replay_move_interval_secs(),
            s.replay_move_interval_secs
        );
    }

    #[test]
    fn settings_replay_move_interval_clamps_to_range() {
        // Negative or oversized values from a hand-edited file must be
        // clamped on load.
        let s = Settings {
            replay_move_interval_secs: 5.0,
            ..Settings::default()
        }
        .sanitized();
        assert_eq!(s.replay_move_interval_secs, REPLAY_MOVE_INTERVAL_MAX_SECS);

        let s2 = Settings {
            replay_move_interval_secs: -1.0,
            ..Settings::default()
        }
        .sanitized();
        assert_eq!(s2.replay_move_interval_secs, REPLAY_MOVE_INTERVAL_MIN_SECS);
    }

    #[test]
    fn adjust_replay_move_interval_clamps_and_rounds() {
        let mut s = Settings { replay_move_interval_secs: 0.45, ..Default::default() };
        // Step down to 0.40.
        assert!((s.adjust_replay_move_interval(-0.05) - 0.40).abs() < 1e-6);
        // Big positive jump clamps to MAX.
        assert!(
            (s.adjust_replay_move_interval(99.0) - REPLAY_MOVE_INTERVAL_MAX_SECS).abs() < 1e-6
        );
        // Big negative jump clamps to MIN.
        assert!(
            (s.adjust_replay_move_interval(-99.0) - REPLAY_MOVE_INTERVAL_MIN_SECS).abs() < 1e-6
        );

        // Repeated 0.05 steps must not drift past the 0.05 grid.
        let mut s2 = Settings { replay_move_interval_secs: 0.10, ..Default::default() };
        for _ in 0..6 {
            s2.adjust_replay_move_interval(0.05);
        }
        // After six +0.05 steps from 0.10, value should be exactly 0.40 (2 decimals).
        assert!(
            (s2.replay_move_interval_secs - 0.40).abs() < 1e-6,
            "rounding should pin repeated 0.05 steps to the decimal grid, got {}",
            s2.replay_move_interval_secs
        );
    }
}
