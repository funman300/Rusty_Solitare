//! Mode-launcher overlay shown when the player presses **M** or clicks the
//! Modes affordance.
//!
//! Replaces the prior "keyboard shortcut reference" Home modal with a
//! vertical stack of five mode cards — Classic, Daily Challenge, Zen,
//! Challenge, Time Attack. Clicking a card fires the same launch event
//! the corresponding hotkey does, then closes the overlay. The shortcut
//! reference now lives only in Help (`F1`), which is the canonical place
//! for that information.
//!
//! Level-gated modes (Zen, Challenge, Time Attack) are disabled below
//! `CHALLENGE_UNLOCK_LEVEL`; clicking a locked card fires an
//! [`InfoToastEvent`] explaining the gate but does not launch the mode
//! or close the overlay.

use bevy::input::ButtonInput;
use bevy::prelude::*;
use solitaire_core::game_state::DrawMode;
use solitaire_data::save_settings_to;

use crate::challenge_plugin::CHALLENGE_UNLOCK_LEVEL;
use crate::events::{
    InfoToastEvent, NewGameRequestEvent, StartChallengeRequestEvent,
    StartDailyChallengeRequestEvent, StartTimeAttackRequestEvent, StartZenRequestEvent,
    ToggleProfileRequestEvent,
};
use crate::font_plugin::FontResource;
use crate::progress_plugin::ProgressResource;
use crate::settings_plugin::{
    SettingsChangedEvent, SettingsResource, SettingsStoragePath,
};
use crate::stats_plugin::StatsResource;
use crate::ui_focus::{Disabled, FocusGroup, Focusable};
use crate::ui_modal::{
    spawn_modal, spawn_modal_actions, spawn_modal_button, spawn_modal_header, ButtonVariant,
    ScrimDismissible,
};
use crate::ui_theme::{
    ACCENT_PRIMARY, BG_ELEVATED, BG_ELEVATED_HI, BORDER_STRONG, BORDER_SUBTLE, RADIUS_MD,
    STATE_INFO, TEXT_DISABLED, TEXT_PRIMARY, TEXT_SECONDARY, TYPE_BODY, TYPE_BODY_LG,
    TYPE_CAPTION, VAL_SPACE_1, VAL_SPACE_2, VAL_SPACE_3, Z_MODAL_PANEL,
};

// ---------------------------------------------------------------------------
// Public marker components
// ---------------------------------------------------------------------------

/// Marker component on the Home overlay root entity (the modal scrim).
#[derive(Component, Debug)]
pub struct HomeScreen;

/// Marker on the bottom-row "Cancel" button that dismisses the Home modal
/// without launching a mode.
#[derive(Component, Debug)]
pub struct HomeCancelButton;

/// Marker on the player-stats chip strip at the top of the Home modal.
/// Clicking the strip opens the Profile overlay so the player can drill
/// into level / XP / cosmetics without first dismissing Home.
#[derive(Component, Debug)]
struct HomeProfileChip;

/// Marker on the "Draw 1" toggle button inside the Home modal's
/// draw-mode row. Clicking flips `Settings.draw_mode` to `DrawOne` and
/// fires `SettingsChangedEvent` so audio / UI dependents react.
#[derive(Component, Debug)]
struct HomeDrawOneButton;

/// Marker on the "Draw 3" toggle button inside the Home modal's
/// draw-mode row. Mirror of [`HomeDrawOneButton`] for `DrawThree`.
#[derive(Component, Debug)]
struct HomeDrawThreeButton;

// ---------------------------------------------------------------------------
// Private mode-card data shape
// ---------------------------------------------------------------------------

/// Which game mode a [`HomeModeCard`] represents.
///
/// Kept private — external consumers should write the corresponding
/// `Start*RequestEvent` (or [`NewGameRequestEvent`] for Classic) directly.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
enum HomeMode {
    Classic,
    Daily,
    Zen,
    Challenge,
    TimeAttack,
}

impl HomeMode {
    /// Display title shown on the card.
    fn title(self) -> &'static str {
        match self {
            HomeMode::Classic => "Classic",
            HomeMode::Daily => "Daily Challenge",
            HomeMode::Zen => "Zen Mode",
            HomeMode::Challenge => "Challenge",
            HomeMode::TimeAttack => "Time Attack",
        }
    }

    /// One-line description shown below the title.
    fn description(self) -> &'static str {
        match self {
            HomeMode::Classic => "The standard Klondike deal — score, time, and a fresh shuffle.",
            HomeMode::Daily => "Today's seed, same for everyone. Build a streak.",
            HomeMode::Zen => "No timer, no score. Just the cards.",
            HomeMode::Challenge => "Hand-picked hard deals. No undo. Win to advance.",
            HomeMode::TimeAttack => "How many can you finish in ten minutes?",
        }
    }

    /// The keyboard accelerator that dispatches the same launch event,
    /// shown in a small chip on the card.
    fn hotkey(self) -> &'static str {
        match self {
            HomeMode::Classic => "N",
            HomeMode::Daily => "C",
            HomeMode::Zen => "Z",
            HomeMode::Challenge => "X",
            HomeMode::TimeAttack => "T",
        }
    }

    /// `true` when the mode is gated behind `CHALLENGE_UNLOCK_LEVEL`.
    fn requires_unlock(self) -> bool {
        matches!(self, HomeMode::Zen | HomeMode::Challenge | HomeMode::TimeAttack)
    }

    /// `true` if the player at `level` is allowed to launch the mode.
    fn is_unlocked(self, level: u32) -> bool {
        !self.requires_unlock() || level >= CHALLENGE_UNLOCK_LEVEL
    }
}

/// Marker component placed on each mode-card `Button` so the click
/// handler can identify which mode was pressed.
#[derive(Component, Debug)]
struct HomeModeCard(HomeMode);

/// Tracks whether the launch-time Home modal has already been auto-shown
/// for this app session. Flipped to `true` by [`spawn_home_on_launch`]
/// the first time it spawns the modal, so the auto-show is one-shot per
/// process — subsequent dismissals (Cancel / mode pick) don't trigger
/// a respawn, but the player can still re-open the picker with `M`.
///
/// Other plugins (e.g. `game_plugin`'s restore-prompt handler) can flip
/// the flag manually to suppress the launch auto-show when the player
/// has already made a launch-time choice through a different surface.
#[derive(Resource, Debug, Default)]
pub struct LaunchHomeShown(pub bool);

// ---------------------------------------------------------------------------
// Plugin
// ---------------------------------------------------------------------------

/// Registers the M-key toggle, the mode-card click handler, and the
/// Cancel-button handler.
///
/// `auto_show_on_launch` (default true) controls whether the picker
/// auto-spawns once the splash clears at app start. Headless tests use
/// [`HomePlugin::headless`] to opt out so each test starts with no
/// modal in the world.
pub struct HomePlugin {
    auto_show_on_launch: bool,
}

