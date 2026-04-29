//! Loads, updates, and persists `StatsSnapshot` in response to game events,
//! and provides a toggleable full-window stats overlay (press `S`).
//!
//! The persistence path is configurable via `StatsPlugin::storage_path`.
//! In production, `StatsPlugin::default()` loads/saves from the platform
//! data dir. In tests, use `StatsPlugin::headless()` to disable all file
//! I/O so the user's real stats file is neither read nor overwritten.

use std::path::PathBuf;

use bevy::input::ButtonInput;
use bevy::prelude::*;
use solitaire_data::{
    load_stats_from, save_stats_to, stats_file_path, PlayerProgress, StatsExt, StatsSnapshot,
    WEEKLY_GOALS,
};

use crate::auto_complete_plugin::AutoCompleteState;
use crate::challenge_plugin::challenge_progress_label;
use crate::events::{ForfeitEvent, GameWonEvent, InfoToastEvent, NewGameRequestEvent};
use crate::game_plugin::GameMutation;
use crate::progress_plugin::ProgressResource;
use crate::resources::GameStateResource;
use crate::time_attack_plugin::TimeAttackResource;

/// Bevy resource wrapping the current stats.
#[derive(Resource, Debug, Clone)]
pub struct StatsResource(pub StatsSnapshot);

/// Persistence path for `StatsResource`. `None` disables I/O.
#[derive(Resource, Debug, Clone)]
pub struct StatsStoragePath(pub Option<PathBuf>);

/// System set for the stats-mutating systems. Downstream plugins that read
/// `StatsResource` after a win/abandon should run `.after(StatsUpdate)`.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct StatsUpdate;

/// Marker component on the stats overlay root node.
#[derive(Component, Debug)]
pub struct StatsScreen;

/// Marker component on an individual stat cell inside the stats overlay.
///
/// Each cell contains a large value label and a small descriptor label below it.
#[derive(Component, Debug)]
pub struct StatsCell;

/// Registers stats resources, update systems, and the UI toggle.
pub struct StatsPlugin {
    /// Where to persist stats. `None` disables all file I/O (for tests).
    pub storage_path: Option<PathBuf>,
}

impl Default for StatsPlugin {
    fn default() -> Self {
        Self {
            storage_path: stats_file_path(),
        }
    }
}

impl StatsPlugin {
    /// Plugin configured with no persistence. Use in tests and headless apps
    /// where touching `~/.local/share/solitaire_quest/stats.json` would be
    /// incorrect.
    pub fn headless() -> Self {
        Self { storage_path: None }
    }
}

impl Plugin for StatsPlugin {
    fn build(&self, app: &mut App) {
        let loaded = match &self.storage_path {
            Some(path) => load_stats_from(path),
            None => StatsSnapshot::default(),
        };
        app.insert_resource(StatsResource(loaded))
            .insert_resource(StatsStoragePath(self.storage_path.clone()))
            .add_message::<GameWonEvent>()
            .add_message::<NewGameRequestEvent>()
            .add_message::<ForfeitEvent>()
            .add_message::<InfoToastEvent>()
            // record_abandoned must read `move_count` BEFORE handle_new_game
            // clobbers it with a fresh game. These are NOT in StatsUpdate because
            // StatsUpdate (as a set) is ordered after GameMutation by external
            // constraints (win_summary_plugin: cache_win_data.before(StatsUpdate)),
            // and a system cannot be both inside a set and individually before a
            // set-level ordering constraint.
            .add_systems(
                Update,
                update_stats_on_new_game.before(GameMutation),
            )
            .add_systems(
                Update,
                update_stats_on_win.after(GameMutation).in_set(StatsUpdate),
            )
            .add_systems(
                Update,
                handle_forfeit.before(GameMutation),
            )
            .add_systems(Update, toggle_stats_screen.after(GameMutation));
    }
}

fn persist(path: &StatsStoragePath, stats: &StatsSnapshot, context: &str) {
    let Some(target) = &path.0 else {
        return;
    };
    if let Err(e) = save_stats_to(target, stats) {
        warn!("failed to save stats after {context}: {e}");
    }
}

fn update_stats_on_win(
    mut events: MessageReader<GameWonEvent>,
    game: Res<GameStateResource>,
    mut stats: ResMut<StatsResource>,
    path: Res<StatsStoragePath>,
) {
    for ev in events.read() {
        stats
            .0
            .update_on_win(ev.score, ev.time_seconds, &game.0.draw_mode);
        persist(&path, &stats.0, "win");
    }
}

