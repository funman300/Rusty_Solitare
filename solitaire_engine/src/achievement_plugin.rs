//! Evaluates achievements on `GameWonEvent`, persists unlocks, and fires
//! `AchievementUnlockedEvent` for each newly unlocked achievement.
//!
//! The persistence path is configurable via `AchievementPlugin::storage_path`.
//! `AchievementPlugin::default()` uses the platform data dir;
//! `AchievementPlugin::headless()` disables I/O entirely (for tests).

use std::path::PathBuf;

use bevy::prelude::*;
use chrono::{Local, Timelike, Utc};
use solitaire_core::achievement::{
    achievement_by_id, check_achievements, AchievementContext, ALL_ACHIEVEMENTS,
};
use solitaire_data::{
    achievements_file_path, load_achievements_from, save_achievements_to, AchievementRecord,
};

use crate::events::{AchievementUnlockedEvent, GameWonEvent};
use crate::game_plugin::GameMutation;
use crate::progress_plugin::{ProgressResource, ProgressUpdate};
use crate::resources::GameStateResource;
use crate::stats_plugin::{StatsResource, StatsUpdate};

/// All per-player achievement records (one per known achievement).
#[derive(Resource, Debug, Clone)]
pub struct AchievementsResource(pub Vec<AchievementRecord>);

/// Persistence path for `AchievementsResource`. `None` disables I/O.
#[derive(Resource, Debug, Clone)]
pub struct AchievementsStoragePath(pub Option<PathBuf>);

pub struct AchievementPlugin {
    pub storage_path: Option<PathBuf>,
}

impl Default for AchievementPlugin {
    fn default() -> Self {
        Self {
            storage_path: achievements_file_path(),
        }
    }
}

impl AchievementPlugin {
    /// Plugin configured with no persistence.
    pub fn headless() -> Self {
        Self { storage_path: None }
    }
}

impl Plugin for AchievementPlugin {
    fn build(&self, app: &mut App) {
        let mut records = match &self.storage_path {
            Some(path) => load_achievements_from(path),
            None => Vec::new(),
        };
        // Ensure every known achievement has a record. Keeps file forward-compatible
        // when new achievements are added in future releases.
        for def in ALL_ACHIEVEMENTS {
            if !records.iter().any(|r| r.id == def.id) {
                records.push(AchievementRecord::locked(def.id));
            }
        }

        app.insert_resource(AchievementsResource(records))
            .insert_resource(AchievementsStoragePath(self.storage_path.clone()))
            .add_event::<AchievementUnlockedEvent>()
            .add_event::<GameWonEvent>()
            // Run after GameMutation (so GameWonEvent is available), after
            // StatsUpdate (so stats reflect this win), and after ProgressUpdate
            // (so daily_challenge_streak is up to date for daily_devotee).
            .add_systems(
                Update,
                evaluate_on_win
                    .after(GameMutation)
                    .after(StatsUpdate)
                    .after(ProgressUpdate),
            );
    }
}

#[allow(clippy::too_many_arguments)]
fn evaluate_on_win(
    mut wins: EventReader<GameWonEvent>,
    mut unlocks: EventWriter<AchievementUnlockedEvent>,
    game: Res<GameStateResource>,
    stats: Res<StatsResource>,
    progress: Res<ProgressResource>,
    path: Res<AchievementsStoragePath>,
    mut achievements: ResMut<AchievementsResource>,
) {
    let Some(ev) = wins.read().last() else {
        return;
    };

    let ctx = AchievementContext {
        games_played: stats.0.games_played,
        games_won: stats.0.games_won,
        win_streak_current: stats.0.win_streak_current,
        best_single_score: stats.0.best_single_score,
        lifetime_score: stats.0.lifetime_score,
        draw_three_wins: stats.0.draw_three_wins,
        daily_challenge_streak: progress.0.daily_challenge_streak,
        last_win_score: ev.score,
        last_win_time_seconds: ev.time_seconds,
        last_win_used_undo: game.0.undo_count > 0,
        wall_clock_hour: Some(Local::now().hour()),
    };

    let hits = check_achievements(&ctx);
    if hits.is_empty() {
        return;
    }

    let now = Utc::now();
    let mut changed = false;
    for def in hits {
        let Some(record) = achievements.0.iter_mut().find(|r| r.id == def.id) else {
            continue;
        };
        if record.unlocked {
            continue;
        }
        record.unlock(now);
        changed = true;
        unlocks.send(AchievementUnlockedEvent(record.clone()));
    }

    if changed {
        if let Some(target) = &path.0 {
            if let Err(e) = save_achievements_to(target, &achievements.0) {
                warn!("failed to save achievements: {e}");
            }
        }
    }
}

