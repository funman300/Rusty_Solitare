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
use bevy::ui::{ComputedNode, UiGlobalTransform};
use bevy::window::{WindowMoved, WindowResized};
use solitaire_core::game_state::DrawMode;
use solitaire_data::{
    load_settings_from, save_settings_to, settings_file_path, settings::Theme, AnimSpeed, Settings,
    WindowGeometry, REPLAY_MOVE_INTERVAL_STEP_SECS, TIME_BONUS_MULTIPLIER_STEP,
    TOOLTIP_DELAY_STEP_SECS,
};

use crate::events::{InfoToastEvent, ManualSyncRequestEvent, ToggleSettingsRequestEvent};
use crate::font_plugin::FontResource;
use crate::progress_plugin::ProgressResource;
use crate::resources::{SettingsScrollPos, SyncStatus, SyncStatusResource};
use crate::theme::{ThemeThumbnailCache, ThemeThumbnailPair};
use crate::ui_focus::{FocusGroup, FocusRow, Focusable, FocusedButton};
use crate::ui_modal::{
    spawn_modal, spawn_modal_actions, spawn_modal_button, spawn_modal_header, ButtonVariant,
    ModalButton, ModalScrim,
};
use crate::ui_tooltip::Tooltip;
use crate::ui_theme::{
    BG_BASE, BG_ELEVATED, BG_ELEVATED_HI, BORDER_SUBTLE, RADIUS_SM, SPACE_2, STATE_SUCCESS,
    TEXT_PRIMARY, TEXT_SECONDARY, TYPE_BODY, TYPE_BODY_LG, TYPE_CAPTION, VAL_SPACE_2, VAL_SPACE_3,
    Z_MODAL_PANEL,
};

/// Side length of a swatch button in the card-back / background pickers.
/// Smaller than the smallest spacing rung so it stays a literal.
const SWATCH_PX: f32 = 40.0;

/// Side length of a small toggle / cycle button (e.g. the "⇄" affordances).
/// Sub-rung sizing — kept as a literal, see SWATCH_PX. 32 px meets the
/// minimum desktop hit-target threshold while staying smaller than `SWATCH_PX`.
const ICON_BUTTON_PX: f32 = 32.0;

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

/// Debounce window for persisting window-geometry changes, in seconds.
///
/// `WindowResized` and `WindowMoved` fire continuously during a resize/
/// move drag, so writing to disk on every event would thrash the file
/// system. Instead the geometry-watch system records the pending value
/// and waits this long after the *last* event before saving.
pub const WINDOW_GEOMETRY_DEBOUNCE_SECS: f32 = 0.5;

/// Tracks a pending window-geometry change so the saver can debounce
/// `WindowResized` / `WindowMoved` storms during a resize / move drag.
#[derive(Resource, Debug, Default, Clone, Copy)]
pub struct PendingWindowGeometry {
    /// Most recent observed geometry. `None` when nothing is pending.
    pub geometry: Option<WindowGeometry>,
    /// `Time::elapsed_secs()` value at which `geometry` was last updated.
    pub last_changed_secs: f32,
}

/// Fired whenever settings change so consumers (audio, UI) can react.
#[derive(Message, Debug, Clone)]
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

/// Marks the `Text` node showing the live tooltip-delay value.
#[derive(Component, Debug)]
struct TooltipDelayText;

/// Marks the `Text` node showing the live time-bonus-multiplier value.
#[derive(Component, Debug)]
struct TimeBonusMultiplierText;

/// Marks the `Text` node showing the live replay-playback per-move
/// interval value. The Gameplay-section row beside this label lets the
/// player tune `Settings::replay_move_interval_secs`.
#[derive(Component, Debug)]
struct ReplayMoveIntervalText;

/// Marks the `Text` node showing the current "Winnable deals only"
/// state ("ON" / "OFF") in the Gameplay section.
#[derive(Component, Debug)]
struct WinnableDealsOnlyText;

/// Marks the scrollable inner card so the mouse-wheel system can target it.
#[derive(Component, Debug)]
struct SettingsPanelScrollable;

/// Marks the scrollable inner card so its `ScrollPosition` can be read before despawn.
#[derive(Component, Debug)]
struct SettingsScrollNode;

/// Snapshot row used by [`spawn_settings_panel`] to render the card-art
/// theme picker. Carries the `ThemeRegistry` entry's display fields plus
/// the (optional) thumbnail pair from [`ThemeThumbnailCache`]. A `None`
/// thumbnail means the picker should render a placeholder swatch — used
/// when the cache hasn't generated handles yet, or when a user theme
/// is missing one of the required preview SVGs.
#[derive(Debug, Clone)]
struct ThemePickerEntry {
    /// Stable theme id (matches `ThemeMeta::id`).
    id: String,
    /// Player-facing label.
    display_name: String,
    /// Pre-generated picker preview pair, when ready. `None` collapses
    /// the chip to its plain-text fallback.
    thumbnails: Option<ThemeThumbnailPair>,
}

/// Tags interactive buttons inside the Settings panel.
#[derive(Component, Debug)]
enum SettingsButton {
    SfxDown,
    SfxUp,
    MusicDown,
    MusicUp,
    ToggleDrawMode,
    CycleAnimSpeed,
    /// Decrement the tooltip-hover dwell delay by one step.
    TooltipDelayDown,
    /// Increment the tooltip-hover dwell delay by one step.
    TooltipDelayUp,
    /// Decrement the cosmetic time-bonus multiplier by one step.
    TimeBonusDown,
    /// Increment the cosmetic time-bonus multiplier by one step.
    TimeBonusUp,
    /// Decrement the replay-playback per-move interval by one step
    /// (i.e. speed playback up).
    ReplayMoveIntervalDown,
    /// Increment the replay-playback per-move interval by one step
    /// (i.e. slow playback down).
    ReplayMoveIntervalUp,
    ToggleTheme,
    ToggleColorBlind,
    /// Toggle the [`Settings::winnable_deals_only`] flag. When on, new
    /// random Classic-mode deals are filtered through
    /// [`solitaire_core::solver::try_solve`] until one is provably
    /// winnable (or the retry cap is hit). Off by default.
    ToggleWinnableDealsOnly,
    SyncNow,
    Done,
    /// Select a specific card-back by index from the picker row.
    SelectCardBack(usize),
    /// Select a specific background by index from the picker row.
    SelectBackground(usize),
    /// Select a specific card-art theme by `meta.id` from the
    /// `ThemeRegistry`. The string is owned so the click handler can
    /// hand it directly to `Settings::selected_theme_id`.
    SelectTheme(String),
}

