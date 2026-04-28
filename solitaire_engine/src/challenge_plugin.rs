//! Challenge-mode bookkeeping: serves the current challenge seed, advances
//! `PlayerProgress::challenge_index` on a Challenge-mode win, persists.
//!
//! Pressing **X** starts a new game with the current Challenge seed in
//! `GameMode::Challenge` (gated by level ≥ `CHALLENGE_UNLOCK_LEVEL`).

use bevy::prelude::*;
use solitaire_core::game_state::GameMode;
use solitaire_data::{challenge_count, challenge_seed_for, save_progress_to};

use crate::events::{GameWonEvent, InfoToastEvent, NewGameRequestEvent};
use crate::game_plugin::GameMutation;
use crate::progress_plugin::{ProgressResource, ProgressStoragePath, ProgressUpdate};
use crate::resources::GameStateResource;

/// Minimum player level required to start a Challenge run.
pub const CHALLENGE_UNLOCK_LEVEL: u32 = 5;

/// Fired when the player has just completed a Challenge-mode game and the
/// `challenge_index` cursor advances.
#[derive(Message, Debug, Clone, Copy)]
pub struct ChallengeAdvancedEvent {
    pub previous_index: u32,
    pub new_index: u32,
}

pub struct ChallengePlugin;

impl Plugin for ChallengePlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<ChallengeAdvancedEvent>()
            .add_message::<GameWonEvent>()
            .add_message::<NewGameRequestEvent>()
            .add_message::<InfoToastEvent>()
            // Run after ProgressUpdate so we don't fight ProgressPlugin's add_xp.
            .add_systems(Update, advance_on_challenge_win.after(ProgressUpdate))
            .add_systems(Update, handle_start_challenge_request.before(GameMutation));
    }
}

fn advance_on_challenge_win(
    mut wins: MessageReader<GameWonEvent>,
    game: Res<GameStateResource>,
    mut progress: ResMut<ProgressResource>,
    path: Res<ProgressStoragePath>,
    mut advanced: MessageWriter<ChallengeAdvancedEvent>,
    mut toast: MessageWriter<InfoToastEvent>,
) {
    for _ in wins.read() {
        if game.0.mode != GameMode::Challenge {
            continue;
        }
        let prev = progress.0.challenge_index;
        progress.0.challenge_index = prev.saturating_add(1);
        if let Some(target) = &path.0 {
            if let Err(e) = save_progress_to(target, &progress.0) {
                warn!("failed to save progress after challenge advance: {e}");
            }
        }
        // Human-readable level is 1-based (index 0 → "Challenge 1").
        let level_number = prev.saturating_add(1);
        toast.write(InfoToastEvent(format!("Challenge {level_number} complete!")));
        advanced.write(ChallengeAdvancedEvent {
            previous_index: prev,
            new_index: progress.0.challenge_index,
        });
    }
}

fn handle_start_challenge_request(
    keys: Res<ButtonInput<KeyCode>>,
    progress: Res<ProgressResource>,
    mut new_game: MessageWriter<NewGameRequestEvent>,
    mut info_toast: MessageWriter<InfoToastEvent>,
) {
    if !keys.just_pressed(KeyCode::KeyX) {
        return;
    }
    if progress.0.level < CHALLENGE_UNLOCK_LEVEL {
        info_toast.write(InfoToastEvent(format!(
            "Challenge mode unlocks at level {CHALLENGE_UNLOCK_LEVEL}"
        )));
        return;
    }
    let Some(seed) = challenge_seed_for(progress.0.challenge_index) else {
        warn!("challenge seed list is empty");
        return;
    };
    new_game.write(NewGameRequestEvent {
        seed: Some(seed),
        mode: Some(GameMode::Challenge),
    });
}