/// Convenience: resolve an achievement ID to its human-readable name.
/// Used by the toast renderer in `animation_plugin`.
pub fn display_name_for(id: &str) -> String {
    achievement_by_id(id)
        .map(|d| d.name.to_string())
        .unwrap_or_else(|| id.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game_plugin::GamePlugin;
    use crate::stats_plugin::StatsPlugin;
    use crate::table_plugin::TablePlugin;

    fn headless_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(GamePlugin)
            .add_plugins(TablePlugin)
            .add_plugins(StatsPlugin::headless())
            .add_plugins(crate::progress_plugin::ProgressPlugin::headless())
            .add_plugins(AchievementPlugin::headless());
        // StatsPlugin's UI toggle system reads ButtonInput<KeyCode>; under
        // MinimalPlugins it isn't auto-registered.
        app.init_resource::<bevy::input::ButtonInput<KeyCode>>();
        app.update();
        app
    }

    #[test]
    fn resource_is_populated_with_all_known_ids() {
        let app = headless_app();
        let records = &app.world().resource::<AchievementsResource>().0;
        assert_eq!(records.len(), ALL_ACHIEVEMENTS.len());
        for def in ALL_ACHIEVEMENTS {
            assert!(records.iter().any(|r| r.id == def.id && !r.unlocked));
        }
    }

    #[test]
    fn win_unlocks_first_win_and_fires_event() {
        let mut app = headless_app();

        // StatsPlugin runs update_stats_on_win first (after GameMutation); that
        // bumps games_won to 1 before evaluate_on_win reads StatsResource.
        app.world_mut().send_event(GameWonEvent {
            score: 1000,
            time_seconds: 300,
        });
        app.update();

        let unlocked_first_win = app
            .world()
            .resource::<AchievementsResource>()
            .0
            .iter()
            .find(|r| r.id == "first_win")
            .map(|r| r.unlocked)
            .unwrap_or(false);
        assert!(unlocked_first_win);

        // Verify the event was emitted.
        let events = app.world().resource::<Events<AchievementUnlockedEvent>>();
        let mut cursor = events.get_cursor();
        let fired: Vec<String> = cursor.read(events).map(|e| e.0.id.clone()).collect();
        assert!(fired.contains(&"first_win".to_string()));
    }

    #[test]
    fn repeated_win_does_not_refire_already_unlocked_achievement() {
        let mut app = headless_app();

        app.world_mut().send_event(GameWonEvent {
            score: 1000,
            time_seconds: 300,
        });
        app.update();

        // Clear events from first win.
        app.world_mut()
            .resource_mut::<Events<AchievementUnlockedEvent>>()
            .clear();

        app.world_mut().send_event(GameWonEvent {
            score: 1000,
            time_seconds: 300,
        });
        app.update();

        let events = app.world().resource::<Events<AchievementUnlockedEvent>>();
        let mut cursor = events.get_cursor();
        let fired: Vec<String> = cursor.read(events).map(|e| e.0.id.clone()).collect();
        assert!(
            !fired.contains(&"first_win".to_string()),
            "first_win must not re-fire on subsequent wins"
        );
    }

    #[test]
    fn display_name_resolves_known_and_unknown_ids() {
        assert_eq!(display_name_for("first_win"), "First Win");
        assert_eq!(display_name_for("bogus"), "bogus");
    }
}
