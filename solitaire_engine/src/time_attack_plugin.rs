//! Time Attack mode runtime: 10-minute countdown wrapped around back-to-back
//! `GameMode::TimeAttack` games. Pressing **T** starts a session (gated by
//! level ≥ `CHALLENGE_UNLOCK_LEVEL`); each win during the session bumps the
//! counter and auto-deals a fresh game. When the timer expires the session
//! ends and `TimeAttackEndedEvent` fires.

use bevy::prelude::*;
use solitaire_core::game_state::GameMode;

use crate::challenge_plugin::CHALLENGE_UNLOCK_LEVEL;
use crate::events::{GameWonEvent, InfoToastEvent, NewGameRequestEvent};
use crate::game_plugin::GameMutation;
use crate::progress_plugin::ProgressResource;
use crate::resources::GameStateResource;

/// Length of a Time Attack session in real-world seconds (10 minutes).
pub const TIME_ATTACK_DURATION_SECS: f32 = 600.0;

/// Session state for an in-progress Time Attack run. Not persisted.
#[derive(Resource, Debug, Clone, Default)]
pub struct TimeAttackResource {
    pub active: bool,
    pub remaining_secs: f32,
    pub wins: u32,
}

/// Fired when the Time Attack timer expires. The summary toast in
/// `AnimationPlugin` consumes this; UI/stats consumers can also subscribe.
#[derive(Message, Debug, Clone, Copy)]
pub struct TimeAttackEndedEvent {
    pub wins: u32,
}

pub struct TimeAttackPlugin;

impl Plugin for TimeAttackPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TimeAttackResource>()
            .add_message::<TimeAttackEndedEvent>()
            .add_message::<GameWonEvent>()
            .add_message::<NewGameRequestEvent>()
            .add_message::<InfoToastEvent>()
            .add_systems(
                Update,
                handle_start_time_attack_request.before(GameMutation),
            )
            .add_systems(Update, advance_time_attack)
            .add_systems(Update, auto_deal_on_time_attack_win.after(GameMutation));
    }
}

fn handle_start_time_attack_request(
    keys: Res<ButtonInput<KeyCode>>,
    progress: Res<ProgressResource>,
    mut session: ResMut<TimeAttackResource>,
    mut new_game: MessageWriter<NewGameRequestEvent>,
    mut info_toast: MessageWriter<InfoToastEvent>,
) {
    if !keys.just_pressed(KeyCode::KeyT) {
        return;
    }
    if progress.0.level < CHALLENGE_UNLOCK_LEVEL {
        info_toast.write(InfoToastEvent(format!(
            "Time Attack unlocks at level {CHALLENGE_UNLOCK_LEVEL}"
        )));
        return;
    }
    *session = TimeAttackResource {
        active: true,
        remaining_secs: TIME_ATTACK_DURATION_SECS,
        wins: 0,
    };
    new_game.write(NewGameRequestEvent {
        seed: None,
        mode: Some(GameMode::TimeAttack),
    });
}

fn advance_time_attack(
    time: Res<Time>,
    mut session: ResMut<TimeAttackResource>,
    mut ended: MessageWriter<TimeAttackEndedEvent>,
    paused: Option<Res<crate::pause_plugin::PausedResource>>,
) {
    if !session.active {
        return;
    }
    if paused.is_some_and(|p| p.0) {
        return;
    }
    session.remaining_secs -= time.delta_secs();
    if session.remaining_secs <= 0.0 {
        let wins = session.wins;
        session.active = false;
        session.remaining_secs = 0.0;
        ended.write(TimeAttackEndedEvent { wins });
    }
}