impl Default for HomePlugin {
    fn default() -> Self {
        Self {
            auto_show_on_launch: true,
        }
    }
}

impl HomePlugin {
    /// Test-only constructor that disables the launch-time auto-show.
    /// `MinimalPlugins` test setups don't include a splash, so the
    /// gating system would otherwise fire on the first tick and
    /// pre-spawn the modal that every test asserts is absent.
    pub fn headless() -> Self {
        Self {
            auto_show_on_launch: false,
        }
    }
}

impl Plugin for HomePlugin {
    fn build(&self, app: &mut App) {
        // Pre-mark the auto-show as already done in headless mode so the
        // gating system is a permanent no-op for tests.
        app.insert_resource(LaunchHomeShown(!self.auto_show_on_launch))
            .add_message::<NewGameRequestEvent>()
            .add_message::<StartZenRequestEvent>()
            .add_message::<StartChallengeRequestEvent>()
            .add_message::<StartTimeAttackRequestEvent>()
            .add_message::<StartDailyChallengeRequestEvent>()
            .add_message::<InfoToastEvent>()
            .add_message::<ToggleProfileRequestEvent>()
            .add_message::<SettingsChangedEvent>()
            // `.chain()` because several systems (M-toggle, card click,
            // cancel button, digit-key shortcut) all read the
            // `HomeScreen` entity and may queue a despawn on it in the
            // same tick. Bevy's parallel scheduler would otherwise let
            // two of them run simultaneously and double-despawn the
            // entity, panicking when the second command buffer is
            // applied. Chaining serialises these systems and keeps the
            // despawn deterministic.
            .add_systems(
                Update,
                (
                    spawn_home_on_launch,
                    toggle_home_screen,
                    attach_focusable_to_home_mode_cards,
                    handle_home_card_click,
                    handle_home_cancel_button,
                    handle_home_profile_chip,
                    handle_home_draw_mode_buttons,
                    handle_home_digit_keys,
                )
                    .chain(),
            );
    }
}

// ---------------------------------------------------------------------------
// Auto-show on launch
// ---------------------------------------------------------------------------

/// Auto-spawns the Home / mode-picker modal once per app session, so
/// the player lands on a deliberate "what mode do I want to play"
/// screen instead of the default Classic deal.
///
/// Gated on the launch-time UI being clear:
///
/// * `SplashRoot` must be gone — the splash owns the foreground during
///   the brand beat and the home modal appearing under it would feel
///   like a flash of half-rendered UI.
/// * `RestorePromptScreen` must not be open and `PendingRestoredGame`
///   must be empty — when the player has a saved in-progress game the
///   restore prompt takes precedence; the home picker would compete
///   with it for attention.
/// * `HomeScreen` must not already exist (defensive — e.g. the player
///   pressed `M` between ticks).
/// * `LaunchHomeShown` flips to `true` after the first spawn so this
///   system becomes a no-op for the rest of the session. Cancelling
///   the modal therefore goes to the underlying default deal rather
///   than respawning the picker.
#[allow(clippy::too_many_arguments)]
fn spawn_home_on_launch(
    mut commands: Commands,
    mut shown: ResMut<LaunchHomeShown>,
    splash: Query<(), With<crate::splash_plugin::SplashRoot>>,
    restore_prompts: Query<(), With<crate::game_plugin::RestorePromptScreen>>,
    pending_restore: Option<Res<crate::game_plugin::PendingRestoredGame>>,
    existing: Query<(), With<HomeScreen>>,
    progress: Option<Res<ProgressResource>>,
    stats: Option<Res<StatsResource>>,
    settings: Option<Res<SettingsResource>>,
    font_res: Option<Res<FontResource>>,
) {
    if shown.0
        || !splash.is_empty()
        || !restore_prompts.is_empty()
        || pending_restore.as_ref().is_some_and(|p| p.0.is_some())
        || !existing.is_empty()
    {
        return;
    }

    spawn_home_screen(
        &mut commands,
        build_home_context(
            progress.as_deref(),
            stats.as_deref(),
            settings.as_deref(),
            font_res.as_deref(),
        ),
    );
    shown.0 = true;
}

// ---------------------------------------------------------------------------
// M-key toggle
// ---------------------------------------------------------------------------

fn toggle_home_screen(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    progress: Option<Res<ProgressResource>>,
    stats: Option<Res<StatsResource>>,
    settings: Option<Res<SettingsResource>>,
    font_res: Option<Res<FontResource>>,
    screens: Query<Entity, With<HomeScreen>>,
) {
    if !keys.just_pressed(KeyCode::KeyM) {
        return;
    }
    if let Ok(entity) = screens.single() {
        commands.entity(entity).despawn();
    } else {
        spawn_home_screen(
            &mut commands,
            build_home_context(
                progress.as_deref(),
                stats.as_deref(),
                settings.as_deref(),
                font_res.as_deref(),
            ),
        );
    }
}

/// Builds a [`HomeContext`] from the live resources the Home modal
/// reads. Falls back to safe defaults when a resource is missing
/// (typical for `MinimalPlugins` headless tests that don't install
/// every contributor plugin).
fn build_home_context<'a>(
    progress: Option<&ProgressResource>,
    stats: Option<&StatsResource>,
    settings: Option<&SettingsResource>,
    font_res: Option<&'a FontResource>,
) -> HomeContext<'a> {
    HomeContext {
        level: progress.map_or(0, |p| p.0.level),
        total_xp: progress.map_or(0, |p| p.0.total_xp),
        daily_streak: progress.map_or(0, |p| p.0.daily_challenge_streak),
        lifetime_score: stats.map_or(0, |s| s.0.lifetime_score),
        classic_best: stats.map_or(0, |s| s.0.classic_best_score),
        zen_best: stats.map_or(0, |s| s.0.zen_best_score),
        challenge_best: stats.map_or(0, |s| s.0.challenge_best_score),
        draw_mode: settings
            .map(|s| s.0.draw_mode.clone())
            .unwrap_or(DrawMode::DrawOne),
        font_res,
    }
}

// ---------------------------------------------------------------------------
// Card click handler
// ---------------------------------------------------------------------------