impl SettingsButton {
    /// Tab-walk priority — lower numbers visited first. Visual reading
    /// order is top-to-bottom by section, left-to-right inside each row.
    /// Two buttons in the same picker row receive the same `order`;
    /// `handle_focus_keys` then breaks ties by entity index, which
    /// matches `Children` spawn order inside each row.
    fn focus_order(&self) -> i32 {
        match self {
            // Audio section
            SettingsButton::SfxDown => 10,
            SettingsButton::SfxUp => 11,
            SettingsButton::MusicDown => 20,
            SettingsButton::MusicUp => 21,
            // Gameplay section
            SettingsButton::ToggleDrawMode => 30,
            SettingsButton::ToggleWinnableDealsOnly => 35,
            SettingsButton::CycleAnimSpeed => 40,
            SettingsButton::TooltipDelayDown => 45,
            SettingsButton::TooltipDelayUp => 46,
            SettingsButton::TimeBonusDown => 47,
            SettingsButton::TimeBonusUp => 48,
            // Replay-speed slider — last Gameplay-section row, so it
            // sits between TimeBonusUp (48) and the Cosmetic section.
            SettingsButton::ReplayMoveIntervalDown => 49,
            SettingsButton::ReplayMoveIntervalUp => 49,
            // Cosmetic section
            SettingsButton::ToggleTheme => 55,
            SettingsButton::ToggleColorBlind => 60,
            // Picker rows — every swatch in a row shares the row's
            // priority so entity-index tiebreaking yields left → right.
            SettingsButton::SelectCardBack(_) => 70,
            SettingsButton::SelectBackground(_) => 80,
            SettingsButton::SelectTheme(_) => 85,
            // Sync section
            SettingsButton::SyncNow => 90,
            // Done is tagged by `attach_focusable_to_modal_buttons` and
            // never reaches `attach_focusable_to_settings_buttons`; the
            // value here is only a fallback for completeness.
            SettingsButton::Done => 100,
        }
    }
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
            .init_resource::<PendingWindowGeometry>()
            .add_message::<SettingsChangedEvent>()
            .add_message::<ManualSyncRequestEvent>()
            .add_message::<ToggleSettingsRequestEvent>()
            .add_message::<InfoToastEvent>()
            .add_message::<bevy::input::mouse::MouseWheel>()
            // `WindowResized` / `WindowMoved` are real Bevy window events
            // and emitted by the windowing backend under `DefaultPlugins`,
            // but we register them explicitly here so the geometry watcher
            // also runs cleanly under `MinimalPlugins` (tests).
            .add_message::<WindowResized>()
            .add_message::<WindowMoved>()
            .add_systems(
                Update,
                (
                    handle_volume_keys,
                    toggle_settings_screen,
                    scroll_settings_panel,
                    record_window_geometry_changes,
                    persist_window_geometry_after_debounce,
                ),
            );

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
                    update_tooltip_delay_text,
                    update_time_bonus_multiplier_text,
                    update_replay_move_interval_text,
                    update_winnable_deals_only_text,
                    attach_focusable_to_settings_buttons,
                    scroll_focus_into_view,
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

/// Pure helper: returns `true` when a pending geometry change has sat
/// quietly long enough to flush to disk.
///
/// Extracted so the debounce condition can be unit-tested without
/// spinning up a Bevy app.
fn should_persist_geometry(now_secs: f32, last_changed_secs: f32) -> bool {
    (now_secs - last_changed_secs) >= WINDOW_GEOMETRY_DEBOUNCE_SECS
}

/// Returns the geometry implied by an event pair `(width, height, x, y)`,
/// using each component from `existing` when the corresponding event-derived
/// value is `None`. Returns `None` when neither side supplies width/height.
///
/// Pure helper so the merge logic can be unit-tested without an `App`.
fn merge_geometry(
    existing: Option<WindowGeometry>,
    new_size: Option<(u32, u32)>,
    new_pos: Option<(i32, i32)>,
) -> Option<WindowGeometry> {
    let (width, height) = new_size.or_else(|| existing.map(|g| (g.width, g.height)))?;
    let (x, y) = new_pos
        .or_else(|| existing.map(|g| (g.x, g.y)))
        .unwrap_or((0, 0));
    Some(WindowGeometry { width, height, x, y })
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

fn handle_volume_keys(
    keys: Res<ButtonInput<KeyCode>>,
    mut settings: ResMut<SettingsResource>,
    path: Res<SettingsStoragePath>,
    mut changed: MessageWriter<SettingsChangedEvent>,
    mut toast: MessageWriter<InfoToastEvent>,
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
    changed.write(SettingsChangedEvent(settings.0.clone()));
    toast.write(InfoToastEvent(format!(
        "SFX volume: {}%",
        (after * 100.0).round() as i32
    )));
}

/// Opens or closes the Settings panel — `O` keyboard accelerator or
/// `ToggleSettingsRequestEvent` from the HUD Menu popover.
fn toggle_settings_screen(
    keys: Res<ButtonInput<KeyCode>>,
    mut requests: MessageReader<ToggleSettingsRequestEvent>,
    mut screen: ResMut<SettingsScreen>,
) {
    let button_clicked = requests.read().count() > 0;
    if keys.just_pressed(KeyCode::KeyO) || button_clicked {
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
    font_res: Option<Res<FontResource>>,
    theme_registry: Option<Res<crate::theme::ThemeRegistry>>,
    theme_thumbs: Option<Res<ThemeThumbnailCache>>,
    card_images: Option<Res<crate::card_plugin::CardImageSet>>,
) {
    if !screen.is_changed() {
        return;
    }
    if screen.0 {
        if panels.is_empty() {
            let status_label = sync_status
                .map_or_else(|| "Status: local only".to_string(), |s| sync_status_label(&s.0));
            let unlocked_backs = progress
                .as_ref()
                .map_or(&[0][..], |p| p.0.unlocked_card_backs.as_slice());
            let unlocked_bgs = progress
                .as_ref()
                .map_or(&[0][..], |p| p.0.unlocked_backgrounds.as_slice());
            // Snapshot themes by id, display_name and (optional)
            // thumbnail pair so spawn_settings_panel doesn't have to
            // know about the registry / cache shapes. Empty when
            // ThemeRegistryPlugin isn't installed (tests under
            // MinimalPlugins) — the picker row simply won't render.
            // Missing thumbnails (cache not ready, or partial user
            // theme) leave `thumbnails: None` so the chip renders its
            // plain-text fallback instead of a broken sprite.
            let themes: Vec<ThemePickerEntry> = theme_registry
                .as_deref()
                .map(|r| {
                    r.iter()
                        .map(|e| ThemePickerEntry {
                            id: e.id.clone(),
                            display_name: e.display_name.clone(),
                            thumbnails: theme_thumbs
                                .as_deref()
                                .and_then(|c| c.get(&e.id))
                                .filter(|p| p.is_fully_populated())
                                .cloned(),
                        })
                        .collect()
                })
                .unwrap_or_default();
            // The active card-art theme can supply its own back image —
            // see `card_plugin::CardImageSet::theme_back`. When that is
            // populated the legacy "Card Back" picker has no visible
            // effect, so we render it muted with an explanatory caption
            // rather than letting the player click swatches that do
            // nothing. Absent under `MinimalPlugins`; treated as
            // "no override" in that case.
            let theme_overrides_back = card_images
                .as_ref()
                .is_some_and(|cs| cs.theme_back.is_some());
            spawn_settings_panel(
                &mut commands,
                &settings.0,
                &status_label,
                unlocked_backs,
                unlocked_bgs,
                &themes,
                scroll_pos.0,
                font_res.as_deref(),
                theme_overrides_back,
            );
        }
    } else {
        // Save the current scroll offset before despawning the panel.
        if let Ok(sp) = scroll_nodes.single() {
            scroll_pos.0 = sp.0.y;
        }
        for entity in &panels {
            commands.entity(entity).despawn();
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

/// Refreshes the live "Winnable deals only" toggle value in the
/// Gameplay section whenever `SettingsResource` changes (button click,
/// hand-edited `settings.json` reload, etc.).
fn update_winnable_deals_only_text(
    settings: Res<SettingsResource>,
    mut text_nodes: Query<&mut Text, With<WinnableDealsOnlyText>>,
) {
    if !settings.is_changed() {
        return;
    }
    for mut text in &mut text_nodes {
        **text = winnable_deals_only_label(settings.0.winnable_deals_only);
    }
}

/// Refreshes the live tooltip-delay value in the Gameplay section
/// whenever `SettingsResource` changes (slider buttons, hand-edited
/// settings.json reload, etc.).
fn update_tooltip_delay_text(
    settings: Res<SettingsResource>,
    mut text_nodes: Query<&mut Text, With<TooltipDelayText>>,
) {
    if !settings.is_changed() {
        return;
    }
    for mut text in &mut text_nodes {
        **text = tooltip_delay_label(settings.0.tooltip_delay_secs);
    }
}

/// Refreshes the live time-bonus-multiplier value in the Gameplay
/// section whenever `SettingsResource` changes.
fn update_time_bonus_multiplier_text(
    settings: Res<SettingsResource>,
    mut text_nodes: Query<&mut Text, With<TimeBonusMultiplierText>>,
) {
    if !settings.is_changed() {
        return;
    }
    for mut text in &mut text_nodes {
        **text = time_bonus_label(settings.0.time_bonus_multiplier);
    }
}

/// Refreshes the live replay-playback per-move-interval value in the
/// Gameplay section whenever `SettingsResource` changes (slider buttons,
/// hand-edited settings.json reload, etc.).
fn update_replay_move_interval_text(
    settings: Res<SettingsResource>,
    mut text_nodes: Query<&mut Text, With<ReplayMoveIntervalText>>,
) {
    if !settings.is_changed() {
        return;
    }
    for mut text in &mut text_nodes {
        **text = replay_move_interval_label(settings.0.replay_move_interval_secs);
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
    mut changed: MessageWriter<SettingsChangedEvent>,
    mut manual_sync: MessageWriter<ManualSyncRequestEvent>,
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
                    changed.write(SettingsChangedEvent(settings.0.clone()));
                    if let Ok(mut t) = sfx_text.single_mut() {
                        **t = format!("{after:.2}");
                    }
                }
            }
            SettingsButton::SfxUp => {
                let before = settings.0.sfx_volume;
                let after = settings.0.adjust_sfx_volume(SFX_STEP);
                if (before - after).abs() > f32::EPSILON {
                    persist(&path, &settings.0);
                    changed.write(SettingsChangedEvent(settings.0.clone()));
                    if let Ok(mut t) = sfx_text.single_mut() {
                        **t = format!("{after:.2}");
                    }
                }
            }
            SettingsButton::MusicDown => {
                let before = settings.0.music_volume;
                let after = settings.0.adjust_music_volume(-SFX_STEP);
                if (before - after).abs() > f32::EPSILON {
                    persist(&path, &settings.0);
                    changed.write(SettingsChangedEvent(settings.0.clone()));
                    if let Ok(mut t) = music_text.single_mut() {
                        **t = format!("{after:.2}");
                    }
                }
            }
            SettingsButton::MusicUp => {
                let before = settings.0.music_volume;
                let after = settings.0.adjust_music_volume(SFX_STEP);
                if (before - after).abs() > f32::EPSILON {
                    persist(&path, &settings.0);
                    changed.write(SettingsChangedEvent(settings.0.clone()));
                    if let Ok(mut t) = music_text.single_mut() {
                        **t = format!("{after:.2}");
                    }
                }
            }
            SettingsButton::ToggleDrawMode => {
                settings.0.draw_mode = match settings.0.draw_mode {
                    DrawMode::DrawOne => DrawMode::DrawThree,
                    DrawMode::DrawThree => DrawMode::DrawOne,
                };
                persist(&path, &settings.0);
                changed.write(SettingsChangedEvent(settings.0.clone()));
                if let Ok(mut t) = draw_text.single_mut() {
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
                changed.write(SettingsChangedEvent(settings.0.clone()));
                if let Ok(mut t) = anim_speed_text.single_mut() {
                    **t = anim_speed_label(&settings.0.animation_speed);
                }
            }
            SettingsButton::TooltipDelayDown => {
                let before = settings.0.tooltip_delay_secs;
                let after = settings.0.adjust_tooltip_delay(-TOOLTIP_DELAY_STEP_SECS);
                if (before - after).abs() > f32::EPSILON {
                    persist(&path, &settings.0);
                    changed.write(SettingsChangedEvent(settings.0.clone()));
                    // The Text node is refreshed by `update_tooltip_delay_text`
                    // on the next frame via `settings.is_changed()`.
                }
            }
            SettingsButton::TooltipDelayUp => {
                let before = settings.0.tooltip_delay_secs;
                let after = settings.0.adjust_tooltip_delay(TOOLTIP_DELAY_STEP_SECS);
                if (before - after).abs() > f32::EPSILON {
                    persist(&path, &settings.0);
                    changed.write(SettingsChangedEvent(settings.0.clone()));
                }
            }
            SettingsButton::TimeBonusDown => {
                let before = settings.0.time_bonus_multiplier;
                let after = settings.0.adjust_time_bonus_multiplier(-TIME_BONUS_MULTIPLIER_STEP);
                if (before - after).abs() > f32::EPSILON {
                    persist(&path, &settings.0);
                    changed.write(SettingsChangedEvent(settings.0.clone()));
                    // The Text node is refreshed by
                    // `update_time_bonus_multiplier_text` on the next
                    // frame via `settings.is_changed()`.
                }
            }
            SettingsButton::TimeBonusUp => {
                let before = settings.0.time_bonus_multiplier;
                let after = settings.0.adjust_time_bonus_multiplier(TIME_BONUS_MULTIPLIER_STEP);
                if (before - after).abs() > f32::EPSILON {
                    persist(&path, &settings.0);
                    changed.write(SettingsChangedEvent(settings.0.clone()));
                }
            }
            SettingsButton::ReplayMoveIntervalDown => {
                let before = settings.0.replay_move_interval_secs;
                let after = settings
                    .0
                    .adjust_replay_move_interval(-REPLAY_MOVE_INTERVAL_STEP_SECS);
                if (before - after).abs() > f32::EPSILON {
                    persist(&path, &settings.0);
                    changed.write(SettingsChangedEvent(settings.0.clone()));
                    // The Text node is refreshed by
                    // `update_replay_move_interval_text` on the next
                    // frame via `settings.is_changed()`.
                }
            }
            SettingsButton::ReplayMoveIntervalUp => {
                let before = settings.0.replay_move_interval_secs;
                let after = settings
                    .0
                    .adjust_replay_move_interval(REPLAY_MOVE_INTERVAL_STEP_SECS);
                if (before - after).abs() > f32::EPSILON {
                    persist(&path, &settings.0);
                    changed.write(SettingsChangedEvent(settings.0.clone()));
                }
            }
            SettingsButton::ToggleTheme => {
                settings.0.theme = match settings.0.theme {
                    Theme::Green => Theme::Blue,
                    Theme::Blue => Theme::Dark,
                    Theme::Dark => Theme::Green,
                };
                persist(&path, &settings.0);
                changed.write(SettingsChangedEvent(settings.0.clone()));
                if let Ok(mut t) = theme_text.single_mut() {
                    **t = theme_label(&settings.0.theme);
                }
            }
            SettingsButton::ToggleColorBlind => {
                settings.0.color_blind_mode = !settings.0.color_blind_mode;
                persist(&path, &settings.0);
                changed.write(SettingsChangedEvent(settings.0.clone()));
                if let Ok(mut t) = color_blind_text.single_mut() {
                    **t = color_blind_label(settings.0.color_blind_mode);
                }
            }
            SettingsButton::ToggleWinnableDealsOnly => {
                settings.0.winnable_deals_only = !settings.0.winnable_deals_only;
                persist(&path, &settings.0);
                changed.write(SettingsChangedEvent(settings.0.clone()));
                // The Text node is refreshed by `update_winnable_deals_only_text`
                // on the next frame via `settings.is_changed()`.
            }
            SettingsButton::SelectCardBack(idx) => {
                settings.0.selected_card_back = *idx;
                persist(&path, &settings.0);
                changed.write(SettingsChangedEvent(settings.0.clone()));
            }
            SettingsButton::SelectBackground(idx) => {
                settings.0.selected_background = *idx;
                persist(&path, &settings.0);
                changed.write(SettingsChangedEvent(settings.0.clone()));
            }
            SettingsButton::SelectTheme(theme_id) => {
                if settings.0.selected_theme_id != *theme_id {
                    settings.0.selected_theme_id = theme_id.clone();
                    persist(&path, &settings.0);
                    changed.write(SettingsChangedEvent(settings.0.clone()));
                }
            }
            SettingsButton::SyncNow => {
                manual_sync.write(ManualSyncRequestEvent);
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

/// Display string for the "Winnable deals only" toggle. Mirrors
/// [`color_blind_label`] — "ON" / "OFF" — so the layout is uniform
/// with the rest of the Gameplay-section toggles.
fn winnable_deals_only_label(enabled: bool) -> String {
    if enabled { "ON".into() } else { "OFF".into() }
}

/// Formats the tooltip-hover delay for display in the Settings panel.
/// `0.0` reads as `"Instant"` so the zero-delay case has a name; any
/// other value prints as `"{n:.1} s"` (e.g. `"0.5 s"`, `"1.2 s"`).
fn tooltip_delay_label(secs: f32) -> String {
    if secs <= 0.0 {
        "Instant".into()
    } else {
        format!("{secs:.1} s")
    }
}

/// Formats the cosmetic time-bonus multiplier for display in the
/// Settings panel. `0.0` reads as `"Off"` so the player understands the
/// time-bonus row will be hidden; any other value prints as
/// `"{n:.1}×"` (e.g. `"1.0×"`, `"1.5×"`).
fn time_bonus_label(value: f32) -> String {
    if value <= 0.0 {
        "Off".into()
    } else {
        format!("{value:.1}×")
    }
}

/// Formats the replay-playback per-move interval for display in the
/// Settings panel. Mirrors [`tooltip_delay_label`] for parity — the
/// readout is `"{n:.2} s/move"` (e.g. `"0.45 s/move"`, `"0.10 s/move"`),
/// using two decimal places because the step is 0.05 s.
fn replay_move_interval_label(secs: f32) -> String {
    format!("{secs:.2} s/move")
}

/// Auto-attaches [`Focusable`] to every bespoke Settings button — icon
/// buttons (volume +/−, toggle, cycle), swatch buttons (card-back,
/// background pickers), and the "Sync Now" button. The "Done" button is
/// already tagged by `attach_focusable_to_modal_buttons` (it carries
/// [`ModalButton`]) and is filtered out here.
///
/// Walks ancestors via [`ChildOf`] to find the [`ModalScrim`] that owns
/// the panel so the new [`Focusable`]'s group is bound to that scrim —
/// same defensive shape as the Phase 1 / 2 attach systems.
#[allow(clippy::type_complexity)]
fn attach_focusable_to_settings_buttons(
    mut commands: Commands,
    new_buttons: Query<
        (Entity, &SettingsButton),
        (With<Button>, Without<Focusable>, Without<ModalButton>),
    >,
    parents: Query<&ChildOf>,
    scrims: Query<(), With<ModalScrim>>,
) {
    for (button, settings_button) in &new_buttons {
        let mut current = button;
        let mut scrim_entity: Option<Entity> = None;
        for _ in 0..32 {
            if scrims.get(current).is_ok() {
                scrim_entity = Some(current);
                break;
            }
            match parents.get(current) {
                Ok(parent) => current = parent.parent(),
                Err(_) => break,
            }
        }
        if let Some(scrim) = scrim_entity {
            commands.entity(button).insert(Focusable {
                group: FocusGroup::Modal(scrim),
                order: settings_button.focus_order(),
            });
        }
    }
}

/// Vertical padding (logical px) added around the focused button when
/// scrolling it into view. Keeps the focus ring's halo visible above /
/// below the viewport edge.
const FOCUS_SCROLL_PADDING: f32 = SPACE_2;

/// When the focused entity sits outside the visible Settings scroll
/// viewport, adjust the viewport's [`ScrollPosition`] so the button is
/// fully visible. No-op when:
///
/// - `FocusedButton` is `None`
/// - the focused entity has no [`UiGlobalTransform`] / [`ComputedNode`]
///   (e.g. a freshly-spawned modal hasn't laid out yet)
/// - the focused entity is not a descendant of the
///   [`SettingsPanelScrollable`] container
///
/// The viewport's visible Y range is `[scroll_y, scroll_y +
/// viewport_height]` in physical pixels (matching `ComputedNode.size`).
/// The focused button's vertical extent is computed from its
/// `UiGlobalTransform.translation.y` (centre, physical) ± half its
/// `ComputedNode.size.y`. Because the scroll container's local
/// coordinates run [0, content_height] and the visible window is
/// [scroll_y, scroll_y + viewport], we convert the button's window-
/// space Y to container-local Y by subtracting the container's window-
/// space top and adding the current scroll offset.
#[allow(clippy::type_complexity)]
fn scroll_focus_into_view(
    focused: Res<FocusedButton>,
    parents: Query<&ChildOf>,
    nodes: Query<(&UiGlobalTransform, &ComputedNode)>,
    mut containers: Query<
        (&mut ScrollPosition, &UiGlobalTransform, &ComputedNode),
        With<SettingsPanelScrollable>,
    >,
) {
    let Some(target) = focused.0 else { return };
    // Gather button geometry.
    let Ok((target_transform, target_node)) = nodes.get(target) else {
        return;
    };

    // Walk ancestors looking for the scroll container. Bounded to keep
    // a malformed hierarchy from hanging the system.
    let mut current = target;
    let mut container_entity: Option<Entity> = None;
    for _ in 0..32 {
        if containers.get(current).is_ok() {
            container_entity = Some(current);
            break;
        }
        match parents.get(current) {
            Ok(parent) => current = parent.parent(),
            Err(_) => break,
        }
    }
    let Some(container) = container_entity else { return };

    let Ok((mut scroll, container_transform, container_node)) =
        containers.get_mut(container)
    else {
        return;
    };

    // Geometry is reported in physical pixels by `ComputedNode.size` and
    // `UiGlobalTransform.translation`. `ScrollPosition` is in logical px,
    // so convert via `inverse_scale_factor` before we write.
    let inv = target_node.inverse_scale_factor;
    let target_height = target_node.size().y;
    let target_centre_y = target_transform.translation.y;
    let target_top = target_centre_y - target_height * 0.5;
    let target_bottom = target_centre_y + target_height * 0.5;

    let container_height = container_node.size().y;
    let container_top = container_transform.translation.y - container_height * 0.5;

    // Convert button window-space Y to container-local Y. The container
    // is currently scrolled by `scroll.0.y` *logical* pixels — multiply
    // by physical-per-logical to compare with physical pixel extents.
    let scroll_phys = scroll.0.y / inv.max(f32::EPSILON);
    let viewport_top = container_top + scroll_phys;
    let viewport_bottom = viewport_top + container_height;

    // Layout may not have run yet (zero size on first frame) — no
    // sensible scroll target until the container has dimensions.
    if container_height <= 0.0 {
        return;
    }

    let pad_phys = FOCUS_SCROLL_PADDING / inv.max(f32::EPSILON);
    if target_top < viewport_top {
        // Button extends above the viewport — scroll up.
        let new_top = target_top - pad_phys;
        let delta = new_top - viewport_top;
        scroll.0.y = ((scroll_phys + delta) * inv).max(0.0);
    } else if target_bottom > viewport_bottom {
        // Button extends below the viewport — scroll down.
        let new_bottom = target_bottom + pad_phys;
        let delta = new_bottom - viewport_bottom;
        scroll.0.y = ((scroll_phys + delta) * inv).max(0.0);
    }
}

/// Scrolls the settings panel inner card in response to mouse-wheel events.
///
/// `offset_y` increases downward (0 = top of content). Scrolling down (ev.y < 0)
/// adds to the offset; scrolling up subtracts. Clamped to >= 0 so it never
/// scrolls past the top.
fn scroll_settings_panel(
    mut scroll_evr: MessageReader<MouseWheel>,
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
        sp.0.y = (sp.0.y - delta_y).max(0.0);
    }
}

// ---------------------------------------------------------------------------
// Window geometry persistence
// ---------------------------------------------------------------------------

/// Records `WindowResized` and `WindowMoved` events into
/// [`PendingWindowGeometry`], coalescing every event arriving this frame
/// into the latest pending geometry.
///
/// The actual disk write is debounced — see
/// [`persist_window_geometry_after_debounce`] — so the file system isn't
/// hit on every pixel of a resize / move drag.
fn record_window_geometry_changes(
    time: Res<Time>,
    mut resized: MessageReader<WindowResized>,
    mut moved: MessageReader<WindowMoved>,
    settings: Res<SettingsResource>,
    mut pending: ResMut<PendingWindowGeometry>,
) {
    // Read .last() — only the final event matters for persistence; the
    // intermediate sizes/positions are noise during a drag.
    let new_size = resized
        .read()
        .last()
        .map(|ev| (ev.width.round().max(0.0) as u32, ev.height.round().max(0.0) as u32));
    let new_pos = moved.read().last().map(|ev| (ev.position.x, ev.position.y));

    if new_size.is_none() && new_pos.is_none() {
        return;
    }

    // Fold the new components into the existing pending value (if any),
    // otherwise into the persisted geometry from settings.
    let baseline = pending.geometry.or(settings.0.window_geometry);
    let Some(geometry) = merge_geometry(baseline, new_size, new_pos) else {
        return;
    };

    pending.geometry = Some(geometry);
    pending.last_changed_secs = time.elapsed_secs();
}

/// After [`WINDOW_GEOMETRY_DEBOUNCE_SECS`] of quiet (no `WindowResized` or
/// `WindowMoved` events arriving), commits the pending geometry to
/// `SettingsResource` and writes `settings.json`. Skips the write when the
/// pending value already matches the settings (e.g. a resize that was
/// reverted, or a synthetic event with no geometry change).
fn persist_window_geometry_after_debounce(
    time: Res<Time>,
    mut pending: ResMut<PendingWindowGeometry>,
    mut settings: ResMut<SettingsResource>,
    path: Res<SettingsStoragePath>,
    mut changed: MessageWriter<SettingsChangedEvent>,
) {
    let Some(new_geom) = pending.geometry else {
        return;
    };
    if !should_persist_geometry(time.elapsed_secs(), pending.last_changed_secs) {
        return;
    }

    // Always clear the pending slot regardless of whether we end up
    // writing — otherwise an idempotent change would re-trigger this
    // system every tick.
    pending.geometry = None;

    if settings.0.window_geometry == Some(new_geom) {
        return;
    }
    settings.0.window_geometry = Some(new_geom);
    persist(&path, &settings.0);
    changed.write(SettingsChangedEvent(settings.0.clone()));
}

// ---------------------------------------------------------------------------
// UI construction
// ---------------------------------------------------------------------------

/// Spawns the Settings modal.
///
/// `theme_overrides_back` is `true` when the active card-art theme
/// supplies its own back (`CardImageSet::theme_back == Some(_)`). The
/// "Card Back" picker is rendered with a small caption and the
/// swatches are hidden in this state — the theme's back wins
/// regardless of which legacy back is selected, so the picker would
/// be inert otherwise.
#[allow(clippy::too_many_arguments)]
fn spawn_settings_panel(
    commands: &mut Commands,
    settings: &Settings,
    sync_status: &str,
    unlocked_card_backs: &[usize],
    unlocked_backgrounds: &[usize],
    themes: &[ThemePickerEntry],
    scroll_offset: f32,
    font_res: Option<&FontResource>,
    theme_overrides_back: bool,
) {
    spawn_modal(commands, SettingsPanel, Z_MODAL_PANEL, |card| {
        spawn_modal_header(card, "Settings", font_res);

        // Scrollable body — contains every section so tall content stays
        // reachable on short windows. The Done button below stays fixed
        // outside the scroll so it's always one click away.
        card.spawn((
            SettingsPanelScrollable,
            SettingsScrollNode,
            ScrollPosition(Vec2::new(0.0, scroll_offset)),
            Node {
                flex_direction: FlexDirection::Column,
                row_gap: VAL_SPACE_3,
                max_height: Val::Vh(60.0),
                overflow: Overflow::scroll_y(),
                ..default()
            },
        ))
        .with_children(|body| {
            // --- Audio ---
            section_label(body, "Audio", font_res);
            volume_row(
                body,
                "SFX Volume",
                settings.sfx_volume,
                SfxVolumeText,
                SettingsButton::SfxDown,
                SettingsButton::SfxUp,
                "Lower sound effects volume.",
                "Raise sound effects volume.",
                font_res,
            );
            volume_row(
                body,
                "Music Volume",
                settings.music_volume,
                MusicVolumeText,
                SettingsButton::MusicDown,
                SettingsButton::MusicUp,
                "Lower music and ambience volume.",
                "Raise music and ambience volume.",
                font_res,
            );

            // --- Gameplay ---
            section_label(body, "Gameplay", font_res);
            toggle_row(
                body,
                "Draw Mode",
                DrawModeText,
                draw_mode_label(&settings.draw_mode),
                SettingsButton::ToggleDrawMode,
                "Switch between Draw 1 and Draw 3. Takes effect next deal.",
                font_res,
            );
            toggle_row(
                body,
                "Winnable deals only",
                WinnableDealsOnlyText,
                winnable_deals_only_label(settings.winnable_deals_only),
                SettingsButton::ToggleWinnableDealsOnly,
                "When on, fresh Classic deals are filtered through a solver \
                 (may take a moment when on).",
                font_res,
            );
            toggle_row(
                body,
                "Anim Speed",
                AnimSpeedText,
                anim_speed_label(&settings.animation_speed),
                SettingsButton::CycleAnimSpeed,
                "Cycle animation speed: Normal, Fast, Instant.",
                font_res,
            );
            tooltip_delay_row(
                body,
                settings.tooltip_delay_secs,
                font_res,
            );
            time_bonus_multiplier_row(
                body,
                settings.time_bonus_multiplier,
                font_res,
            );
            replay_move_interval_row(
                body,
                settings.replay_move_interval_secs,
                font_res,
            );

            // --- Cosmetic ---
            section_label(body, "Cosmetic", font_res);
            toggle_row(
                body,
                "Theme",
                ThemeText,
                theme_label(&settings.theme),
                SettingsButton::ToggleTheme,
                "Cycle felt color: Green, Blue, Dark.",
                font_res,
            );
            toggle_row(
                body,
                "Color-blind Mode",
                ColorBlindText,
                color_blind_label(settings.color_blind_mode),
                SettingsButton::ToggleColorBlind,
                "Show shape glyphs alongside suit colors. Suit-blind friendly.",
                font_res,
            );
            if theme_overrides_back {
                // The active theme provides its own back; the legacy
                // picker has no visible effect, so we replace its
                // swatch row with an informational caption. The
                // player's `selected_card_back` value still
                // round-trips through `settings.json` — the moment
                // they switch to a theme without a back, the picker
                // re-appears with their previous choice intact.
                picker_row_overridden_by_theme(body, "Card Back", font_res);
            } else {
                picker_row(
                    body,
                    "Card Back",
                    unlocked_card_backs,
                    settings.selected_card_back,
                    SettingsButton::SelectCardBack,
                    "Choose your deck art. New backs unlock at higher levels.",
                    font_res,
                );
            }
            picker_row(
                body,
                "Background",
                unlocked_backgrounds,
                settings.selected_background,
                SettingsButton::SelectBackground,
                "Choose your felt art. New felts unlock at higher levels.",
                font_res,
            );
            // Card-art theme picker — only renders when the registry has
            // entries (production: always; tests: only when
            // ThemeRegistryPlugin is installed).
            if !themes.is_empty() {
                theme_picker_row(
                    body,
                    "Card Theme",
                    themes,
                    &settings.selected_theme_id,
                    "Choose card-face artwork. Imported themes appear here.",
                    font_res,
                );
            }

            // --- Sync ---
            section_label(body, "Sync", font_res);
            sync_row(body, sync_status, font_res);
        });

        // Done is the only action — primary so the player always knows
        // how to leave the modal. `O` toggles it the same way.
        spawn_modal_actions(card, |actions| {
            spawn_modal_button(
                actions,
                SettingsButton::Done,
                "Done",
                Some("O"),
                ButtonVariant::Primary,
                font_res,
            );
        });
    });
}

/// Section divider — small lavender label inside the scrollable body.
fn section_label(parent: &mut ChildSpawnerCommands, title: &str, font_res: Option<&FontResource>) {
    let font = TextFont {
        font: font_res.map(|f| f.0.clone()).unwrap_or_default(),
        font_size: TYPE_BODY,
        ..default()
    };
    parent.spawn((Text::new(title), font, TextColor(TEXT_SECONDARY)));
}

/// `Label  0.80  [−]  [+]` — used for SFX and Music volume rows.
///
/// `tooltip_down` / `tooltip_up` are attached to the `−` / `+` buttons
/// respectively so each glyph carries a one-line reminder of which channel
/// it adjusts.
#[allow(clippy::too_many_arguments)]
fn volume_row<Marker: Component>(
    parent: &mut ChildSpawnerCommands,
    label: &str,
    value: f32,
    marker: Marker,
    btn_down: SettingsButton,
    btn_up: SettingsButton,
    tooltip_down: &'static str,
    tooltip_up: &'static str,
    font_res: Option<&FontResource>,
) {
    let label_font = label_text_font(font_res);
    let value_font = value_text_font(font_res);
    // Row spans the full body width with a flex-grow spacer between
    // the left-aligned label and the right-aligned controls cluster.
    // Without `width: 100%` + the spacer, the label / value / buttons
    // bunch against the left edge and a varying-length value (e.g.
    // "0.80" → "1.00") shifts the +/− buttons sideways frame to
    // frame, visually overlapping with adjacent UI on small windows.
    parent
        .spawn(Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: VAL_SPACE_2,
            width: Val::Percent(100.0),
            ..default()
        })
        .with_children(|row| {
            row.spawn((
                Text::new(label.to_string()),
                label_font,
                TextColor(TEXT_SECONDARY),
            ));
            // Spacer: takes up all remaining horizontal space so the
            // controls cluster sits flush against the right edge.
            row.spawn(Node {
                flex_grow: 1.0,
                ..default()
            });
            // Controls cluster — value + decrement + increment held
            // together so the buttons stay in fixed positions even
            // as the value text width varies.
            row.spawn(Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: VAL_SPACE_2,
                ..default()
            })
            .with_children(|cluster| {
                cluster.spawn((
                    marker,
                    Text::new(format!("{value:.2}")),
                    value_font,
                    TextColor(TEXT_PRIMARY),
                ));
                icon_button(cluster, "−", btn_down, tooltip_down, font_res);
                icon_button(cluster, "+", btn_up, tooltip_up, font_res);
            });
        });
}

/// `Tooltip Delay  0.5 s  [−]  [+]` — slider row for the player-tunable
/// tooltip-hover dwell. Mirrors [`volume_row`] (label, current value,
/// decrement, increment) but formats the value via [`tooltip_delay_label`]
/// so `0.0` reads as `"Instant"` and other values as `"{n:.1} s"`.
fn tooltip_delay_row(
    parent: &mut ChildSpawnerCommands,
    value_secs: f32,
    font_res: Option<&FontResource>,
) {
    let label_font = label_text_font(font_res);
    let value_font = value_text_font(font_res);
    parent
        .spawn(Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: VAL_SPACE_2,
            width: Val::Percent(100.0),
            ..default()
        })
        .with_children(|row| {
            row.spawn((
                Text::new("Tooltip Delay".to_string()),
                label_font,
                TextColor(TEXT_SECONDARY),
            ));
            row.spawn(Node {
                flex_grow: 1.0,
                ..default()
            });
            row.spawn(Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: VAL_SPACE_2,
                ..default()
            })
            .with_children(|cluster| {
                cluster.spawn((
                    TooltipDelayText,
                    Text::new(tooltip_delay_label(value_secs)),
                    value_font,
                    TextColor(TEXT_PRIMARY),
                ));
                icon_button(
                    cluster,
                    "−",
                    SettingsButton::TooltipDelayDown,
                    "Shorten the hover delay before tooltips appear.",
                    font_res,
                );
                icon_button(
                    cluster,
                    "+",
                    SettingsButton::TooltipDelayUp,
                    "Lengthen the hover delay before tooltips appear.",
                    font_res,
                );
            });
        });
}

/// `Time bonus  1.0×  [−]  [+]` — slider row for the cosmetic
/// `Settings::time_bonus_multiplier`. Mirrors [`tooltip_delay_row`]
/// (label, current value, decrement, increment) but formats the value
/// via [`time_bonus_label`] so `0.0` reads as `"Off"` and other values
/// as `"{n:.1}×"`. The multiplier is **cosmetic** — adjusting it
/// changes only the win-modal score breakdown, not the canonical
/// scores recorded in stats / achievements / leaderboards.
fn time_bonus_multiplier_row(
    parent: &mut ChildSpawnerCommands,
    value: f32,
    font_res: Option<&FontResource>,
) {
    let label_font = label_text_font(font_res);
    let value_font = value_text_font(font_res);
    parent
        .spawn(Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: VAL_SPACE_2,
            width: Val::Percent(100.0),
            ..default()
        })
        .with_children(|row| {
            row.spawn((
                Text::new("Time bonus".to_string()),
                label_font,
                TextColor(TEXT_SECONDARY),
            ));
            row.spawn(Node {
                flex_grow: 1.0,
                ..default()
            });
            row.spawn(Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: VAL_SPACE_2,
                ..default()
            })
            .with_children(|cluster| {
                cluster.spawn((
                    TimeBonusMultiplierText,
                    Text::new(time_bonus_label(value)),
                    value_font,
                    TextColor(TEXT_PRIMARY),
                ));
                icon_button(
                    cluster,
                    "−",
                    SettingsButton::TimeBonusDown,
                    "Shrink the time-bonus shown in the win modal. Cosmetic only.",
                    font_res,
                );
                icon_button(
                    cluster,
                    "+",
                    SettingsButton::TimeBonusUp,
                    "Boost the time-bonus shown in the win modal. Cosmetic only.",
                    font_res,
                );
            });
        });
}

/// `Replay speed  0.45 s/move  [−]  [+]` — slider row for the
/// player-tunable replay-playback per-move interval. Mirrors
/// [`tooltip_delay_row`] (label, current value, decrement, increment)
/// but formats the value via [`replay_move_interval_label`] as
/// `"{n:.2} s/move"`. The decrement button speeds playback up
/// (smaller interval); the increment slows it down — same direction
/// convention as the tooltip-delay slider.
fn replay_move_interval_row(
    parent: &mut ChildSpawnerCommands,
    value_secs: f32,
    font_res: Option<&FontResource>,
) {
    let label_font = label_text_font(font_res);
    let value_font = value_text_font(font_res);
    parent
        .spawn(Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: VAL_SPACE_2,
            width: Val::Percent(100.0),
            ..default()
        })
        .with_children(|row| {
            row.spawn((
                Text::new("Replay speed".to_string()),
                label_font,
                TextColor(TEXT_SECONDARY),
            ));
            row.spawn(Node {
                flex_grow: 1.0,
                ..default()
            });
            row.spawn(Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: VAL_SPACE_2,
                ..default()
            })
            .with_children(|cluster| {
                cluster.spawn((
                    ReplayMoveIntervalText,
                    Text::new(replay_move_interval_label(value_secs)),
                    value_font,
                    TextColor(TEXT_PRIMARY),
                ));
                icon_button(
                    cluster,
                    "−",
                    SettingsButton::ReplayMoveIntervalDown,
                    "Speed up replay playback (shorter per-move interval).",
                    font_res,
                );
                icon_button(
                    cluster,
                    "+",
                    SettingsButton::ReplayMoveIntervalUp,
                    "Slow down replay playback (longer per-move interval).",
                    font_res,
                );
            });
        });
}

