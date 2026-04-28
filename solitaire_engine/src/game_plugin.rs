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
    CardFlippedEvent, DrawRequestEvent, GameWonEvent, InfoToastEvent, MoveRequestEvent,
    NewGameRequestEvent, StateChangedEvent, UndoRequestEvent,
};
use crate::resources::{DragState, GameStateResource, SyncStatusResource};

// ---------------------------------------------------------------------------
// Task #57 — Confirm-new-game dialog
// ---------------------------------------------------------------------------

/// Marker on the confirm-new-game modal root node.
#[derive(Component, Debug)]
pub struct ConfirmNewGameScreen;

// ---------------------------------------------------------------------------
// Task #58 — Game-over overlay
// ---------------------------------------------------------------------------

/// Marker on the game-over overlay root node.
#[derive(Component, Debug)]
pub struct GameOverScreen;

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
            .add_message::<MoveRequestEvent>()
            .add_message::<DrawRequestEvent>()
            .add_message::<UndoRequestEvent>()
            .add_message::<NewGameRequestEvent>()
            .add_message::<StateChangedEvent>()
            .add_message::<crate::events::MoveRejectedEvent>()
            .add_message::<GameWonEvent>()
            .add_message::<crate::events::CardFlippedEvent>()
            .add_message::<crate::events::AchievementUnlockedEvent>()
            .add_message::<InfoToastEvent>()
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
            .add_systems(Update, check_no_moves.after(GameMutation))
            .add_systems(Update, handle_confirm_input.after(GameMutation))
            .add_systems(Update, handle_game_over_input.after(GameMutation))
            .init_resource::<AutoSaveTimer>()
            .add_systems(Update, tick_elapsed_time)
            .add_systems(Update, auto_save_game_state)
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

#[allow(clippy::too_many_arguments)]
fn handle_new_game(
    mut commands: Commands,
    mut new_game: MessageReader<NewGameRequestEvent>,
    mut game: ResMut<GameStateResource>,
    mut changed: MessageWriter<StateChangedEvent>,
    settings: Option<Res<crate::settings_plugin::SettingsResource>>,
    path: Option<Res<GameStatePath>>,
    confirm_screens: Query<Entity, With<ConfirmNewGameScreen>>,
    game_over_screens: Query<Entity, With<GameOverScreen>>,
) {
    for ev in new_game.read() {
        // If an active game is in progress, intercept and show a confirm dialog.
        // A game is "active" when moves have been made and it is not yet won.
        let needs_confirm = game.0.move_count > 0 && !game.0.is_won;
        // Skip confirmation if a ConfirmNewGameScreen already exists (prevents duplicates).
        let confirm_already_open = !confirm_screens.is_empty();
        if needs_confirm && !confirm_already_open {
            // Despawn any stale game-over overlay before showing confirm dialog.
            for entity in &game_over_screens {
                commands.entity(entity).despawn();
            }
            spawn_confirm_dialog(&mut commands, *ev);
            continue;
        }

        // Despawn confirm and game-over overlays before starting the new game.
        for entity in &confirm_screens {
            commands.entity(entity).despawn();
        }
        for entity in &game_over_screens {
            commands.entity(entity).despawn();
        }

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
        changed.write(StateChangedEvent);
    }
}

/// Spawns the confirm-new-game modal overlay.
///
/// Shown when the player requests a new game while moves have been made and
/// the game is not yet won. The overlay stores the original request so the
/// `handle_confirm_input` system can replay it on confirmation.
fn spawn_confirm_dialog(commands: &mut Commands, original_request: NewGameRequestEvent) {
    commands
        .spawn((
            ConfirmNewGameScreen,
            // Store the request so we can replay it on confirmation.
            OriginalNewGameRequest(original_request),
            Node {
                position_type: PositionType::Absolute,
                left: Val::Percent(0.0),
                top: Val::Percent(0.0),
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                row_gap: Val::Px(20.0),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.70)),
            ZIndex(250),
        ))
        .with_children(|root| {
            // Dialog card
            root.spawn((
                Node {
                    flex_direction: FlexDirection::Column,
                    padding: UiRect::all(Val::Px(40.0)),
                    row_gap: Val::Px(20.0),
                    min_width: Val::Px(360.0),
                    align_items: AlignItems::Center,
                    ..default()
                },
                BackgroundColor(Color::srgb(0.10, 0.12, 0.15)),
                BorderRadius::all(Val::Px(12.0)),
            ))
            .with_children(|card| {
                // Heading
                card.spawn((
                    Text::new("Abandon current game?"),
                    TextFont { font_size: 30.0, ..default() },
                    TextColor(Color::WHITE),
                ));
                // Button row
                card.spawn((Node {
                    flex_direction: FlexDirection::Row,
                    column_gap: Val::Px(24.0),
                    ..default()
                },))
                .with_children(|row| {
                    // Yes button
                    row.spawn((
                        Text::new("Yes (Y)"),
                        TextFont { font_size: 22.0, ..default() },
                        TextColor(Color::srgb(0.3, 1.0, 0.4)),
                    ));
                    // No button
                    row.spawn((
                        Text::new("No (N)"),
                        TextFont { font_size: 22.0, ..default() },
                        TextColor(Color::srgb(1.0, 0.4, 0.4)),
                    ));
                });
            });
        });
}