/// Dispatches a click on a mode card.
///
/// - **Unlocked** modes fire the matching `Start*RequestEvent` (or
///   [`NewGameRequestEvent`] for Classic) and despawn the modal.
/// - **Locked** modes (level below [`CHALLENGE_UNLOCK_LEVEL`]) fire only
///   an [`InfoToastEvent`] and leave the modal open so the player can
///   pick another mode.
#[allow(clippy::too_many_arguments)]
fn handle_home_card_click(
    mut commands: Commands,
    cards: Query<(&Interaction, &HomeModeCard), Changed<Interaction>>,
    progress: Option<Res<ProgressResource>>,
    screens: Query<Entity, With<HomeScreen>>,
    mut new_game: MessageWriter<NewGameRequestEvent>,
    mut zen: MessageWriter<StartZenRequestEvent>,
    mut challenge: MessageWriter<StartChallengeRequestEvent>,
    mut time_attack: MessageWriter<StartTimeAttackRequestEvent>,
    mut daily: MessageWriter<StartDailyChallengeRequestEvent>,
    mut info_toast: MessageWriter<InfoToastEvent>,
) {
    let level = progress.as_ref().map_or(0, |p| p.0.level);

    for (interaction, card) in &cards {
        if *interaction != Interaction::Pressed {
            continue;
        }

        if !card.0.is_unlocked(level) {
            info_toast.write(InfoToastEvent(format!(
                "{} unlocks at level {CHALLENGE_UNLOCK_LEVEL}",
                card.0.title()
            )));
            // Leave the modal open so the player can pick another mode.
            continue;
        }

        match card.0 {
            HomeMode::Classic => {
                new_game.write(NewGameRequestEvent::default());
            }
            HomeMode::Daily => {
                daily.write(StartDailyChallengeRequestEvent);
            }
            HomeMode::Zen => {
                zen.write(StartZenRequestEvent);
            }
            HomeMode::Challenge => {
                challenge.write(StartChallengeRequestEvent);
            }
            HomeMode::TimeAttack => {
                time_attack.write(StartTimeAttackRequestEvent);
            }
        }

        // Close the modal after dispatching the launch event.
        for entity in &screens {
            commands.entity(entity).despawn();
        }
    }
}

// ---------------------------------------------------------------------------
// Cancel button handler
// ---------------------------------------------------------------------------

fn handle_home_cancel_button(
    mut commands: Commands,
    keys: Option<Res<ButtonInput<KeyCode>>>,
    cancel_buttons: Query<&Interaction, (With<HomeCancelButton>, Changed<Interaction>)>,
    screens: Query<Entity, With<HomeScreen>>,
) {
    if screens.is_empty() {
        return;
    }
    let click = cancel_buttons.iter().any(|i| *i == Interaction::Pressed);
    let esc = keys.is_some_and(|k| k.just_pressed(KeyCode::Escape));
    if !click && !esc {
        return;
    }
    for entity in &screens {
        commands.entity(entity).despawn();
    }
}

// ---------------------------------------------------------------------------
// Header chip + draw-mode button handlers
// ---------------------------------------------------------------------------

/// Click on the player-stats header chip → fire
/// [`ToggleProfileRequestEvent`] so the Profile overlay opens on top
/// of Home. Closing Profile (`P` / `Esc`) returns the player to the
/// Home picker without losing their context.
fn handle_home_profile_chip(
    chips: Query<&Interaction, (With<HomeProfileChip>, Changed<Interaction>)>,
    mut profile: MessageWriter<ToggleProfileRequestEvent>,
) {
    if chips.iter().any(|i| *i == Interaction::Pressed) {
        profile.write(ToggleProfileRequestEvent);
    }
}

/// Click on a draw-mode chip — flip `Settings.draw_mode`, persist,
/// fire `SettingsChangedEvent`, and respawn the Home modal so the
/// active-chip styling reflects the new state. Repaint by full
/// rebuild keeps the helper code small (no per-entity colour
/// surgery) and the modal is light enough to respawn cleanly.
#[allow(clippy::too_many_arguments)]
fn handle_home_draw_mode_buttons(
    mut commands: Commands,
    one_buttons: Query<&Interaction, (With<HomeDrawOneButton>, Changed<Interaction>)>,
    three_buttons: Query<&Interaction, (With<HomeDrawThreeButton>, Changed<Interaction>)>,
    screens: Query<Entity, With<HomeScreen>>,
    mut settings: Option<ResMut<SettingsResource>>,
    storage_path: Option<Res<SettingsStoragePath>>,
    mut changed: MessageWriter<SettingsChangedEvent>,
    progress: Option<Res<ProgressResource>>,
    stats: Option<Res<StatsResource>>,
    font_res: Option<Res<FontResource>>,
) {
    if screens.is_empty() {
        return;
    }
    let want_one = one_buttons.iter().any(|i| *i == Interaction::Pressed);
    let want_three = three_buttons.iter().any(|i| *i == Interaction::Pressed);
    if !want_one && !want_three {
        return;
    }
    let Some(settings) = settings.as_mut() else {
        return;
    };
    let target = if want_one {
        DrawMode::DrawOne
    } else {
        DrawMode::DrawThree
    };
    if settings.0.draw_mode == target {
        return; // already in this mode — avoid a redundant respawn.
    }
    settings.0.draw_mode = target;
    if let Some(p) = storage_path
        && let Some(path) = p.0.as_deref()
        && let Err(e) = save_settings_to(path, &settings.0)
    {
        warn!("home: failed to persist draw-mode change: {e}");
    }
    changed.write(SettingsChangedEvent(settings.0.clone()));

    // Repaint by despawn + respawn so the chip styling and any
    // dependent labels (none today, but Phase B may surface a
    // "Standard (Draw 1)" caption like MSSC) reflect the new state.
    for entity in &screens {
        commands.entity(entity).despawn();
    }
    spawn_home_screen(
        &mut commands,
        build_home_context(
            progress.as_deref(),
            stats.as_deref(),
            Some(settings),
            font_res.as_deref(),
        ),
    );
}

// ---------------------------------------------------------------------------
// Digit-key shortcuts (1-5) — modal-scoped
// ---------------------------------------------------------------------------

/// Maps a [`KeyCode::Digit1`]..[`KeyCode::Digit5`] press to the matching
/// [`HomeMode`]. Returns `None` for any other key. Kept as a small free
/// function so the keyboard handler reads as a clean dispatch table and so
/// the mapping is easy to unit-test in isolation.
fn digit_to_home_mode(key: KeyCode) -> Option<HomeMode> {
    match key {
        KeyCode::Digit1 => Some(HomeMode::Classic),
        KeyCode::Digit2 => Some(HomeMode::Daily),
        KeyCode::Digit3 => Some(HomeMode::Zen),
        KeyCode::Digit4 => Some(HomeMode::Challenge),
        KeyCode::Digit5 => Some(HomeMode::TimeAttack),
        _ => None,
    }
}

