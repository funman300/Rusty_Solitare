//! Routes game-request events to `solitaire_core::GameState` and emits
//! state-change notifications.
//!
//! Game state persistence: on startup the plugin attempts to restore an
//! in-progress game from `game_state.json`. On app exit the current state is
//! written back (unless the game is won). On a win or new-game request the
//! file is deleted so the next launch starts fresh.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use bevy::prelude::*;
use solitaire_core::game_state::{DrawMode, GameState};
use solitaire_data::{delete_game_state_at, game_state_file_path, load_game_state_from,
    save_game_state_to};

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

/// Persistence path for the in-progress game state file. `None` disables I/O.
#[derive(Resource, Debug, Clone)]
pub struct GameStatePath(pub Option<PathBuf>);

/// Registers game resources, events, and the systems that route user intent
/// (events) into mutations on `GameState`.
pub struct GamePlugin;

impl GamePlugin {
    /// Plugin with no persistence. Use in headless tests to avoid touching the
    /// real `game_state.json` on disk.
    pub fn headless() -> Self {
        Self
    }
}

impl Plugin for GamePlugin {
    fn build(&self, app: &mut App) {
        let path = game_state_file_path();
        // Restore any saved in-progress game, falling back to a fresh deal.
        let initial_state = path
            .as_deref()
            .and_then(load_game_state_from)
            .unwrap_or_else(|| GameState::new(seed_from_system_time(), DrawMode::DrawOne));

        app.insert_resource(GameStateResource(initial_state))
            .insert_resource(GameStatePath(path))
            .init_resource::<DragState>()
            .init_resource::<SyncStatusResource>()
            .add_event::<MoveRequestEvent>()
            .add_event::<DrawRequestEvent>()
            .add_event::<UndoRequestEvent>()
            .add_event::<NewGameRequestEvent>()
            .add_event::<StateChangedEvent>()
            .add_event::<crate::events::MoveRejectedEvent>()
            .add_event::<GameWonEvent>()
            .add_event::<crate::events::CardFlippedEvent>()
            .add_event::<crate::events::AchievementUnlockedEvent>()
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
            )
            .add_systems(Update, tick_elapsed_time)
            .add_systems(Last, save_game_state_on_exit);
    }
}

/// Pure, testable helper. Updates `elapsed_seconds` and drains the
/// fractional accumulator into whole-second ticks. No-op when `is_won`.
pub fn advance_elapsed(
    elapsed_seconds: &mut u64,
    accumulator: &mut f32,
    delta_secs: f32,
    is_won: bool,
) {
    if is_won {
        return;
    }
    *accumulator += delta_secs;
    while *accumulator >= 1.0 {
        *elapsed_seconds = elapsed_seconds.saturating_add(1);
        *accumulator -= 1.0;
    }
}