/// Convenience for stat overlays: returns the human-friendly position
/// string `"{index + 1} / {total}"`.
pub fn challenge_progress_label(index: u32) -> String {
    format!("{} / {}", index.saturating_add(1), challenge_count())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game_plugin::GamePlugin;
    use crate::progress_plugin::ProgressPlugin;
    use crate::table_plugin::TablePlugin;
    use solitaire_core::game_state::{DrawMode, GameState};

    fn headless_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(GamePlugin)
            .add_plugins(TablePlugin)
            .add_plugins(ProgressPlugin::headless())
            .add_plugins(ChallengePlugin);
        app.init_resource::<ButtonInput<KeyCode>>();
        app.update();
        app
    }

    #[test]
    fn challenge_win_advances_index() {
        let mut app = headless_app();
        app.world_mut().resource_mut::<GameStateResource>().0 =
            GameState::new_with_mode(1, DrawMode::DrawOne, GameMode::Challenge);

        app.world_mut().write_message(GameWonEvent {
            score: 500,
            time_seconds: 100,
        });
        app.update();

        let p = &app.world().resource::<ProgressResource>().0;
        assert_eq!(p.challenge_index, 1);

        let events = app.world().resource::<Events<ChallengeAdvancedEvent>>();
        let mut cursor = events.get_cursor();
        let fired: Vec<_> = cursor.read(events).copied().collect();
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].previous_index, 0);
        assert_eq!(fired[0].new_index, 1);
    }

    #[test]
    fn classic_win_does_not_advance_challenge_index() {
        let mut app = headless_app();
        // Default GameStateResource is Classic mode.
        app.world_mut().write_message(GameWonEvent {
            score: 500,
            time_seconds: 100,
        });
        app.update();

        let p = &app.world().resource::<ProgressResource>().0;
        assert_eq!(p.challenge_index, 0);

        let events = app.world().resource::<Events<ChallengeAdvancedEvent>>();
        let mut cursor = events.get_cursor();
        assert!(cursor.read(events).next().is_none());
    }

    #[test]
    fn pressing_x_below_unlock_level_is_ignored() {
        let mut app = headless_app();
        // Default level is 0; below CHALLENGE_UNLOCK_LEVEL.
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyX);
        app.update();

        let events = app.world().resource::<Events<NewGameRequestEvent>>();
        let mut cursor = events.get_cursor();
        assert!(cursor.read(events).next().is_none());
    }

    #[test]
    fn pressing_x_at_unlock_level_fires_new_game_with_challenge_seed() {
        let mut app = headless_app();
        app.world_mut().resource_mut::<ProgressResource>().0.level =
            CHALLENGE_UNLOCK_LEVEL;
        app.world_mut()
            .resource_mut::<ProgressResource>()
            .0
            .challenge_index = 2;

        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyX);
        app.update();

        let events = app.world().resource::<Events<NewGameRequestEvent>>();
        let mut cursor = events.get_cursor();
        let fired: Vec<_> = cursor.read(events).copied().collect();
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].seed, challenge_seed_for(2));
        assert_eq!(fired[0].mode, Some(GameMode::Challenge));
    }

    #[test]
    fn challenge_progress_label_uses_human_indexing() {
        let total = challenge_count();
        assert_eq!(challenge_progress_label(0), format!("1 / {total}"));
        assert_eq!(challenge_progress_label(2), format!("3 / {total}"));
    }

    #[test]
    fn challenge_win_fires_complete_toast_with_level_number() {
        let mut app = headless_app();
        // Set challenge_index to 2 so the completed level is "Challenge 3".
        app.world_mut().resource_mut::<ProgressResource>().0.challenge_index = 2;
        app.world_mut().resource_mut::<GameStateResource>().0 =
            GameState::new_with_mode(1, DrawMode::DrawOne, GameMode::Challenge);

        app.world_mut().write_message(GameWonEvent {
            score: 500,
            time_seconds: 100,
        });
        app.update();

        let events = app.world().resource::<Events<InfoToastEvent>>();
        let mut cursor = events.get_cursor();
        let fired: Vec<_> = cursor.read(events).collect();
        assert_eq!(fired.len(), 1, "exactly one toast must fire on challenge win");
        assert!(
            fired[0].0.contains("Challenge 3"),
            "toast must name the 1-based level that was just completed"
        );
    }

    #[test]
    fn classic_win_does_not_fire_challenge_complete_toast() {
        let mut app = headless_app();
        // Default mode is Classic.
        app.world_mut().write_message(GameWonEvent {
            score: 500,
            time_seconds: 100,
        });
        app.update();

        let events = app.world().resource::<Events<InfoToastEvent>>();
        let mut cursor = events.get_cursor();
        assert!(
            cursor.read(events).next().is_none(),
            "no challenge toast should fire for non-Challenge wins"
        );
    }

    #[test]
    fn pressing_x_below_unlock_level_fires_info_toast() {
        let mut app = headless_app();
        // Level 0 is below CHALLENGE_UNLOCK_LEVEL.
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyX);
        app.update();

        let events = app.world().resource::<Events<InfoToastEvent>>();
        let mut cursor = events.get_cursor();
        let fired: Vec<_> = cursor.read(events).collect();
        assert_eq!(fired.len(), 1, "must show a toast explaining the lock");
        assert!(
            fired[0].0.contains(&CHALLENGE_UNLOCK_LEVEL.to_string()),
            "toast must mention the unlock level"
        );
    }
}