/// Direct keyboard activation of a specific mode while the Mode Launcher
/// modal is open. Mirrors the click-handler dispatch in
/// [`handle_home_card_click`]: pressing `1` launches Classic, `2` launches
/// the Daily Challenge, and `3`/`4`/`5` launch Zen / Challenge / Time
/// Attack respectively when the player has reached
/// [`CHALLENGE_UNLOCK_LEVEL`].
///
/// The shortcut is **modal-scoped** — when no [`HomeScreen`] exists the
/// system returns immediately, so digit keys can never accidentally launch
/// a mode mid-game. Pressing a digit for a locked mode is a no-op (matches
/// the click-on-locked-card behaviour) and leaves the modal open so the
/// player can pick another mode.
#[allow(clippy::too_many_arguments)]
fn handle_home_digit_keys(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    progress: Option<Res<ProgressResource>>,
    screens: Query<Entity, With<HomeScreen>>,
    mut new_game: MessageWriter<NewGameRequestEvent>,
    mut zen: MessageWriter<StartZenRequestEvent>,
    mut challenge: MessageWriter<StartChallengeRequestEvent>,
    mut time_attack: MessageWriter<StartTimeAttackRequestEvent>,
    mut daily: MessageWriter<StartDailyChallengeRequestEvent>,
) {
    // Modal-scoped: do nothing when the Mode Launcher isn't open.
    if screens.is_empty() {
        return;
    }

    let Some(mode) = [
        KeyCode::Digit1,
        KeyCode::Digit2,
        KeyCode::Digit3,
        KeyCode::Digit4,
        KeyCode::Digit5,
    ]
    .into_iter()
    .find(|k| keys.just_pressed(*k))
    .and_then(digit_to_home_mode) else {
        return;
    };

    let level = progress.as_ref().map_or(0, |p| p.0.level);
    if !mode.is_unlocked(level) {
        // Locked mode: no-op, modal stays open.
        return;
    }

    match mode {
        HomeMode::Classic => {
            new_game.write(NewGameRequestEvent::default());
        }
        HomeMode::Daily => {
            daily.write(StartDailyChallengeRequestEvent);
        }
        HomeMode::Zen => {
            zen.write(StartZenRequestEvent);
        }
        HomeMode::Challenge => {
            challenge.write(StartChallengeRequestEvent);
        }
        HomeMode::TimeAttack => {
            time_attack.write(StartTimeAttackRequestEvent);
        }
    }

    // Close the modal after dispatching the launch event — same shape as
    // the click handler.
    for entity in &screens {
        commands.entity(entity).despawn();
    }
}

// ---------------------------------------------------------------------------
// Spawn helpers
// ---------------------------------------------------------------------------

/// Bundles the data the Home modal needs to render the new
/// MSSC-inspired header chips, per-mode score chips, and draw-mode
/// row. Built fresh by the two call sites (`spawn_home_on_launch`
/// and `toggle_home_screen`) from the live progress / stats /
/// settings resources, with sensible defaults when a resource is
/// missing under `MinimalPlugins` headless tests.
struct HomeContext<'a> {
    level: u32,
    total_xp: u64,
    lifetime_score: u64,
    classic_best: u32,
    zen_best: u32,
    challenge_best: u32,
    daily_streak: u32,
    draw_mode: DrawMode,
    font_res: Option<&'a FontResource>,
}

/// Spawns the Home modal with the player-stats header strip, draw-mode
/// row, five mode cards, and a Cancel button.
fn spawn_home_screen(commands: &mut Commands, ctx: HomeContext<'_>) {
    let HomeContext { font_res, .. } = ctx;
    let scrim = spawn_modal(commands, HomeScreen, Z_MODAL_PANEL, |card| {
        spawn_modal_header(card, "Choose a Mode", font_res);

        spawn_home_header_chips(card, &ctx);
        spawn_draw_mode_row(card, &ctx);

        for mode in [
            HomeMode::Classic,
            HomeMode::Daily,
            HomeMode::Zen,
            HomeMode::Challenge,
            HomeMode::TimeAttack,
        ] {
            spawn_mode_card(card, mode, &ctx);
        }

        spawn_modal_actions(card, |actions| {
            spawn_modal_button(
                actions,
                HomeCancelButton,
                "Cancel",
                Some("M"),
                ButtonVariant::Tertiary,
                font_res,
            );
        });
    });
    // Home is read-only — opt into click-outside-to-dismiss.
    commands.entity(scrim).insert(ScrimDismissible);
}

/// Player-stats chip strip — Level, XP, Lifetime Score. Clickable as a
/// whole to open the Profile overlay (mirrors the MSSC top-right
/// avatar+rewards corner that surfaces level + premium status). Falls
/// back to plain Text in headless contexts where `Button` interaction
/// isn't driven by the input pipeline anyway.
fn spawn_home_header_chips(parent: &mut ChildSpawnerCommands, ctx: &HomeContext<'_>) {
    let font_handle = ctx.font_res.map(|f| f.0.clone()).unwrap_or_default();
    let font_label = TextFont {
        font: font_handle.clone(),
        font_size: TYPE_CAPTION,
        ..default()
    };
    let font_value = TextFont {
        font: font_handle,
        font_size: TYPE_BODY,
        ..default()
    };

    parent
        .spawn((
            HomeProfileChip,
            Button,
            Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::SpaceBetween,
                column_gap: VAL_SPACE_2,
                padding: UiRect::axes(VAL_SPACE_3, VAL_SPACE_2),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(RADIUS_MD)),
                width: Val::Percent(100.0),
                ..default()
            },
            BackgroundColor(BG_ELEVATED),
            BorderColor::all(BORDER_SUBTLE),
        ))
        .with_children(|row| {
            for (label, value) in [
                ("Level".to_string(), format_compact(ctx.level as u64)),
                ("XP".to_string(), format_compact(ctx.total_xp)),
                ("Score".to_string(), format_compact(ctx.lifetime_score)),
            ] {
                row.spawn(Node {
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    row_gap: VAL_SPACE_1,
                    ..default()
                })
                .with_children(|col| {
                    col.spawn((
                        Text::new(label),
                        font_label.clone(),
                        TextColor(TEXT_SECONDARY),
                    ));
                    col.spawn((
                        Text::new(value),
                        font_value.clone(),
                        TextColor(ACCENT_PRIMARY),
                    ));
                });
            }
        });
}

/// Draw-mode row — "Draw 1" / "Draw 3" toggle. Affects the next Classic
/// deal (the Settings value the new-game flow reads). Surfacing it on
/// the Home modal keeps the per-game choice one tap away rather than
/// buried in Settings, mirroring the dropdown MSSC puts on its
/// difficulty picker.
fn spawn_draw_mode_row(parent: &mut ChildSpawnerCommands, ctx: &HomeContext<'_>) {
    let font_handle = ctx.font_res.map(|f| f.0.clone()).unwrap_or_default();
    let font_label = TextFont {
        font: font_handle.clone(),
        font_size: TYPE_CAPTION,
        ..default()
    };
    let font_btn = TextFont {
        font: font_handle,
        font_size: TYPE_BODY,
        ..default()
    };

    let active_one = matches!(ctx.draw_mode, DrawMode::DrawOne);

    parent
        .spawn(Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: VAL_SPACE_3,
            ..default()
        })
        .with_children(|row| {
            row.spawn((
                Text::new("Draw mode"),
                font_label.clone(),
                TextColor(TEXT_SECONDARY),
            ));
            spawn_draw_mode_chip::<HomeDrawOneButton>(
                row,
                HomeDrawOneButton,
                "Draw 1",
                active_one,
                &font_btn,
            );
            spawn_draw_mode_chip::<HomeDrawThreeButton>(
                row,
                HomeDrawThreeButton,
                "Draw 3",
                !active_one,
                &font_btn,
            );
        });
}

