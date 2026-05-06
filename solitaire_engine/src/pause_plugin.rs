//! Pause overlay (Esc).
//!
//! While paused:
//! - The `PausedResource` flag is true.
//! - Elapsed-time and Time Attack tickers stop counting (they read this
//!   resource and bail out early).
//!
//! The pause modal is built on the standard `ui_modal` scaffold:
//! uniform scrim, centred card, real Resume / Forfeit buttons. Clicking
//! Forfeit (or pressing the `G` accelerator) fires
//! `ForfeitRequestEvent`, which spawns a `ForfeitConfirmScreen` modal
//! stacked above the pause card; confirming there fires `ForfeitEvent`
//! and dismisses both modals so the new game can start cleanly.
//!
//! This replaces the prior double-G keyboard countdown — see commit
//! history for the legacy `forfeit_countdown` toast design.
//!
//! **Drag cancellation:** when Esc is pressed while a mouse drag is in
//! progress, the drag is cancelled (cards snap back to their origin) and
//! the pause overlay is **not** opened. Pressing Esc again with no drag
//! active opens the overlay as normal.

use bevy::prelude::*;
use solitaire_core::game_state::DrawMode;
use solitaire_data::save_game_state_to;

use crate::events::{
    ForfeitEvent, ForfeitRequestEvent, InfoToastEvent, PauseRequestEvent, StateChangedEvent,
};
use crate::font_plugin::FontResource;
use crate::game_plugin::{GameOverScreen, GameStatePath};
use crate::progress_plugin::ProgressResource;
use crate::resources::{DragState, GameStateResource};
use crate::selection_plugin::{SelectionKeySet, SelectionState};
use crate::settings_plugin::{SettingsChangedEvent, SettingsResource, SettingsStoragePath};
use crate::stats_plugin::StatsResource;
use crate::ui_modal::{
    spawn_modal, spawn_modal_actions, spawn_modal_body_text, spawn_modal_button,
    spawn_modal_header, ButtonVariant, ModalScrim,
};
use bevy::ecs::system::SystemParam;
use crate::ui_theme::{
    self, TEXT_PRIMARY, TEXT_SECONDARY, TYPE_BODY_LG, TYPE_CAPTION, VAL_SPACE_3,
};

/// Toggleable flag read by `tick_elapsed_time` and `advance_time_attack`.
#[derive(Resource, Debug, Default)]
pub struct PausedResource(pub bool);

/// Marker on the pause overlay scrim.
#[derive(Component, Debug)]
pub struct PauseScreen;

/// Marker on the draw-mode toggle button inside the pause overlay.
#[derive(Component, Debug)]
struct PauseDrawToggle;

/// Marker on the Resume primary button on the pause modal.
#[derive(Component, Debug)]
struct PauseResumeButton;

/// Marker on the Forfeit tertiary button on the pause modal. A click
/// fires `ForfeitRequestEvent`, the same event the `G` accelerator
/// fires, so the same code path opens the confirm modal either way.
#[derive(Component, Debug)]
struct PauseForfeitButton;

/// Marker on the forfeit-confirm modal scrim.
#[derive(Component, Debug)]
pub struct ForfeitConfirmScreen;

/// Marker on the Cancel secondary button inside the forfeit-confirm modal.
#[derive(Component, Debug)]
struct ForfeitCancelButton;

/// Marker on the "Forfeit" primary button inside the forfeit-confirm modal.
#[derive(Component, Debug)]
struct ForfeitConfirmButton;

/// Returns the human-readable label for a draw mode.
///
/// Used on the pause overlay draw-mode toggle button.
pub fn draw_mode_label(mode: DrawMode) -> &'static str {
    match mode {
        DrawMode::DrawOne => "Draw 1",
        DrawMode::DrawThree => "Draw 3",
    }
}

/// Handles pause and resume: toggles the pause overlay on Esc, freezes
/// game-input systems via `PausedResource`, saves the in-progress game
/// to disk, and routes the Forfeit confirm-modal flow.
pub struct PausePlugin;

impl Plugin for PausePlugin {
    fn build(&self, app: &mut App) {
        // add_message is idempotent — other plugins may register these
        // first, but a duplicate call is always safe.
        app.add_message::<SettingsChangedEvent>()
            .add_message::<StateChangedEvent>()
            .add_message::<PauseRequestEvent>()
            .add_message::<ForfeitRequestEvent>()
            .add_message::<ForfeitEvent>()
            .add_message::<InfoToastEvent>()
            .init_resource::<PausedResource>()
            .add_systems(
                Update,
                (
                    // toggle_pause must see SelectionState *before* handle_selection_keys
                    // clears it, so it can skip Escape when a card is selected.
                    // It must also run *before* handle_forfeit_keyboard so the
                    // ForfeitConfirmScreen is still alive when toggle_pause's
                    // early-return guard checks for it — otherwise an Esc that
                    // closes the forfeit modal would also open pause in the
                    // same frame.
                    toggle_pause
                        .before(SelectionKeySet)
                        .before(handle_forfeit_keyboard),
                    handle_pause_draw_toggle,
                    handle_pause_resume_button,
                    handle_pause_forfeit_button,
                    handle_forfeit_request,
                    handle_forfeit_confirm_buttons,
                    handle_forfeit_keyboard,
                ),
            );
    }
}

