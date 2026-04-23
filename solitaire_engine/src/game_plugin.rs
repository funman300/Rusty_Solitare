//! Routes game-request events to `solitaire_core::GameState` and emits
//! state-change notifications.

use std::time::{SystemTime, UNIX_EPOCH};

use bevy::prelude::*;
use solitaire_core::game_state::{DrawMode, GameState};

use crate::events::{
    DrawRequestEvent, GameWonEvent, MoveRequestEvent, NewGameRequestEvent, StateChangedEvent,
    UndoRequestEvent,
};
use crate::resources::{DragState, GameStateResource, SyncStatusResource};

/// System set for `GamePlugin`'s state-mutating systems. Downstream plugins
/// that read the resulting `StateChangedEvent` should schedule themselves
/// `.after(GameMutation)` so updates propagate within a single frame.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct GameMutation;

/// Registers game resources, events, and the systems that route user intent
/// (events) into mutations on `GameState`.
pub struct GamePlugin;

impl Plugin for GamePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(GameStateResource(GameState::new(
            seed_from_system_time(),
            DrawMode::DrawOne,
        )))
            .init_resource::<DragState>()
            .init_resource::<SyncStatusResource>()
            .add_event::<MoveRequestEvent>()
            .add_event::<DrawRequestEvent>()
            .add_event::<UndoRequestEvent>()
            .add_event::<NewGameRequestEvent>()
            .add_event::<StateChangedEvent>()
            .add_event::<GameWonEvent>()
            .add_event::<crate::events::CardFlippedEvent>()
            .add_systems(
                Update,
                (
                    handle_new_game,
                    handle_draw,
                    handle_move,
                    handle_undo,
                )
                    .chain()
                    .in_set(GameMutation),
            );
    }
}

fn seed_from_system_time() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

fn handle_new_game(
    mut new_game: EventReader<NewGameRequestEvent>,
    mut game: ResMut<GameStateResource>,
    mut changed: EventWriter<StateChangedEvent>,
) {
    for ev in new_game.read() {
        let seed = ev.seed.unwrap_or_else(seed_from_system_time);
        let draw_mode = game.0.draw_mode.clone();
        game.0 = GameState::new(seed, draw_mode);
        changed.send(StateChangedEvent);
    }
}

fn handle_draw(
    mut draws: EventReader<DrawRequestEvent>,
    mut game: ResMut<GameStateResource>,
    mut changed: EventWriter<StateChangedEvent>,
) {
    for _ in draws.read() {
        match game.0.draw() {
            Ok(()) => {
                changed.send(StateChangedEvent);
            }
            Err(e) => warn!("draw rejected: {e}"),
        }
    }
}

fn handle_move(
    mut moves: EventReader<MoveRequestEvent>,
    mut game: ResMut<GameStateResource>,
    mut changed: EventWriter<StateChangedEvent>,
    mut won: EventWriter<GameWonEvent>,
) {
    for ev in moves.read() {
        let was_won = game.0.is_won;
        match game.0.move_cards(ev.from.clone(), ev.to.clone(), ev.count) {
            Ok(()) => {
                changed.send(StateChangedEvent);
                if !was_won && game.0.is_won {
                    won.send(GameWonEvent {
                        score: game.0.score,
                        time_seconds: game.0.elapsed_seconds,
                    });
                }
            }
            Err(e) => warn!("move rejected {:?} -> {:?} x{}: {e}", ev.from, ev.to, ev.count),
        }
    }
}

fn handle_undo(
    mut undos: EventReader<UndoRequestEvent>,
    mut game: ResMut<GameStateResource>,
    mut changed: EventWriter<StateChangedEvent>,
) {
    for _ in undos.read() {
        match game.0.undo() {
            Ok(()) => {
                changed.send(StateChangedEvent);
            }
            Err(e) => warn!("undo rejected: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solitaire_core::pile::PileType;

    /// Build a minimal headless `App` with just `GamePlugin` installed.
    /// Overrides the default random seed so tests are deterministic.
    fn test_app(seed: u64) -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins).add_plugins(GamePlugin);
        // Override the system-time seed with a known value.
        app.world_mut()
            .resource_mut::<GameStateResource>()
            .0 = GameState::new(seed, DrawMode::DrawOne);
        app
    }

    #[test]
    fn plugin_inserts_game_state_resource() {
        let app = test_app(1);
        assert!(app.world().get_resource::<GameStateResource>().is_some());
        assert!(app.world().get_resource::<DragState>().is_some());
        assert!(app.world().get_resource::<SyncStatusResource>().is_some());
    }

    #[test]
    fn draw_request_advances_game_state() {
        let mut app = test_app(42);
        let stock_before = app
            .world()
            .resource::<GameStateResource>()
            .0
            .piles[&PileType::Stock]
            .cards
            .len();

        app.world_mut().send_event(DrawRequestEvent);
        app.update();

        let stock_after = app
            .world()
            .resource::<GameStateResource>()
            .0
            .piles[&PileType::Stock]
            .cards
            .len();
        let waste_after = app
            .world()
            .resource::<GameStateResource>()
            .0
            .piles[&PileType::Waste]
            .cards
            .len();
        assert_eq!(stock_after, stock_before - 1);
        assert_eq!(waste_after, 1);
    }

    #[test]
    fn draw_request_fires_state_changed_event() {
        let mut app = test_app(42);
        app.world_mut().send_event(DrawRequestEvent);
        app.update();
        let events = app.world().resource::<Events<StateChangedEvent>>();
        let mut reader = events.get_cursor();
        assert!(reader.read(events).next().is_some());
    }

    #[test]
    fn undo_after_draw_restores_state() {
        let mut app = test_app(42);
        app.world_mut().send_event(DrawRequestEvent);
        app.update();
        app.world_mut().send_event(UndoRequestEvent);
        app.update();
        let g = &app.world().resource::<GameStateResource>().0;
        assert_eq!(g.piles[&PileType::Stock].cards.len(), 24);
        assert_eq!(g.piles[&PileType::Waste].cards.len(), 0);
    }

    #[test]
    fn new_game_request_reseeds() {
        let mut app = test_app(1);
        let before: Vec<u32> = app
            .world()
            .resource::<GameStateResource>()
            .0
            .piles[&PileType::Tableau(0)]
            .cards
            .iter()
            .map(|c| c.id)
            .collect();

        app.world_mut().send_event(NewGameRequestEvent { seed: Some(999) });
        app.update();

        let after: Vec<u32> = app
            .world()
            .resource::<GameStateResource>()
            .0
            .piles[&PileType::Tableau(0)]
            .cards
            .iter()
            .map(|c| c.id)
            .collect();
        assert_ne!(before, after);
    }

    #[test]
    fn invalid_move_does_not_fire_state_changed() {
        let mut app = test_app(42);
        // Stock -> Waste is InvalidDestination; no state change expected.
        app.world_mut().send_event(MoveRequestEvent {
            from: PileType::Stock,
            to: PileType::Waste,
            count: 1,
        });
        app.update();
        let events = app.world().resource::<Events<StateChangedEvent>>();
        let mut reader = events.get_cursor();
        assert!(reader.read(events).next().is_none());
    }
}
