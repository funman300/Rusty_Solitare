//! Persists `solitaire_data::Settings`, exposes hotkeys for live tuning,
//! and renders a Bevy UI Settings panel.
//!
//! Hotkeys (always active, no overlay required):
//! - `[` — decrease SFX volume by `SFX_STEP`
//! - `]` — increase SFX volume by `SFX_STEP`
//! - `O` — open / close the Settings panel
//!
//! On change, the plugin persists `settings.json` and fires
//! `SettingsChangedEvent` so dependents (e.g. `AudioPlugin`) can react.

use std::path::PathBuf;

use bevy::prelude::*;
use solitaire_data::{load_settings_from, save_settings_to, settings_file_path, Settings};

/// Volume adjustment step applied by the `[` / `]` hotkeys.
pub const SFX_STEP: f32 = 0.1;

/// Bevy resource wrapping the current `Settings`.
#[derive(Resource, Debug, Clone)]
pub struct SettingsResource(pub Settings);

/// Persistence path for `SettingsResource`. `None` disables I/O (used in tests).
#[derive(Resource, Debug, Clone)]
pub struct SettingsStoragePath(pub Option<PathBuf>);

/// Whether the Settings panel is currently visible. Toggle with `O`.
#[derive(Resource, Debug, Clone, Default)]
pub struct SettingsScreen(pub bool);

/// Fired whenever settings change so consumers (audio, UI) can react.
#[derive(Event, Debug, Clone)]
pub struct SettingsChangedEvent(pub Settings);

/// Marker on the root Settings panel entity.
#[derive(Component, Debug)]
struct SettingsPanel;

/// Marks the `Text` node that displays the live SFX volume value.
#[derive(Component, Debug)]
struct SfxVolumeText;

/// Tags interactive buttons inside the Settings panel.
#[derive(Component, Debug)]
enum SettingsButton {
    SfxDown,
    SfxUp,
    Done,
}

/// Plugin that owns the settings lifecycle.
pub struct SettingsPlugin {
    /// Path to `settings.json`. `None` in headless/test mode.
    pub storage_path: Option<PathBuf>,
    /// When `false`, panel spawn/despawn systems are not registered.
    /// Use [`SettingsPlugin::headless`] for tests running under `MinimalPlugins`.
    pub ui_enabled: bool,
}

impl Default for SettingsPlugin {
    fn default() -> Self {
        Self {
            storage_path: settings_file_path(),
            ui_enabled: true,
        }
    }
}

