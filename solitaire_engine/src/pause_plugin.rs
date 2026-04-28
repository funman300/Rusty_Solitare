//! Pause overlay (Esc).
//!
//! While paused:
//! - The `PausedResource` flag is true.
//! - Elapsed-time and Time Attack tickers stop counting (they read this
//!   resource and bail out early).
//!
//! Pressing Esc again dismisses the overlay and resumes ticking. Other
//! input (drag, keyboard hotkeys) is **not** blocked — pause is purely a
//! "stop the clock" screen for now. A future polish slice can layer
//! input-blocking on top if desired.
//!
//! **Drag cancellation:** when Esc is pressed while a mouse drag is in
//! progress, the drag is cancelled (cards snap back to their origin) and
//! the pause overlay is **not** opened. Pressing Esc again with no drag
//! active opens the overlay as normal.

use bevy::prelude::*;
use solitaire_core::game_state::DrawMode;
use solitaire_data::save_game_state_to;

use crate::events::StateChangedEvent;
use crate::game_plugin::{GameOverScreen, GameStatePath};
use crate::progress_plugin::ProgressResource;
use crate::resources::{DragState, GameStateResource};
use crate::settings_plugin::{SettingsChangedEvent, SettingsResource, SettingsStoragePath};
use crate::stats_plugin::StatsResource;

/// Toggleable flag read by `tick_elapsed_time` and `advance_time_attack`.
#[derive(Resource, Debug, Default)]
pub struct PausedResource(pub bool);

/// Marker on the pause overlay root node.
#[derive(Component, Debug)]
pub struct PauseScreen;

/// Marker on the draw-mode toggle button inside the pause overlay.
#[derive(Component, Debug)]
struct PauseDrawToggle;

/// Returns the human-readable label for a draw mode.
///
/// Used on the pause overlay draw-mode toggle button.
pub fn draw_mode_label(mode: DrawMode) -> &'static str {
    match mode {
        DrawMode::DrawOne => "Draw 1",
        DrawMode::DrawThree => "Draw 3",
    }
}

pub struct PausePlugin;

impl Plugin for PausePlugin {
    fn build(&self, app: &mut App) {
        // Both add_event calls are idempotent — other plugins may register these
        // events first, but calling add_event again is always safe.
        app.add_event::<SettingsChangedEvent>()
            .add_event::<StateChangedEvent>()
            .init_resource::<PausedResource>()
            .add_systems(Update, (toggle_pause, handle_pause_draw_toggle));
    }
}

#[allow(clippy::too_many_arguments)]
fn toggle_pause(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    mut paused: ResMut<PausedResource>,
    screens: Query<Entity, With<PauseScreen>>,
    game_over_screens: Query<Entity, With<GameOverScreen>>,
    game: Option<Res<GameStateResource>>,
    path: Option<Res<GameStatePath>>,
    progress: Option<Res<ProgressResource>>,
    stats: Option<Res<StatsResource>>,
    settings: Option<Res<SettingsResource>>,
    mut drag: Option<ResMut<DragState>>,
    mut changed: EventWriter<StateChangedEvent>,
) {
    if !keys.just_pressed(KeyCode::Escape) {
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
    if let Some(ref mut d) = drag {
        if !d.is_idle() {
            d.clear();
            changed.send(StateChangedEvent);
            return;
        }
    }
    if let Ok(entity) = screens.get_single() {
        commands.entity(entity).despawn_recursive();
        paused.0 = false;
    } else {
        // Snapshot current level and streak at pause time.
        let level = progress.as_deref().map(|p| p.0.level);
        let streak = stats.as_deref().map(|s| s.0.win_streak_current);
        let draw_mode = settings.as_deref().map(|s| s.0.draw_mode.clone());
        spawn_pause_screen(&mut commands, level, streak, draw_mode);
        paused.0 = true;
        // Persist the current game state whenever the player opens the pause
        // overlay so an OS-level kill still leaves a resumable save.
        if let (Some(g), Some(p)) = (game, path) {
            if let Some(disk_path) = p.0.as_deref() {
                if let Err(e) = save_game_state_to(disk_path, &g.0) {
                    warn!("game_state: failed to save on pause: {e}");
                }
            }
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
    mut changed: EventWriter<SettingsChangedEvent>,
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
        if let Some(p) = &path {
            if let Some(target) = &p.0 {
                if let Err(e) = solitaire_data::save_settings_to(target, &settings.0) {
                    warn!("failed to save settings after draw-mode toggle: {e}");
                }
            }
        }
        changed.send(SettingsChangedEvent(settings.0.clone()));
    }
}

/// Spawns the full-screen pause overlay.
///
/// `level` and `streak` are optional snapshots taken at pause time. When
/// `ProgressResource` or `StatsResource` is not installed (e.g. in headless
/// tests), those lines are omitted from the overlay.
///
/// `draw_mode` is the current draw mode shown on the toggle button. When
/// `SettingsResource` is absent the draw-mode row is omitted.
fn spawn_pause_screen(
    commands: &mut Commands,
    level: Option<u32>,
    streak: Option<u32>,
    draw_mode: Option<DrawMode>,
) {
    commands
        .spawn((
            PauseScreen,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Percent(0.0),
                top: Val::Percent(0.0),
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                row_gap: Val::Px(8.0),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.82)),
            ZIndex(220),
        ))
        .with_children(|b| {
            b.spawn((
                Text::new("Paused"),
                TextFont {
                    font_size: 48.0,
                    ..default()
                },
                TextColor(Color::srgb(1.0, 0.87, 0.0)),
            ));
            // Level and streak line — only shown when the resources are present.
            if level.is_some() || streak.is_some() {
                let info = build_level_streak_line(level, streak);
                b.spawn((
                    Text::new(info),
                    TextFont {
                        font_size: 22.0,
                        ..default()
                    },
                    TextColor(Color::srgb(0.75, 0.95, 0.75)),
                ));
            }
            // Draw-mode toggle row — only shown when SettingsResource is present.
            if let Some(mode) = draw_mode {
                b.spawn(Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(12.0),
                    ..default()
                })
                .with_children(|row| {
                    row.spawn((
                        Text::new("Draw Mode:"),
                        TextFont { font_size: 20.0, ..default() },
                        TextColor(Color::srgb(0.85, 0.85, 0.80)),
                    ));
                    row.spawn((
                        PauseDrawToggle,
                        Button,
                        Node {
                            padding: UiRect::axes(Val::Px(14.0), Val::Px(6.0)),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            ..default()
                        },
                        BackgroundColor(Color::srgb(0.20, 0.30, 0.45)),
                        BorderRadius::all(Val::Px(4.0)),
                    ))
                    .with_children(|btn| {
                        btn.spawn((
                            Text::new(draw_mode_label(mode)),
                            TextFont { font_size: 18.0, ..default() },
                            TextColor(Color::WHITE),
                        ));
                    });
                });
                b.spawn((
                    Text::new("Takes effect next game"),
                    TextFont { font_size: 14.0, ..default() },
                    TextColor(Color::srgb(0.55, 0.55, 0.60)),
                ));
            }
            b.spawn((
                Text::new("Press Esc to resume"),
                TextFont {
                    font_size: 22.0,
                    ..default()
                },
                TextColor(Color::srgb(0.85, 0.85, 0.80)),
            ));
        });
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
        let events = app.world().resource::<Events<SettingsChangedEvent>>();
        let mut cursor = events.get_cursor();
        let count = cursor.read(events).count();
        assert!(count >= 1, "SettingsChangedEvent must be fired on toggle");

        // Restore default settings state for hygiene.
        let _ = Settings::default();
    }
}
