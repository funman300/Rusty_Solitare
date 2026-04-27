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
use solitaire_core::game_state::DrawMode;
use solitaire_data::{load_settings_from, save_settings_to, settings_file_path, settings::Theme, AnimSpeed, Settings};

use crate::events::ManualSyncRequestEvent;
use crate::progress_plugin::ProgressResource;
use crate::resources::{SyncStatus, SyncStatusResource};

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

/// Marks the `Text` node showing the live SFX volume value.
#[derive(Component, Debug)]
struct SfxVolumeText;

/// Marks the `Text` node showing the live music volume value.
#[derive(Component, Debug)]
struct MusicVolumeText;

/// Marks the `Text` node showing the current draw mode.
#[derive(Component, Debug)]
struct DrawModeText;

/// Marks the `Text` node showing the current theme.
#[derive(Component, Debug)]
struct ThemeText;

/// Marks the `Text` node showing the live sync status.
#[derive(Component, Debug)]
struct SyncStatusText;

/// Marks the `Text` node showing the active card-back index.
#[derive(Component, Debug)]
struct CardBackText;

/// Marks the `Text` node showing the current animation speed.
#[derive(Component, Debug)]
struct AnimSpeedText;

/// Marks the `Text` node showing the active background index.
#[derive(Component, Debug)]
struct BackgroundText;

/// Tags interactive buttons inside the Settings panel.
#[derive(Component, Debug)]
enum SettingsButton {
    SfxDown,
    SfxUp,
    MusicDown,
    MusicUp,
    ToggleDrawMode,
    CycleAnimSpeed,
    ToggleTheme,
    CycleCardBack,
    CycleBackground,
    SyncNow,
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
            .add_event::<ManualSyncRequestEvent>()
            .add_systems(Update, (handle_volume_keys, toggle_settings_screen));