impl SettingsPlugin {
    /// No persistence, no UI — safe to use under `MinimalPlugins` in tests.
    pub fn headless() -> Self {
        Self {
            storage_path: None,
            ui_enabled: false,
        }
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
            .init_resource::<SettingsScreen>()
            .add_event::<SettingsChangedEvent>()
            .add_systems(Update, (handle_volume_keys, toggle_settings_screen));

        if self.ui_enabled {
            app.add_systems(
                Update,
                (sync_settings_panel_visibility, handle_settings_buttons),
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn persist(path: &SettingsStoragePath, settings: &Settings) {
    let Some(target) = &path.0 else { return };
    if let Err(e) = save_settings_to(target, settings) {
        warn!("failed to save settings: {e}");
    }
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

fn handle_volume_keys(
    keys: Res<ButtonInput<KeyCode>>,
    mut settings: ResMut<SettingsResource>,
    path: Res<SettingsStoragePath>,
    mut changed: EventWriter<SettingsChangedEvent>,
) {
    let mut delta = 0.0_f32;
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
        return;
    }
    persist(&path, &settings.0);
    changed.send(SettingsChangedEvent(settings.0.clone()));
}

/// Opens or closes the Settings panel when `O` is pressed.
fn toggle_settings_screen(
    keys: Res<ButtonInput<KeyCode>>,
    mut screen: ResMut<SettingsScreen>,
) {
    if keys.just_pressed(KeyCode::KeyO) {
        screen.0 = !screen.0;
    }
}

/// Spawns the Settings panel when `SettingsScreen` becomes `true`;
/// despawns it when it becomes `false`.
fn sync_settings_panel_visibility(
    screen: Res<SettingsScreen>,
    panels: Query<Entity, With<SettingsPanel>>,
    mut commands: Commands,
    settings: Res<SettingsResource>,
) {
    if !screen.is_changed() {
        return;
    }
    if screen.0 {
        if panels.is_empty() {
            spawn_settings_panel(&mut commands, &settings.0);
        }
    } else {
        for entity in &panels {
            commands.entity(entity).despawn_recursive();
        }
    }
}

/// Reacts to button presses inside the Settings panel.
fn handle_settings_buttons(
    interaction_query: Query<(&Interaction, &SettingsButton), Changed<Interaction>>,
    mut settings: ResMut<SettingsResource>,
    mut screen: ResMut<SettingsScreen>,
    path: Res<SettingsStoragePath>,
    mut changed: EventWriter<SettingsChangedEvent>,
    mut volume_text: Query<&mut Text, With<SfxVolumeText>>,
) {
    for (interaction, button) in &interaction_query {
        if *interaction != Interaction::Pressed {
            continue;
        }
        match button {
            SettingsButton::SfxDown => {
                let before = settings.0.sfx_volume;
                let after = settings.0.adjust_sfx_volume(-SFX_STEP);
                if (before - after).abs() > f32::EPSILON {
                    persist(&path, &settings.0);
                    changed.send(SettingsChangedEvent(settings.0.clone()));
                    if let Ok(mut text) = volume_text.get_single_mut() {
                        **text = format!("{:.2}", after);
                    }
                }
            }
            SettingsButton::SfxUp => {
                let before = settings.0.sfx_volume;
                let after = settings.0.adjust_sfx_volume(SFX_STEP);
                if (before - after).abs() > f32::EPSILON {
                    persist(&path, &settings.0);
                    changed.send(SettingsChangedEvent(settings.0.clone()));
                    if let Ok(mut text) = volume_text.get_single_mut() {
                        **text = format!("{:.2}", after);
                    }
                }
            }
            SettingsButton::Done => {
                screen.0 = false;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// UI construction
// ---------------------------------------------------------------------------

fn spawn_settings_panel(commands: &mut Commands, settings: &Settings) {
    commands
        .spawn((
            SettingsPanel,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Percent(0.0),
                top: Val::Percent(0.0),
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.72)),
            ZIndex(200),
        ))
        .with_children(|root| {
            // Inner card
            root.spawn((
                Node {
                    flex_direction: FlexDirection::Column,
                    padding: UiRect::all(Val::Px(28.0)),
                    row_gap: Val::Px(14.0),
                    min_width: Val::Px(340.0),
                    ..default()
                },
                BackgroundColor(Color::srgb(0.11, 0.11, 0.14)),
                BorderRadius::all(Val::Px(8.0)),
            ))
            .with_children(|card| {
                // Title
                card.spawn((
                    Text::new("Settings"),
                    TextFont {
                        font_size: 30.0,
                        ..default()
                    },
                    TextColor(Color::WHITE),
                ));

                // --- Audio section ---
                section_label(card, "Audio");

                // SFX volume row: label | value | [−] | [+]
                card.spawn(Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(8.0),
                    ..default()
                })
                .with_children(|row| {
                    row.spawn((
                        Text::new("SFX Volume"),
                        TextFont {
                            font_size: 18.0,
                            ..default()
                        },
                        TextColor(Color::srgb(0.85, 0.85, 0.80)),
                    ));
                    row.spawn((
                        SfxVolumeText,
                        Text::new(format!("{:.2}", settings.sfx_volume)),
                        TextFont {
                            font_size: 18.0,
                            ..default()
                        },
                        TextColor(Color::WHITE),
                    ));
                    icon_button(row, "−", SettingsButton::SfxDown);
                    icon_button(row, "+", SettingsButton::SfxUp);
                });

                coming_soon_row(card, "Music Volume");

                // --- Gameplay section ---
                section_label(card, "Gameplay");
                coming_soon_row(card, "Draw Mode");

                // --- Appearance section ---
                section_label(card, "Appearance");
                coming_soon_row(card, "Theme");

                // --- Sync section ---
                section_label(card, "Sync");
                coming_soon_row(card, "Sync Backend");

                // Done button
                card.spawn((
                    SettingsButton::Done,
                    Button,
                    Node {
                        padding: UiRect::axes(Val::Px(20.0), Val::Px(8.0)),
                        justify_content: JustifyContent::Center,
                        margin: UiRect::top(Val::Px(6.0)),
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.22, 0.45, 0.22)),
                    BorderRadius::all(Val::Px(4.0)),
                ))
                .with_children(|b| {
                    b.spawn((
                        Text::new("Done"),
                        TextFont {
                            font_size: 18.0,
                            ..default()
                        },
                        TextColor(Color::WHITE),
                    ));
                });
            });
        });
}

fn section_label(parent: &mut ChildBuilder, title: &str) {
    parent.spawn((
        Text::new(title),
        TextFont {
            font_size: 14.0,
            ..default()
        },
        TextColor(Color::srgb(0.55, 0.75, 0.55)),
    ));
}

fn coming_soon_row(parent: &mut ChildBuilder, label: &str) {
    parent.spawn((
        Text::new(format!("{label} — coming soon")),
        TextFont {
            font_size: 16.0,
            ..default()
        },
        TextColor(Color::srgb(0.45, 0.45, 0.45)),
    ));
}

fn icon_button(parent: &mut ChildBuilder, label: &str, action: SettingsButton) {
    parent
        .spawn((
            action,
            Button,
            Node {
                width: Val::Px(28.0),
                height: Val::Px(28.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(Color::srgb(0.25, 0.25, 0.30)),
            BorderRadius::all(Val::Px(4.0)),
        ))
        .with_children(|b| {
            b.spawn((
                Text::new(label.to_string()),
                TextFont {
                    font_size: 18.0,
                    ..default()
                },
                TextColor(Color::WHITE),
            ));
        });
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

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
        app.world_mut().resource_mut::<SettingsResource>().0.sfx_volume = 0.5;

        press(&mut app, KeyCode::BracketRight);
        app.update();

        let after = app.world().resource::<SettingsResource>().0.sfx_volume;
        assert!((after - 0.6).abs() < 1e-3);
    }

    #[test]
    fn clamped_change_does_not_emit_event() {
        let mut app = headless_app();
        app.world_mut().resource_mut::<SettingsResource>().0.sfx_volume = 1.0;

        press(&mut app, KeyCode::BracketRight);
        app.update();

        let events = app.world().resource::<Events<SettingsChangedEvent>>();
        let mut cursor = events.get_cursor();
        assert_eq!(cursor.read(events).count(), 0);
    }
}