/// `Label  Value  [⇄]` — used for cycle/toggle rows (draw mode, theme,
/// anim speed, colour-blind).
///
/// `tooltip` is attached to the `⇄` button so the cycle glyph carries a
/// one-line reminder of what it iterates through.
#[allow(clippy::too_many_arguments)]
fn toggle_row<Marker: Component>(
    parent: &mut ChildSpawnerCommands,
    label: &str,
    marker: Marker,
    value: String,
    action: SettingsButton,
    tooltip: &'static str,
    font_res: Option<&FontResource>,
) {
    let label_font = label_text_font(font_res);
    let value_font = value_text_font(font_res);
    parent
        .spawn(Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: VAL_SPACE_2,
            width: Val::Percent(100.0),
            ..default()
        })
        .with_children(|row| {
            row.spawn((
                Text::new(label.to_string()),
                label_font,
                TextColor(TEXT_SECONDARY),
            ));
            row.spawn(Node {
                flex_grow: 1.0,
                ..default()
            });
            row.spawn(Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: VAL_SPACE_2,
                ..default()
            })
            .with_children(|cluster| {
                cluster.spawn((marker, Text::new(value), value_font, TextColor(TEXT_PRIMARY)));
                icon_button(cluster, "⇄", action, tooltip, font_res);
            });
        });
}

