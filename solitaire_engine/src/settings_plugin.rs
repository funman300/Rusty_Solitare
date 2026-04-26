//! Persists `solitaire_data::Settings` and exposes hotkeys for live tuning.
//!
//! Hotkeys (always active, no overlay required):
//! - `[` decrease SFX volume by `SFX_STEP`
//! - `]` increase SFX volume by `SFX_STEP`
//!
//! On change, the plugin persists `settings.json` and fires
//! `SettingsChangedEvent` so dependents (e.g. `AudioPlugin`) can react.

use std::path::PathBuf;

use bevy::prelude::*;
use solitaire_data::{load_settings_from, save_settings_to, settings_file_path, Settings};

/// Volume adjustment step.
pub const SFX_STEP: f32 = 0.1;

/// Bevy resource wrapping the current `Settings`.
#[derive(Resource, Debug, Clone)]
pub struct SettingsResource(pub Settings);

/// Persistence path for `SettingsResource`. `None` disables I/O.
#[derive(Resource, Debug, Clone)]
pub struct SettingsStoragePath(pub Option<PathBuf>);

/// Fired any time settings change so consumers (audio, UI) can react.
#[derive(Event, Debug, Clone)]
pub struct SettingsChangedEvent(pub Settings);

pub struct SettingsPlugin {
    pub storage_path: Option<PathBuf>,
}

impl Default for SettingsPlugin {
    fn default() -> Self {
        Self {
            storage_path: settings_file_path(),
        }
    }
}

impl SettingsPlugin {
    /// Plugin configured with no persistence — for tests and headless apps.
    pub fn headless() -> Self {
        Self { storage_path: None }
    }
}

impl Plugin for SettingsPlugin {
    fn build(&self, app: &mut App) {
        let loaded = match &self.storage_path {
            Some(path) => load_settings_from(path),
            None => Settings::default(),
        };
        app.insert_resource(SettingsResource(loaded))
            .insert_resource(SettingsStoragePath(self.storage_path.clone()))
            .add_event::<SettingsChangedEvent>()
            .add_systems(Update, handle_volume_keys);
    }
}

fn persist(path: &SettingsStoragePath, settings: &Settings) {
    let Some(target) = &path.0 else {
        return;
    };
    if let Err(e) = save_settings_to(target, settings) {
        warn!("failed to save settings: {e}");
    }
}

fn handle_volume_keys(
    keys: Res<ButtonInput<KeyCode>>,
    mut settings: ResMut<SettingsResource>,
    path: Res<SettingsStoragePath>,
    mut changed: EventWriter<SettingsChangedEvent>,
) {
    let mut delta = 0.0;
    if keys.just_pressed(KeyCode::BracketLeft) {
        delta -= SFX_STEP;
    }
    if keys.just_pressed(KeyCode::BracketRight) {
        delta += SFX_STEP;
    }
    if delta == 0.0 {
        return;
    }
    let before = settings.0.sfx_volume;
    let after = settings.0.adjust_sfx_volume(delta);
    if (before - after).abs() < f32::EPSILON {
        // Already at the rail — no point persisting or notifying.
        return;
    }
    persist(&path, &settings.0);
    changed.send(SettingsChangedEvent(settings.0.clone()));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headless_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(SettingsPlugin::headless());
        app.init_resource::<ButtonInput<KeyCode>>();
        app.update();
        app
    }

    fn press(app: &mut App, key: KeyCode) {
        let mut input = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        input.release(key);
        input.clear();
        input.press(key);
    }

    #[test]
    fn defaults_are_loaded() {
        let app = headless_app();
        assert_eq!(
            app.world().resource::<SettingsResource>().0,
            Settings::default()
        );
    }

    #[test]
    fn pressing_left_bracket_decreases_volume_and_emits_event() {
        let mut app = headless_app();
        let before = app.world().resource::<SettingsResource>().0.sfx_volume;

        press(&mut app, KeyCode::BracketLeft);
        app.update();

        let after = app.world().resource::<SettingsResource>().0.sfx_volume;
        assert!(after < before);

        let events = app.world().resource::<Events<SettingsChangedEvent>>();
        let mut cursor = events.get_cursor();
        assert_eq!(cursor.read(events).count(), 1);
    }

    #[test]
    fn pressing_right_bracket_increases_volume() {
        let mut app = headless_app();
        // Drop volume first so there's headroom to grow.
        app.world_mut().resource_mut::<SettingsResource>().0.sfx_volume = 0.5;

        press(&mut app, KeyCode::BracketRight);
        app.update();

        let after = app.world().resource::<SettingsResource>().0.sfx_volume;
        assert!((after - 0.6).abs() < 1e-3);
    }

    #[test]
    fn clamped_change_does_not_emit_event() {
        let mut app = headless_app();
        // Already at max — pressing right bracket should be a no-op.
        app.world_mut().resource_mut::<SettingsResource>().0.sfx_volume = 1.0;

        press(&mut app, KeyCode::BracketRight);
        app.update();

        let events = app.world().resource::<Events<SettingsChangedEvent>>();
        let mut cursor = events.get_cursor();
        assert_eq!(cursor.read(events).count(), 0);
    }
}