/// Bundles the modal-related queries `toggle_pause` reads each tick.
/// Pulled into a [`SystemParam`] so the system stays under Bevy's 16-
/// parameter cap after the cross-modal Esc guard query was added.
#[derive(SystemParam)]
struct PauseModalQueries<'w, 's> {
    pause_screens: Query<'w, 's, Entity, With<PauseScreen>>,
    forfeit_screens: Query<'w, 's, Entity, With<ForfeitConfirmScreen>>,
    game_over_screens: Query<'w, 's, Entity, With<GameOverScreen>>,
    other_modal_scrims: Query<'w, 's, Entity, (With<ModalScrim>, Without<PauseScreen>)>,
}

#[allow(clippy::too_many_arguments)]
fn toggle_pause(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    mut requests: MessageReader<PauseRequestEvent>,
    mut paused: ResMut<PausedResource>,
    modal_queries: PauseModalQueries<'_, '_>,
    game: Option<Res<GameStateResource>>,
    path: Option<Res<GameStatePath>>,
    progress: Option<Res<ProgressResource>>,
    stats: Option<Res<StatsResource>>,
    settings: Option<Res<SettingsResource>>,
    font_res: Option<Res<FontResource>>,
    mut drag: Option<ResMut<DragState>>,
    mut changed: MessageWriter<StateChangedEvent>,
    selection: Option<Res<SelectionState>>,
) {
    let PauseModalQueries {
        pause_screens: screens,
        forfeit_screens,
        game_over_screens,
        other_modal_scrims,
    } = modal_queries;

    // Either Esc or a click on the HUD "Pause" button (which fires
    // PauseRequestEvent) opens or closes the overlay. Drain the queue so a
    // burst of clicks doesn't queue future toggles.
    let button_clicked = requests.read().count() > 0;
    if !keys.just_pressed(KeyCode::Escape) && !button_clicked {
        return;
    }
    // Forfeit confirm modal eats Esc — let `handle_forfeit_keyboard`
    // close it instead of toggling pause.
    if !forfeit_screens.is_empty() {
        return;
    }
    // Any other modal (Confirm New Game, Restore, Home, Onboarding,
    // Settings, etc.) owns its own dismissal — pause must not stack
    // on top of it. Without this guard a single Esc both closes the
    // open modal AND spawns the pause overlay underneath, leaving the
    // player on a screen they didn't ask for. The HUD-button path
    // (`button_clicked`) is gated too; clicking Pause while another
    // modal is up is almost always an accident.
    if !other_modal_scrims.is_empty() {
        return;
    }
    // If a card is currently selected, let SelectionPlugin handle this Escape
    // (it will clear the selection). Pause must not also open in the same frame.
    if selection.is_some_and(|s| s.selected_pile.is_some()) {
        return;
    }
    // If the game-over overlay is visible, let handle_game_over_input consume
    // the Escape key (to start a new game). Do not open the pause overlay.
    if !game_over_screens.is_empty() {
        return;
    }
    // If a drag is in progress, cancel it instead of opening the pause overlay.
    // Clearing DragState and emitting StateChangedEvent snaps the dragged cards
    // back to their resting positions exactly as a rejected drop does.
    if let Some(ref mut d) = drag
        && !d.is_idle() {
            d.clear();
            changed.write(StateChangedEvent);
            return;
        }
    if let Ok(entity) = screens.single() {
        commands.entity(entity).despawn();
        paused.0 = false;
    } else {
        // Snapshot current level and streak at pause time.
        let level = progress.as_deref().map(|p| p.0.level);
        let streak = stats.as_deref().map(|s| s.0.win_streak_current);
        let draw_mode = settings.as_deref().map(|s| s.0.draw_mode.clone());
        spawn_pause_screen(
            &mut commands,
            level,
            streak,
            draw_mode,
            font_res.as_deref(),
        );
        paused.0 = true;
        // Persist the current game state whenever the player opens the pause
        // overlay so an OS-level kill still leaves a resumable save.
        if let (Some(g), Some(p)) = (game, path)
            && let Some(disk_path) = p.0.as_deref()
                && let Err(e) = save_game_state_to(disk_path, &g.0) {
                    warn!("game_state: failed to save on pause: {e}");
                }
    }
}