/// Wrapping row of indexed swatch buttons — used for card-back and
/// background pickers. The currently-selected swatch is tinted with
/// `STATE_SUCCESS` so the user can see it without reading a label.
///
/// `tooltip` is attached to every swatch in the row so hovering any chip
/// reveals what the picker controls and how new entries unlock.
#[allow(clippy::too_many_arguments)]
fn picker_row(
    parent: &mut ChildSpawnerCommands,
    label: &str,
    unlocked: &[usize],
    selected: usize,
    make_button: impl Fn(usize) -> SettingsButton,
    tooltip: &'static str,
    font_res: Option<&FontResource>,
) {
    let label_font = label_text_font(font_res);
    let chip_font = TextFont {
        font: font_res.map(|f| f.0.clone()).unwrap_or_default(),
        font_size: TYPE_BODY,
        ..default()
    };
    parent
        .spawn((
            // The row container is a `FocusRow` so Left / Right arrow
            // keys cycle within its swatch children. Tab still escapes
            // the row to the next focusable in the modal.
            FocusRow,
            Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: VAL_SPACE_2,
                flex_wrap: FlexWrap::Wrap,
                ..default()
            },
        ))
        .with_children(|row| {
            row.spawn((
                Text::new(label.to_string()),
                label_font,
                TextColor(TEXT_SECONDARY),
            ));
            // Always show at least swatch 0 (default).
            let entries: &[usize] = if unlocked.is_empty() { &[0] } else { unlocked };
            for &idx in entries {
                let is_selected = idx == selected;
                let bg = if is_selected { STATE_SUCCESS } else { BG_ELEVATED_HI };
                row.spawn((
                    make_button(idx),
                    Button,
                    Tooltip::new(tooltip),
                    Node {
                        width: Val::Px(SWATCH_PX),
                        height: Val::Px(SWATCH_PX),
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        border: UiRect::all(Val::Px(1.0)),
                        border_radius: BorderRadius::all(Val::Px(RADIUS_SM)),
                        ..default()
                    },
                    BackgroundColor(bg),
                    BorderColor::all(BORDER_SUBTLE),
                ))
                .with_children(|b| {
                    let text_color = if is_selected { BG_BASE } else { TEXT_PRIMARY };
                    b.spawn((
                        Text::new(format!("{}", idx + 1)),
                        chip_font.clone(),
                        TextColor(text_color),
                    ));
                });
            }
        });
}

