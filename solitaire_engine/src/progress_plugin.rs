//! Awards XP on `GameWonEvent`, persists `PlayerProgress`, and emits a
//! `LevelUpEvent` when a win pushes the player to a new level.
//!
//! Configurable storage path:
//! - `ProgressPlugin::default()` uses the platform data dir
//! - `ProgressPlugin::headless()` disables I/O for tests

use std::path::PathBuf;

use bevy::prelude::*;
use solitaire_data::{
    load_progress_from, progress_file_path, save_progress_to, xp_for_win, PlayerProgress,
};

use crate::events::GameWonEvent;
use crate::game_plugin::GameMutation;
use crate::resources::GameStateResource;

/// Bevy resource wrapping the current `PlayerProgress`.
#[derive(Resource, Debug, Clone)]
pub struct ProgressResource(pub PlayerProgress);

/// Persistence path for `ProgressResource`. `None` disables I/O.
#[derive(Resource, Debug, Clone)]
pub struct ProgressStoragePath(pub Option<PathBuf>);

/// Fired when a win pushes the player to a new level.
#[derive(Event, Debug, Clone, Copy)]
pub struct LevelUpEvent {
    pub previous_level: u32,
    pub new_level: u32,
    pub total_xp: u64,
}

/// System set for the progress-mutating systems. Downstream plugins that
/// read `ProgressResource` after a win should run `.after(ProgressUpdate)`.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProgressUpdate;

pub struct ProgressPlugin {
    pub storage_path: Option<PathBuf>,
}

impl Default for ProgressPlugin {
    fn default() -> Self {
        Self {
            storage_path: progress_file_path(),
        }
    }
}

impl ProgressPlugin {
    /// Plugin configured with no persistence — for tests and headless apps.
    pub fn headless() -> Self {
        Self { storage_path: None }
    }
}

impl Plugin for ProgressPlugin {
    fn build(&self, app: &mut App) {
        let loaded = match &self.storage_path {
            Some(path) => load_progress_from(path),
            None => PlayerProgress::default(),
        };
        app.insert_resource(ProgressResource(loaded))
            .insert_resource(ProgressStoragePath(self.storage_path.clone()))
            .add_event::<LevelUpEvent>()
            .add_event::<GameWonEvent>()
            .add_systems(
                Update,
                award_xp_on_win
                    .after(GameMutation)
                    .in_set(ProgressUpdate),
            );
    }
}

fn award_xp_on_win(
    mut wins: EventReader<GameWonEvent>,
    mut levelups: EventWriter<LevelUpEvent>,
    game: Res<GameStateResource>,
    path: Res<ProgressStoragePath>,
    mut progress: ResMut<ProgressResource>,
) {
    for ev in wins.read() {
        let used_undo = game.0.undo_count > 0;
        let amount = xp_for_win(ev.time_seconds, used_undo);
        let prev_level = progress.0.add_xp(amount);
        if progress.0.leveled_up_from(prev_level) {
            levelups.send(LevelUpEvent {
                previous_level: prev_level,
                new_level: progress.0.level,
                total_xp: progress.0.total_xp,
            });
        }
        if let Some(target) = &path.0 {
            if let Err(e) = save_progress_to(target, &progress.0) {
                warn!("failed to save progress: {e}");
            }
        }
    }
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
            .add_plugins(ProgressPlugin::headless());
        app.update();
        app
    }

    #[test]
    fn progress_resource_starts_at_default() {
        let app = headless_app();
        let p = &app.world().resource::<ProgressResource>().0;
        assert_eq!(p, &PlayerProgress::default());
    }

    #[test]
    fn win_awards_base_xp() {
        let mut app = headless_app();
        // Game starts with undo_count = 0, so the no-undo bonus applies.
        app.world_mut().send_event(GameWonEvent {
            score: 500,
            time_seconds: 300, // no speed bonus
        });
        app.update();

        let xp = app.world().resource::<ProgressResource>().0.total_xp;
        // base 50 + no_undo 25 = 75
        assert_eq!(xp, 75);
    }

    #[test]
    fn win_after_undo_grants_no_undo_bonus_off() {
        let mut app = headless_app();
        app.world_mut()
            .resource_mut::<GameStateResource>()
            .0
            .undo_count = 1;

        app.world_mut().send_event(GameWonEvent {
            score: 500,
            time_seconds: 300,
        });
        app.update();

        let xp = app.world().resource::<ProgressResource>().0.total_xp;
        // base 50 only, since undo was used
        assert_eq!(xp, 50);
    }

    #[test]
    fn fast_win_includes_speed_bonus() {
        let mut app = headless_app();
        app.world_mut().send_event(GameWonEvent {
            score: 500,
            time_seconds: 0,
        });
        app.update();

        // base 50 + speed 50 + no_undo 25 = 125
        let xp = app.world().resource::<ProgressResource>().0.total_xp;
        assert_eq!(xp, 125);
    }

    #[test]
    fn crossing_500_xp_fires_levelup_event() {
        let mut app = headless_app();
        // Pre-load 480 XP so a 75-XP win pushes us over the 500 boundary.
        app.world_mut().resource_mut::<ProgressResource>().0.total_xp = 480;

        app.world_mut().send_event(GameWonEvent {
            score: 500,
            time_seconds: 300,
        });
        app.update();

        let events = app.world().resource::<Events<LevelUpEvent>>();
        let mut cursor = events.get_cursor();
        let fired: Vec<_> = cursor.read(events).copied().collect();
        assert_eq!(fired.len(), 1, "exactly one level-up");
        assert_eq!(fired[0].previous_level, 0);
        assert_eq!(fired[0].new_level, 1);
    }

    #[test]
    fn win_without_level_change_does_not_fire_levelup() {
        let mut app = headless_app();
        app.world_mut().send_event(GameWonEvent {
            score: 500,
            time_seconds: 300,
        });
        app.update();

        let events = app.world().resource::<Events<LevelUpEvent>>();
        let mut cursor = events.get_cursor();
        assert!(cursor.read(events).next().is_none());
    }
}