/// Increment `GameState.elapsed_seconds` once per real-world second while
/// the game is in progress (not won) and not paused. Stops counting on
/// win so the final time reflects how long the player took to solve the
/// deal; stops while the pause overlay is open.
fn tick_elapsed_time(
    time: Res<Time>,
    mut game: ResMut<GameStateResource>,
    mut accumulator: Local<f32>,
    paused: Option<Res<crate::pause_plugin::PausedResource>>,
) {
    if paused.is_some_and(|p| p.0) {
        return;
    }
    let is_won = game.0.is_won;
    advance_elapsed(
        &mut game.0.elapsed_seconds,
        &mut accumulator,
        time.delta_secs(),
        is_won,
    );
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
    settings: Option<Res<crate::settings_plugin::SettingsResource>>,
    path: Option<Res<GameStatePath>>,
) {
    for ev in new_game.read() {
        let seed = ev.seed.unwrap_or_else(seed_from_system_time);
        // Prefer the draw mode from Settings when starting a fresh game.
        // Fall back to the current game's draw mode in headless/test contexts
        // where SettingsPlugin is not installed.
        let draw_mode = settings
            .as_ref()
            .map(|s| s.0.draw_mode.clone())
            .unwrap_or_else(|| game.0.draw_mode.clone());
        let mode = ev.mode.unwrap_or(game.0.mode);
        game.0 = GameState::new_with_mode(seed, draw_mode, mode);
        // Delete any previously saved in-progress state — this is a fresh game.
        if let Some(p) = path.as_ref().and_then(|r| r.0.as_deref()) {
            if let Err(e) = delete_game_state_at(p) {
                warn!("game_state: failed to delete saved game: {e}");
            }
        }
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
    path: Option<Res<GameStatePath>>,
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
                    // Delete the saved state — a won game should not be resumed.
                    if let Some(p) = path.as_ref().and_then(|r| r.0.as_deref()) {
                        if let Err(e) = delete_game_state_at(p) {
                            warn!("game_state: failed to delete on win: {e}");
                        }
                    }
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

/// Last-schedule system: persists the current game state on `AppExit` so the
/// player can resume where they left off. Won games are not saved (the
/// `save_game_state_to` helper skips them). Blocking on exit is acceptable
/// because the game loop is already shutting down.
fn save_game_state_on_exit(
    mut exit_events: EventReader<AppExit>,
    game: Res<GameStateResource>,
    path: Res<GameStatePath>,
) {
    if exit_events.is_empty() {
        return;
    }
    exit_events.clear();
    let Some(p) = path.0.as_deref() else { return };
    if let Err(e) = save_game_state_to(p, &game.0) {
        warn!("game_state: failed to save on exit: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solitaire_core::pile::PileType;

    /// Build a minimal headless `App` with just `GamePlugin` installed.
    /// Disables persistence and overrides the seed so tests are deterministic
    /// and don't touch `~/.local/share/solitaire_quest/game_state.json`.
    fn test_app(seed: u64) -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins).add_plugins(GamePlugin);
        // Disable I/O — tests must not touch the real game state file.
        app.insert_resource(GameStatePath(None));
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
        assert!(app.world().get_resource::<GameStatePath>().is_some());
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

        app.world_mut().send_event(NewGameRequestEvent { seed: Some(999), mode: None });
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
    fn advance_elapsed_drains_accumulator_into_whole_seconds() {
        let mut elapsed = 0;
        let mut acc = 0.0;
        advance_elapsed(&mut elapsed, &mut acc, 2.5, false);
        assert_eq!(elapsed, 2);
        // Remaining 0.5 should still be in the accumulator.
        advance_elapsed(&mut elapsed, &mut acc, 0.5, false);
        assert_eq!(elapsed, 3);
    }

    #[test]
    fn advance_elapsed_is_noop_when_won() {
        let mut elapsed = 100;
        let mut acc = 0.0;
        advance_elapsed(&mut elapsed, &mut acc, 5.0, true);
        assert_eq!(elapsed, 100);
        assert_eq!(acc, 0.0);
    }

    #[test]
    fn advance_elapsed_handles_subsecond_deltas_without_skipping() {
        let mut elapsed = 0;
        let mut acc = 0.0;
        // 4 × 0.25 = 1.0 (exactly representable in f32) — must produce 1 tick.
        for _ in 0..4 {
            advance_elapsed(&mut elapsed, &mut acc, 0.25, false);
        }
        assert_eq!(elapsed, 1);
        // Repeat once more for a total of 2 seconds.
        for _ in 0..4 {
            advance_elapsed(&mut elapsed, &mut acc, 0.25, false);
        }
        assert_eq!(elapsed, 2);
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

    // -----------------------------------------------------------------------
    // Persistence tests
    // -----------------------------------------------------------------------

    fn tmp_gs_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("engine_test_gs_{name}.json"))
    }

    /// save_game_state_on_exit writes to disk when AppExit fires.
    #[test]
    fn exit_saves_game_state() {
        use solitaire_data::load_game_state_from;

        let path = tmp_gs_path("exit_save");
        let _ = std::fs::remove_file(&path);

        let mut app = test_app(7);
        // Point persistence at our temp file.
        app.insert_resource(GameStatePath(Some(path.clone())));
        // Override the seed so we can verify it was written.
        app.world_mut().resource_mut::<GameStateResource>().0 =
            GameState::new(7654, DrawMode::DrawOne);

        app.world_mut().send_event(AppExit::Success);
        app.update();

        let loaded = load_game_state_from(&path).expect("file should exist after exit");
        assert_eq!(loaded.seed, 7654);

        let _ = std::fs::remove_file(&path);
    }

    /// new_game_request deletes any previously saved state file.
    #[test]
    fn new_game_deletes_saved_state() {
        use solitaire_data::save_game_state_to;

        let path = tmp_gs_path("new_game_delete");
        // Pre-create a saved file.
        save_game_state_to(&path, &GameState::new(1, DrawMode::DrawOne)).unwrap();
        assert!(path.exists());

        let mut app = test_app(1);
        app.insert_resource(GameStatePath(Some(path.clone())));
        app.world_mut().send_event(NewGameRequestEvent { seed: Some(2), mode: None });
        app.update();

        assert!(!path.exists(), "saved file should be deleted after new game");
    }
}