/// Marker on the row spawned by [`picker_row_overridden_by_theme`] so
/// tests can find the caption without depending on text-content
/// matching.
#[derive(Component, Debug)]
pub(crate) struct CardBackPickerOverriddenByTheme;

/// Marker placed on every preview-thumbnail [`ImageNode`] inside a
/// theme picker chip. Lets tests assert that a chip's children include
/// the rasterised preview pair, and lets a future system update or
/// hot-swap thumbnails without scanning the whole UI tree.
#[derive(Component, Debug)]
pub(crate) struct ThemeThumbnailMarker;

/// Renders the "Card Back" row in its overridden-by-theme state: a
/// labelled caption explaining why the swatches are hidden, with no
/// interactive children. This is what the player sees when the active
/// card-art theme supplies its own `back.svg` — the theme's back wins
/// over the legacy `selected_card_back` choice, so showing the
/// swatches would only confuse the player into thinking they were
/// changing something when they weren't.
fn picker_row_overridden_by_theme(
    parent: &mut ChildSpawnerCommands,
    label: &str,
    font_res: Option<&FontResource>,
) {
    let label_font = label_text_font(font_res);
    let caption_font = TextFont {
        font: font_res.map(|f| f.0.clone()).unwrap_or_default(),
        font_size: TYPE_CAPTION,
        ..default()
    };
    parent
        .spawn((
            CardBackPickerOverriddenByTheme,
            Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: VAL_SPACE_2,
                ..default()
            },
        ))
        .with_children(|row| {
            row.spawn((
                Text::new(label.to_string()),
                label_font,
                TextColor(TEXT_SECONDARY),
            ));
            row.spawn((
                Text::new("Active theme provides its own back"),
                caption_font,
                TextColor(TEXT_SECONDARY),
            ));
        });
}