/// Handles the draw-mode toggle button on the pause overlay.
///
/// Toggling flips the draw mode in `SettingsResource`, persists settings, and
/// fires `SettingsChangedEvent`. The change takes effect on the next new game.
fn handle_pause_draw_toggle(
    interaction_query: Query<&Interaction, (Changed<Interaction>, With<PauseDrawToggle>)>,
    paused: Res<PausedResource>,
    settings: Option<ResMut<SettingsResource>>,
    path: Option<Res<SettingsStoragePath>>,
    mut changed: MessageWriter<SettingsChangedEvent>,
) {
    if !paused.0 {
        return;
    }
    let Some(mut settings) = settings else { return };
    for interaction in &interaction_query {
        if *interaction != Interaction::Pressed {
            continue;
        }
        settings.0.draw_mode = match settings.0.draw_mode {
            DrawMode::DrawOne => DrawMode::DrawThree,
            DrawMode::DrawThree => DrawMode::DrawOne,
        };
        if let Some(p) = &path
            && let Some(target) = &p.0
                && let Err(e) = solitaire_data::save_settings_to(target, &settings.0) {
                    warn!("failed to save settings after draw-mode toggle: {e}");
                }
        changed.write(SettingsChangedEvent(settings.0.clone()));
    }
}

/// Closes the pause modal when the player clicks the Resume button.
/// Routes through `PauseRequestEvent` so the same toggle path runs as
/// when Esc is pressed or the HUD Pause button is clicked.
fn handle_pause_resume_button(
    interaction_query: Query<&Interaction, (Changed<Interaction>, With<PauseResumeButton>)>,
    mut requests: MessageWriter<PauseRequestEvent>,
) {
    for interaction in &interaction_query {
        if *interaction == Interaction::Pressed {
            requests.write(PauseRequestEvent);
        }
    }
}

/// Translates a click on the pause modal's Forfeit button into a
/// `ForfeitRequestEvent` so `handle_forfeit_request` can spawn the
/// confirm modal — same code path as the `G` accelerator.
fn handle_pause_forfeit_button(
    interaction_query: Query<&Interaction, (Changed<Interaction>, With<PauseForfeitButton>)>,
    mut requests: MessageWriter<ForfeitRequestEvent>,
) {
    for interaction in &interaction_query {
        if *interaction == Interaction::Pressed {
            requests.write(ForfeitRequestEvent);
        }
    }
}

/// Spawns `ForfeitConfirmScreen` in response to a `ForfeitRequestEvent`
/// (from the `G` accelerator or the Pause modal's Forfeit button).
///
/// Surfaces a toast and bails when there is no game to forfeit (won
/// state, or no `GameStateResource` at all) so the request is never
/// silently dropped — the prior implementation's silent no-op made the
/// pause modal's Forfeit button feel broken.
fn handle_forfeit_request(
    mut commands: Commands,
    mut requests: MessageReader<ForfeitRequestEvent>,
    forfeit_screens: Query<Entity, With<ForfeitConfirmScreen>>,
    game: Option<Res<GameStateResource>>,
    font_res: Option<Res<FontResource>>,
    mut toast: MessageWriter<InfoToastEvent>,
) {
    let requested = requests.read().count() > 0;
    if !requested {
        return;
    }
    if !forfeit_screens.is_empty() {
        return;
    }
    let game_in_progress = game.as_ref().is_some_and(|g| !g.0.is_won);
    if !game_in_progress {
        toast.write(InfoToastEvent("No game to forfeit".to_string()));
        return;
    }
    spawn_forfeit_confirm_screen(&mut commands, font_res.as_deref());
}

/// Mouse / touch handler for the forfeit-confirm modal buttons.
///
/// Cancel despawns the confirm modal and leaves the pause modal (if
/// any) untouched. Confirm despawns both modals, clears the paused
/// flag, and fires `ForfeitEvent` for `StatsPlugin` to consume.
#[allow(clippy::too_many_arguments)]
fn handle_forfeit_confirm_buttons(
    mut commands: Commands,
    yes_buttons: Query<&Interaction, (Changed<Interaction>, With<ForfeitConfirmButton>)>,
    no_buttons: Query<&Interaction, (Changed<Interaction>, With<ForfeitCancelButton>)>,
    forfeit_screens: Query<Entity, With<ForfeitConfirmScreen>>,
    pause_screens: Query<Entity, With<PauseScreen>>,
    paused: ResMut<PausedResource>,
    forfeit: MessageWriter<ForfeitEvent>,
) {
    let confirmed = yes_buttons.iter().any(|i| *i == Interaction::Pressed);
    let cancelled = no_buttons.iter().any(|i| *i == Interaction::Pressed);
    if !confirmed && !cancelled {
        return;
    }
    close_forfeit_modal(
        &mut commands,
        confirmed,
        &forfeit_screens,
        &pause_screens,
        paused,
        forfeit,
    );
}

