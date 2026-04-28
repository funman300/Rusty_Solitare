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

use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::prelude::*;
use solitaire_core::game_state::DrawMode;
use solitaire_data::{load_settings_from, save_settings_to, settings_file_path, settings::Theme, AnimSpeed, Settings};

use crate::events::ManualSyncRequestEvent;
use crate::progress_plugin::ProgressResource;
use crate::resources::{SettingsScrollPos, SyncStatus, SyncStatusResource};

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

/// Marks the `Text` node showing the current color-blind mode state.
#[derive(Component, Debug)]
struct ColorBlindText;

/// Marks the scrollable inner card so the mouse-wheel system can target it.
#[derive(Component, Debug)]
struct SettingsPanelScrollable;

/// Marks the scrollable inner card so its `ScrollPosition` can be read before despawn.
#[derive(Component, Debug)]
struct SettingsScrollNode;

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
    ToggleColorBlind,
    SyncNow,
    Done,
    /// Select a specific card-back by index from the picker row.
    SelectCardBack(usize),
    /// Select a specific background by index from the picker row.
    SelectBackground(usize),
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
            .init_resource::<SettingsScrollPos>()
            .add_event::<SettingsChangedEvent>()
            .add_event::<ManualSyncRequestEvent>()
            .add_event::<bevy::input::mouse::MouseWheel>()
            .add_systems(Update, (handle_volume_keys, toggle_settings_screen, scroll_settings_panel));

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
                    update_color_blind_text,
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
#[allow(clippy::too_many_arguments)]
fn sync_settings_panel_visibility(
    screen: Res<SettingsScreen>,
    panels: Query<Entity, With<SettingsPanel>>,
    scroll_nodes: Query<&ScrollPosition, With<SettingsScrollNode>>,
    mut scroll_pos: ResMut<SettingsScrollPos>,
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
                scroll_pos.0,
            );
        }
    } else {
        // Save the current scroll offset before despawning the panel.
        if let Ok(sp) = scroll_nodes.get_single() {
            scroll_pos.0 = sp.offset_y;
        }
        for entity in &panels {
            commands.entity(entity).despawn_recursive();
        }
    }
}

/// Returns the next unlocked index after `current` in the sorted `unlocked` list.
/// Wraps around. Falls back to `unlocked[0]` if `current` is not found.
#[cfg(test)]
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