/// Carries the original `NewGameRequestEvent` on the confirm overlay so
/// `handle_confirm_input` can replay it with the same seed / mode.
#[derive(Component, Debug, Clone, Copy)]
struct OriginalNewGameRequest(NewGameRequestEvent);

/// Handles keyboard input while `ConfirmNewGameScreen` is open.
///
/// `Y` or `Enter` confirms: despawns the overlay and fires `NewGameRequestEvent`.
/// `N` or `Escape` cancels: despawns the overlay without starting a new game.
fn handle_confirm_input(
    mut commands: Commands,
    keys: Option<Res<ButtonInput<KeyCode>>>,
    screens: Query<(Entity, &OriginalNewGameRequest), With<ConfirmNewGameScreen>>,
    mut new_game: MessageWriter<NewGameRequestEvent>,
) {
    let Ok((entity, original)) = screens.single() else {
        return;
    };
    let Some(keys) = keys else {
        return;
    };

    let confirmed = keys.just_pressed(KeyCode::KeyY) || keys.just_pressed(KeyCode::Enter);
    let cancelled = keys.just_pressed(KeyCode::KeyN) || keys.just_pressed(KeyCode::Escape);

    if confirmed {
        commands.entity(entity).despawn();
        // Re-send with move_count already 0 would bypass the dialog next time.
        // We fire the event — handle_new_game will skip the dialog because
        // the screen is despawned before the next read.
        new_game.write(NewGameRequestEvent {
            seed: original.0.seed,
            mode: original.0.mode,
        });
    } else if cancelled {
        commands.entity(entity).despawn();
    }
}

fn handle_draw(
    mut draws: MessageReader<DrawRequestEvent>,
    mut game: ResMut<GameStateResource>,
    mut changed: MessageWriter<StateChangedEvent>,
    mut flipped: MessageWriter<CardFlippedEvent>,
) {
    use solitaire_core::pile::PileType;

    for _ in draws.read() {
        // Capture which cards are about to be drawn (top of the stock pile)
        // so we can fire flip events after they land face-up in the waste.
        // Only relevant when stock is non-empty; a recycle moves waste back to
        // stock face-down, so no flip events are needed in that case.
        let drawn_ids: Vec<u32> = {
            let stock = game.0.piles.get(&PileType::Stock);
            match stock {
                Some(p) if !p.cards.is_empty() => {
                    let draw_count = match game.0.draw_mode {
                        DrawMode::DrawOne => 1_usize,
                        DrawMode::DrawThree => 3_usize,
                    };
                    let n = p.cards.len();
                    let take = n.min(draw_count);
                    // The top `take` cards (at the end of the vec) will be drawn.
                    p.cards[n - take..].iter().map(|c| c.id).collect()
                }
                _ => Vec::new(),
            }
        };

        match game.0.draw() {
            Ok(()) => {
                // Fire a flip event for each card that moved from stock to waste.
                for id in drawn_ids {
                    flipped.write(CardFlippedEvent(id));
                }
                changed.write(StateChangedEvent);
            }
            Err(e) => warn!("draw rejected: {e}"),
        }
    }
}