fn update_stats_on_new_game(
    mut events: MessageReader<NewGameRequestEvent>,
    game: Res<GameStateResource>,
    mut stats: ResMut<StatsResource>,
    path: Res<StatsStoragePath>,
    mut toast: MessageWriter<InfoToastEvent>,
) {
    for _ in events.read() {
        if game.0.move_count > 0 && !game.0.is_won {
            let streak = stats.0.win_streak_current;
            stats.0.record_abandoned();
            persist(&path, &stats.0, "abandoned game");
            if streak > 1 {
                toast.write(InfoToastEvent(format!("Streak of {streak} broken!")));
            }
        }
    }
}

/// When the player presses G to forfeit, record the game as abandoned, save
/// stats, fire an informational toast, and start a new game.
///
/// `AutoCompleteState` is reset here so the "AUTO" badge and chime do not bleed
/// into the new deal (task #41).
fn handle_forfeit(
    mut events: MessageReader<ForfeitEvent>,
    game: Res<GameStateResource>,
    mut stats: ResMut<StatsResource>,
    path: Res<StatsStoragePath>,
    mut new_game: MessageWriter<NewGameRequestEvent>,
    mut toast: MessageWriter<InfoToastEvent>,
    mut auto_complete: Option<ResMut<AutoCompleteState>>,
) {
    for _ in events.read() {
        if game.0.move_count > 0 && !game.0.is_won {
            let streak = stats.0.win_streak_current;
            stats.0.record_abandoned();
            persist(&path, &stats.0, "forfeit");
            if streak > 1 {
                toast.write(InfoToastEvent(format!("Streak of {streak} broken!")));
            }
        }
        // Reset auto-complete so the badge and chime don't carry over to the
        // new game that is about to start.
        if let Some(ref mut ac) = auto_complete {
            **ac = AutoCompleteState::default();
        }
        toast.write(InfoToastEvent("Game forfeited".to_string()));
        new_game.write(NewGameRequestEvent::default());
    }
}

fn toggle_stats_screen(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    stats: Res<StatsResource>,
    progress: Option<Res<ProgressResource>>,
    time_attack: Option<Res<TimeAttackResource>>,
    screens: Query<Entity, With<StatsScreen>>,
) {
    if !keys.just_pressed(KeyCode::KeyS) {
        return;
    }
    if let Ok(entity) = screens.single() {
        commands.entity(entity).despawn();
    } else {
        spawn_stats_screen(
            &mut commands,
            &stats.0,
            progress.as_deref().map(|p| &p.0),
            time_attack.as_deref(),
        );
    }
}