fn spawn_draw_mode_chip<M: Component>(
    parent: &mut ChildSpawnerCommands,
    marker: M,
    label: &str,
    active: bool,
    font: &TextFont,
) {
    let (bg, fg) = if active {
        (ACCENT_PRIMARY, BG_ELEVATED)
    } else {
        (BG_ELEVATED_HI, TEXT_PRIMARY)
    };
    parent
        .spawn((
            marker,
            Button,
            Node {
                padding: UiRect::axes(VAL_SPACE_3, VAL_SPACE_1),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(RADIUS_MD)),
                ..default()
            },
            BackgroundColor(bg),
            BorderColor::all(BORDER_SUBTLE),
        ))
        .with_children(|c| {
            c.spawn((Text::new(label.to_string()), font.clone(), TextColor(fg)));
        });
}

/// Compact decimal formatter: `1234567` → `"1.2M"`, `12345` → `"12.3K"`,
/// otherwise the raw number with thousands separators. Keeps chip text
/// short enough to fit a 3-up header strip without wrapping.
fn format_compact(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 10_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else if n >= 1_000 {
        let (high, low) = (n / 1_000, n % 1_000);
        format!("{high},{low:03}")
    } else {
        n.to_string()
    }
}

/// Per-mode score / streak chip text. `None` for modes where no
/// per-mode best exists yet (Time Attack uses session scoring; modes
/// with `0` recorded mean "no win yet" and we hide the chip rather
/// than show a 0).
fn score_chip_text_for(mode: HomeMode, ctx: &HomeContext<'_>) -> Option<String> {
    match mode {
        HomeMode::Classic if ctx.classic_best > 0 => {
            Some(format!("Best {}", format_compact(ctx.classic_best as u64)))
        }
        HomeMode::Zen if ctx.zen_best > 0 => {
            Some(format!("Best {}", format_compact(ctx.zen_best as u64)))
        }
        HomeMode::Challenge if ctx.challenge_best > 0 => {
            Some(format!("Best {}", format_compact(ctx.challenge_best as u64)))
        }
        HomeMode::Daily if ctx.daily_streak > 0 => {
            Some(format!("Streak {}", ctx.daily_streak))
        }
        _ => None,
    }
}

/// Tab-walk order for each mode card, matching the visual top-to-bottom
/// stack inside the Home modal. Lower numbers receive focus first under
/// `Focusable`'s sort.
fn home_mode_focus_order(mode: HomeMode) -> i32 {
    match mode {
        HomeMode::Classic => 0,
        HomeMode::Daily => 1,
        HomeMode::Zen => 2,
        HomeMode::Challenge => 3,
        HomeMode::TimeAttack => 4,
    }
}

/// Auto-attaches [`Focusable`] (and [`Disabled`] when locked) to every
/// newly-spawned [`HomeModeCard`]. Walks ancestors to find the
/// [`crate::ui_modal::ModalScrim`] so each card's focus group is bound
/// to its parent modal — mirrors the convention that
/// `attach_focusable_to_modal_buttons` uses for `ModalButton`s.
///
/// Doing this in a system (instead of inline at spawn time) lets
/// `spawn_home_screen` keep using the existing `spawn_modal`'s
/// build-closure shape; the scrim entity isn't visible inside that
/// closure, only after the call returns. The system runs every frame
/// and is a no-op once every card has been tagged.
fn attach_focusable_to_home_mode_cards(
    mut commands: Commands,
    new_cards: Query<(Entity, &HomeModeCard), Without<Focusable>>,
    parents: Query<&ChildOf>,
    scrims: Query<(), With<crate::ui_modal::ModalScrim>>,
    progress: Option<Res<ProgressResource>>,
) {
    let level = progress.as_ref().map_or(0, |p| p.0.level);
    for (card_entity, card) in &new_cards {
        // Walk ancestors until we find the ModalScrim. Bounded loop so a
        // malformed hierarchy can't hang the system — same defensive
        // shape as `attach_focusable_to_modal_buttons`.
        let mut current = card_entity;
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
        let Some(scrim) = scrim_entity else { continue };
        commands.entity(card_entity).insert(Focusable {
            group: FocusGroup::Modal(scrim),
            order: home_mode_focus_order(card.0),
        });
        if !card.0.is_unlocked(level) {
            commands.entity(card_entity).insert(Disabled);
        }
    }
}