/// Keyboard accelerator for the forfeit-confirm modal — Y / Enter
/// confirms; N / Escape cancels.
fn handle_forfeit_keyboard(
    mut commands: Commands,
    keys: Option<Res<ButtonInput<KeyCode>>>,
    forfeit_screens: Query<Entity, With<ForfeitConfirmScreen>>,
    pause_screens: Query<Entity, With<PauseScreen>>,
    paused: ResMut<PausedResource>,
    forfeit: MessageWriter<ForfeitEvent>,
) {
    if forfeit_screens.is_empty() {
        return;
    }
    let Some(keys) = keys else { return };
    let confirmed = keys.just_pressed(KeyCode::KeyY) || keys.just_pressed(KeyCode::Enter);
    let cancelled = keys.just_pressed(KeyCode::KeyN) || keys.just_pressed(KeyCode::Escape);
    if !confirmed && !cancelled {
        return;
    }
    close_forfeit_modal(
        &mut commands,
        confirmed,
        &forfeit_screens,
        &pause_screens,
        paused,
        forfeit,
    );
}

/// Common cleanup shared by the click and keyboard close paths.
///
/// On `confirm`: despawn both modals, clear the paused flag, and fire
/// `ForfeitEvent`. On cancel: despawn only the confirm modal so the
/// pause modal (if any) stays open.
fn close_forfeit_modal(
    commands: &mut Commands,
    confirm: bool,
    forfeit_screens: &Query<Entity, With<ForfeitConfirmScreen>>,
    pause_screens: &Query<Entity, With<PauseScreen>>,
    mut paused: ResMut<PausedResource>,
    mut forfeit: MessageWriter<ForfeitEvent>,
) {
    for entity in forfeit_screens {
        commands.entity(entity).despawn();
    }
    if confirm {
        for entity in pause_screens {
            commands.entity(entity).despawn();
        }
        paused.0 = false;
        forfeit.write(ForfeitEvent);
    }
}

/// Spawns the pause modal using the standard `ui_modal` scaffold —
/// uniform scrim, centred card, `Resume` primary + `Forfeit` tertiary
/// action buttons, plus a Draw Mode toggle row when settings are
/// installed.
///
/// `level` and `streak` are optional snapshots taken at pause time —
/// rendered as the modal's body line. `draw_mode` is the current draw
/// mode shown on the toggle button. When the corresponding resources
/// are absent (e.g. headless tests) the related sections are omitted.
fn spawn_pause_screen(
    commands: &mut Commands,
    level: Option<u32>,
    streak: Option<u32>,
    draw_mode: Option<DrawMode>,
    font_res: Option<&FontResource>,
) {
    spawn_modal(commands, PauseScreen, ui_theme::Z_PAUSE, |card| {
        spawn_modal_header(card, "Paused", font_res);
        if level.is_some() || streak.is_some() {
            let info = build_level_streak_line(level, streak);
            spawn_modal_body_text(card, info, TEXT_SECONDARY, font_res);
        }
        if let Some(mode) = draw_mode {
            spawn_draw_mode_row(card, mode, font_res);
        }
        spawn_modal_actions(card, |actions| {
            spawn_modal_button(
                actions,
                PauseForfeitButton,
                "Forfeit",
                Some("G"),
                ButtonVariant::Tertiary,
                font_res,
            );
            spawn_modal_button(
                actions,
                PauseResumeButton,
                "Resume",
                Some("Esc"),
                ButtonVariant::Primary,
                font_res,
            );
        });
    });
}

/// Inline "Draw Mode  [Draw 1]" row + a caption explaining the change
/// applies to the next game. Spawned inside the modal body.
fn spawn_draw_mode_row(
    parent: &mut ChildSpawnerCommands,
    mode: DrawMode,
    font_res: Option<&FontResource>,
) {
    let label_font = TextFont {
        font: font_res.map(|f| f.0.clone()).unwrap_or_default(),
        font_size: TYPE_BODY_LG,
        ..default()
    };
    let caption_font = TextFont {
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
                Text::new("Draw Mode"),
                label_font,
                TextColor(TEXT_PRIMARY),
            ));
            spawn_modal_button(
                row,
                PauseDrawToggle,
                draw_mode_label(mode),
                None,
                ButtonVariant::Secondary,
                font_res,
            );
        });
    parent.spawn((
        Text::new("Takes effect next game"),
        caption_font,
        TextColor(TEXT_SECONDARY),
    ));
}