        if self.ui_enabled {
            app.add_systems(
                Update,
                (
                    sync_settings_panel_visibility,
                    handle_settings_buttons,
                    update_sync_status_text,
                    update_card_back_text,
                    update_background_text,
                    update_anim_speed_text,
                ),
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
    sync_status: Option<Res<SyncStatusResource>>,
    progress: Option<Res<ProgressResource>>,
) {
    if !screen.is_changed() {
        return;
    }
    if screen.0 {
        if panels.is_empty() {
            let status_label = sync_status
                .map(|s| sync_status_label(&s.0))
                .unwrap_or_else(|| "Status: not configured".to_string());
            let unlocked_backs = progress
                .as_ref()
                .map(|p| p.0.unlocked_card_backs.as_slice())
                .unwrap_or(&[0]);
            let unlocked_bgs = progress
                .as_ref()
                .map(|p| p.0.unlocked_backgrounds.as_slice())
                .unwrap_or(&[0]);
            spawn_settings_panel(
                &mut commands,
                &settings.0,
                &status_label,
                unlocked_backs,
                unlocked_bgs,
            );
        }
    } else {
        for entity in &panels {
            commands.entity(entity).despawn_recursive();
        }
    }
}

/// Returns the next unlocked index after `current` in the sorted `unlocked` list.
/// Wraps around. Falls back to `unlocked[0]` if `current` is not found.
fn cycle_unlocked(unlocked: &[usize], current: usize) -> usize {
    if unlocked.is_empty() {
        return 0;
    }
    let pos = unlocked.iter().position(|&i| i == current).unwrap_or(0);
    unlocked[(pos + 1) % unlocked.len()]
}

/// Keeps the sync-status text node current while the panel is open.
fn update_sync_status_text(
    sync_status: Option<Res<SyncStatusResource>>,
    mut text_nodes: Query<&mut Text, With<SyncStatusText>>,
) {
    let Some(status) = sync_status else {
        return;
    };
    if !status.is_changed() {
        return;
    }
    let label = sync_status_label(&status.0);
    for mut text in &mut text_nodes {
        **text = label.clone();
    }
}

fn update_card_back_text(
    settings: Res<SettingsResource>,
    mut text_nodes: Query<&mut Text, With<CardBackText>>,
) {
    if !settings.is_changed() {
        return;
    }
    for mut text in &mut text_nodes {
        **text = card_back_label(settings.0.selected_card_back);
    }
}

fn update_background_text(
    settings: Res<SettingsResource>,
    mut text_nodes: Query<&mut Text, With<BackgroundText>>,
) {
    if !settings.is_changed() {
        return;
    }
    for mut text in &mut text_nodes {
        **text = background_label(settings.0.selected_background);
    }
}

fn update_anim_speed_text(
    settings: Res<SettingsResource>,
    mut text_nodes: Query<&mut Text, With<AnimSpeedText>>,
) {
    if !settings.is_changed() {
        return;
    }
    for mut text in &mut text_nodes {
        **text = anim_speed_label(&settings.0.animation_speed);
    }
}

fn card_back_label(idx: usize) -> String {
    if idx == 0 {
        "Default".to_string()
    } else {
        format!("Style {idx}")
    }
}

fn background_label(idx: usize) -> String {
    if idx == 0 {
        "Default".to_string()
    } else {
        format!("Style {idx}")
    }
}

fn sync_status_label(status: &SyncStatus) -> String {
    match status {
        SyncStatus::Idle => "Status: idle".to_string(),
        SyncStatus::Syncing => "Status: syncing…".to_string(),
        SyncStatus::LastSynced(t) => {
            let secs = chrono::Utc::now()
                .signed_duration_since(*t)
                .num_seconds()
                .max(0);
            if secs < 60 {
                format!("Last synced: {secs}s ago")
            } else {
                format!("Last synced: {}m ago", secs / 60)
            }
        }
        SyncStatus::Error(e) => format!("Sync error: {e}"),
    }
}

/// Reacts to button presses inside the Settings panel.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn handle_settings_buttons(
    interaction_query: Query<(&Interaction, &SettingsButton), Changed<Interaction>>,
    mut settings: ResMut<SettingsResource>,
    mut screen: ResMut<SettingsScreen>,
    path: Res<SettingsStoragePath>,
    mut changed: EventWriter<SettingsChangedEvent>,
    mut manual_sync: EventWriter<ManualSyncRequestEvent>,
    progress: Option<Res<ProgressResource>>,
    mut sfx_text: Query<&mut Text, (With<SfxVolumeText>, Without<MusicVolumeText>, Without<DrawModeText>, Without<ThemeText>, Without<AnimSpeedText>)>,
    mut music_text: Query<&mut Text, (With<MusicVolumeText>, Without<SfxVolumeText>, Without<DrawModeText>, Without<ThemeText>, Without<AnimSpeedText>)>,
    mut draw_text: Query<&mut Text, (With<DrawModeText>, Without<SfxVolumeText>, Without<MusicVolumeText>, Without<ThemeText>, Without<AnimSpeedText>)>,
    mut theme_text: Query<&mut Text, (With<ThemeText>, Without<SfxVolumeText>, Without<MusicVolumeText>, Without<DrawModeText>, Without<AnimSpeedText>)>,
    mut anim_speed_text: Query<&mut Text, (With<AnimSpeedText>, Without<SfxVolumeText>, Without<MusicVolumeText>, Without<DrawModeText>, Without<ThemeText>)>,
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
                    if let Ok(mut t) = sfx_text.get_single_mut() {
                        **t = format!("{:.2}", after);
                    }
                }
            }
            SettingsButton::SfxUp => {
                let before = settings.0.sfx_volume;
                let after = settings.0.adjust_sfx_volume(SFX_STEP);
                if (before - after).abs() > f32::EPSILON {
                    persist(&path, &settings.0);
                    changed.send(SettingsChangedEvent(settings.0.clone()));
                    if let Ok(mut t) = sfx_text.get_single_mut() {
                        **t = format!("{:.2}", after);
                    }
                }
            }
            SettingsButton::MusicDown => {
                let before = settings.0.music_volume;
                let after = settings.0.adjust_music_volume(-SFX_STEP);
                if (before - after).abs() > f32::EPSILON {
                    persist(&path, &settings.0);
                    changed.send(SettingsChangedEvent(settings.0.clone()));
                    if let Ok(mut t) = music_text.get_single_mut() {
                        **t = format!("{:.2}", after);
                    }
                }
            }
            SettingsButton::MusicUp => {
                let before = settings.0.music_volume;
                let after = settings.0.adjust_music_volume(SFX_STEP);
                if (before - after).abs() > f32::EPSILON {
                    persist(&path, &settings.0);
                    changed.send(SettingsChangedEvent(settings.0.clone()));
                    if let Ok(mut t) = music_text.get_single_mut() {
                        **t = format!("{:.2}", after);
                    }
                }
            }
            SettingsButton::ToggleDrawMode => {
                settings.0.draw_mode = match settings.0.draw_mode {
                    DrawMode::DrawOne => DrawMode::DrawThree,
                    DrawMode::DrawThree => DrawMode::DrawOne,
                };
                persist(&path, &settings.0);
                changed.send(SettingsChangedEvent(settings.0.clone()));
                if let Ok(mut t) = draw_text.get_single_mut() {
                    **t = draw_mode_label(&settings.0.draw_mode);
                }
            }
            SettingsButton::CycleAnimSpeed => {
                settings.0.animation_speed = match settings.0.animation_speed {
                    AnimSpeed::Normal => AnimSpeed::Fast,
                    AnimSpeed::Fast => AnimSpeed::Instant,
                    AnimSpeed::Instant => AnimSpeed::Normal,
                };
                persist(&path, &settings.0);
                changed.send(SettingsChangedEvent(settings.0.clone()));
                if let Ok(mut t) = anim_speed_text.get_single_mut() {
                    **t = anim_speed_label(&settings.0.animation_speed);
                }
            }
            SettingsButton::ToggleTheme => {
                settings.0.theme = match settings.0.theme {
                    Theme::Green => Theme::Blue,
                    Theme::Blue => Theme::Dark,
                    Theme::Dark => Theme::Green,
                };
                persist(&path, &settings.0);
                changed.send(SettingsChangedEvent(settings.0.clone()));
                if let Ok(mut t) = theme_text.get_single_mut() {
                    **t = theme_label(&settings.0.theme);
                }
            }
            SettingsButton::CycleCardBack => {
                let unlocked = progress
                    .as_ref()
                    .map(|p| p.0.unlocked_card_backs.clone())
                    .unwrap_or_else(|| vec![0]);
                settings.0.selected_card_back =
                    cycle_unlocked(&unlocked, settings.0.selected_card_back);
                persist(&path, &settings.0);
                changed.send(SettingsChangedEvent(settings.0.clone()));
            }
            SettingsButton::CycleBackground => {
                let unlocked = progress
                    .as_ref()
                    .map(|p| p.0.unlocked_backgrounds.clone())
                    .unwrap_or_else(|| vec![0]);
                settings.0.selected_background =
                    cycle_unlocked(&unlocked, settings.0.selected_background);
                persist(&path, &settings.0);
                changed.send(SettingsChangedEvent(settings.0.clone()));
            }
            SettingsButton::SyncNow => {
                manual_sync.send(ManualSyncRequestEvent);
            }
            SettingsButton::Done => {
                screen.0 = false;
            }
        }
    }
}