/// Logical width (px) of one preview thumbnail inside a picker chip.
/// Mirrors [`crate::theme::THEME_THUMBNAIL_WIDTH_PX`] but at the UI
/// scale used by Bevy's flex layout. The rasterised image itself is
/// 100×140 px; the chip displays it at the same logical size so
/// scaling artifacts stay minimal.
const THUMBNAIL_LOGICAL_WIDTH_PX: f32 = 50.0;
/// Logical height counterpart to [`THUMBNAIL_LOGICAL_WIDTH_PX`] —
/// preserves the 2:3 card aspect.
const THUMBNAIL_LOGICAL_HEIGHT_PX: f32 = 70.0;

/// Picker row for card-art themes. Distinct from [`picker_row`]
/// because themes are identified by `String` ids (matching
/// `ThemeMeta::id`) instead of dense indices, and each chip carries
/// the theme's display name plus a small Ace + back preview pair
/// (when available in [`ThemeThumbnailCache`]).
fn theme_picker_row(
    parent: &mut ChildSpawnerCommands,
    label: &str,
    themes: &[ThemePickerEntry],
    selected_id: &str,
    tooltip: &'static str,
    font_res: Option<&FontResource>,
) {
    let label_font = label_text_font(font_res);
    let chip_font = TextFont {
        font: font_res.map(|f| f.0.clone()).unwrap_or_default(),
        font_size: TYPE_BODY,
        ..default()
    };
    parent
        .spawn((
            FocusRow,
            Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: VAL_SPACE_2,
                flex_wrap: FlexWrap::Wrap,
                ..default()
            },
        ))
        .with_children(|row| {
            row.spawn((
                Text::new(label.to_string()),
                label_font,
                TextColor(TEXT_SECONDARY),
            ));
            for entry in themes {
                let is_selected = entry.id == selected_id;
                let bg = if is_selected { STATE_SUCCESS } else { BG_ELEVATED_HI };
                row.spawn((
                    SettingsButton::SelectTheme(entry.id.clone()),
                    Button,
                    Tooltip::new(tooltip),
                    Node {
                        // Chips with thumbnails stack the preview pair
                        // above the label so a glance reveals the
                        // theme's art without hovering for the
                        // tooltip.
                        flex_direction: FlexDirection::Column,
                        // Theme names are wider than numeric chips —
                        // pad horizontally instead of using a fixed
                        // square swatch.
                        padding: UiRect::axes(VAL_SPACE_2, VAL_SPACE_2),
                        min_height: Val::Px(SWATCH_PX),
                        row_gap: VAL_SPACE_2,
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        border: UiRect::all(Val::Px(1.0)),
                        border_radius: BorderRadius::all(Val::Px(RADIUS_SM)),
                        ..default()
                    },
                    BackgroundColor(bg),
                    BorderColor::all(BORDER_SUBTLE),
                ))
                .with_children(|b| {
                    spawn_thumbnail_pair(b, entry.thumbnails.as_ref());
                    let text_color = if is_selected { BG_BASE } else { TEXT_PRIMARY };
                    b.spawn((
                        Text::new(entry.display_name.clone()),
                        chip_font.clone(),
                        TextColor(text_color),
                    ));
                });
            }
        });
}

/// Spawns the Ace + back preview pair for a theme picker chip.
///
/// When `thumbnails` is `Some(_)` and both handles are non-default,
/// renders two `ImageNode` siblings (Ace on the left, back on the
/// right). When the thumbnails are missing or only partially loaded,
/// renders two muted `BG_ELEVATED` placeholder rectangles at the same
/// logical size — keeping the chip's overall footprint stable so the
/// picker row layout doesn't reflow as the cache fills in.
fn spawn_thumbnail_pair(
    parent: &mut ChildSpawnerCommands,
    thumbnails: Option<&ThemeThumbnailPair>,
) {
    parent
        .spawn(Node {
            flex_direction: FlexDirection::Row,
            column_gap: VAL_SPACE_2,
            align_items: AlignItems::Center,
            ..default()
        })
        .with_children(|pair| {
            match thumbnails {
                Some(t) if t.is_fully_populated() => {
                    spawn_thumbnail_image(pair, t.ace.clone());
                    spawn_thumbnail_image(pair, t.back.clone());
                }
                _ => {
                    spawn_thumbnail_placeholder(pair);
                    spawn_thumbnail_placeholder(pair);
                }
            }
        });
}

/// Spawns one `ImageNode` thumbnail at the canonical preview size.
/// Tagged with [`ThemeThumbnailMarker`] so tests can scan a chip's
/// children for the rendered preview without crawling the whole UI.
fn spawn_thumbnail_image(parent: &mut ChildSpawnerCommands, image: Handle<Image>) {
    parent.spawn((
        ThemeThumbnailMarker,
        ImageNode::new(image),
        Node {
            width: Val::Px(THUMBNAIL_LOGICAL_WIDTH_PX),
            height: Val::Px(THUMBNAIL_LOGICAL_HEIGHT_PX),
            ..default()
        },
    ));
}

