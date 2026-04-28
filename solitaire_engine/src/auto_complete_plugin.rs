//! Automatic card-to-foundation sequencing once `is_auto_completable` is set.
//!
//! When `GameState::is_auto_completable` becomes `true`, this plugin fires
//! `MoveRequestEvent` for one card per `STEP_INTERVAL` seconds until the game
//! is won. A single toast announces the sequence; no player input is required.
//!
//! The plugin is intentionally passive: it only reads `GameStateResource` and
//! fires `MoveRequestEvent`. If for some reason `next_auto_complete_move`
//! returns `None` (e.g. a transient state), the plugin retries next tick.

use bevy::prelude::*;

use crate::audio_plugin::{AudioState, SoundLibrary};
use crate::events::{MoveRequestEvent, StateChangedEvent};
use crate::game_plugin::GameMutation;
use crate::resources::GameStateResource;

/// Volume amplitude used for the auto-complete activation chime.
///
/// Plays the win fanfare at half volume so it is clearly distinguishable from
/// both normal card-place sounds and the full win fanfare that fires later.
const AUTO_COMPLETE_CHIME_VOLUME: f64 = 0.5;

/// Seconds between consecutive auto-complete moves.
const STEP_INTERVAL: f32 = 0.12;

/// Tracks whether auto-complete is active and when the next move fires.
#[derive(Resource, Default, Debug)]
pub struct AutoCompleteState {
    /// `true` once we've detected `is_auto_completable` and started firing moves.
    pub active: bool,
    /// Countdown to the next move, in seconds.
    cooldown: f32,
}

/// Plugin that drives the auto-complete sequence.
pub struct AutoCompletePlugin;

impl Plugin for AutoCompletePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AutoCompleteState>()
            .add_systems(
                Update,
                (
                    detect_auto_complete,
                    on_auto_complete_start,
                    drive_auto_complete,
                )
                    .chain()
                    .after(GameMutation),
            );
    }
}

/// Activates auto-complete when `is_auto_completable` flips to `true`.
/// Deactivates it on win or new game (any state where it should not be running).
fn detect_auto_complete(
    mut state: ResMut<AutoCompleteState>,
    game: Res<GameStateResource>,
    mut changed: EventReader<StateChangedEvent>,
) {
    // Only re-evaluate on state changes to avoid per-frame allocations.
    if changed.is_empty() && !game.is_changed() {
        return;
    }
    changed.clear();

    if game.0.is_won {
        state.active = false;
        return;
    }
    if game.0.is_auto_completable && !state.active {
        state.active = true;
        state.cooldown = 0.0; // fire first move immediately
    } else if !game.0.is_auto_completable {
        state.active = false;
    }
}

/// Plays a distinct chime the moment auto-complete first activates.
///
/// Uses a `Local<bool>` to remember the previous `active` state and fires
/// exactly once on the `false → true` edge. The win fanfare is played at half
/// volume (`AUTO_COMPLETE_CHIME_VOLUME`) so it is clearly recognisable but does
/// not overwhelm the card-place sounds that follow immediately.
fn on_auto_complete_start(
    state: Res<AutoCompleteState>,
    mut was_active: Local<bool>,
    mut audio: Option<NonSendMut<AudioState>>,
    lib: Option<Res<SoundLibrary>>,
) {
    let now_active = state.active;
    let edge = now_active && !*was_active;
    *was_active = now_active;

    if !edge {
        return;
    }

    let (Some(audio), Some(lib)) = (audio.as_mut(), lib) else { return };
    audio.play_sfx_at_volume(&lib.fanfare, AUTO_COMPLETE_CHIME_VOLUME);
}

/// Fires one `MoveRequestEvent` per `STEP_INTERVAL` while auto-complete is active.
fn drive_auto_complete(
    mut state: ResMut<AutoCompleteState>,
    game: Res<GameStateResource>,
    time: Res<Time>,
    mut moves: EventWriter<MoveRequestEvent>,
) {
    if !state.active {
        return;
    }

    state.cooldown -= time.delta_secs();
    if state.cooldown > 0.0 {
        return;
    }

    let Some((from, to)) = game.0.next_auto_complete_move() else {
        // No move available yet (race with game state update); try next tick.
        return;
    };

    moves.send(MoveRequestEvent { from, to, count: 1 });
    state.cooldown = STEP_INTERVAL;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game_plugin::GamePlugin;
    use crate::table_plugin::TablePlugin;
    use solitaire_core::card::{Card, Rank, Suit};
    use solitaire_core::game_state::{DrawMode, GameState};
    use solitaire_core::pile::PileType;

    fn headless_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(GamePlugin)
            .add_plugins(TablePlugin)
            .add_plugins(AutoCompletePlugin);
        app.init_resource::<bevy::input::ButtonInput<KeyCode>>();
        app.update();
        app
    }

    /// Build a nearly-won game: one Ace of Clubs in Tableau(0), all other
    /// tableau piles empty, stock/waste empty, Clubs foundation empty.
    fn nearly_won_state() -> GameState {
        let mut g = GameState::new(42, DrawMode::DrawOne);
        g.piles.get_mut(&PileType::Stock).unwrap().cards.clear();
        g.piles.get_mut(&PileType::Waste).unwrap().cards.clear();
        for i in 0..7 {
            g.piles.get_mut(&PileType::Tableau(i)).unwrap().cards.clear();
        }
        g.piles.get_mut(&PileType::Tableau(0)).unwrap().cards.push(Card {
            id: 99,
            suit: Suit::Clubs,
            rank: Rank::Ace,
            face_up: true,
        });
        g.is_auto_completable = true;
        g
    }

    #[test]
    fn state_starts_inactive() {
        let app = headless_app();
        assert!(!app.world().resource::<AutoCompleteState>().active);
    }

    #[test]
    fn detect_activates_when_auto_completable() {
        let mut app = headless_app();
        // Install a nearly-won state and fire StateChangedEvent.
        app.world_mut().resource_mut::<GameStateResource>().0 = nearly_won_state();
        app.world_mut().send_event(StateChangedEvent);
        app.update();

        assert!(app.world().resource::<AutoCompleteState>().active);
    }

    #[test]
    fn drive_fires_move_request_when_active() {
        let mut app = headless_app();
        app.world_mut().resource_mut::<GameStateResource>().0 = nearly_won_state();
        app.world_mut().send_event(StateChangedEvent);
        app.update(); // detect runs, sets active
        app.update(); // drive fires the move

        let events = app.world().resource::<Events<MoveRequestEvent>>();
        let mut cursor = events.get_cursor();
        let fired: Vec<_> = cursor.read(events).collect();
        // At least one MoveRequestEvent should have been fired.
        assert!(!fired.is_empty(), "expected at least one MoveRequestEvent");
        assert_eq!(fired[0].from, PileType::Tableau(0));
        assert_eq!(fired[0].to, PileType::Foundation(Suit::Clubs));
    }

    #[test]
    fn drive_deactivates_on_win() {
        let mut app = headless_app();
        // Inject a won game state — active should not be set.
        let mut gs = nearly_won_state();
        gs.is_won = true;
        app.world_mut().resource_mut::<GameStateResource>().0 = gs;
        app.world_mut().send_event(StateChangedEvent);
        app.update();

        assert!(!app.world().resource::<AutoCompleteState>().active);
    }
}