fn spawn_stats_screen(
    commands: &mut Commands,
    stats: &StatsSnapshot,
    progress: Option<&PlayerProgress>,
    time_attack: Option<&TimeAttackResource>,
) {
    // --- primary stat cells (tasks #65, #66, and #38) ---
    let win_rate_str  = format_win_rate(stats);
    let played_str    = format_stat_value(stats.games_played);
    let won_str       = format_stat_value(stats.games_won);
    let lost_str      = format_stat_value(stats.games_lost);
    let fastest_str   = format_fastest_win(stats.fastest_win_seconds);
    let avg_time_str  = format_avg_time(stats);
    let best_score_str = format_optional_u32(stats.best_single_score);
    let best_streak_str = format_stat_value(stats.win_streak_best);

    commands
        .spawn((
            StatsScreen,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Percent(0.0),
                top: Val::Percent(0.0),
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::FlexStart,
                align_items: AlignItems::Center,
                row_gap: Val::Px(6.0),
                padding: UiRect::all(Val::Px(24.0)),
                overflow: Overflow::clip(),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.88)),
            ZIndex(200),
        ))
        .with_children(|root| {
            // Title
            root.spawn((
                Text::new("Statistics"),
                TextFont { font_size: 28.0, ..default() },
                TextColor(Color::srgb(1.0, 0.85, 0.3)),
            ));

            // Two-column grid of stat cells
            root.spawn(Node {
                flex_direction: FlexDirection::Row,
                flex_wrap: FlexWrap::Wrap,
                justify_content: JustifyContent::Center,
                align_items: AlignItems::FlexStart,
                column_gap: Val::Px(24.0),
                row_gap: Val::Px(16.0),
                width: Val::Percent(100.0),
                margin: UiRect::top(Val::Px(16.0)),
                ..default()
            })
            .with_children(|grid| {
                spawn_stat_cell(grid, &win_rate_str,    "Win Rate");
                spawn_stat_cell(grid, &played_str,      "Games Played");
                spawn_stat_cell(grid, &won_str,         "Games Won");
                spawn_stat_cell(grid, &lost_str,        "Games Lost");
                spawn_stat_cell(grid, &fastest_str,     "Fastest Win");
                spawn_stat_cell(grid, &avg_time_str,    "Avg Time");
                spawn_stat_cell(grid, &best_score_str,  "Best Score");
                spawn_stat_cell(grid, &best_streak_str, "Best Streak");
            });

            // Progression section
            if let Some(p) = progress {
                root.spawn((
                    Text::new("Progression"),
                    TextFont { font_size: 22.0, ..default() },
                    TextColor(Color::srgb(0.7, 0.9, 1.0)),
                ));

                let level_str = format_stat_value(p.level);
                let xp_str    = format_stat_value(p.total_xp as u32);
                let next_label = xp_to_next_level_label(p.total_xp, p.level);
                let daily_str  = format_stat_value(p.daily_challenge_streak);
                let challenge_str = challenge_progress_label(p.challenge_index);

                root.spawn(Node {
                    flex_direction: FlexDirection::Row,
                    flex_wrap: FlexWrap::Wrap,
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::FlexStart,
                    column_gap: Val::Px(24.0),
                    row_gap: Val::Px(12.0),
                    width: Val::Percent(100.0),
                    ..default()
                })
                .with_children(|grid| {
                    spawn_stat_cell(grid, &level_str,     "Level");
                    spawn_stat_cell(grid, &xp_str,        "Total XP");
                    spawn_stat_cell(grid, &next_label,    "Next Level");
                    spawn_stat_cell(grid, &daily_str,     "Daily Streak");
                    spawn_stat_cell(grid, &challenge_str, "Challenge");
                });

                // Weekly goals row
                root.spawn((
                    Text::new("Weekly Goals"),
                    TextFont { font_size: 18.0, ..default() },
                    TextColor(Color::srgb(0.8, 0.8, 0.8)),
                ));
                for goal in WEEKLY_GOALS {
                    let pv = p.weekly_goal_progress.get(goal.id).copied().unwrap_or(0);
                    root.spawn((
                        Text::new(format!("  {}: {}/{}", goal.description, pv, goal.target)),
                        TextFont { font_size: 16.0, ..default() },
                        TextColor(Color::srgb(0.85, 0.85, 0.80)),
                    ));
                }

                // Unlocks row
                root.spawn((
                    Text::new(format!(
                        "Card Backs: {}  |  Backgrounds: {}",
                        format_id_list(&p.unlocked_card_backs),
                        format_id_list(&p.unlocked_backgrounds),
                    )),
                    TextFont { font_size: 16.0, ..default() },
                    TextColor(Color::srgb(0.75, 0.75, 0.75)),
                ));
            }

            // Time Attack section
            if let Some(ta) = time_attack
                && ta.active {
                    let mins = (ta.remaining_secs / 60.0).floor() as u64;
                    let secs = (ta.remaining_secs % 60.0).floor() as u64;
                    root.spawn((
                        Text::new(format!("Time Attack — {mins}m {secs:02}s left  |  Wins: {}", ta.wins)),
                        TextFont { font_size: 18.0, ..default() },
                        TextColor(Color::srgb(1.0, 0.6, 0.2)),
                    ));
                }

            // Dismiss hint
            root.spawn((
                Text::new("Press S to close"),
                TextFont { font_size: 16.0, ..default() },
                TextColor(Color::srgb(0.6, 0.6, 0.6)),
            ));
        });
}