fn auto_deal_on_time_attack_win(
    mut wins: MessageReader<GameWonEvent>,
    game: Res<GameStateResource>,
    mut session: ResMut<TimeAttackResource>,
    mut new_game: MessageWriter<NewGameRequestEvent>,
) {
    for _ in wins.read() {
        if !session.active || game.0.mode != GameMode::TimeAttack {
            continue;
        }
        session.wins = session.wins.saturating_add(1);
        new_game.write(NewGameRequestEvent {
            seed: None,
            mode: Some(GameMode::TimeAttack),
        });
    }
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
            .add_plugins(TimeAttackPlugin);
        app.init_resource::<ButtonInput<KeyCode>>();
        app.update();
        app
    }

    fn press_t(app: &mut App) {
        let mut input = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        input.release(KeyCode::KeyT);
        input.clear();
        input.press(KeyCode::KeyT);
    }

    #[test]
    fn pressing_t_below_unlock_level_is_ignored() {
        let mut app = headless_app();
        press_t(&mut app);
        app.update();

        let session = app.world().resource::<TimeAttackResource>();
        assert!(!session.active);

        let events = app.world().resource::<Events<NewGameRequestEvent>>();
        let mut cursor = events.get_cursor();
        assert!(cursor.read(events).next().is_none());
    }

    #[test]
    fn pressing_t_at_unlock_level_starts_session_and_deals_time_attack_game() {
        let mut app = headless_app();
        app.world_mut().resource_mut::<ProgressResource>().0.level = CHALLENGE_UNLOCK_LEVEL;

        press_t(&mut app);
        app.update();

        let session = app.world().resource::<TimeAttackResource>().clone();
        assert!(session.active);
        assert_eq!(session.wins, 0);
        assert!((session.remaining_secs - TIME_ATTACK_DURATION_SECS).abs() < 1.0);

        let events = app.world().resource::<Events<NewGameRequestEvent>>();
        let mut cursor = events.get_cursor();
        let fired: Vec<_> = cursor.read(events).copied().collect();
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].mode, Some(GameMode::TimeAttack));
    }

    #[test]
    fn timer_expiry_fires_ended_event_and_clears_active() {
        let mut app = headless_app();
        // Set the session to an already-expired state (remaining < 0).
        // MinimalPlugins time delta is nonzero so we skip the intermediate
        // 0.001-remaining step to avoid a double-fire.
        *app.world_mut().resource_mut::<TimeAttackResource>() = TimeAttackResource {
            active: true,
            remaining_secs: -1.0,
            wins: 5,
        };
        app.update();

        let session = app.world().resource::<TimeAttackResource>();
        assert!(!session.active);
        assert_eq!(session.remaining_secs, 0.0);

        let events = app.world().resource::<Events<TimeAttackEndedEvent>>();
        let mut cursor = events.get_cursor();
        let fired: Vec<_> = cursor.read(events).copied().collect();
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].wins, 5);
    }

    #[test]
    fn win_during_session_increments_wins_and_auto_deals() {
        let mut app = headless_app();
        // Start a session manually.
        *app.world_mut().resource_mut::<TimeAttackResource>() = TimeAttackResource {
            active: true,
            remaining_secs: 100.0,
            wins: 0,
        };
        // The current game must be in TimeAttack mode for auto-deal to fire.
        app.world_mut().resource_mut::<GameStateResource>().0 =
            GameState::new_with_mode(7, DrawMode::DrawOne, GameMode::TimeAttack);

        app.world_mut().write_message(GameWonEvent {
            score: 500,
            time_seconds: 60,
        });
        app.update();

        let session = app.world().resource::<TimeAttackResource>();
        assert_eq!(session.wins, 1);

        let events = app.world().resource::<Events<NewGameRequestEvent>>();
        let mut cursor = events.get_cursor();
        let fired: Vec<_> = cursor.read(events).copied().collect();
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].mode, Some(GameMode::TimeAttack));
        assert!(fired[0].seed.is_none());
    }

    #[test]
    fn win_when_session_inactive_does_not_increment() {
        let mut app = headless_app();
        // Default session is inactive. Game is TimeAttack mode — still no count.
        app.world_mut().resource_mut::<GameStateResource>().0 =
            GameState::new_with_mode(7, DrawMode::DrawOne, GameMode::TimeAttack);

        app.world_mut().write_message(GameWonEvent {
            score: 500,
            time_seconds: 60,
        });
        app.update();

        let session = app.world().resource::<TimeAttackResource>();
        assert_eq!(session.wins, 0);
    }

    #[test]
    fn classic_win_during_session_does_not_increment() {
        let mut app = headless_app();
        *app.world_mut().resource_mut::<TimeAttackResource>() = TimeAttackResource {
            active: true,
            remaining_secs: 100.0,
            wins: 0,
        };
        // GameStateResource defaults to Classic mode.
        app.world_mut().write_message(GameWonEvent {
            score: 500,
            time_seconds: 60,
        });
        app.update();

        let session = app.world().resource::<TimeAttackResource>();
        assert_eq!(session.wins, 0);
    }

    #[test]
    fn paused_session_does_not_fire_ended_event() {
        // Insert PausedResource(true) so the advance system exits early.
        // Even with remaining_secs at -1 (which would normally trigger expiry),
        // the timer must not fire while the game is paused.
        let mut app = headless_app();
        app.insert_resource(crate::pause_plugin::PausedResource(true));
        *app.world_mut().resource_mut::<TimeAttackResource>() = TimeAttackResource {
            active: true,
            remaining_secs: -1.0, // would normally expire
            wins: 3,
        };
        app.update();

        // remaining_secs must not have been reset to 0.0 (pause blocked the update).
        let session = app.world().resource::<TimeAttackResource>();
        assert!(session.active, "session must still be active while paused");
        assert!(session.remaining_secs < 0.0, "remaining_secs must not change while paused");

        // No ended event must have been emitted.
        let events = app.world().resource::<Events<TimeAttackEndedEvent>>();
        let mut cursor = events.get_cursor();
        assert!(
            cursor.read(events).next().is_none(),
            "TimeAttackEndedEvent must not fire while paused"
        );
    }
}