fn update_color_blind_text(
    settings: Res<SettingsResource>,
    mut text_nodes: Query<&mut Text, With<ColorBlindText>>,
) {
    if !settings.is_changed() {
        return;
    }
    for mut text in &mut text_nodes {
        **text = color_blind_label(settings.0.color_blind_mode);
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
    mut sfx_text: Query<&mut Text, (With<SfxVolumeText>, Without<MusicVolumeText>, Without<DrawModeText>, Without<ThemeText>, Without<AnimSpeedText>, Without<ColorBlindText>)>,
    mut music_text: Query<&mut Text, (With<MusicVolumeText>, Without<SfxVolumeText>, Without<DrawModeText>, Without<ThemeText>, Without<AnimSpeedText>, Without<ColorBlindText>)>,
    mut draw_text: Query<&mut Text, (With<DrawModeText>, Without<SfxVolumeText>, Without<MusicVolumeText>, Without<ThemeText>, Without<AnimSpeedText>, Without<ColorBlindText>)>,
    mut theme_text: Query<&mut Text, (With<ThemeText>, Without<SfxVolumeText>, Without<MusicVolumeText>, Without<DrawModeText>, Without<AnimSpeedText>, Without<ColorBlindText>)>,
    mut anim_speed_text: Query<&mut Text, (With<AnimSpeedText>, Without<SfxVolumeText>, Without<MusicVolumeText>, Without<DrawModeText>, Without<ThemeText>, Without<ColorBlindText>)>,
    mut color_blind_text: Query<&mut Text, (With<ColorBlindText>, Without<SfxVolumeText>, Without<MusicVolumeText>, Without<DrawModeText>, Without<ThemeText>, Without<AnimSpeedText>)>,
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
            SettingsButton::ToggleColorBlind => {
                settings.0.color_blind_mode = !settings.0.color_blind_mode;
                persist(&path, &settings.0);
                changed.send(SettingsChangedEvent(settings.0.clone()));
                if let Ok(mut t) = color_blind_text.get_single_mut() {
                    **t = color_blind_label(settings.0.color_blind_mode);
                }
            }
            SettingsButton::SelectCardBack(idx) => {
                settings.0.selected_card_back = *idx;
                persist(&path, &settings.0);
                changed.send(SettingsChangedEvent(settings.0.clone()));
            }
            SettingsButton::SelectBackground(idx) => {
                settings.0.selected_background = *idx;
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

fn color_blind_label(enabled: bool) -> String {
    if enabled { "ON".into() } else { "OFF".into() }
}

/// Scrolls the settings panel inner card in response to mouse-wheel events.
///
/// `offset_y` increases downward (0 = top of content). Scrolling down (ev.y < 0)
/// adds to the offset; scrolling up subtracts. Clamped to >= 0 so it never
/// scrolls past the top.
fn scroll_settings_panel(
    mut scroll_evr: EventReader<MouseWheel>,
    screen: Res<SettingsScreen>,
    mut scrollables: Query<&mut ScrollPosition, With<SettingsPanelScrollable>>,
) {
    if !screen.0 {
        scroll_evr.clear();
        return;
    }
    let delta_y: f32 = scroll_evr
        .read()
        .map(|ev| match ev.unit {
            MouseScrollUnit::Line => ev.y * 50.0,
            MouseScrollUnit::Pixel => ev.y,
        })
        .sum();
    if delta_y == 0.0 {
        return;
    }
    for mut sp in scrollables.iter_mut() {
        sp.offset_y = (sp.offset_y - delta_y).max(0.0);
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
    scroll_offset: f32,
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
            // Inner card — max_height + scroll_y lets the player reach all rows
            // on small windows by scrolling with the mouse wheel.
            root.spawn((
                SettingsPanelScrollable,
                SettingsScrollNode,
                ScrollPosition { offset_y: scroll_offset, ..default() },
                Node {
                    flex_direction: FlexDirection::Column,
                    padding: UiRect::all(Val::Px(28.0)),
                    row_gap: Val::Px(14.0),
                    min_width: Val::Px(340.0),
                    max_height: Val::Percent(88.0),
                    overflow: Overflow::scroll_y(),
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

                // Color-blind mode row
                card.spawn(Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(8.0),
                    ..default()
                })
                .with_children(|row| {
                    row.spawn((
                        Text::new("Color-blind Mode"),
                        TextFont { font_size: 18.0, ..default() },
                        TextColor(Color::srgb(0.85, 0.85, 0.80)),
                    ));
                    row.spawn((
                        ColorBlindText,
                        Text::new(color_blind_label(settings.color_blind_mode)),
                        TextFont { font_size: 18.0, ..default() },
                        TextColor(Color::WHITE),
                    ));
                    icon_button(row, "⇄", SettingsButton::ToggleColorBlind);
                });

                // --- Card Back section ---
                section_label(card, "Card Back");
                card.spawn(Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(8.0),
                    flex_wrap: FlexWrap::Wrap,
                    ..default()
                })
                .with_children(|row| {
                    // Always show at least button "1" (index 0 = default).
                    let backs = if unlocked_card_backs.is_empty() {
                        &[0usize][..]
                    } else {
                        unlocked_card_backs
                    };
                    for &back_idx in backs {
                        let is_selected = back_idx == settings.selected_card_back;
                        let bg_color = if is_selected {
                            Color::srgb(0.2, 0.9, 0.3)
                        } else {
                            Color::srgb(0.25, 0.25, 0.30)
                        };
                        row.spawn((
                            SettingsButton::SelectCardBack(back_idx),
                            Button,
                            Node {
                                width: Val::Px(40.0),
                                height: Val::Px(40.0),
                                justify_content: JustifyContent::Center,
                                align_items: AlignItems::Center,
                                ..default()
                            },
                            BackgroundColor(bg_color),
                            BorderRadius::all(Val::Px(4.0)),
                        ))
                        .with_children(|b| {
                            b.spawn((
                                Text::new(format!("{}", back_idx + 1)),
                                TextFont { font_size: 16.0, ..default() },
                                TextColor(Color::WHITE),
                            ));
                        });
                    }
                });

                // --- Background section ---
                section_label(card, "Background");
                card.spawn(Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(8.0),
                    flex_wrap: FlexWrap::Wrap,
                    ..default()
                })
                .with_children(|row| {
                    // Always show at least button "1" (index 0 = default).
                    let bgs = if unlocked_backgrounds.is_empty() {
                        &[0usize][..]
                    } else {
                        unlocked_backgrounds
                    };
                    for &bg_idx in bgs {
                        let is_selected = bg_idx == settings.selected_background;
                        let bg_color = if is_selected {
                            Color::srgb(0.2, 0.9, 0.3)
                        } else {
                            Color::srgb(0.25, 0.25, 0.30)
                        };
                        row.spawn((
                            SettingsButton::SelectBackground(bg_idx),
                            Button,
                            Node {
                                width: Val::Px(40.0),
                                height: Val::Px(40.0),
                                justify_content: JustifyContent::Center,
                                align_items: AlignItems::Center,
                                ..default()
                            },
                            BackgroundColor(bg_color),
                            BorderRadius::all(Val::Px(4.0)),
                        ))
                        .with_children(|b| {
                            b.spawn((
                                Text::new(format!("{}", bg_idx + 1)),
                                TextFont { font_size: 16.0, ..default() },
                                TextColor(Color::WHITE),
                            ));
                        });
                    }
                });

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

    #[test]
    fn volume_clamped_at_zero_does_not_emit_event() {
        let mut app = headless_app();
        app.world_mut().resource_mut::<SettingsResource>().0.sfx_volume = 0.0;

        press(&mut app, KeyCode::BracketLeft);
        app.update();

        let after = app.world().resource::<SettingsResource>().0.sfx_volume;
        assert!(after >= 0.0, "volume must not go below zero");

        let events = app.world().resource::<Events<SettingsChangedEvent>>();
        let mut cursor = events.get_cursor();
        assert_eq!(cursor.read(events).count(), 0, "no event when clamped at floor");
    }

    #[test]
    fn pressing_o_toggles_settings_screen_flag() {
        let mut app = headless_app();
        assert!(!app.world().resource::<SettingsScreen>().0, "screen is closed initially");

        press(&mut app, KeyCode::KeyO);
        app.update();
        assert!(app.world().resource::<SettingsScreen>().0, "O opens settings");

        press(&mut app, KeyCode::KeyO);
        app.update();
        assert!(!app.world().resource::<SettingsScreen>().0, "second O closes settings");
    }

    // cycle_unlocked pure-function tests
    #[test]
    fn cycle_unlocked_wraps_at_end() {
        // [0, 1, 2] → cycling from 2 wraps to 0
        assert_eq!(cycle_unlocked(&[0, 1, 2], 2), 0);
    }

    #[test]
    fn cycle_unlocked_advances_normally() {
        assert_eq!(cycle_unlocked(&[0, 1, 2], 0), 1);
        assert_eq!(cycle_unlocked(&[0, 1, 2], 1), 2);
    }

    #[test]
    fn cycle_unlocked_single_element_stays() {
        // Only one unlockable — cycling always returns it.
        assert_eq!(cycle_unlocked(&[0], 0), 0);
    }

    #[test]
    fn cycle_unlocked_current_not_in_list_falls_back_to_second() {
        // current=5 is not in [0,1,2]; falls back to pos=0, so next = unlocked[1] = 1
        assert_eq!(cycle_unlocked(&[0, 1, 2], 5), 1);
    }

    #[test]
    fn cycle_unlocked_empty_returns_zero() {
        assert_eq!(cycle_unlocked(&[], 0), 0);
    }

    #[test]
    fn scroll_is_noop_when_settings_panel_closed() {
        use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
        let mut app = headless_app();
        // Panel starts closed (SettingsScreen(false)); spawn a scrollable entity.
        let entity = app
            .world_mut()
            .spawn((SettingsPanelScrollable, ScrollPosition::default()))
            .id();
        // Send a downward scroll event while the panel is closed.
        app.world_mut().send_event(MouseWheel {
            unit: MouseScrollUnit::Line,
            x: 0.0,
            y: -3.0,
            window: bevy::ecs::entity::Entity::PLACEHOLDER,
        });
        app.update();
        // ScrollPosition must remain at 0.0 — panel was closed.
        let offset = app
            .world()
            .entity(entity)
            .get::<ScrollPosition>()
            .unwrap()
            .offset_y;
        assert_eq!(offset, 0.0, "scroll must not move when panel is closed");
    }

    #[test]
    fn scroll_moves_offset_when_panel_open() {
        use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
        let mut app = headless_app();
        // Open the panel.
        app.world_mut().resource_mut::<SettingsScreen>().0 = true;
        // Spawn a scrollable entity with an existing offset so we can distinguish clamping.
        let entity = app
            .world_mut()
            .spawn((
                SettingsPanelScrollable,
                ScrollPosition { offset_y: 100.0, ..default() },
            ))
            .id();
        // Scroll down by 2 lines (50 px/line → +100 px added to offset_y).
        app.world_mut().send_event(MouseWheel {
            unit: MouseScrollUnit::Line,
            x: 0.0,
            y: -2.0,
            window: bevy::ecs::entity::Entity::PLACEHOLDER,
        });
        app.update();
        let offset = app
            .world()
            .entity(entity)
            .get::<ScrollPosition>()
            .unwrap()
            .offset_y;
        assert!((offset - 200.0).abs() < 1e-3, "scrolling down should increase offset_y; got {offset}");
    }

    #[test]
    fn scroll_clamps_offset_to_zero_at_top() {
        use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
        let mut app = headless_app();
        app.world_mut().resource_mut::<SettingsScreen>().0 = true;
        // Entity starts at 10 px offset.
        let entity = app
            .world_mut()
            .spawn((
                SettingsPanelScrollable,
                ScrollPosition { offset_y: 10.0, ..default() },
            ))
            .id();
        // Scroll up by 5 lines → would subtract 250 px, but must clamp to 0.
        app.world_mut().send_event(MouseWheel {
            unit: MouseScrollUnit::Line,
            x: 0.0,
            y: 5.0,
            window: bevy::ecs::entity::Entity::PLACEHOLDER,
        });
        app.update();
        let offset = app
            .world()
            .entity(entity)
            .get::<ScrollPosition>()
            .unwrap()
            .offset_y;
        assert_eq!(offset, 0.0, "scrolling past top must clamp to 0, got {offset}");
    }
}