/// Spawns a muted placeholder rectangle for the case where the cache
/// has not yet generated thumbnails for a theme — or when a user theme
/// is missing one of its preview SVGs. Same logical size as
/// [`spawn_thumbnail_image`] so chip layout stays stable.
fn spawn_thumbnail_placeholder(parent: &mut ChildSpawnerCommands) {
    parent.spawn((
        Node {
            width: Val::Px(THUMBNAIL_LOGICAL_WIDTH_PX),
            height: Val::Px(THUMBNAIL_LOGICAL_HEIGHT_PX),
            border_radius: BorderRadius::all(Val::Px(RADIUS_SM)),
            ..default()
        },
        BackgroundColor(BG_ELEVATED),
    ));
}

/// Status text + manual "Sync Now" button.
fn sync_row(parent: &mut ChildSpawnerCommands, status_text: &str, font_res: Option<&FontResource>) {
    let status_font = TextFont {
        font: font_res.map(|f| f.0.clone()).unwrap_or_default(),
        font_size: TYPE_BODY,
        ..default()
    };
    let button_font = TextFont {
        font: font_res.map(|f| f.0.clone()).unwrap_or_default(),
        font_size: TYPE_CAPTION,
        ..default()
    };
    parent
        .spawn(Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: VAL_SPACE_3,
            ..default()
        })
        .with_children(|row| {
            row.spawn((
                SyncStatusText,
                Text::new(status_text.to_string()),
                status_font,
                TextColor(TEXT_SECONDARY),
            ));
            // ManualSyncRequestEvent is always registered, so this
            // button is safe to show even when SyncPlugin is absent.
            row.spawn((
                SettingsButton::SyncNow,
                Button,
                Tooltip::new(
                    "Push and pull stats now. Runs automatically on launch and exit.",
                ),
                Node {
                    padding: UiRect::axes(VAL_SPACE_3, VAL_SPACE_2),
                    justify_content: JustifyContent::Center,
                    border: UiRect::all(Val::Px(1.0)),
                    border_radius: BorderRadius::all(Val::Px(RADIUS_SM)),
                    ..default()
                },
                BackgroundColor(BG_ELEVATED_HI),
                BorderColor::all(BORDER_SUBTLE),
            ))
            .with_children(|b| {
                b.spawn((
                    Text::new("Sync Now"),
                    button_font,
                    TextColor(TEXT_PRIMARY),
                ));
            });
        });
}

fn label_text_font(font_res: Option<&FontResource>) -> TextFont {
    TextFont {
        font: font_res.map(|f| f.0.clone()).unwrap_or_default(),
        font_size: TYPE_BODY_LG,
        ..default()
    }
}

fn value_text_font(font_res: Option<&FontResource>) -> TextFont {
    TextFont {
        font: font_res.map(|f| f.0.clone()).unwrap_or_default(),
        font_size: TYPE_BODY_LG,
        ..default()
    }
}