fn handle_move(
    mut moves: MessageReader<MoveRequestEvent>,
    mut game: ResMut<GameStateResource>,
    mut changed: MessageWriter<StateChangedEvent>,
    mut won: MessageWriter<GameWonEvent>,
    mut flipped: MessageWriter<crate::events::CardFlippedEvent>,
    path: Option<Res<GameStatePath>>,
) {
    for ev in moves.read() {
        let was_won = game.0.is_won;
        // Identify the card that will be exposed (and may flip face-up) by the move.
        // It's the card just below the bottom of the moving stack in the source pile.
        let flip_candidate_id = game.0.piles.get(&ev.from).and_then(|p| {
            let n = p.cards.len();
            if n > ev.count {
                let c = &p.cards[n - ev.count - 1];
                if !c.face_up { Some(c.id) } else { None }
            } else {
                None
            }
        });
        match game.0.move_cards(ev.from.clone(), ev.to.clone(), ev.count) {
            Ok(()) => {
                // Fire flip event if the candidate card is now face-up.
                if let Some(fid) = flip_candidate_id {
                    if game.0.piles.get(&ev.from)
                        .and_then(|p| p.cards.last())
                        .is_some_and(|c| c.id == fid && c.face_up)
                    {
                        flipped.write(crate::events::CardFlippedEvent(fid));
                    }
                }
                changed.write(StateChangedEvent);
                if !was_won && game.0.is_won {
                    won.write(GameWonEvent {
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
    mut undos: MessageReader<UndoRequestEvent>,
    mut game: ResMut<GameStateResource>,
    mut changed: MessageWriter<StateChangedEvent>,
    mut toast: MessageWriter<InfoToastEvent>,
) {
    use solitaire_core::error::MoveError;

    for _ in undos.read() {
        match game.0.undo() {
            Ok(()) => {
                changed.write(StateChangedEvent);
            }
            Err(MoveError::UndoStackEmpty) => {
                toast.write(InfoToastEvent("Nothing to undo".to_string()));
            }
            Err(e) => warn!("undo rejected: {e}"),
        }
    }
}

// ---------------------------------------------------------------------------
// Task #29 — No-moves detection
// ---------------------------------------------------------------------------

/// Returns `true` if the current game state has at least one legal move.
///
/// Considers:
/// - Any non-empty Stock or Waste pile (draw / recycle is always available).
/// - Any face-up card on Waste or Tableau piles that can legally move to any
///   Foundation or Tableau destination.
pub fn has_legal_moves(game: &GameState) -> bool {
    use solitaire_core::card::Suit;
    use solitaire_core::pile::PileType;
    use solitaire_core::rules::{can_place_on_foundation, can_place_on_tableau};

    // If stock or waste is non-empty, the player can always draw.
    if !game.piles.get(&PileType::Stock).is_some_and(|p| p.cards.is_empty())
        || !game.piles.get(&PileType::Waste).is_some_and(|p| p.cards.is_empty())
    {
        return true;
    }

    let suits = [Suit::Clubs, Suit::Diamonds, Suit::Hearts, Suit::Spades];

    // Check each playable source pile.
    let sources: Vec<PileType> = {
        let mut v = vec![PileType::Waste];
        for i in 0..7_usize {
            v.push(PileType::Tableau(i));
        }
        v
    };

    for from in &sources {
        let Some(from_pile) = game.piles.get(from) else { continue };
        let Some(card) = from_pile.cards.last().filter(|c| c.face_up) else { continue };

        // Check foundations.
        for &suit in &suits {
            let dest = PileType::Foundation(suit);
            if let Some(dest_pile) = game.piles.get(&dest) {
                if can_place_on_foundation(card, dest_pile, suit) {
                    return true;
                }
            }
        }

        // Check tableau piles.
        for i in 0..7_usize {
            let dest = PileType::Tableau(i);
            if dest == *from {
                continue;
            }
            if let Some(dest_pile) = game.piles.get(&dest) {
                if can_place_on_tableau(card, dest_pile) {
                    return true;
                }
            }
        }
    }

    false
}

/// After each `StateChangedEvent`, check if the game has no legal moves.
///
/// When stuck (no legal moves and game not won), fires `InfoToastEvent` and
/// spawns a `GameOverScreen` overlay. The overlay is despawned automatically
/// when `has_legal_moves` returns true again (e.g. after undo) or when the
/// game is won.
fn check_no_moves(
    mut commands: Commands,
    mut events: MessageReader<StateChangedEvent>,
    game: Res<GameStateResource>,
    mut toast: MessageWriter<InfoToastEvent>,
    mut already_fired: Local<bool>,
    game_over_screens: Query<Entity, With<GameOverScreen>>,
) {
    // Reset the debounce flag on every state change so if something changes
    // we re-evaluate on the next state change.
    let had_event = events.read().next().is_some();
    // Drain remaining events to avoid leaking.
    events.clear();

    if !had_event {
        return;
    }

    // Reset debounce whenever the state changes.
    *already_fired = false;

    // Despawn game-over overlay whenever moves become available again or game is won.
    let moves_ok = has_legal_moves(&game.0);
    if moves_ok || game.0.is_won {
        for entity in &game_over_screens {
            commands.entity(entity).despawn();
        }
    }

    if game.0.is_won {
        return;
    }

    if !moves_ok && !*already_fired {
        toast.write(InfoToastEvent(
            "No moves available \u{2014} press D to draw or N for a new game".to_string(),
        ));
        *already_fired = true;
        // Only spawn the overlay if one does not already exist.
        if game_over_screens.is_empty() {
            spawn_game_over_screen(&mut commands, game.0.score);
        }
    }
}

/// Spawns the full-screen game-over overlay with score display and action hints.
///
/// The background is intentionally semi-transparent (alpha 0.6) so the stuck
/// card layout remains visible behind the dialog.
fn spawn_game_over_screen(commands: &mut Commands, score: i32) {
    commands
        .spawn((
            GameOverScreen,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Percent(0.0),
                top: Val::Percent(0.0),
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                row_gap: Val::Px(20.0),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.6)),
            ZIndex(200),
        ))
        .with_children(|root| {
            root.spawn((
                Node {
                    flex_direction: FlexDirection::Column,
                    padding: UiRect::all(Val::Px(40.0)),
                    row_gap: Val::Px(16.0),
                    min_width: Val::Px(340.0),
                    align_items: AlignItems::Center,
                    ..default()
                },
                BackgroundColor(Color::srgb(0.10, 0.08, 0.08)),
                BorderRadius::all(Val::Px(12.0)),
            ))
            .with_children(|card| {
                // Header — explains why the overlay appeared.
                card.spawn((
                    Text::new("No more moves available"),
                    TextFont { font_size: 36.0, ..default() },
                    TextColor(Color::srgb(1.0, 0.4, 0.1)),
                ));
                // Score
                card.spawn((
                    Text::new(format!("Score: {score}")),
                    TextFont { font_size: 24.0, ..default() },
                    TextColor(Color::WHITE),
                ));
                // Action hints — stacked vertically for legibility.
                card.spawn((
                    Node {
                        flex_direction: FlexDirection::Column,
                        row_gap: Val::Px(8.0),
                        margin: UiRect::top(Val::Px(8.0)),
                        align_items: AlignItems::Center,
                        ..default()
                    },
                ))
                .with_children(|hints| {
                    hints.spawn((
                        Text::new("Press N or Escape for a new game"),
                        TextFont { font_size: 20.0, ..default() },
                        TextColor(Color::srgb(0.3, 1.0, 0.4)),
                    ));
                    hints.spawn((
                        Text::new("Press G to forfeit (counts as a loss)"),
                        TextFont { font_size: 20.0, ..default() },
                        TextColor(Color::srgb(1.0, 0.6, 0.2)),
                    ));
                });
            });
        });
}

/// Handles keyboard input while `GameOverScreen` is open.
///
/// `N` or `Escape` fires `NewGameRequestEvent` (which will trigger the confirm
/// dialog if moves have been made). `U` fires `UndoRequestEvent` and despawns
/// the overlay — the `check_no_moves` system will re-show it on the next
/// `StateChangedEvent` if the undo did not restore any legal moves.
fn handle_game_over_input(
    mut commands: Commands,
    keys: Option<Res<ButtonInput<KeyCode>>>,
    screens: Query<Entity, With<GameOverScreen>>,
    mut new_game: MessageWriter<NewGameRequestEvent>,
    mut undo: MessageWriter<UndoRequestEvent>,
) {
    if screens.is_empty() {
        return;
    }
    let Some(keys) = keys else {
        return;
    };

    if keys.just_pressed(KeyCode::KeyN) || keys.just_pressed(KeyCode::Escape) {
        new_game.write(NewGameRequestEvent::default());
    } else if keys.just_pressed(KeyCode::KeyU) {
        for entity in &screens {
            commands.entity(entity).despawn();
        }
        undo.write(UndoRequestEvent);
    }
}

const AUTO_SAVE_INTERVAL_SECS: f32 = 30.0;

/// Accumulated real-world seconds since the last auto-save. Exposed as a
/// `Resource` so tests can pre-seed it past the threshold without needing to
/// control `Time::delta_secs()`.
#[derive(Resource, Default)]
pub struct AutoSaveTimer(pub f32);

/// Periodically saves game state every 30 real-world seconds while a game is
/// in progress. The timer uses real delta time (not game elapsed_seconds) so
/// it keeps ticking even if the game clock is paused.
fn auto_save_game_state(
    time: Res<Time>,
    game: Res<GameStateResource>,
    path: Option<Res<GameStatePath>>,
    mut timer: ResMut<AutoSaveTimer>,
    paused: Option<Res<crate::pause_plugin::PausedResource>>,
) {
    // Don't save if paused, game is won, or no moves have been made yet.
    if paused.is_some_and(|p| p.0) || game.0.is_won || game.0.move_count == 0 {
        return;
    }
    timer.0 += time.delta_secs();
    if timer.0 >= AUTO_SAVE_INTERVAL_SECS {
        timer.0 -= AUTO_SAVE_INTERVAL_SECS;
        let Some(p) = path.as_ref().and_then(|r| r.0.as_deref()) else { return };
        if let Err(e) = save_game_state_to(p, &game.0) {
            warn!("game_state: auto-save failed: {e}");
        }
    }
}

/// Last-schedule system: persists the current game state on `AppExit` so the
/// player can resume where they left off. Won games are not saved (the
/// `save_game_state_to` helper skips them). Blocking on exit is acceptable
/// because the game loop is already shutting down.
fn save_game_state_on_exit(
    mut exit_events: MessageReader<AppExit>,
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

        app.world_mut().write_message(DrawRequestEvent);
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
        app.world_mut().write_message(DrawRequestEvent);
        app.update();
        let events = app.world().resource::<Events<StateChangedEvent>>();
        let mut reader = events.get_cursor();
        assert!(reader.read(events).next().is_some());
    }

    #[test]
    fn undo_after_draw_restores_state() {
        let mut app = test_app(42);
        app.world_mut().write_message(DrawRequestEvent);
        app.update();
        app.world_mut().write_message(UndoRequestEvent);
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

        app.world_mut().write_message(NewGameRequestEvent { seed: Some(999), mode: None });
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
    fn advance_elapsed_saturates_at_u64_max() {
        let mut elapsed = u64::MAX;
        let mut acc = 0.0;
        advance_elapsed(&mut elapsed, &mut acc, 5.0, false);
        assert_eq!(elapsed, u64::MAX, "elapsed must not overflow past u64::MAX");
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
        app.world_mut().write_message(MoveRequestEvent {
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

        app.world_mut().write_message(AppExit::Success);
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
        app.world_mut().write_message(NewGameRequestEvent { seed: Some(2), mode: None });
        app.update();

        assert!(!path.exists(), "saved file should be deleted after new game");
    }

    #[test]
    fn moving_cards_off_face_down_card_fires_card_flipped_event() {
        use solitaire_core::card::{Card, Rank, Suit};
        let mut app = test_app(1);
        // Build a tableau with two cards: a face-down King at bottom, face-up Queen on top.
        {
            let mut gs = app.world_mut().resource_mut::<GameStateResource>();
            let t = gs.0.piles.get_mut(&PileType::Tableau(0)).unwrap();
            t.cards.clear();
            t.cards.push(Card { id: 900, suit: Suit::Spades, rank: Rank::King, face_up: false });
            t.cards.push(Card { id: 901, suit: Suit::Hearts, rank: Rank::Queen, face_up: true });
        }
        // Set up an empty Tableau(1) for the Queen to land on.
        app.world_mut()
            .resource_mut::<GameStateResource>()
            .0
            .piles
            .get_mut(&PileType::Tableau(1))
            .unwrap()
            .cards
            .clear();

        // A King must be in Tableau(1) for Queen to land there; skip validation
        // by placing a King first.
        {
            let mut gs = app.world_mut().resource_mut::<GameStateResource>();
            let t = gs.0.piles.get_mut(&PileType::Tableau(1)).unwrap();
            t.cards.push(Card { id: 902, suit: Suit::Clubs, rank: Rank::King, face_up: true });
        }

        app.world_mut().write_message(MoveRequestEvent {
            from: PileType::Tableau(0),
            to: PileType::Tableau(1),
            count: 1,
        });
        app.update();

        let events = app.world().resource::<Events<crate::events::CardFlippedEvent>>();
        let mut cursor = events.get_cursor();
        let fired: Vec<_> = cursor.read(events).collect();
        assert_eq!(fired.len(), 1, "CardFlippedEvent must fire when a face-down card is exposed");
        assert_eq!(fired[0].0, 900, "event must carry the flipped card's id");
    }

    /// auto_save_game_state writes to disk once the accumulator crosses 30 s.
    #[test]
    fn auto_save_writes_after_30_seconds() {
        use solitaire_data::load_game_state_from;

        let path = tmp_gs_path("auto_save_30s");
        let _ = std::fs::remove_file(&path);

        let mut app = test_app(42);
        app.insert_resource(GameStatePath(Some(path.clone())));
        // Give the game one move so move_count > 0 (auto-save guard).
        app.world_mut()
            .resource_mut::<GameStateResource>()
            .0
            .move_count = 1;

        // Pre-seed the timer just past the threshold. The system will trigger
        // on the very next update() without needing to control Time::delta_secs().
        app.insert_resource(AutoSaveTimer(AUTO_SAVE_INTERVAL_SECS + 0.1));
        app.update();

        assert!(path.exists(), "auto-save file must exist after timer crosses threshold");
        let loaded = load_game_state_from(&path).expect("file must be loadable");
        assert_eq!(loaded.seed, 42);

        let _ = std::fs::remove_file(&path);
    }

    /// auto_save_game_state does NOT write to disk when no moves have been made.
    #[test]
    fn auto_save_skips_when_no_moves() {
        let path = tmp_gs_path("auto_save_skip");
        let _ = std::fs::remove_file(&path);

        let mut app = test_app(99);
        app.insert_resource(GameStatePath(Some(path.clone())));
        // move_count stays at 0 (fresh game); timer is past threshold.
        app.insert_resource(AutoSaveTimer(AUTO_SAVE_INTERVAL_SECS + 0.1));
        app.update();

        assert!(!path.exists(), "auto-save must not fire when move_count == 0");
    }

    #[test]
    fn moving_cards_off_face_up_card_does_not_fire_card_flipped_event() {
        use solitaire_core::card::{Card, Rank, Suit};
        let mut app = test_app(1);
        // Build a tableau with two face-up cards.
        {
            let mut gs = app.world_mut().resource_mut::<GameStateResource>();
            let t = gs.0.piles.get_mut(&PileType::Tableau(0)).unwrap();
            t.cards.clear();
            t.cards.push(Card { id: 910, suit: Suit::Clubs, rank: Rank::King, face_up: true });
            t.cards.push(Card { id: 911, suit: Suit::Hearts, rank: Rank::Queen, face_up: true });
        }
        app.world_mut()
            .resource_mut::<GameStateResource>()
            .0
            .piles
            .get_mut(&PileType::Tableau(1))
            .unwrap()
            .cards
            .clear();
        {
            let mut gs = app.world_mut().resource_mut::<GameStateResource>();
            gs.0.piles
                .get_mut(&PileType::Tableau(1))
                .unwrap()
                .cards
                .push(Card { id: 912, suit: Suit::Spades, rank: Rank::King, face_up: true });
        }

        app.world_mut().write_message(MoveRequestEvent {
            from: PileType::Tableau(0),
            to: PileType::Tableau(1),
            count: 1,
        });
        app.update();

        let events = app.world().resource::<Events<crate::events::CardFlippedEvent>>();
        let mut cursor = events.get_cursor();
        let fired: Vec<_> = cursor.read(events).collect();
        assert!(fired.is_empty(), "no flip event when exposed card was already face-up");
    }

    // -----------------------------------------------------------------------
    // Task #29 — has_legal_moves pure-function tests
    // -----------------------------------------------------------------------

    #[test]
    fn has_legal_moves_returns_true_when_stock_nonempty() {
        // A fresh game has 24 cards in stock — draw is always available.
        let game = GameState::new(42, DrawMode::DrawOne);
        assert!(has_legal_moves(&game), "draw is always available when stock is non-empty");
    }

    #[test]
    fn has_legal_moves_returns_true_when_ace_can_go_to_foundation() {
        use solitaire_core::card::{Card, Rank, Suit};
        let mut game = GameState::new(1, DrawMode::DrawOne);

        // Empty stock and waste so draw is NOT available.
        game.piles.get_mut(&PileType::Stock).unwrap().cards.clear();
        game.piles.get_mut(&PileType::Waste).unwrap().cards.clear();

        // Clear all tableau and foundations, put Ace of Clubs on tableau 0.
        for suit in [Suit::Clubs, Suit::Diamonds, Suit::Hearts, Suit::Spades] {
            game.piles.get_mut(&PileType::Foundation(suit)).unwrap().cards.clear();
        }
        for i in 0..7_usize {
            game.piles.get_mut(&PileType::Tableau(i)).unwrap().cards.clear();
        }
        game.piles.get_mut(&PileType::Tableau(0)).unwrap().cards.push(Card {
            id: 1, suit: Suit::Clubs, rank: Rank::Ace, face_up: true,
        });

        assert!(has_legal_moves(&game), "Ace can always go to an empty foundation");
    }

    #[test]
    fn has_legal_moves_returns_false_when_stuck() {
        use solitaire_core::card::{Card, Rank, Suit};
        let mut game = GameState::new(1, DrawMode::DrawOne);

        // Empty stock and waste.
        game.piles.get_mut(&PileType::Stock).unwrap().cards.clear();
        game.piles.get_mut(&PileType::Waste).unwrap().cards.clear();

        // Clear all foundations and all tableau.
        for suit in [Suit::Clubs, Suit::Diamonds, Suit::Hearts, Suit::Spades] {
            game.piles.get_mut(&PileType::Foundation(suit)).unwrap().cards.clear();
        }
        for i in 0..7_usize {
            game.piles.get_mut(&PileType::Tableau(i)).unwrap().cards.clear();
        }

        // Place a Two of Clubs with no legal destination.
        game.piles.get_mut(&PileType::Tableau(0)).unwrap().cards.push(Card {
            id: 2, suit: Suit::Clubs, rank: Rank::Two, face_up: true,
        });

        assert!(!has_legal_moves(&game), "Two of Clubs with empty board has no legal move");
    }

    // -----------------------------------------------------------------------
    // Task #57 — Confirm-new-game dialog tests
    // -----------------------------------------------------------------------

    /// Helper that also initialises `ButtonInput<KeyCode>` so the keyboard
    /// systems do not panic in MinimalPlugins environments.
    fn test_app_with_input(seed: u64) -> App {
        let mut app = test_app(seed);
        app.init_resource::<ButtonInput<KeyCode>>();
        app
    }

    #[test]
    fn new_game_request_with_moves_spawns_confirm_dialog() {
        let mut app = test_app_with_input(42);
        // Simulate an active game with moves made.
        app.world_mut().resource_mut::<GameStateResource>().0.move_count = 5;
        app.world_mut()
            .write_message(NewGameRequestEvent { seed: None, mode: None });
        app.update();

        let count = app
            .world_mut()
            .query::<&ConfirmNewGameScreen>()
            .iter(app.world())
            .count();
        assert_eq!(count, 1, "ConfirmNewGameScreen must be spawned when move_count > 0");
    }

    #[test]
    fn new_game_request_on_fresh_game_skips_confirm() {
        let mut app = test_app_with_input(42);
        // move_count stays at 0 (fresh game).
        assert_eq!(
            app.world().resource::<GameStateResource>().0.move_count,
            0,
            "test assumes a fresh game with no moves"
        );
        app.world_mut()
            .write_message(NewGameRequestEvent { seed: None, mode: None });
        app.update();

        let count = app
            .world_mut()
            .query::<&ConfirmNewGameScreen>()
            .iter(app.world())
            .count();
        assert_eq!(count, 0, "ConfirmNewGameScreen must NOT appear for a fresh game");
    }

    // -----------------------------------------------------------------------
    // Task #58 — Game-over overlay tests
    // -----------------------------------------------------------------------

    #[test]
    fn game_over_screen_absent_when_moves_available() {
        // A fresh game always has moves (stock is non-empty).
        let mut app = test_app_with_input(42);
        app.world_mut().write_message(StateChangedEvent);
        app.update();

        let count = app
            .world_mut()
            .query::<&GameOverScreen>()
            .iter(app.world())
            .count();
        assert_eq!(count, 0, "GameOverScreen must not appear when moves are available");
    }

    #[test]
    fn game_over_screen_spawns_when_stuck() {
        use solitaire_core::card::{Card, Rank, Suit};
        let mut app = test_app_with_input(1);

        // Force a stuck state: empty all piles + stock/waste, leave only a
        // Two of Clubs on tableau 0 with no legal destination.
        {
            let mut gs = app.world_mut().resource_mut::<GameStateResource>();
            gs.0.piles.get_mut(&PileType::Stock).unwrap().cards.clear();
            gs.0.piles.get_mut(&PileType::Waste).unwrap().cards.clear();
            for suit in [Suit::Clubs, Suit::Diamonds, Suit::Hearts, Suit::Spades] {
                gs.0.piles.get_mut(&PileType::Foundation(suit)).unwrap().cards.clear();
            }
            for i in 0..7_usize {
                gs.0.piles.get_mut(&PileType::Tableau(i)).unwrap().cards.clear();
            }
            gs.0.piles.get_mut(&PileType::Tableau(0)).unwrap().cards.push(Card {
                id: 1,
                suit: Suit::Clubs,
                rank: Rank::Two,
                face_up: true,
            });
        }

        app.world_mut().write_message(StateChangedEvent);
        app.update();

        let count = app
            .world_mut()
            .query::<&GameOverScreen>()
            .iter(app.world())
            .count();
        assert_eq!(count, 1, "GameOverScreen must appear when no legal moves exist");
    }

    /// Verify that the game-over overlay contains the expected header text and
    /// action-hint strings so players understand why the overlay appeared and
    /// what keys to press.
    #[test]
    fn game_over_screen_text_content() {
        use solitaire_core::card::{Card, Rank, Suit};

        let mut app = test_app_with_input(1);

        // Force a stuck state identical to `game_over_screen_spawns_when_stuck`.
        {
            let mut gs = app.world_mut().resource_mut::<GameStateResource>();
            gs.0.piles.get_mut(&PileType::Stock).unwrap().cards.clear();
            gs.0.piles.get_mut(&PileType::Waste).unwrap().cards.clear();
            for suit in [Suit::Clubs, Suit::Diamonds, Suit::Hearts, Suit::Spades] {
                gs.0.piles.get_mut(&PileType::Foundation(suit)).unwrap().cards.clear();
            }
            for i in 0..7_usize {
                gs.0.piles.get_mut(&PileType::Tableau(i)).unwrap().cards.clear();
            }
            gs.0.piles.get_mut(&PileType::Tableau(0)).unwrap().cards.push(Card {
                id: 1,
                suit: Suit::Clubs,
                rank: Rank::Two,
                face_up: true,
            });
        }

        app.world_mut().write_message(StateChangedEvent);
        app.update();

        // Collect all Text values that are children of the GameOverScreen entity tree.
        let texts: Vec<String> = app
            .world_mut()
            .query::<&Text>()
            .iter(app.world())
            .map(|t| t.0.clone())
            .collect();

        assert!(
            texts.iter().any(|t| t == "No more moves available"),
            "header must read 'No more moves available'; found: {texts:?}"
        );
        assert!(
            texts.iter().any(|t| t == "Press N or Escape for a new game"),
            "hint 1 must read 'Press N or Escape for a new game'; found: {texts:?}"
        );
        assert!(
            texts.iter().any(|t| t == "Press G to forfeit (counts as a loss)"),
            "hint 2 must read 'Press G to forfeit (counts as a loss)'; found: {texts:?}"
        );
    }

    // -----------------------------------------------------------------------
    // Task #56 — Escape dismisses GameOverScreen and starts new game
    // -----------------------------------------------------------------------

    /// Pressing Escape while `GameOverScreen` is visible must fire
    /// `NewGameRequestEvent` — identical behaviour to pressing N.
    #[test]
    fn escape_on_game_over_screen_fires_new_game_request() {
        use solitaire_core::card::{Card, Rank, Suit};

        let mut app = test_app_with_input(1);

        // Force a stuck state so GameOverScreen spawns.
        {
            let mut gs = app.world_mut().resource_mut::<GameStateResource>();
            gs.0.piles.get_mut(&PileType::Stock).unwrap().cards.clear();
            gs.0.piles.get_mut(&PileType::Waste).unwrap().cards.clear();
            for suit in [Suit::Clubs, Suit::Diamonds, Suit::Hearts, Suit::Spades] {
                gs.0.piles.get_mut(&PileType::Foundation(suit)).unwrap().cards.clear();
            }
            for i in 0..7_usize {
                gs.0.piles.get_mut(&PileType::Tableau(i)).unwrap().cards.clear();
            }
            gs.0.piles.get_mut(&PileType::Tableau(0)).unwrap().cards.push(Card {
                id: 1,
                suit: Suit::Clubs,
                rank: Rank::Two,
                face_up: true,
            });
        }
        app.world_mut().write_message(StateChangedEvent);
        app.update();

        // Confirm the overlay is present.
        assert_eq!(
            app.world_mut()
                .query::<&GameOverScreen>()
                .iter(app.world())
                .count(),
            1,
            "GameOverScreen must be present before pressing Escape"
        );

        // Clear the NewGameRequestEvent queue so we start with a clean slate.
        app.world_mut().resource_mut::<Events<NewGameRequestEvent>>().clear();

        // Simulate Escape press.
        {
            let mut input = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            input.clear();
            input.press(KeyCode::Escape);
        }
        app.update();

        // NewGameRequestEvent must have been fired.
        let events = app.world().resource::<Events<NewGameRequestEvent>>();
        let mut reader = events.get_cursor();
        assert!(
            reader.read(events).next().is_some(),
            "Escape on GameOverScreen must fire NewGameRequestEvent"
        );
    }

    // -----------------------------------------------------------------------
    // Task #48 — Undo with empty stack fires InfoToastEvent
    // -----------------------------------------------------------------------

    /// Sending `UndoRequestEvent` on a fresh game (empty undo stack) must fire
    /// exactly one `InfoToastEvent` with the message "Nothing to undo".
    #[test]
    fn undo_on_empty_stack_fires_info_toast() {
        let mut app = test_app(42);
        // Fresh game — undo stack is empty, so undo() returns UndoStackEmpty.
        app.world_mut().write_message(UndoRequestEvent);
        app.update();

        let events = app.world().resource::<Events<InfoToastEvent>>();
        let mut reader = events.get_cursor();
        let fired: Vec<_> = reader.read(events).collect();
        assert_eq!(fired.len(), 1, "exactly one InfoToastEvent must fire on empty-stack undo");
        assert_eq!(
            fired[0].0,
            "Nothing to undo",
            "toast message must be 'Nothing to undo'"
        );
    }

    /// A successful undo must NOT fire an `InfoToastEvent`.
    #[test]
    fn undo_after_draw_does_not_fire_info_toast() {
        let mut app = test_app(42);
        // Make a move so the undo stack is non-empty.
        app.world_mut().write_message(DrawRequestEvent);
        app.update();
        // Clear events from the draw so we start with a clean slate.
        app.world_mut().resource_mut::<Events<InfoToastEvent>>().clear();

        app.world_mut().write_message(UndoRequestEvent);
        app.update();

        let events = app.world().resource::<Events<InfoToastEvent>>();
        let mut reader = events.get_cursor();
        let fired: Vec<_> = reader.read(events).collect();
        assert!(
            fired.is_empty(),
            "no InfoToastEvent must fire on a successful undo"
        );
    }
}
