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

use crate::events::{GameWonEvent, NewGameRequestEvent};
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
            .add_event::<GameWonEvent>()
            .add_event::<NewGameRequestEvent>()
            // record_abandoned must read `move_count` BEFORE handle_new_game
            // clobbers it with a fresh game.
            .add_systems(
                Update,
                update_stats_on_new_game
                    .before(GameMutation)
                    .in_set(StatsUpdate),
            )
            .add_systems(
                Update,
                update_stats_on_win.after(GameMutation).in_set(StatsUpdate),
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
    mut events: EventReader<GameWonEvent>,
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
    mut events: EventReader<NewGameRequestEvent>,
    game: Res<GameStateResource>,
    mut stats: ResMut<StatsResource>,
    path: Res<StatsStoragePath>,
) {
    for _ in events.read() {
        if game.0.move_count > 0 && !game.0.is_won {
            stats.0.record_abandoned();
            persist(&path, &stats.0, "abandoned game");
        }
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
    if let Ok(entity) = screens.get_single() {
        commands.entity(entity).despawn_recursive();
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
    let win_rate = stats
        .win_rate()
        .map_or("N/A".to_string(), |r| format!("{r:.1}%"));
    let fastest = if stats.fastest_win_seconds == u64::MAX {
        "N/A".to_string()
    } else {
        format_duration(stats.fastest_win_seconds)
    };
    let avg = if stats.games_won == 0 {
        "N/A".to_string()
    } else {
        format_duration(stats.avg_time_seconds)
    };

    let mut lines: Vec<String> = vec![
        "=== Statistics ===".to_string(),
        format!("Games Played:  {}", stats.games_played),
        format!("Games Won:     {}", stats.games_won),
        format!("Win Rate:      {win_rate}"),
        format!(
            "Win Streak:    {} (Best: {})",
            stats.win_streak_current, stats.win_streak_best
        ),
        format!("Best Score:    {}", stats.best_single_score),
        format!("Fastest Win:   {fastest}"),
        format!("Avg Win Time:  {avg}"),
    ];

    if let Some(p) = progress {
        lines.push(String::new());
        lines.push("=== Progression ===".to_string());
        lines.push(format!("Level:         {}", p.level));
        lines.push(format!("Total XP:      {}", p.total_xp));
        lines.push(format!(
            "Daily Streak:  {}",
            p.daily_challenge_streak
        ));
        lines.push(String::new());
        lines.push("-- Weekly Goals --".to_string());
        for goal in WEEKLY_GOALS {
            let progress_value = p
                .weekly_goal_progress
                .get(goal.id)
                .copied()
                .unwrap_or(0);
            lines.push(format!(
                "  {}: {}/{}",
                goal.description, progress_value, goal.target
            ));
        }
        lines.push(String::new());
        lines.push("-- Unlocks --".to_string());
        lines.push(format!(
            "  Card Backs:    {}",
            format_id_list(&p.unlocked_card_backs)
        ));
        lines.push(format!(
            "  Backgrounds:   {}",
            format_id_list(&p.unlocked_backgrounds)
        ));
    }

    if let Some(ta) = time_attack {
        if ta.active {
            let mins = (ta.remaining_secs / 60.0).floor() as u64;
            let secs = (ta.remaining_secs % 60.0).floor() as u64;
            lines.push(String::new());
            lines.push("=== Time Attack ===".to_string());
            lines.push(format!("Remaining:     {mins}m {secs:02}s"));
            lines.push(format!("Wins:          {}", ta.wins));
        }
    }

    lines.push(String::new());
    lines.push("Press S to close".to_string());

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
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                row_gap: Val::Px(6.0),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.88)),
            ZIndex(200),
        ))
        .with_children(|b| {
            for line in lines {
                b.spawn((
                    Text::new(line),
                    TextFont {
                        font_size: 24.0,
                        ..default()
                    },
                    TextColor(Color::srgb(0.95, 0.95, 0.90)),
                ));
            }
        });
}

fn format_duration(secs: u64) -> String {
    let m = secs / 60;
    let s = secs % 60;
    format!("{m}m {s:02}s")
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
        app.world_mut().send_event(GameWonEvent {
            score: 1000,
            time_seconds: 120,
        });
        app.update();

        let stats = &app.world().resource::<StatsResource>().0;
        assert_eq!(stats.games_won, 1);
        assert_eq!(stats.games_played, 1);
    }

    #[test]
    fn new_game_after_moves_records_abandoned() {
        let mut app = headless_app();

        app.world_mut()
            .resource_mut::<crate::resources::GameStateResource>()
            .0
            .move_count = 3;

        app.world_mut()
            .send_event(NewGameRequestEvent { seed: Some(999), mode: None });
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
            .send_event(NewGameRequestEvent { seed: Some(42), mode: None });
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
}