/// Spawns a small square icon button (volume +/−, toggle, cycle).
///
/// `tooltip` is the hover-reveal caption attached via [`Tooltip`]. Every
/// Settings icon button ships with one because the glyph alone (`+`, `−`,
/// `⇄`) does not name what it adjusts; the tooltip carries that meaning.
fn icon_button(
    parent: &mut ChildSpawnerCommands,
    label: &str,
    action: SettingsButton,
    tooltip: &'static str,
    font_res: Option<&FontResource>,
) {
    let glyph_font = TextFont {
        font: font_res.map(|f| f.0.clone()).unwrap_or_default(),
        font_size: TYPE_BODY_LG,
        ..default()
    };
    parent
        .spawn((
            action,
            Button,
            Tooltip::new(tooltip),
            Node {
                width: Val::Px(ICON_BUTTON_PX),
                height: Val::Px(ICON_BUTTON_PX),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(RADIUS_SM)),
                ..default()
            },
            BackgroundColor(BG_ELEVATED_HI),
            BorderColor::all(BORDER_SUBTLE),
        ))
        .with_children(|b| {
            b.spawn((Text::new(label.to_string()), glyph_font, TextColor(TEXT_PRIMARY)));
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

        let events = app.world().resource::<Messages<SettingsChangedEvent>>();
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

        let events = app.world().resource::<Messages<SettingsChangedEvent>>();
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

        let events = app.world().resource::<Messages<SettingsChangedEvent>>();
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
        app.world_mut().write_message(MouseWheel {
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
            .0.y;
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
                ScrollPosition(Vec2::new(0.0, 100.0)),
            ))
            .id();
        // Scroll down by 2 lines (50 px/line → +100 px added to offset_y).
        app.world_mut().write_message(MouseWheel {
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
            .0.y;
        assert!((offset - 200.0).abs() < 1e-3, "scrolling down should increase offset_y; got {offset}");
    }

    // -----------------------------------------------------------------------
    // Phase 3 — keyboard focus ring, Settings buttons + FocusRow
    // -----------------------------------------------------------------------

    /// Headless app that runs the *real* (UI-enabled) `SettingsPlugin`
    /// alongside `UiModalPlugin` and `UiFocusPlugin`, so the spawn /
    /// auto-tag systems fire end-to-end without writing to disk.
    fn headless_app_with_focus() -> App {
        use crate::ui_focus::UiFocusPlugin;
        use crate::ui_modal::UiModalPlugin;

        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(UiModalPlugin)
            .add_plugins(UiFocusPlugin)
            .add_plugins(SettingsPlugin {
                // No persistence — keep the test isolated.
                storage_path: None,
                ui_enabled: true,
            });
        app.init_resource::<ButtonInput<KeyCode>>();
        app.update();
        app
    }

    #[test]
    fn settings_buttons_get_focusable_marker() {
        let mut app = headless_app_with_focus();

        // Open the panel.
        app.world_mut().resource_mut::<SettingsScreen>().0 = true;
        app.update();
        // Two more ticks: the first runs `sync_settings_panel_visibility`
        // and queues the spawn commands; the second flushes them and
        // runs `attach_focusable_to_settings_buttons`.
        app.update();
        app.update();

        // Every bespoke `SettingsButton` (not `Done`, which is also a
        // `ModalButton`) must carry a `Focusable`.
        let untagged: Vec<&SettingsButton> = app
            .world_mut()
            .query_filtered::<&SettingsButton, (With<Button>, Without<Focusable>, Without<ModalButton>)>()
            .iter(app.world())
            .collect();

        assert!(
            untagged.is_empty(),
            "every bespoke Settings button must carry Focusable; missing: {:?}",
            untagged
        );

        // And there must be at least one tagged `SettingsButton` so the
        // assertion above isn't vacuously true (the panel really did
        // spawn).
        let tagged_count = app
            .world_mut()
            .query_filtered::<&SettingsButton, With<Focusable>>()
            .iter(app.world())
            .count();
        assert!(
            tagged_count >= 6,
            "expected the panel to spawn many bespoke buttons (volume up/down ×2, toggles ×4, sync, swatches…); got {tagged_count}"
        );
    }

    /// Every bespoke `SettingsButton` (volume +/−, toggles, swatches,
    /// Sync Now) must spawn with a `Tooltip` so the glyph-only icons and
    /// indexed swatches carry hover-reveal context. Mirrors
    /// `settings_buttons_get_focusable_marker` (Phase 3 focus test) so
    /// the invariant — every interactive Settings element except the
    /// `Done` modal button has a tooltip — is asserted consistently.
    #[test]
    fn settings_buttons_carry_tooltip() {
        let mut app = headless_app_with_focus();

        // Open the panel and let spawn + child-flush run.
        app.world_mut().resource_mut::<SettingsScreen>().0 = true;
        app.update();
        app.update();
        app.update();

        // No bespoke `SettingsButton` (i.e. excluding `Done`, which is
        // also a `ModalButton`) may be missing a `Tooltip`.
        let untipped: Vec<&SettingsButton> = app
            .world_mut()
            .query_filtered::<&SettingsButton, (With<Button>, Without<Tooltip>, Without<ModalButton>)>()
            .iter(app.world())
            .collect();
        assert!(
            untipped.is_empty(),
            "every bespoke Settings button must carry Tooltip; missing: {:?}",
            untipped
        );

        // And there must be at least 6 tipped buttons so the assertion
        // above isn't vacuously true: SFX +/−, Music +/−, Draw Mode,
        // Anim Speed, Theme, Color-blind, Sync Now, plus at least one
        // card-back and one background swatch — well over the floor.
        let tipped_count = app
            .world_mut()
            .query_filtered::<&SettingsButton, With<Tooltip>>()
            .iter(app.world())
            .count();
        assert!(
            tipped_count >= 6,
            "expected the panel to spawn many tooltipped buttons; got {tipped_count}"
        );

        // Spot-check: the Sync Now button's tooltip text is the
        // canonical microcopy. We find it via the `SettingsButton`
        // discriminant — there is exactly one Sync Now entity per panel.
        let sync_tip = app
            .world_mut()
            .query::<(&SettingsButton, &Tooltip)>()
            .iter(app.world())
            .find_map(|(btn, tip)| matches!(btn, SettingsButton::SyncNow).then(|| tip.0.clone()))
            .expect("Sync Now button should spawn with a Tooltip");
        assert_eq!(
            sync_tip.as_ref(),
            "Push and pull stats now. Runs automatically on launch and exit.",
            "Sync Now tooltip must use the canonical microcopy"
        );
    }

    #[test]
    fn settings_picker_rows_get_focus_row_marker() {
        let mut app = headless_app_with_focus();

        app.world_mut().resource_mut::<SettingsScreen>().0 = true;
        app.update();
        app.update();
        app.update();

        // Two picker rows are spawned (card-back + background); each
        // must carry the FocusRow marker.
        let row_count = app
            .world_mut()
            .query_filtered::<Entity, With<FocusRow>>()
            .iter(app.world())
            .count();
        assert!(
            row_count >= 2,
            "expected at least two FocusRow containers (card-back + background); got {row_count}"
        );
    }

    /// Test 3 of the thumbnail-picker spec: when [`ThemeRegistry`] has
    /// at least one theme and the [`ThemeThumbnailCache`] holds a
    /// fully-populated [`ThemeThumbnailPair`] for that theme's id, the
    /// rendered chip carries a [`ThemeThumbnailMarker`]-tagged
    /// `ImageNode` for each preview slot.
    #[test]
    fn theme_picker_chip_includes_thumbnail_sprite_when_thumbnails_loaded() {
        use crate::theme::{ThemeEntry, ThemeRegistry, ThemeThumbnailCache, ThemeThumbnailPair};

        let mut app = headless_app_with_focus();
        // Prime an Assets<Image> resource so we can mint stable handles
        // for the synthetic thumbnail pair.
        app.init_resource::<Assets<Image>>();
        let (ace_handle, back_handle) = {
            let mut images = app.world_mut().resource_mut::<Assets<Image>>();
            let ace = images.add(Image::default());
            let back = images.add(Image::default());
            (ace, back)
        };
        // Inject one theme entry + a matching thumbnail pair.
        app.insert_resource(ThemeRegistry {
            entries: vec![ThemeEntry {
                id: "test_theme".into(),
                display_name: "Test Theme".into(),
                manifest_url: "themes://test_theme/theme.ron".into(),
                meta: crate::theme::ThemeMeta {
                    id: "test_theme".into(),
                    name: "Test Theme".into(),
                    author: "x".into(),
                    version: "x".into(),
                    card_aspect: (2, 3),
                },
            }],
        });
        let mut cache = ThemeThumbnailCache::default();
        cache.entries.insert(
            "test_theme".into(),
            ThemeThumbnailPair {
                ace: ace_handle.clone(),
                back: back_handle.clone(),
            },
        );
        app.insert_resource(cache);

        // Open the panel and let the spawn + child-flush systems run.
        app.world_mut().resource_mut::<SettingsScreen>().0 = true;
        app.update();
        app.update();
        app.update();

        // Find every ImageNode tagged with ThemeThumbnailMarker — the
        // theme picker chip for "test_theme" must contribute exactly
        // two of them (ace + back).
        let thumbnail_count = app
            .world_mut()
            .query_filtered::<&ImageNode, With<ThemeThumbnailMarker>>()
            .iter(app.world())
            .count();
        assert!(
            thumbnail_count >= 2,
            "expected at least one ace + back thumbnail (2 sprites); got {thumbnail_count}"
        );

        // Spot-check: at least one thumbnail's image handle matches one
        // of the ones we inserted into the cache. This guards against a
        // future refactor that accidentally clones the wrong handle.
        let any_matches = app
            .world_mut()
            .query_filtered::<&ImageNode, With<ThemeThumbnailMarker>>()
            .iter(app.world())
            .any(|node| node.image == ace_handle || node.image == back_handle);
        assert!(
            any_matches,
            "at least one rendered thumbnail must reuse the cached handle"
        );
    }

    // -----------------------------------------------------------------------
    // Window geometry persistence
    // -----------------------------------------------------------------------

    #[test]
    fn should_persist_geometry_respects_debounce_window() {
        // Within the debounce window: not yet.
        assert!(!should_persist_geometry(10.0, 9.7));
        assert!(!should_persist_geometry(
            10.0,
            10.0 - WINDOW_GEOMETRY_DEBOUNCE_SECS + 0.01
        ));
        // Exactly the debounce window: allowed (>= comparison).
        assert!(should_persist_geometry(
            10.0,
            10.0 - WINDOW_GEOMETRY_DEBOUNCE_SECS
        ));
        // Well past the debounce window: allowed.
        assert!(should_persist_geometry(20.0, 10.0));
    }

    #[test]
    fn merge_geometry_uses_existing_when_event_components_missing() {
        let existing = WindowGeometry { width: 1280, height: 800, x: 100, y: 50 };
        // Position-only event keeps existing size.
        let merged = merge_geometry(Some(existing), None, Some((200, 75))).unwrap();
        assert_eq!(merged.width, 1280);
        assert_eq!(merged.height, 800);
        assert_eq!(merged.x, 200);
        assert_eq!(merged.y, 75);
        // Size-only event keeps existing position.
        let merged = merge_geometry(Some(existing), Some((1024, 768)), None).unwrap();
        assert_eq!(merged.width, 1024);
        assert_eq!(merged.height, 768);
        assert_eq!(merged.x, 100);
        assert_eq!(merged.y, 50);
    }

    #[test]
    fn merge_geometry_returns_none_when_size_unknown() {
        // No existing geometry, no size in the event → can't fabricate one.
        assert!(merge_geometry(None, None, Some((10, 20))).is_none());
    }

    /// Drives `app.update()` past [`WINDOW_GEOMETRY_DEBOUNCE_SECS`] using
    /// `TimeUpdateStrategy::ManualDuration`. `Time<Virtual>` clamps each
    /// frame's delta to `max_delta` (default 250 ms), so we step in 150 ms
    /// slices and run enough ticks to comfortably exceed the debounce
    /// window after the first record tick has set `last_changed_secs`.
    fn advance_past_geometry_debounce(app: &mut App) {
        use bevy::time::TimeUpdateStrategy;
        use std::time::Duration;
        app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f32(
            0.15,
        )));
        // Tick 1 sets last_changed_secs from any pending events. Each
        // subsequent tick advances the clock by 150 ms; five ticks total
        // buys 0.75 s of elapsed time relative to the record tick — well
        // past the 0.5 s debounce window.
        for _ in 0..5 {
            app.update();
        }
    }

    fn fire_resize(app: &mut App, width: f32, height: f32) {
        app.world_mut().write_message(WindowResized {
            window: bevy::ecs::entity::Entity::PLACEHOLDER,
            width,
            height,
        });
    }

    fn fire_move(app: &mut App, x: i32, y: i32) {
        app.world_mut().write_message(WindowMoved {
            window: bevy::ecs::entity::Entity::PLACEHOLDER,
            position: IVec2::new(x, y),
        });
    }

    #[test]
    fn resize_event_then_quiet_persists_window_geometry() {
        let mut app = headless_app();
        // Sanity: geometry starts unset (default).
        assert!(
            app.world()
                .resource::<SettingsResource>()
                .0
                .window_geometry
                .is_none()
        );

        // Fire a resize, then go quiet for past the debounce.
        fire_resize(&mut app, 1500.0, 950.0);
        advance_past_geometry_debounce(&mut app);

        let geom = app
            .world()
            .resource::<SettingsResource>()
            .0
            .window_geometry
            .expect("geometry should be persisted after debounce");
        assert_eq!(geom.width, 1500);
        assert_eq!(geom.height, 950);
        // Position not yet observed → defaults to 0, 0 since there was
        // no existing geometry to fall back on.
        assert_eq!(geom.x, 0);
        assert_eq!(geom.y, 0);
    }

    #[test]
    fn move_event_after_resize_updates_position_only() {
        let mut app = headless_app();

        // First, establish a baseline geometry via a resize event.
        fire_resize(&mut app, 1280.0, 800.0);
        advance_past_geometry_debounce(&mut app);
        let baseline = app
            .world()
            .resource::<SettingsResource>()
            .0
            .window_geometry
            .unwrap();
        assert_eq!(baseline.width, 1280);

        // Now fire a move-only event — size must be preserved from the
        // existing geometry.
        fire_move(&mut app, 250, 175);
        advance_past_geometry_debounce(&mut app);

        let geom = app
            .world()
            .resource::<SettingsResource>()
            .0
            .window_geometry
            .unwrap();
        assert_eq!(geom.width, 1280, "size must be preserved across a move-only update");
        assert_eq!(geom.height, 800);
        assert_eq!(geom.x, 250);
        assert_eq!(geom.y, 175);
    }

    #[test]
    fn rapid_resize_storm_only_persists_final_size() {
        let mut app = headless_app();

        // Burst of resize events on a single frame — only the last one
        // should be the eventually-persisted size.
        fire_resize(&mut app, 900.0, 600.0);
        fire_resize(&mut app, 1100.0, 700.0);
        fire_resize(&mut app, 1400.0, 850.0);
        advance_past_geometry_debounce(&mut app);

        let geom = app
            .world()
            .resource::<SettingsResource>()
            .0
            .window_geometry
            .unwrap();
        assert_eq!((geom.width, geom.height), (1400, 850));
    }

    #[test]
    fn no_window_events_no_geometry_change() {
        let mut app = headless_app();
        // Just advance time — without any events, settings must stay clean.
        advance_past_geometry_debounce(&mut app);
        assert!(
            app.world()
                .resource::<SettingsResource>()
                .0
                .window_geometry
                .is_none()
        );
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
                ScrollPosition(Vec2::new(0.0, 10.0)),
            ))
            .id();
        // Scroll up by 5 lines → would subtract 250 px, but must clamp to 0.
        app.world_mut().write_message(MouseWheel {
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
            .0.y;
        assert_eq!(offset, 0.0, "scrolling past top must clamp to 0, got {offset}");
    }
}