/// Spawn a single stat cell: a large value label on top and a small grey
/// descriptor below, inside a fixed-width column node with a [`StatsCell`] marker.
fn spawn_stat_cell(parent: &mut ChildSpawnerCommands, value: &str, label: &str) {
    parent
        .spawn((
            StatsCell,
            Node {
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                min_width: Val::Px(110.0),
                padding: UiRect::all(Val::Px(8.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.06)),
        ))
        .with_children(|cell| {
            // Large value label.
            cell.spawn((
                Text::new(value.to_string()),
                TextFont { font_size: 32.0, ..default() },
                TextColor(Color::srgb(1.0, 1.0, 1.0)),
            ));
            // Small descriptor below.
            cell.spawn((
                Text::new(label.to_string()),
                TextFont { font_size: 14.0, ..default() },
                TextColor(Color::srgb(0.65, 0.65, 0.65)),
            ));
        });
}

/// Format a win-rate value for display.
///
/// Returns `"—"` when no games have been played, otherwise `"N%"`.
pub fn format_win_rate(stats: &StatsSnapshot) -> String {
    match stats.win_rate() {
        None => "\u{2014}".to_string(),
        Some(r) => format!("{}%", (r) as u32),
    }
}

/// Format `fastest_win_seconds` for display.
///
/// Returns `"—"` when the value is `u64::MAX` (sentinel for "no wins yet") or
/// zero. Otherwise delegates to [`format_duration`].
pub fn format_fastest_win(fastest_win_seconds: u64) -> String {
    if fastest_win_seconds == u64::MAX || fastest_win_seconds == 0 {
        "\u{2014}".to_string()
    } else {
        format_duration(fastest_win_seconds)
    }
}

/// Format `avg_time_seconds` for display.
///
/// Returns `"—"` when no games have been won yet (`games_won == 0`), otherwise
/// delegates to [`format_duration`].
pub fn format_avg_time(stats: &StatsSnapshot) -> String {
    if stats.games_won == 0 {
        "\u{2014}".to_string()
    } else {
        format_duration(stats.avg_time_seconds)
    }
}

/// Format an optional `u32` statistic.
///
/// Returns `"—"` when `value` is `0`, otherwise the decimal representation.
pub fn format_optional_u32(value: u32) -> String {
    if value == 0 {
        "\u{2014}".to_string()
    } else {
        value.to_string()
    }
}

/// Format any `u32`-like stat value as a decimal string.
///
/// Unlike [`format_optional_u32`], this always shows the number (even if zero).
pub fn format_stat_value<T: std::fmt::Display>(value: T) -> String {
    format!("{value}")
}

/// Returns XP remaining until next level, formatted as "N XP (P%)".
fn xp_to_next_level_label(total_xp: u64, level: u32) -> String {
    let xp_current = if level < 10 {
        level as u64 * 500
    } else {
        5_000 + (level as u64 - 10) * 1_000
    };
    let xp_next = if level < 10 {
        (level as u64 + 1) * 500
    } else {
        5_000 + (level as u64 - 9) * 1_000
    };
    let span = xp_next - xp_current;
    let done = total_xp.saturating_sub(xp_current).min(span);
    let pct = if span == 0 { 100 } else { done.saturating_mul(100).checked_div(span).unwrap_or(100) };
    let remaining = span - done;
    format!("{remaining} XP ({pct}%)")
}

/// Format a duration given in whole seconds as `"M:SS"`.
///
/// Example: `90` → `"1:30"`.
pub fn format_duration(secs: u64) -> String {
    let m = secs / 60;
    let s = secs % 60;
    format!("{m}:{s:02}")
}

/// Renders a sorted, comma-separated list of unlock indexes for the overlay.
/// Empty list shows as "None".
fn format_id_list(ids: &[usize]) -> String {
    if ids.is_empty() {
        return "None".to_string();
    }
    let mut sorted: Vec<usize> = ids.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    sorted
        .iter()
        .map(|i| format!("#{i}"))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game_plugin::GamePlugin;
    use crate::table_plugin::TablePlugin;

    fn headless_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(GamePlugin)
            .add_plugins(TablePlugin)
            .add_plugins(StatsPlugin::headless());
        // MinimalPlugins doesn't register keyboard input — add it so the
        // toggle system can read ButtonInput<KeyCode> in tests.
        app.init_resource::<ButtonInput<KeyCode>>();
        // ProgressResource is an optional dependency for the stats screen;
        // include it so toggle tests exercise the progression panel.
        app.add_plugins(crate::progress_plugin::ProgressPlugin::headless());
        app.update();
        app
    }

    #[test]
    fn stats_resource_exists_after_startup() {
        let app = headless_app();
        assert!(app.world().get_resource::<StatsResource>().is_some());
    }

    #[test]
    fn headless_plugin_starts_with_default_stats() {
        let app = headless_app();
        let stats = &app.world().resource::<StatsResource>().0;
        assert_eq!(stats, &StatsSnapshot::default());
    }

    #[test]
    fn win_event_increments_games_won() {
        let mut app = headless_app();
        app.world_mut().write_message(GameWonEvent {
            score: 1000,
            time_seconds: 120,
        });
        app.update();

        let stats = &app.world().resource::<StatsResource>().0;
        assert_eq!(stats.games_won, 1);
        assert_eq!(stats.games_played, 1);
    }

    #[test]
    fn draw_three_win_increments_draw_three_wins_only() {
        let mut app = headless_app();
        app.world_mut()
            .resource_mut::<crate::resources::GameStateResource>()
            .0
            .draw_mode = solitaire_core::game_state::DrawMode::DrawThree;

        app.world_mut().write_message(GameWonEvent {
            score: 500,
            time_seconds: 200,
        });
        app.update();

        let stats = &app.world().resource::<StatsResource>().0;
        assert_eq!(stats.draw_three_wins, 1, "draw_three_wins must increment for DrawThree mode");
        assert_eq!(stats.draw_one_wins, 0, "draw_one_wins must not increment for DrawThree mode");
    }

    #[test]
    fn new_game_after_moves_records_abandoned() {
        let mut app = headless_app();

        app.world_mut()
            .resource_mut::<crate::resources::GameStateResource>()
            .0
            .move_count = 3;

        app.world_mut()
            .write_message(NewGameRequestEvent { seed: Some(999), mode: None });
        app.update();

        let stats = &app.world().resource::<StatsResource>().0;
        assert_eq!(stats.games_played, 1);
        assert_eq!(stats.games_lost, 1);
        assert_eq!(stats.win_streak_current, 0);
    }

    #[test]
    fn new_game_without_moves_does_not_record_abandoned() {
        let mut app = headless_app();
        app.world_mut()
            .write_message(NewGameRequestEvent { seed: Some(42), mode: None });
        app.update();

        let stats = &app.world().resource::<StatsResource>().0;
        assert_eq!(stats.games_played, 0);
    }

    #[test]
    fn pressing_s_spawns_stats_screen() {
        let mut app = headless_app();
        assert_eq!(
            app.world_mut()
                .query::<&StatsScreen>()
                .iter(app.world())
                .count(),
            0
        );

        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyS);
        app.update();

        assert_eq!(
            app.world_mut()
                .query::<&StatsScreen>()
                .iter(app.world())
                .count(),
            1
        );
    }

    #[test]
    fn pressing_s_twice_closes_stats_screen() {
        let mut app = headless_app();

        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyS);
        app.update();

        // Release + clear + press: `press()` is a no-op if the key is already
        // in `pressed`, and MinimalPlugins doesn't include bevy_input's
        // per-frame updater to drain `just_pressed`, so we cycle manually.
        {
            let mut input = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            input.release(KeyCode::KeyS);
            input.clear();
            input.press(KeyCode::KeyS);
        }
        app.update();

        assert_eq!(
            app.world_mut()
                .query::<&StatsScreen>()
                .iter(app.world())
                .count(),
            0
        );
    }

    #[test]
    fn format_id_list_renders_empty_as_none() {
        assert_eq!(format_id_list(&[]), "None");
    }

    #[test]
    fn format_id_list_sorts_dedups_and_prefixes() {
        assert_eq!(format_id_list(&[3, 1, 1, 2]), "#1, #2, #3");
    }

    #[test]
    fn xp_to_next_level_label_at_zero_xp() {
        // Level 0, 0 XP: 500 needed, 0% done.
        assert_eq!(xp_to_next_level_label(0, 0), "500 XP (0%)");
    }

    #[test]
    fn xp_to_next_level_label_halfway_through_level_1() {
        // Level 1 starts at 500 XP, level 2 at 1000 XP.
        // At 750 XP: 250 done of 500, 50%, 250 remaining.
        assert_eq!(xp_to_next_level_label(750, 1), "250 XP (50%)");
    }

    #[test]
    fn xp_to_next_level_label_at_level_10_boundary() {
        // Level 10 starts at 5000 XP, level 11 at 6000 XP.
        // At 5000 XP: 0 done, 0%, 1000 remaining.
        assert_eq!(xp_to_next_level_label(5_000, 10), "1000 XP (0%)");
    }

    // -----------------------------------------------------------------------
    // format_duration
    // -----------------------------------------------------------------------

    #[test]
    fn format_duration_zero_seconds() {
        assert_eq!(format_duration(0), "0:00");
    }

    #[test]
    fn format_duration_pads_seconds_to_two_digits() {
        assert_eq!(format_duration(65), "1:05");
    }

    #[test]
    fn format_duration_exactly_one_hour() {
        assert_eq!(format_duration(3600), "60:00");
    }

    #[test]
    fn format_duration_handles_sub_minute() {
        assert_eq!(format_duration(59), "0:59");
    }

    // -----------------------------------------------------------------------
    // Task #65 — win rate and stat cell pure-function tests
    // -----------------------------------------------------------------------

    #[test]
    fn format_win_rate_zero() {
        // 0 wins, 0 played → "—"
        let s = StatsSnapshot::default();
        assert_eq!(format_win_rate(&s), "\u{2014}");
    }

    #[test]
    fn format_win_rate_half() {
        // 5 wins out of 10 played → "50%"
        let s = StatsSnapshot {
            games_played: 10,
            games_won: 5,
            ..StatsSnapshot::default()
        };
        assert_eq!(format_win_rate(&s), "50%");
    }

    #[test]
    fn format_stat_value_zero_returns_zero() {
        assert_eq!(format_stat_value(0u32), "0");
    }

    // -----------------------------------------------------------------------
    // Task #66 — fastest win, best score, streak pure-function tests
    // -----------------------------------------------------------------------

    #[test]
    fn format_fastest_win_unset() {
        // fastest_win_seconds == u64::MAX → "—"
        assert_eq!(format_fastest_win(u64::MAX), "\u{2014}");
    }

    #[test]
    fn format_fastest_win_90s() {
        // 90 seconds → "1:30"
        assert_eq!(format_fastest_win(90), "1:30");
    }

    #[test]
    fn best_score_display_zero() {
        // best_single_score == 0 → "—"
        assert_eq!(format_optional_u32(0), "\u{2014}");
    }

    // -----------------------------------------------------------------------
    // Task #38 — avg time pure-function tests
    // -----------------------------------------------------------------------

    #[test]
    fn format_avg_time_no_wins_shows_dash() {
        // games_won == 0 → "—"
        let s = StatsSnapshot::default();
        assert_eq!(format_avg_time(&s), "\u{2014}");
    }

    #[test]
    fn format_avg_time_after_single_win() {
        // After one win of 90 s avg should be "1:30"
        let s = StatsSnapshot {
            games_won: 1,
            avg_time_seconds: 90,
            ..StatsSnapshot::default()
        };
        assert_eq!(format_avg_time(&s), "1:30");
    }

    #[test]
    fn format_avg_time_after_multiple_wins() {
        // avg_time_seconds = 200 s → "3:20"
        let s = StatsSnapshot {
            games_won: 3,
            avg_time_seconds: 200,
            ..StatsSnapshot::default()
        };
        assert_eq!(format_avg_time(&s), "3:20");
    }

    // -----------------------------------------------------------------------
    // Task #49 — streak-broken toast on forfeit
    // -----------------------------------------------------------------------

    #[test]
    fn forfeit_with_streak_fires_streak_broken_toast() {
        let mut app = headless_app();

        // Set up a streak of 3 and at least one move so forfeit counts.
        {
            let mut stats = app.world_mut().resource_mut::<StatsResource>();
            stats.0.win_streak_current = 3;
        }
        app.world_mut()
            .resource_mut::<crate::resources::GameStateResource>()
            .0
            .move_count = 1;

        app.world_mut().write_message(ForfeitEvent);
        app.update();

        let events = app.world().resource::<Messages<InfoToastEvent>>();
        let mut reader = events.get_cursor();
        let messages: Vec<&str> = reader
            .read(events)
            .map(|e| e.0.as_str())
            .collect();

        assert!(
            messages.contains(&"Streak of 3 broken!"),
            "expected 'Streak of 3 broken!' in toasts, got: {messages:?}"
        );
    }

    #[test]
    fn forfeit_with_streak_of_one_does_not_fire_streak_broken_toast() {
        let mut app = headless_app();

        {
            let mut stats = app.world_mut().resource_mut::<StatsResource>();
            stats.0.win_streak_current = 1;
        }
        app.world_mut()
            .resource_mut::<crate::resources::GameStateResource>()
            .0
            .move_count = 1;

        app.world_mut().write_message(ForfeitEvent);
        app.update();

        let events = app.world().resource::<Messages<InfoToastEvent>>();
        let mut reader = events.get_cursor();
        let messages: Vec<&str> = reader
            .read(events)
            .map(|e| e.0.as_str())
            .collect();

        assert!(
            !messages.iter().any(|m| m.contains("broken")),
            "expected no streak-broken toast for streak of 1, got: {messages:?}"
        );
    }
}