/// Spawns `ForfeitConfirmScreen` — a Cancel / "Forfeit" modal
/// stacked above the pause modal at `Z_PAUSE_DIALOG`.
fn spawn_forfeit_confirm_screen(commands: &mut Commands, font_res: Option<&FontResource>) {
    spawn_modal(
        commands,
        ForfeitConfirmScreen,
        ui_theme::Z_PAUSE_DIALOG,
        |card| {
            spawn_modal_header(card, "Forfeit this game?", font_res);
            spawn_modal_body_text(
                card,
                "This will count as a loss and break your win streak.",
                TEXT_SECONDARY,
                font_res,
            );
            spawn_modal_actions(card, |actions| {
                spawn_modal_button(
                    actions,
                    ForfeitCancelButton,
                    "Cancel",
                    Some("Esc"),
                    ButtonVariant::Secondary,
                    font_res,
                );
                spawn_modal_button(
                    actions,
                    ForfeitConfirmButton,
                    "Forfeit",
                    Some("Y"),
                    ButtonVariant::Primary,
                    font_res,
                );
            });
        },
    );
}

/// Formats the level / win-streak summary line for the pause overlay.
///
/// Both values are optional because either resource may be absent in
/// headless or partially-configured app contexts.
fn build_level_streak_line(level: Option<u32>, streak: Option<u32>) -> String {
    match (level, streak) {
        (Some(l), Some(s)) => format!("Level {l}   Win streak: {s}"),
        (Some(l), None) => format!("Level {l}"),
        (None, Some(s)) => format!("Win streak: {s}"),
        (None, None) => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headless_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins).add_plugins(PausePlugin);
        app.init_resource::<ButtonInput<KeyCode>>();
        app.update();
        app
    }

    fn press_esc(app: &mut App) {
        let mut input = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        input.release(KeyCode::Escape);
        input.clear();
        input.press(KeyCode::Escape);
    }

    fn press_key(app: &mut App, key: KeyCode) {
        let mut input = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        input.release(key);
        input.clear();
        input.press(key);
    }

    /// `MinimalPlugins` does not include the input-plugin tick that
    /// transitions `just_pressed` → `pressed` between frames, so a key
    /// press would otherwise stay "just-pressed" forever. Call this
    /// helper between `app.update()` calls in tests that span multiple
    /// frames without re-pressing — it mirrors the real input cycle.
    fn advance_input(app: &mut App) {
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .clear();
    }

    #[test]
    fn pressing_esc_pauses() {
        let mut app = headless_app();
        press_esc(&mut app);
        app.update();
        assert!(app.world().resource::<PausedResource>().0);
        assert_eq!(
            app.world_mut()
                .query::<&PauseScreen>()
                .iter(app.world())
                .count(),
            1
        );
    }

    #[test]
    fn pressing_esc_twice_resumes() {
        let mut app = headless_app();
        press_esc(&mut app);
        app.update();
        press_esc(&mut app);
        app.update();
        assert!(!app.world().resource::<PausedResource>().0);
        assert_eq!(
            app.world_mut()
                .query::<&PauseScreen>()
                .iter(app.world())
                .count(),
            0
        );
    }

    #[test]
    fn toggle_is_symmetric_for_multiple_cycles() {
        let mut app = headless_app();
        // Third press re-pauses after resume.
        press_esc(&mut app);
        app.update();
        press_esc(&mut app);
        app.update();
        press_esc(&mut app);
        app.update();
        assert!(
            app.world().resource::<PausedResource>().0,
            "third Esc must re-pause"
        );
        assert_eq!(
            app.world_mut()
                .query::<&PauseScreen>()
                .iter(app.world())
                .count(),
            1,
            "third Esc must re-spawn PauseScreen"
        );
    }

    // -----------------------------------------------------------------------
    // build_level_streak_line (pure function)
    // -----------------------------------------------------------------------

    #[test]
    fn level_streak_both_present() {
        assert_eq!(
            build_level_streak_line(Some(7), Some(3)),
            "Level 7   Win streak: 3"
        );
    }

    #[test]
    fn level_streak_only_level() {
        assert_eq!(build_level_streak_line(Some(5), None), "Level 5");
    }

    #[test]
    fn level_streak_only_streak() {
        assert_eq!(build_level_streak_line(None, Some(4)), "Win streak: 4");
    }

    #[test]
    fn level_streak_neither() {
        assert_eq!(build_level_streak_line(None, None), "");
    }

    // -----------------------------------------------------------------------
    // Pause screen with progress / stats resources present
    // -----------------------------------------------------------------------

    #[test]
    fn pause_screen_spawns_with_level_and_streak_when_resources_present() {
        use crate::progress_plugin::{ProgressPlugin, ProgressResource};
        use crate::settings_plugin::SettingsPlugin;
        use crate::stats_plugin::{StatsPlugin, StatsResource};

        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(crate::game_plugin::GamePlugin)
            .add_plugins(crate::table_plugin::TablePlugin)
            .add_plugins(ProgressPlugin::headless())
            .add_plugins(StatsPlugin::headless())
            .add_plugins(SettingsPlugin::headless())
            .add_plugins(PausePlugin);
        app.init_resource::<ButtonInput<KeyCode>>();
        app.update();

        // Set known values.
        app.world_mut().resource_mut::<ProgressResource>().0.level = 7;
        app.world_mut().resource_mut::<StatsResource>().0.win_streak_current = 3;

        press_esc(&mut app);
        app.update();

        // Verify the screen was spawned.
        assert!(app.world().resource::<PausedResource>().0);

        // Find the text nodes on the PauseScreen children and check one contains
        // the expected level/streak string.
        let texts: Vec<String> = app
            .world_mut()
            .query::<&Text>()
            .iter(app.world())
            .map(|t| t.0.clone())
            .collect();
        assert!(
            texts.iter().any(|t| t == "Level 7   Win streak: 3"),
            "expected level/streak line in pause screen texts, got: {texts:?}"
        );
    }

    // -----------------------------------------------------------------------
    // PausedResource default (pure)
    // -----------------------------------------------------------------------

    #[test]
    fn paused_resource_default_is_unpaused() {
        let p = PausedResource::default();
        assert!(!p.0, "game must start unpaused");
    }

    // -----------------------------------------------------------------------
    // draw_mode_label (pure function) — Task #64
    // -----------------------------------------------------------------------

    #[test]
    fn draw_mode_label_draw_one() {
        assert_eq!(draw_mode_label(DrawMode::DrawOne), "Draw 1");
    }

    #[test]
    fn draw_mode_label_draw_three() {
        assert_eq!(draw_mode_label(DrawMode::DrawThree), "Draw 3");
    }

    /// Both variants are covered so the match is exhaustive — this test would
    /// fail to compile if a new DrawMode variant were added without updating
    /// `draw_mode_label`.
    #[test]
    fn draw_mode_label_covers_all_variants() {
        for mode in [DrawMode::DrawOne, DrawMode::DrawThree] {
            let label = draw_mode_label(mode);
            assert!(!label.is_empty(), "draw_mode_label must never return an empty string");
        }
    }

    // -----------------------------------------------------------------------
    // pause_draw_toggle_flips_draw_mode — Task #64
    // -----------------------------------------------------------------------

    #[test]
    fn pause_draw_toggle_flips_draw_mode() {
        use crate::settings_plugin::{SettingsPlugin, SettingsResource};
        use solitaire_data::Settings;

        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(SettingsPlugin::headless())
            .add_plugins(PausePlugin);
        app.init_resource::<ButtonInput<KeyCode>>();
        app.update();

        // Ensure we start with DrawOne.
        app.world_mut()
            .resource_mut::<SettingsResource>()
            .0
            .draw_mode = DrawMode::DrawOne;

        // Set paused so handle_pause_draw_toggle acts.
        app.world_mut().resource_mut::<PausedResource>().0 = true;

        // Spawn a PauseDrawToggle button with Pressed interaction.
        app.world_mut().spawn((
            PauseDrawToggle,
            Button,
            Interaction::Pressed,
        ));

        app.update();

        let mode = &app
            .world()
            .resource::<SettingsResource>()
            .0
            .draw_mode;
        assert_eq!(
            *mode,
            DrawMode::DrawThree,
            "draw mode must flip from DrawOne to DrawThree when toggle is pressed"
        );

        // A second press should flip back.
        {
            let mut interaction_query = app
                .world_mut()
                .query::<&mut Interaction>();
            for mut i in interaction_query.iter_mut(app.world_mut()) {
                *i = Interaction::Pressed;
            }
        }
        app.update();

        let mode2 = &app
            .world()
            .resource::<SettingsResource>()
            .0
            .draw_mode;
        assert_eq!(
            *mode2,
            DrawMode::DrawOne,
            "draw mode must flip back from DrawThree to DrawOne on second press"
        );

        // Verify a SettingsChangedEvent was fired.
        let events = app.world().resource::<Messages<SettingsChangedEvent>>();
        let mut cursor = events.get_cursor();
        let count = cursor.read(events).count();
        assert!(count >= 1, "SettingsChangedEvent must be fired on toggle");

        // Restore default settings state for hygiene.
        let _ = Settings::default();
    }

    // -----------------------------------------------------------------------
    // Pause modal exposes Resume + Forfeit buttons
    // -----------------------------------------------------------------------

    #[test]
    fn pause_modal_has_resume_and_forfeit_buttons() {
        let mut app = headless_app();
        press_esc(&mut app);
        app.update();

        let resume_count = app
            .world_mut()
            .query::<&PauseResumeButton>()
            .iter(app.world())
            .count();
        let forfeit_count = app
            .world_mut()
            .query::<&PauseForfeitButton>()
            .iter(app.world())
            .count();
        assert_eq!(resume_count, 1, "Resume button must be present on the pause modal");
        assert_eq!(forfeit_count, 1, "Forfeit button must be present on the pause modal");
    }

    /// Clicking the Resume button (via Pressed interaction) closes the
    /// pause modal — same outcome as a second Esc.
    #[test]
    fn pause_resume_button_closes_modal() {
        let mut app = headless_app();
        press_esc(&mut app);
        app.update();
        assert!(app.world().resource::<PausedResource>().0);

        // Mark the Resume button as Pressed.
        let resume_entity = {
            let mut q = app.world_mut().query_filtered::<Entity, With<PauseResumeButton>>();
            q.iter(app.world()).next().expect("Resume button must exist")
        };
        app.world_mut()
            .entity_mut(resume_entity)
            .insert(Interaction::Pressed);

        // Clear keys so the simulated "click" isn't competing with a real Esc press.
        app.world_mut().resource_mut::<ButtonInput<KeyCode>>().clear();
        app.update();
        // One more frame so the resulting PauseRequestEvent is consumed by toggle_pause.
        app.update();

        assert!(!app.world().resource::<PausedResource>().0, "Resume must clear PausedResource");
        assert_eq!(
            app.world_mut()
                .query::<&PauseScreen>()
                .iter(app.world())
                .count(),
            0,
            "Resume must despawn PauseScreen"
        );
    }

    // -----------------------------------------------------------------------
    // Forfeit confirm modal
    // -----------------------------------------------------------------------

    /// Test app with the resources `handle_forfeit_request` reads.
    /// Provides a fresh `GameStateResource` (not won) so the modal can
    /// open. `move_count` doesn't matter — the gate is just `!is_won`.
    fn forfeit_app() -> App {
        use solitaire_core::game_state::{DrawMode, GameState};
        let mut app = App::new();
        app.add_plugins(MinimalPlugins).add_plugins(PausePlugin);
        app.init_resource::<ButtonInput<KeyCode>>();
        app.insert_resource(GameStateResource(GameState::new(1, DrawMode::DrawOne)));
        app.update();
        app
    }

    #[test]
    fn forfeit_request_event_spawns_forfeit_confirm_screen() {
        let mut app = forfeit_app();
        app.world_mut()
            .resource_mut::<Messages<ForfeitRequestEvent>>()
            .write(ForfeitRequestEvent);
        app.update();

        assert_eq!(
            app.world_mut()
                .query::<&ForfeitConfirmScreen>()
                .iter(app.world())
                .count(),
            1,
            "ForfeitRequestEvent must spawn the forfeit-confirm modal"
        );
    }

    #[test]
    fn forfeit_request_does_not_double_spawn() {
        let mut app = forfeit_app();
        app.world_mut()
            .resource_mut::<Messages<ForfeitRequestEvent>>()
            .write(ForfeitRequestEvent);
        app.update();
        app.world_mut()
            .resource_mut::<Messages<ForfeitRequestEvent>>()
            .write(ForfeitRequestEvent);
        app.update();

        assert_eq!(
            app.world_mut()
                .query::<&ForfeitConfirmScreen>()
                .iter(app.world())
                .count(),
            1,
            "second ForfeitRequestEvent must not stack a second modal"
        );
    }

    /// When the game is already won, a `ForfeitRequestEvent` must not
    /// open the modal (you can't forfeit a finished game) and instead
    /// surface an `InfoToastEvent` so the user gets feedback that the
    /// hotkey was received but is currently a no-op.
    #[test]
    fn forfeit_request_emits_toast_and_skips_modal_when_game_is_won() {
        use solitaire_core::game_state::{DrawMode, GameState};
        let mut app = App::new();
        app.add_plugins(MinimalPlugins).add_plugins(PausePlugin);
        app.init_resource::<ButtonInput<KeyCode>>();
        let mut game = GameState::new(1, DrawMode::DrawOne);
        game.is_won = true;
        app.insert_resource(GameStateResource(game));
        app.update();

        app.world_mut()
            .resource_mut::<Messages<ForfeitRequestEvent>>()
            .write(ForfeitRequestEvent);
        app.update();

        assert_eq!(
            app.world_mut()
                .query::<&ForfeitConfirmScreen>()
                .iter(app.world())
                .count(),
            0,
            "the forfeit modal must not open when the current game is already won"
        );
        let events = app.world().resource::<Messages<InfoToastEvent>>();
        let mut cursor = events.get_cursor();
        assert!(
            cursor.read(events).any(|t| t.0 == "No game to forfeit"),
            "an InfoToastEvent must be fired so the player gets feedback"
        );
    }

    #[test]
    fn forfeit_confirm_y_key_fires_forfeit_event_and_despawns() {
        let mut app = forfeit_app();
        // Open the forfeit confirm modal.
        app.world_mut()
            .resource_mut::<Messages<ForfeitRequestEvent>>()
            .write(ForfeitRequestEvent);
        app.update();
        assert_eq!(
            app.world_mut()
                .query::<&ForfeitConfirmScreen>()
                .iter(app.world())
                .count(),
            1
        );

        // Press Y to confirm.
        press_key(&mut app, KeyCode::KeyY);
        app.update();

        // Modal despawned.
        assert_eq!(
            app.world_mut()
                .query::<&ForfeitConfirmScreen>()
                .iter(app.world())
                .count(),
            0,
            "Y must despawn the forfeit-confirm modal"
        );
        // ForfeitEvent fired.
        let events = app.world().resource::<Messages<ForfeitEvent>>();
        let mut cursor = events.get_cursor();
        assert_eq!(
            cursor.read(events).count(),
            1,
            "Y must fire exactly one ForfeitEvent"
        );
    }

    #[test]
    fn forfeit_confirm_n_key_cancels_without_firing_event() {
        let mut app = forfeit_app();
        app.world_mut()
            .resource_mut::<Messages<ForfeitRequestEvent>>()
            .write(ForfeitRequestEvent);
        app.update();

        press_key(&mut app, KeyCode::KeyN);
        app.update();

        assert_eq!(
            app.world_mut()
                .query::<&ForfeitConfirmScreen>()
                .iter(app.world())
                .count(),
            0,
            "N must despawn the forfeit-confirm modal"
        );
        let events = app.world().resource::<Messages<ForfeitEvent>>();
        let mut cursor = events.get_cursor();
        assert_eq!(
            cursor.read(events).count(),
            0,
            "Cancelling the modal must not fire ForfeitEvent"
        );
    }

    #[test]
    fn forfeit_confirm_esc_cancels_without_toggling_pause() {
        let mut app = forfeit_app();
        // Open the forfeit confirm modal directly via G-equivalent event.
        app.world_mut()
            .resource_mut::<Messages<ForfeitRequestEvent>>()
            .write(ForfeitRequestEvent);
        app.update();
        // Pause is NOT open at this point.
        assert!(!app.world().resource::<PausedResource>().0);

        press_esc(&mut app);
        app.update();

        // Forfeit modal is gone.
        assert_eq!(
            app.world_mut()
                .query::<&ForfeitConfirmScreen>()
                .iter(app.world())
                .count(),
            0
        );
        // Pause must NOT have toggled on — the Esc was consumed by the
        // forfeit-confirm handler, and toggle_pause must early-return
        // when a forfeit modal is visible (the despawn happens in the
        // same frame, but the early-return guard runs first).
        assert!(
            !app.world().resource::<PausedResource>().0,
            "Esc that closes the forfeit modal must not also open pause"
        );
    }

    #[test]
    fn forfeit_confirm_y_also_closes_pause_modal() {
        let mut app = forfeit_app();
        // Open pause first.
        press_esc(&mut app);
        app.update();
        assert!(app.world().resource::<PausedResource>().0);
        // Cycle input so Esc is no longer treated as `just_pressed` —
        // otherwise the next update would toggle pause back off and the
        // freshly-spawned forfeit modal would also be cancelled by the
        // stale Esc in handle_forfeit_keyboard.
        advance_input(&mut app);
        // Open forfeit-confirm on top of pause.
        app.world_mut()
            .resource_mut::<Messages<ForfeitRequestEvent>>()
            .write(ForfeitRequestEvent);
        app.update();
        assert_eq!(
            app.world_mut()
                .query::<&ForfeitConfirmScreen>()
                .iter(app.world())
                .count(),
            1
        );

        press_key(&mut app, KeyCode::KeyY);
        app.update();

        assert_eq!(
            app.world_mut()
                .query::<&PauseScreen>()
                .iter(app.world())
                .count(),
            0,
            "confirming forfeit must also despawn the pause modal"
        );
        assert!(
            !app.world().resource::<PausedResource>().0,
            "PausedResource must be cleared when a forfeit is confirmed"
        );
    }
}