fn draw_mode_label(mode: &DrawMode) -> String {
    match mode {
        DrawMode::DrawOne => "Draw 1".into(),
        DrawMode::DrawThree => "Draw 3".into(),
    }
}

fn anim_speed_label(speed: &AnimSpeed) -> String {
    match speed {
        AnimSpeed::Normal => "Normal".into(),
        AnimSpeed::Fast => "Fast".into(),
        AnimSpeed::Instant => "Instant".into(),
    }
}

fn theme_label(theme: &Theme) -> String {
    match theme {
        Theme::Green => "Green".into(),
        Theme::Blue => "Blue".into(),
        Theme::Dark => "Dark".into(),
    }
}

// ---------------------------------------------------------------------------
// UI construction
// ---------------------------------------------------------------------------

fn spawn_settings_panel(
    commands: &mut Commands,
    settings: &Settings,
    sync_status: &str,
    unlocked_card_backs: &[usize],
    unlocked_backgrounds: &[usize],
) {
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

                // SFX volume row
                volume_row(card, "SFX Volume", settings.sfx_volume, SfxVolumeText,
                    SettingsButton::SfxDown, SettingsButton::SfxUp);

                // Music volume row
                volume_row(card, "Music Volume", settings.music_volume, MusicVolumeText,
                    SettingsButton::MusicDown, SettingsButton::MusicUp);

                // --- Gameplay section ---
                section_label(card, "Gameplay");

                // Draw mode row
                card.spawn(Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(8.0),
                    ..default()
                })
                .with_children(|row| {
                    row.spawn((
                        Text::new("Draw Mode"),
                        TextFont { font_size: 18.0, ..default() },
                        TextColor(Color::srgb(0.85, 0.85, 0.80)),
                    ));
                    row.spawn((
                        DrawModeText,
                        Text::new(draw_mode_label(&settings.draw_mode)),
                        TextFont { font_size: 18.0, ..default() },
                        TextColor(Color::WHITE),
                    ));
                    icon_button(row, "⇄", SettingsButton::ToggleDrawMode);
                });

                // Animation speed row
                card.spawn(Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(8.0),
                    ..default()
                })
                .with_children(|row| {
                    row.spawn((
                        Text::new("Anim Speed"),
                        TextFont { font_size: 18.0, ..default() },
                        TextColor(Color::srgb(0.85, 0.85, 0.80)),
                    ));
                    row.spawn((
                        AnimSpeedText,
                        Text::new(anim_speed_label(&settings.animation_speed)),
                        TextFont { font_size: 18.0, ..default() },
                        TextColor(Color::WHITE),
                    ));
                    icon_button(row, "⇄", SettingsButton::CycleAnimSpeed);
                });

                // --- Appearance section ---
                section_label(card, "Appearance");

                // Theme row
                card.spawn(Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(8.0),
                    ..default()
                })
                .with_children(|row| {
                    row.spawn((
                        Text::new("Theme"),
                        TextFont { font_size: 18.0, ..default() },
                        TextColor(Color::srgb(0.85, 0.85, 0.80)),
                    ));
                    row.spawn((
                        ThemeText,
                        Text::new(theme_label(&settings.theme)),
                        TextFont { font_size: 18.0, ..default() },
                        TextColor(Color::WHITE),
                    ));
                    icon_button(row, "⇄", SettingsButton::ToggleTheme);
                });

                // Card back row — only shown when the player has unlocked more than one.
                if unlocked_card_backs.len() > 1 {
                    card.spawn(Node {
                        flex_direction: FlexDirection::Row,
                        align_items: AlignItems::Center,
                        column_gap: Val::Px(8.0),
                        ..default()
                    })
                    .with_children(|row| {
                        row.spawn((
                            Text::new("Card Back"),
                            TextFont { font_size: 18.0, ..default() },
                            TextColor(Color::srgb(0.85, 0.85, 0.80)),
                        ));
                        row.spawn((
                            CardBackText,
                            Text::new(card_back_label(settings.selected_card_back)),
                            TextFont { font_size: 18.0, ..default() },
                            TextColor(Color::WHITE),
                        ));
                        icon_button(row, "⇄", SettingsButton::CycleCardBack);
                    });
                }

                // Background row — only shown when the player has unlocked more than one.
                if unlocked_backgrounds.len() > 1 {
                    card.spawn(Node {
                        flex_direction: FlexDirection::Row,
                        align_items: AlignItems::Center,
                        column_gap: Val::Px(8.0),
                        ..default()
                    })
                    .with_children(|row| {
                        row.spawn((
                            Text::new("Background"),
                            TextFont { font_size: 18.0, ..default() },
                            TextColor(Color::srgb(0.85, 0.85, 0.80)),
                        ));
                        row.spawn((
                            BackgroundText,
                            Text::new(background_label(settings.selected_background)),
                            TextFont { font_size: 18.0, ..default() },
                            TextColor(Color::WHITE),
                        ));
                        icon_button(row, "⇄", SettingsButton::CycleBackground);
                    });
                }

                // --- Sync section ---
                section_label(card, "Sync");
                card.spawn(Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(10.0),
                    ..default()
                })
                .with_children(|row| {
                    row.spawn((
                        SyncStatusText,
                        Text::new(sync_status.to_string()),
                        TextFont { font_size: 16.0, ..default() },
                        TextColor(Color::srgb(0.65, 0.65, 0.70)),
                    ));
                    // "Sync Now" button — hidden when SyncPlugin is not installed;
                    // visible because ManualSyncRequestEvent is always registered.
                    row.spawn((
                        SettingsButton::SyncNow,
                        Button,
                        Node {
                            padding: UiRect::axes(Val::Px(10.0), Val::Px(4.0)),
                            justify_content: JustifyContent::Center,
                            ..default()
                        },
                        BackgroundColor(Color::srgb(0.20, 0.30, 0.45)),
                        BorderRadius::all(Val::Px(4.0)),
                    ))
                    .with_children(|b| {
                        b.spawn((
                            Text::new("Sync Now"),
                            TextFont { font_size: 14.0, ..default() },
                            TextColor(Color::WHITE),
                        ));
                    });
                });

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

/// Generic volume row: `Label  0.80  [−]  [+]`
fn volume_row<Marker: Component>(
    parent: &mut ChildBuilder,
    label: &str,
    value: f32,
    marker: Marker,
    btn_down: SettingsButton,
    btn_up: SettingsButton,
) {
    parent
        .spawn(Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: Val::Px(8.0),
            ..default()
        })
        .with_children(|row| {
            row.spawn((
                Text::new(label.to_string()),
                TextFont { font_size: 18.0, ..default() },
                TextColor(Color::srgb(0.85, 0.85, 0.80)),
            ));
            row.spawn((
                marker,
                Text::new(format!("{:.2}", value)),
                TextFont { font_size: 18.0, ..default() },
                TextColor(Color::WHITE),
            ));
            icon_button(row, "−", btn_down);
            icon_button(row, "+", btn_up);
        });
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