/// Spawns one mode card — a `Button` whose children are a title row, a
/// description line, and (when locked) a "Reach level N" hint.
///
/// The visual deliberately diverges from `spawn_modal_button` because a
/// mode card is a wide, two-line tile rather than a compact action; the
/// `ButtonVariant` palette would not apply cleanly here. Hover/press
/// feedback is supplied by `paint_modal_buttons` via the `ModalButton`
/// component, which we attach with `ButtonVariant::Secondary` so the card
/// reads as a standard interactive surface.
fn spawn_mode_card(
    parent: &mut ChildSpawnerCommands,
    mode: HomeMode,
    ctx: &HomeContext<'_>,
) {
    let level = ctx.level;
    let font_res = ctx.font_res;
    let score_chip = score_chip_text_for(mode, ctx);
    let unlocked = mode.is_unlocked(level);
    let font_handle = font_res.map(|f| f.0.clone()).unwrap_or_default();
    let font_title = TextFont {
        font: font_handle.clone(),
        font_size: TYPE_BODY_LG,
        ..default()
    };
    let font_desc = TextFont {
        font: font_handle.clone(),
        font_size: TYPE_BODY,
        ..default()
    };
    let font_chip = TextFont {
        font: font_handle,
        font_size: TYPE_CAPTION,
        ..default()
    };

    // Locked cards mute their text to communicate the disabled state at
    // a glance; the explicit "Unlocks at level N" caption underneath
    // backs that up with copy.
    let title_color = if unlocked { TEXT_PRIMARY } else { TEXT_DISABLED };
    let desc_color = if unlocked { TEXT_SECONDARY } else { TEXT_DISABLED };
    let border_color = if unlocked { BORDER_SUBTLE } else { BORDER_STRONG };

    parent
        .spawn((
            HomeModeCard(mode),
            // Keep this a real Button entity so clicks resolve through
            // bevy::ui — the click handler queries on `&Interaction`
            // which Button drives.
            Button,
            Node {
                flex_direction: FlexDirection::Column,
                row_gap: VAL_SPACE_1,
                padding: UiRect::all(VAL_SPACE_3),
                width: Val::Percent(100.0),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(RADIUS_MD)),
                ..default()
            },
            BackgroundColor(BG_ELEVATED_HI),
            BorderColor::all(border_color),
        ))
        .with_children(|c| {
            // Title row — title text on the left, hotkey chip on the right.
            c.spawn(Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::SpaceBetween,
                column_gap: VAL_SPACE_3,
                ..default()
            })
            .with_children(|row| {
                row.spawn((
                    Text::new(mode.title().to_string()),
                    font_title.clone(),
                    TextColor(title_color),
                ));

                if unlocked {
                    // Hotkey chip — same look as the kbd-chip rows used
                    // elsewhere so accelerators read consistently.
                    row.spawn((
                        Node {
                            padding: UiRect::axes(VAL_SPACE_2, VAL_SPACE_1),
                            min_width: Val::Px(32.0),
                            justify_content: JustifyContent::Center,
                            border: UiRect::all(Val::Px(1.0)),
                            border_radius: BorderRadius::all(Val::Px(RADIUS_MD)),
                            ..default()
                        },
                        BorderColor::all(BORDER_SUBTLE),
                    ))
                    .with_children(|chip| {
                        chip.spawn((
                            Text::new(mode.hotkey().to_string()),
                            font_chip.clone(),
                            TextColor(TEXT_SECONDARY),
                        ));
                    });
                } else {
                    // Lock icon stand-in — text glyph keeps the layout
                    // dependency-free (no asset loader required) and
                    // reads at every supported font size.
                    row.spawn((
                        Text::new("LOCKED".to_string()),
                        font_chip.clone(),
                        TextColor(STATE_INFO),
                    ));
                }
            });

            // Description line.
            c.spawn((
                Text::new(mode.description().to_string()),
                font_desc.clone(),
                TextColor(desc_color),
            ));

            // Per-mode score / streak chip — populated only when the
            // player has data for this mode. Hidden on a 0 best so a
            // fresh profile doesn't show "Best 0" everywhere.
            if let Some(text) = score_chip.clone()
                && unlocked
            {
                c.spawn((
                    Text::new(text),
                    font_chip.clone(),
                    TextColor(ACCENT_PRIMARY),
                    Node {
                        margin: UiRect::top(VAL_SPACE_1),
                        ..default()
                    },
                ));
            }

            // Locked footnote — explicit copy so the gate is unambiguous.
            if !unlocked {
                c.spawn((
                    Text::new(format!(
                        "Unlocks at level {CHALLENGE_UNLOCK_LEVEL}"
                    )),
                    TextFont {
                        font: font_desc.font.clone(),
                        font_size: TYPE_CAPTION,
                        ..default()
                    },
                    TextColor(ACCENT_PRIMARY),
                    Node {
                        margin: UiRect::top(VAL_SPACE_1),
                        ..default()
                    },
                ));
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game_plugin::GamePlugin;
    use crate::progress_plugin::ProgressPlugin;
    use crate::table_plugin::TablePlugin;
    use bevy::ecs::message::Messages;

    /// Builds a headless `App` with just the plugins Home actually
    /// reaches into. We deliberately skip input_plugin /
    /// challenge_plugin / time_attack_plugin / daily_challenge_plugin —
    /// Home only needs to dispatch their request events; the events
    /// themselves are registered defensively by `HomePlugin::build`.
    fn headless_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(GamePlugin)
            .add_plugins(TablePlugin)
            .add_plugins(ProgressPlugin::headless())
            .add_plugins(HomePlugin::headless());
        app.init_resource::<ButtonInput<KeyCode>>();
        app.update();
        app
    }

    /// Press M, run a tick, and return the resulting screen entity.
    /// Panics if the modal does not appear (failure mode that any later
    /// assertion would mask anyway). The keyboard input is cleared after
    /// the press so the next `app.update()` doesn't re-toggle the modal
    /// closed — `MinimalPlugins` doesn't run the bevy_input update system
    /// that would normally clear `just_pressed` between frames.
    fn open_home(app: &mut App) -> Entity {
        {
            let mut input = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            input.press(KeyCode::KeyM);
        }
        app.update();
        {
            let mut input = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            input.release(KeyCode::KeyM);
            input.clear();
        }
        app.world_mut()
            .query::<(Entity, &HomeScreen)>()
            .single(app.world())
            .map(|(e, _)| e)
            .expect("HomeScreen must spawn after M press")
    }

    /// Pump a button-press synthetic interaction onto the entity. Bevy
    /// 0.18 surfaces interactions through the `Interaction` component
    /// driven by the UI input pipeline, but MinimalPlugins does not run
    /// that pipeline — so we insert `Interaction::Pressed` directly,
    /// which triggers `Changed<Interaction>` on the next update tick.
    /// Pattern is borrowed verbatim from `pause_plugin`'s tests.
    fn press_button(app: &mut App, entity: Entity) {
        app.world_mut()
            .entity_mut(entity)
            .insert(Interaction::Pressed);
        app.update();
    }

    /// Find the unique `HomeModeCard` entity for a specific mode. Used
    /// by the click-handler tests to target the right card.
    fn find_card(app: &mut App, mode: HomeMode) -> Entity {
        app.world_mut()
            .query::<(Entity, &HomeModeCard)>()
            .iter(app.world())
            .find(|(_, c)| c.0 == mode)
            .map(|(e, _)| e)
            .unwrap_or_else(|| panic!("no HomeModeCard for {mode:?}"))
    }

    #[test]
    fn pressing_m_spawns_home_screen() {
        let mut app = headless_app();
        assert_eq!(
            app.world_mut()
                .query::<&HomeScreen>()
                .iter(app.world())
                .count(),
            0
        );

        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyM);
        app.update();

        assert_eq!(
            app.world_mut()
                .query::<&HomeScreen>()
                .iter(app.world())
                .count(),
            1
        );
    }

    #[test]
    fn pressing_m_twice_closes_home_screen() {
        let mut app = headless_app();

        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyM);
        app.update();

        {
            let mut input = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            input.release(KeyCode::KeyM);
            input.clear();
            input.press(KeyCode::KeyM);
        }
        app.update();

        assert_eq!(
            app.world_mut()
                .query::<&HomeScreen>()
                .iter(app.world())
                .count(),
            0
        );
    }

    #[test]
    fn modal_contains_a_card_for_each_mode() {
        let mut app = headless_app();
        let _ = open_home(&mut app);

        let modes: Vec<HomeMode> = app
            .world_mut()
            .query::<&HomeModeCard>()
            .iter(app.world())
            .map(|c| c.0)
            .collect();

        for expected in [
            HomeMode::Classic,
            HomeMode::Daily,
            HomeMode::Zen,
            HomeMode::Challenge,
            HomeMode::TimeAttack,
        ] {
            assert!(
                modes.contains(&expected),
                "missing card for {expected:?}; found {modes:?}"
            );
        }
        assert_eq!(modes.len(), 5, "exactly five cards expected");
    }

    #[test]
    fn classic_click_fires_new_game_event_and_closes_modal() {
        let mut app = headless_app();
        let _ = open_home(&mut app);

        // Drain any pre-existing NewGameRequestEvent so the assertion
        // only sees the click-driven write.
        app.world_mut()
            .resource_mut::<Messages<NewGameRequestEvent>>()
            .clear();

        let card = find_card(&mut app, HomeMode::Classic);
        press_button(&mut app, card);

        let events = app.world().resource::<Messages<NewGameRequestEvent>>();
        let mut cursor = events.get_cursor();
        let fired: Vec<_> = cursor.read(events).copied().collect();
        assert_eq!(fired.len(), 1, "one NewGameRequestEvent must fire");

        assert_eq!(
            app.world_mut()
                .query::<&HomeScreen>()
                .iter(app.world())
                .count(),
            0,
            "Home modal must close after launching Classic"
        );
    }

    #[test]
    fn locked_zen_click_is_a_noop_below_unlock_level() {
        let mut app = headless_app();
        // Default level is 0 — Zen is locked.
        let _ = open_home(&mut app);

        // Reset event queues so the assertion is clean.
        app.world_mut()
            .resource_mut::<Messages<NewGameRequestEvent>>()
            .clear();
        app.world_mut()
            .resource_mut::<Messages<StartZenRequestEvent>>()
            .clear();

        let card = find_card(&mut app, HomeMode::Zen);
        press_button(&mut app, card);

        // No launch events should have fired.
        let new_game = app.world().resource::<Messages<NewGameRequestEvent>>();
        let mut nc = new_game.get_cursor();
        assert!(
            nc.read(new_game).next().is_none(),
            "locked Zen click must not fire NewGameRequestEvent"
        );
        let zen = app.world().resource::<Messages<StartZenRequestEvent>>();
        let mut zc = zen.get_cursor();
        assert!(
            zc.read(zen).next().is_none(),
            "locked Zen click must not fire StartZenRequestEvent"
        );

        // Modal must still be open so the player can pick another mode.
        assert_eq!(
            app.world_mut()
                .query::<&HomeScreen>()
                .iter(app.world())
                .count(),
            1,
            "Home modal must remain open after a locked-mode click"
        );
    }

    #[test]
    fn unlocked_zen_click_fires_start_zen_event_and_closes_modal() {
        let mut app = headless_app();
        // Bump the player to the unlock level.
        app.world_mut()
            .resource_mut::<ProgressResource>()
            .0
            .level = CHALLENGE_UNLOCK_LEVEL;
        let _ = open_home(&mut app);

        app.world_mut()
            .resource_mut::<Messages<StartZenRequestEvent>>()
            .clear();

        let card = find_card(&mut app, HomeMode::Zen);
        press_button(&mut app, card);

        let zen = app.world().resource::<Messages<StartZenRequestEvent>>();
        let mut zc = zen.get_cursor();
        assert_eq!(
            zc.read(zen).count(),
            1,
            "unlocked Zen click must fire exactly one StartZenRequestEvent"
        );

        assert_eq!(
            app.world_mut()
                .query::<&HomeScreen>()
                .iter(app.world())
                .count(),
            0,
            "Home modal must close after launching Zen"
        );
    }

    #[test]
    fn cancel_button_closes_modal_without_launching_anything() {
        let mut app = headless_app();
        let _ = open_home(&mut app);

        app.world_mut()
            .resource_mut::<Messages<NewGameRequestEvent>>()
            .clear();

        let cancel = app
            .world_mut()
            .query::<(Entity, &HomeCancelButton)>()
            .single(app.world())
            .map(|(e, _)| e)
            .expect("HomeCancelButton must exist when modal is open");
        press_button(&mut app, cancel);

        assert_eq!(
            app.world_mut()
                .query::<&HomeScreen>()
                .iter(app.world())
                .count(),
            0,
            "Cancel must despawn the modal"
        );

        let new_game = app.world().resource::<Messages<NewGameRequestEvent>>();
        let mut nc = new_game.get_cursor();
        assert!(
            nc.read(new_game).next().is_none(),
            "Cancel must not fire NewGameRequestEvent"
        );
    }

    // -----------------------------------------------------------------------
    // Phase 2: keyboard focus ring — Home mode cards
    // -----------------------------------------------------------------------

    /// Headless app variant that also installs the focus and modal
    /// plugins so `attach_focusable_to_modal_buttons` and Phase 2's
    /// `attach_focusable_to_home_mode_cards` can run.
    fn headless_app_with_focus() -> App {
        use crate::ui_focus::UiFocusPlugin;
        use crate::ui_modal::UiModalPlugin;

        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(UiModalPlugin)
            .add_plugins(UiFocusPlugin)
            .add_plugins(GamePlugin)
            .add_plugins(TablePlugin)
            .add_plugins(ProgressPlugin::headless())
            .add_plugins(HomePlugin::headless());
        app.init_resource::<ButtonInput<KeyCode>>();
        app.update();
        app
    }

    /// Open the Home modal at the given player level. Tags the cards
    /// with `Focusable` (and, when locked, `Disabled`) by running an
    /// extra tick after the M press so the focus-attach system fires.
    fn open_home_at_level(app: &mut App, level: u32) -> Entity {
        app.world_mut().resource_mut::<ProgressResource>().0.level = level;
        let entity = open_home(app);
        // One more tick so `attach_focusable_to_home_mode_cards` runs
        // on the freshly-spawned cards.
        app.update();
        entity
    }

    #[test]
    fn home_mode_cards_get_focusable_marker() {
        let mut app = headless_app_with_focus();
        let scrim = open_home_at_level(&mut app, CHALLENGE_UNLOCK_LEVEL);

        // Every card carries `Focusable` in `FocusGroup::Modal(scrim)`.
        let cards: Vec<(HomeMode, Focusable)> = app
            .world_mut()
            .query::<(&HomeModeCard, &Focusable)>()
            .iter(app.world())
            .map(|(c, f)| (c.0, *f))
            .collect();

        assert_eq!(cards.len(), 5, "all five cards must carry a Focusable");
        for (mode, focusable) in &cards {
            assert_eq!(
                focusable.group,
                FocusGroup::Modal(scrim),
                "{mode:?} card must be in the Home scrim's focus group"
            );
        }
    }

    #[test]
    fn home_locked_cards_get_disabled_marker() {
        let mut app = headless_app_with_focus();
        // Level 0: Zen, Challenge, Time Attack are locked; Classic and
        // Daily are not.
        let _ = open_home_at_level(&mut app, 0);

        let states: Vec<(HomeMode, bool)> = app
            .world_mut()
            .query::<(&HomeModeCard, bevy::ecs::query::Has<Disabled>)>()
            .iter(app.world())
            .map(|(c, d)| (c.0, d))
            .collect();

        for (mode, disabled) in states {
            match mode {
                HomeMode::Classic | HomeMode::Daily => assert!(
                    !disabled,
                    "{mode:?} must not be Disabled at level 0 (it's never locked)"
                ),
                HomeMode::Zen | HomeMode::Challenge | HomeMode::TimeAttack => assert!(
                    disabled,
                    "{mode:?} must carry the Disabled marker at level 0 so Tab skips it"
                ),
            }
        }
    }

    #[test]
    fn home_unlocked_cards_no_disabled_marker() {
        let mut app = headless_app_with_focus();
        let _ = open_home_at_level(&mut app, CHALLENGE_UNLOCK_LEVEL);

        let any_disabled = app
            .world_mut()
            .query_filtered::<&HomeModeCard, With<Disabled>>()
            .iter(app.world())
            .next()
            .is_some();

        assert!(
            !any_disabled,
            "no card may be Disabled when the player is at the unlock level"
        );
    }

    // -----------------------------------------------------------------------
    // Digit-key shortcuts (1-5) — modal-scoped direct mode launch
    // -----------------------------------------------------------------------

    /// Press a key and clear the input afterwards so the next `update()`
    /// doesn't re-fire `just_pressed`. Mirrors the open_home() pattern but
    /// for an arbitrary key (the M-press helper releases & clears KeyM,
    /// which is also what we need here for Digit keys).
    fn press_and_clear(app: &mut App, key: KeyCode) {
        {
            let mut input = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            input.press(key);
        }
        app.update();
        {
            let mut input = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            input.release(key);
            input.clear();
        }
    }

    #[test]
    fn digit1_in_home_modal_starts_classic_and_closes_modal() {
        let mut app = headless_app();
        let _ = open_home(&mut app);

        // Drain any pre-existing NewGameRequestEvent so the assertion
        // only sees the digit-key driven write.
        app.world_mut()
            .resource_mut::<Messages<NewGameRequestEvent>>()
            .clear();

        press_and_clear(&mut app, KeyCode::Digit1);

        let events = app.world().resource::<Messages<NewGameRequestEvent>>();
        let mut cursor = events.get_cursor();
        let fired: Vec<_> = cursor.read(events).copied().collect();
        assert_eq!(
            fired.len(),
            1,
            "exactly one NewGameRequestEvent must fire for Digit1"
        );

        assert_eq!(
            app.world_mut()
                .query::<&HomeScreen>()
                .iter(app.world())
                .count(),
            0,
            "Home modal must close after launching Classic via Digit1"
        );
    }

    #[test]
    fn digit3_at_level_zero_is_a_noop() {
        let mut app = headless_app();
        // Default level is 0 — Zen is locked.
        let _ = open_home(&mut app);

        app.world_mut()
            .resource_mut::<Messages<StartZenRequestEvent>>()
            .clear();

        press_and_clear(&mut app, KeyCode::Digit3);

        let zen = app.world().resource::<Messages<StartZenRequestEvent>>();
        let mut zc = zen.get_cursor();
        assert!(
            zc.read(zen).next().is_none(),
            "Digit3 at level 0 must not fire StartZenRequestEvent"
        );

        assert_eq!(
            app.world_mut()
                .query::<&HomeScreen>()
                .iter(app.world())
                .count(),
            1,
            "Home modal must remain open after a locked-mode digit press"
        );
    }

    #[test]
    fn digit3_at_unlock_level_starts_zen_and_closes_modal() {
        let mut app = headless_app();
        // Bump the player to the unlock level *before* opening the modal
        // so the Mode Launcher is in its unlocked state.
        app.world_mut()
            .resource_mut::<ProgressResource>()
            .0
            .level = CHALLENGE_UNLOCK_LEVEL;
        let _ = open_home(&mut app);

        app.world_mut()
            .resource_mut::<Messages<StartZenRequestEvent>>()
            .clear();

        press_and_clear(&mut app, KeyCode::Digit3);

        let zen = app.world().resource::<Messages<StartZenRequestEvent>>();
        let mut zc = zen.get_cursor();
        assert_eq!(
            zc.read(zen).count(),
            1,
            "Digit3 at unlock level must fire exactly one StartZenRequestEvent"
        );

        assert_eq!(
            app.world_mut()
                .query::<&HomeScreen>()
                .iter(app.world())
                .count(),
            0,
            "Home modal must close after launching Zen via Digit3"
        );
    }

    #[test]
    fn digit_keys_outside_home_modal_are_noop() {
        let mut app = headless_app();
        // Modal is NOT open. Bump level so Zen would otherwise be allowed
        // — this isolates the modal-scope guard from the unlock check.
        app.world_mut()
            .resource_mut::<ProgressResource>()
            .0
            .level = CHALLENGE_UNLOCK_LEVEL;

        // Drain any pre-existing events.
        app.world_mut()
            .resource_mut::<Messages<NewGameRequestEvent>>()
            .clear();
        app.world_mut()
            .resource_mut::<Messages<StartZenRequestEvent>>()
            .clear();
        app.world_mut()
            .resource_mut::<Messages<StartChallengeRequestEvent>>()
            .clear();
        app.world_mut()
            .resource_mut::<Messages<StartTimeAttackRequestEvent>>()
            .clear();
        app.world_mut()
            .resource_mut::<Messages<StartDailyChallengeRequestEvent>>()
            .clear();

        // Press every digit 1-5 in turn — none should trigger a launch.
        for key in [
            KeyCode::Digit1,
            KeyCode::Digit2,
            KeyCode::Digit3,
            KeyCode::Digit4,
            KeyCode::Digit5,
        ] {
            press_and_clear(&mut app, key);
        }

        let new_game = app.world().resource::<Messages<NewGameRequestEvent>>();
        let mut nc = new_game.get_cursor();
        assert!(
            nc.read(new_game).next().is_none(),
            "Digit keys with no modal open must not fire NewGameRequestEvent"
        );
        let zen = app.world().resource::<Messages<StartZenRequestEvent>>();
        let mut zc = zen.get_cursor();
        assert!(
            zc.read(zen).next().is_none(),
            "Digit keys with no modal open must not fire StartZenRequestEvent"
        );
        let chal = app.world().resource::<Messages<StartChallengeRequestEvent>>();
        let mut cc = chal.get_cursor();
        assert!(
            cc.read(chal).next().is_none(),
            "Digit keys with no modal open must not fire StartChallengeRequestEvent"
        );
        let ta = app.world().resource::<Messages<StartTimeAttackRequestEvent>>();
        let mut tc = ta.get_cursor();
        assert!(
            tc.read(ta).next().is_none(),
            "Digit keys with no modal open must not fire StartTimeAttackRequestEvent"
        );
        let daily = app.world().resource::<Messages<StartDailyChallengeRequestEvent>>();
        let mut dc = daily.get_cursor();
        assert!(
            dc.read(daily).next().is_none(),
            "Digit keys with no modal open must not fire StartDailyChallengeRequestEvent"
        );
    }
}
