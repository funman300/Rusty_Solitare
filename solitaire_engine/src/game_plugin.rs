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
use chrono::Utc;
use solitaire_core::game_state::{DrawMode, GameState};
use solitaire_core::pile::PileType;
use solitaire_data::{delete_game_state_at, game_state_file_path, latest_replay_path,
    load_game_state_from, save_game_state_to, save_latest_replay_to, Replay, ReplayMove};

use crate::events::{
    CardFlippedEvent, DrawRequestEvent, FoundationCompletedEvent, GameWonEvent, InfoToastEvent,
    MoveRequestEvent, NewGameRequestEvent, StateChangedEvent, UndoRequestEvent,
};
use crate::font_plugin::FontResource;
use crate::resources::{DragState, GameStateResource, SyncStatusResource};
use crate::ui_modal::{
    spawn_modal, spawn_modal_actions, spawn_modal_body_text, spawn_modal_button,
    spawn_modal_header, ButtonVariant,
};
use crate::ui_theme;

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

/// Persistence path for the most recent winning replay. `None` disables I/O.
#[derive(Resource, Debug, Clone)]
pub struct ReplayPath(pub Option<PathBuf>);

/// In-memory accumulator for [`ReplayMove`] entries during the current
/// game. Cleared on every new-game start; frozen into a [`Replay`] and
/// flushed to disk by [`record_replay_on_win`] when the player wins.
///
/// Recording captures only successful state-mutating events the player
/// drove (`MoveRequestEvent`, `DrawRequestEvent`). `UndoRequestEvent` is
/// intentionally not recorded — see [`solitaire_data::replay`] for the
/// design rationale.
#[derive(Resource, Debug, Default, Clone)]
pub struct RecordingReplay {
    /// Ordered list of moves applied so far this game.
    pub moves: Vec<ReplayMove>,
}

impl RecordingReplay {
    /// Reset the recording. Called on every `NewGameRequestEvent` so a
    /// fresh deal starts with an empty move list.
    pub fn clear(&mut self) {
        self.moves.clear();
    }
}

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
            .insert_resource(ReplayPath(latest_replay_path()))
            .init_resource::<RecordingReplay>()
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
            .add_message::<FoundationCompletedEvent>()
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
            .add_systems(Update, record_replay_on_win.after(GameMutation))
            .add_systems(Update, handle_confirm_input.after(GameMutation))
            .add_systems(Update, handle_confirm_button_input.after(GameMutation))
            .add_systems(Update, handle_game_over_input.after(GameMutation))
            .add_systems(Update, handle_game_over_button_input.after(GameMutation))
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
        .map_or(0, |d| d.as_nanos() as u64)
}

#[allow(clippy::too_many_arguments)]
fn handle_new_game(
    mut commands: Commands,
    mut new_game: MessageReader<NewGameRequestEvent>,
    mut game: ResMut<GameStateResource>,
    mut changed: MessageWriter<StateChangedEvent>,
    mut recording: ResMut<RecordingReplay>,
    settings: Option<Res<crate::settings_plugin::SettingsResource>>,
    path: Option<Res<GameStatePath>>,
    font_res: Option<Res<FontResource>>,
    confirm_screens: Query<Entity, With<ConfirmNewGameScreen>>,
    game_over_screens: Query<Entity, With<GameOverScreen>>,
    layout: Option<Res<crate::layout::LayoutResource>>,
    mut card_transforms: Query<&mut Transform, With<crate::card_plugin::CardEntity>>,
) {
    for ev in new_game.read() {
        // If an active game is in progress, intercept and show a confirm dialog.
        // A game is "active" when moves have been made and it is not yet won.
        let needs_confirm = game.0.move_count > 0 && !game.0.is_won;
        // Skip confirmation if a ConfirmNewGameScreen already exists (prevents
        // duplicates) or if the event itself was already confirmed by the
        // player pressing Y on the modal — without the `confirmed` check the
        // modal would be respawned the frame after the despawn flushes.
        let confirm_already_open = !confirm_screens.is_empty();
        if needs_confirm && !confirm_already_open && !ev.confirmed {
            // Despawn any stale game-over overlay before showing confirm dialog.
            for entity in &game_over_screens {
                commands.entity(entity).despawn();
            }
            spawn_confirm_dialog(&mut commands, *ev, font_res.as_deref());
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
            .map_or_else(|| game.0.draw_mode.clone(), |s| s.0.draw_mode.clone());
        let mode = ev.mode.unwrap_or(game.0.mode);
        game.0 = GameState::new_with_mode(seed, draw_mode, mode);
        // Reset the in-flight replay buffer — a fresh deal starts with
        // an empty move list. The previously saved replay on disk
        // (latest_replay.json) is preserved until the player wins again.
        recording.clear();
        // Delete any previously saved in-progress state — this is a fresh game.
        if let Some(p) = path.as_ref().and_then(|r| r.0.as_deref())
            && let Err(e) = delete_game_state_at(p) {
                warn!("game_state: failed to delete saved game: {e}");
            }
        // Snap every existing card sprite to the stock position before the
        // deal animation starts. Without this the per-card slide tween reads
        // each card's previous-game Transform as its source, which lets a
        // careful observer track origin points to deduce where face-down
        // cards came from. Funnelling all sprites through the deck position
        // hides that information and reads naturally as "dealt from the
        // deck." Skipped when LayoutResource isn't present (headless tests).
        if let Some(layout) = layout.as_ref()
            && let Some(stock) = layout
                .0
                .pile_positions
                .get(&solitaire_core::pile::PileType::Stock)
        {
            for mut tx in &mut card_transforms {
                tx.translation.x = stock.x;
                tx.translation.y = stock.y;
            }
        }
        changed.write(StateChangedEvent);
    }
}

/// Marker on the primary "New game" button inside the confirm modal.
#[derive(Component, Debug)]
pub struct ConfirmYesButton;

/// Marker on the secondary "Cancel" button inside the confirm modal.
#[derive(Component, Debug)]
pub struct ConfirmNoButton;

/// Spawns the confirm-new-game modal using the standard `ui_modal`
/// primitive — uniform scrim, centred card, real buttons with hover /
/// press states.
///
/// Shown when the player requests a new game while moves have been made
/// and the game is not yet won. The original `NewGameRequestEvent` is
/// stored on the scrim entity so `handle_confirm_input` /
/// `handle_confirm_button` can replay it with the same seed / mode on
/// confirmation.
///
/// Replaces a bespoke layout that used plain `Text` labels for "Yes (Y)"
/// and "No (N)" — those were not real Button entities, so the player
/// had no hover / press feedback and the modal felt like a debug panel
/// (the user's smoke-test "#2 complaint").
fn spawn_confirm_dialog(
    commands: &mut Commands,
    original_request: NewGameRequestEvent,
    font_res: Option<&FontResource>,
) {
    let scrim = spawn_modal(
        commands,
        ConfirmNewGameScreen,
        ui_theme::Z_MODAL_PANEL,
        |card| {
            spawn_modal_header(card, "Abandon current game?", font_res);
            spawn_modal_body_text(
                card,
                "Your progress will be lost.",
                ui_theme::TEXT_SECONDARY,
                font_res,
            );
            spawn_modal_actions(card, |actions| {
                spawn_modal_button(
                    actions,
                    ConfirmNoButton,
                    "Cancel",
                    Some("Esc"),
                    ButtonVariant::Secondary,
                    font_res,
                );
                spawn_modal_button(
                    actions,
                    ConfirmYesButton,
                    "New game",
                    Some("Y"),
                    ButtonVariant::Primary,
                    font_res,
                );
            });
        },
    );
    // Attach the original request to the scrim so handle_confirm_input
    // and handle_confirm_button can read it on confirmation.
    commands
        .entity(scrim)
        .insert(OriginalNewGameRequest(original_request));
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
        // Set `confirmed: true` so handle_new_game skips the dialog spawn
        // and goes straight to the start-game branch. Without this flag the
        // modal would respawn the frame after the despawn flushes (because
        // confirm_screens is empty by then) and the new game would never
        // actually start.
        new_game.write(NewGameRequestEvent {
            seed: original.0.seed,
            mode: original.0.mode,
            confirmed: true,
        });
    } else if cancelled {
        commands.entity(entity).despawn();
    }
}

/// Mouse / touch counterpart to `handle_confirm_input`. Reads
/// `Changed<Interaction>` on the modal's `ConfirmYesButton` /
/// `ConfirmNoButton` so the modal closes and (on confirm) starts a new
/// game whether the player uses the keyboard accelerator or clicks.
///
/// This is the system that closes the user's #2 smoke-test complaint:
/// previously the dialog had only `Text::new("Yes (Y)")` labels — not
/// real button entities — so clicks did nothing and the only path
/// through the modal was the keyboard.
#[allow(clippy::too_many_arguments)]
fn handle_confirm_button_input(
    mut commands: Commands,
    yes_buttons: Query<&Interaction, (With<ConfirmYesButton>, Changed<Interaction>)>,
    no_buttons: Query<&Interaction, (With<ConfirmNoButton>, Changed<Interaction>)>,
    screens: Query<(Entity, &OriginalNewGameRequest), With<ConfirmNewGameScreen>>,
    mut new_game: MessageWriter<NewGameRequestEvent>,
) {
    let Ok((entity, original)) = screens.single() else {
        return;
    };
    let confirmed = yes_buttons.iter().any(|i| *i == Interaction::Pressed);
    let cancelled = no_buttons.iter().any(|i| *i == Interaction::Pressed);

    if confirmed {
        commands.entity(entity).despawn();
        new_game.write(NewGameRequestEvent {
            seed: original.0.seed,
            mode: original.0.mode,
            confirmed: true,
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
    mut recording: ResMut<RecordingReplay>,
) {
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
                // Record the atomic player input. Whether the engine
                // resolves this to a draw or a waste→stock recycle is
                // a deterministic function of stock state at the time
                // the click happens — re-executing on the same starting
                // deal produces the same effect, so the input alone is
                // sufficient to recover the move on playback.
                recording.moves.push(ReplayMove::StockClick);
                changed.write(StateChangedEvent);
            }
            Err(e) => warn!("draw rejected: {e}"),
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_move(
    mut moves: MessageReader<MoveRequestEvent>,
    mut game: ResMut<GameStateResource>,
    mut changed: MessageWriter<StateChangedEvent>,
    mut won: MessageWriter<GameWonEvent>,
    mut flipped: MessageWriter<crate::events::CardFlippedEvent>,
    mut foundation_done: MessageWriter<FoundationCompletedEvent>,
    mut recording: ResMut<RecordingReplay>,
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
                // Record the move in the in-flight replay buffer. Done
                // first so the entry is captured even if a subsequent
                // event-write or pile-lookup happens to bail out below.
                recording.moves.push(ReplayMove::Move {
                    from: ev.from.clone(),
                    to: ev.to.clone(),
                    count: ev.count,
                });
                // Fire flip event if the candidate card is now face-up.
                if let Some(fid) = flip_candidate_id
                    && game.0.piles.get(&ev.from)
                        .and_then(|p| p.cards.last())
                        .is_some_and(|c| c.id == fid && c.face_up)
                    {
                        flipped.write(crate::events::CardFlippedEvent(fid));
                    }
                // If this move landed on a foundation pile and that pile is
                // now complete (Ace → King, 13 cards), fire the per-suit
                // flourish event. Drives a brief decorative scale-pulse on
                // the King + a golden tint on the foundation marker plus a
                // short audio ping. Purely a UI / audio cue — does not
                // cross `solitaire_sync` and is not persisted.
                if let PileType::Foundation(slot) = ev.to
                    && let Some(pile) = game.0.piles.get(&ev.to)
                    && pile.cards.len() == 13
                    && let Some(suit) = pile.claimed_suit()
                {
                    foundation_done.write(FoundationCompletedEvent { slot, suit });
                }
                changed.write(StateChangedEvent);
                if !was_won && game.0.is_won {
                    won.write(GameWonEvent {
                        score: game.0.score,
                        time_seconds: game.0.elapsed_seconds,
                    });
                    // Delete the saved state — a won game should not be resumed.
                    if let Some(p) = path.as_ref().and_then(|r| r.0.as_deref())
                        && let Err(e) = delete_game_state_at(p) {
                            warn!("game_state: failed to delete on win: {e}");
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

/// On every `GameWonEvent`, freeze the in-flight [`RecordingReplay`] into
/// a [`Replay`] tagged with the deal seed/mode, the win's score and
/// elapsed time, and today's date — then persist it atomically to
/// `<data_dir>/solitaire_quest/latest_replay.json` (or to whichever path
/// `ReplayPath` carries; tests inject a temp path).
///
/// Only the most recent winning replay is retained — the existing file is
/// overwritten. The recording buffer is left intact after the win so a
/// subsequent state-change does not erase the move list before the save
/// completes; it gets cleared on the next `NewGameRequestEvent`.
pub fn record_replay_on_win(
    mut wins: MessageReader<GameWonEvent>,
    game: Res<GameStateResource>,
    recording: Res<RecordingReplay>,
    path: Option<Res<ReplayPath>>,
) {
    for ev in wins.read() {
        // Skip persistence when the recording is empty. This guards
        // against unrelated tests in other plugins that synthesise a
        // `GameWonEvent` (e.g. to exercise XP / streak / weekly goal
        // logic) without driving any actual moves — those wins should
        // not silently overwrite the developer's real replay file.
        // A real win always has at least one recorded `Move`.
        if recording.moves.is_empty() {
            continue;
        }
        let replay = Replay::new(
            game.0.seed,
            game.0.draw_mode.clone(),
            game.0.mode,
            ev.time_seconds,
            ev.score,
            Utc::now().date_naive(),
            recording.moves.clone(),
        );
        let Some(p) = path.as_ref().and_then(|r| r.0.as_deref()) else {
            // No persistence path configured (e.g. tests / minimal Linux
            // containers without dirs::data_dir). The in-memory replay
            // is still available via the resource for callers that want
            // to inspect it without going through the disk.
            continue;
        };
        if let Err(e) = save_latest_replay_to(p, &replay) {
            warn!("replay: failed to save winning replay: {e}");
        }
    }
}

// ---------------------------------------------------------------------------
// Task #29 — No-moves detection
// ---------------------------------------------------------------------------

/// Returns `true` if the current game state has at least one legal move
/// that could ever lead to progress.
///
/// Considers a card "playable" if it's currently face-up on the top of
/// the Waste or any Tableau, OR if it lives in the Stock or Waste pile
/// at all (every card in those piles eventually rotates through the
/// Waste's top in both Draw-One and Draw-Three over the course of
/// recycling). For each such candidate, checks whether it can land on
/// any Foundation or any Tableau in the current state.
///
/// Returns `false` only when *no* card anywhere can land anywhere —
/// the player can keep drawing through the stock forever and nothing
/// will ever come of it. This treats "draw cycle with no useful drop"
/// as a softlock rather than as "legal moves remain", which the
/// previous heuristic incorrectly did (Quat hit this with 4 cards
/// remaining and the game just sat there).
pub fn has_legal_moves(game: &GameState) -> bool {
    use solitaire_core::card::Card;
    use solitaire_core::pile::PileType;
    use solitaire_core::rules::{can_place_on_foundation, can_place_on_tableau};

    let mut sources: Vec<Card> = Vec::new();
    for ty in [PileType::Stock, PileType::Waste] {
        if let Some(p) = game.piles.get(&ty) {
            sources.extend(p.cards.iter().cloned());
        }
    }
    for i in 0..7_usize {
        if let Some(t) = game.piles.get(&PileType::Tableau(i))
            && let Some(top) = t.cards.last().filter(|c| c.face_up)
        {
            sources.push(top.clone());
        }
    }

    for card in &sources {
        for slot in 0..4_u8 {
            if let Some(dest) = game.piles.get(&PileType::Foundation(slot))
                && can_place_on_foundation(card, dest)
            {
                return true;
            }
        }
        for i in 0..7_usize {
            if let Some(dest) = game.piles.get(&PileType::Tableau(i))
                && can_place_on_tableau(card, dest)
            {
                return true;
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
#[allow(clippy::too_many_arguments)]
fn check_no_moves(
    mut commands: Commands,
    mut events: MessageReader<StateChangedEvent>,
    game: Res<GameStateResource>,
    mut toast: MessageWriter<InfoToastEvent>,
    mut already_fired: Local<bool>,
    game_over_screens: Query<Entity, With<GameOverScreen>>,
    font_res: Option<Res<FontResource>>,
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
            spawn_game_over_screen(&mut commands, game.0.score, font_res.as_deref());
        }
    }
}

/// Marker on the "Undo" secondary button inside the game-over modal.
#[derive(Component, Debug)]
pub struct GameOverUndoButton;

/// Marker on the "New Game" primary button inside the game-over modal.
#[derive(Component, Debug)]
pub struct GameOverNewGameButton;

/// Spawns the game-over modal using the standard `ui_modal` primitive.
///
/// Replaces a bespoke layout that listed action hints as plain text
/// ("Press N for a new game", "Press G to forfeit") — the audit
/// flagged this as the same class of "feels like a debug panel"
/// problem the confirm modal had. Now there are real buttons with
/// hover/press feedback; the keyboard accelerators stay as optional
/// shortcuts displayed inside the buttons' caption chips.
fn spawn_game_over_screen(
    commands: &mut Commands,
    score: i32,
    font_res: Option<&FontResource>,
) {
    spawn_modal(
        commands,
        GameOverScreen,
        ui_theme::Z_MODAL_PANEL,
        |card| {
            spawn_modal_header(card, "No more moves available", font_res);
            spawn_modal_body_text(
                card,
                format!("Final score: {score}"),
                ui_theme::TEXT_PRIMARY,
                font_res,
            );
            spawn_modal_actions(card, |actions| {
                spawn_modal_button(
                    actions,
                    GameOverUndoButton,
                    "Undo",
                    Some("U"),
                    ButtonVariant::Secondary,
                    font_res,
                );
                spawn_modal_button(
                    actions,
                    GameOverNewGameButton,
                    "New Game",
                    Some("N"),
                    ButtonVariant::Primary,
                    font_res,
                );
            });
        },
    );
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

/// Mouse / touch counterpart to `handle_game_over_input`. Click on the
/// modal's Undo button → fire `UndoRequestEvent` and despawn so
/// `check_no_moves` can re-evaluate. Click on New Game → fire
/// `NewGameRequestEvent` (the abandon-current-game guard does not apply
/// here because the game is unwinnable).
#[allow(clippy::type_complexity)]
fn handle_game_over_button_input(
    mut commands: Commands,
    new_game_buttons: Query<&Interaction, (With<GameOverNewGameButton>, Changed<Interaction>)>,
    undo_buttons: Query<&Interaction, (With<GameOverUndoButton>, Changed<Interaction>)>,
    screens: Query<Entity, With<GameOverScreen>>,
    mut new_game: MessageWriter<NewGameRequestEvent>,
    mut undo: MessageWriter<UndoRequestEvent>,
) {
    if screens.is_empty() {
        return;
    }
    if new_game_buttons.iter().any(|i| *i == Interaction::Pressed) {
        new_game.write(NewGameRequestEvent::default());
    } else if undo_buttons.iter().any(|i| *i == Interaction::Pressed) {
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
        // Disable I/O — tests must not touch the real game state file or
        // the real replay file. Both default to dirs::data_dir() in the
        // plugin's build path; clearing them keeps tests self-contained.
        app.insert_resource(GameStatePath(None));
        app.insert_resource(ReplayPath(None));
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
        let events = app.world().resource::<Messages<StateChangedEvent>>();
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

        app.world_mut().write_message(NewGameRequestEvent { seed: Some(999), mode: None, confirmed: false });
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
        let events = app.world().resource::<Messages<StateChangedEvent>>();
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
        app.world_mut().write_message(NewGameRequestEvent { seed: Some(2), mode: None, confirmed: false });
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

        let events = app.world().resource::<Messages<crate::events::CardFlippedEvent>>();
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

        let events = app.world().resource::<Messages<crate::events::CardFlippedEvent>>();
        let mut cursor = events.get_cursor();
        let fired: Vec<_> = cursor.read(events).collect();
        assert!(fired.is_empty(), "no flip event when exposed card was already face-up");
    }

    // -----------------------------------------------------------------------
    // Task #29 — has_legal_moves pure-function tests
    // -----------------------------------------------------------------------

    #[test]
    fn has_legal_moves_returns_true_for_fresh_game() {
        // A fresh deal always contains at least one playable card —
        // typically several tableau→tableau opportunities plus any Aces
        // that surface as a tableau column's bottom card.
        let game = GameState::new(42, DrawMode::DrawOne);
        assert!(has_legal_moves(&game), "fresh deal must contain at least one legal move");
    }

    #[test]
    fn has_legal_moves_returns_false_when_stock_only_holds_unplayable_cards() {
        // Reproduces Quat's softlock: stock has cards but no card anywhere
        // (stock or otherwise) can land on any pile. The previous heuristic
        // returned `true` here because stock was non-empty, so the game
        // sat there forever instead of declaring softlock.
        use solitaire_core::card::{Card, Rank, Suit};
        let mut game = GameState::new(1, DrawMode::DrawOne);
        for slot in 0..4_u8 {
            game.piles.get_mut(&PileType::Foundation(slot)).unwrap().cards.clear();
        }
        for i in 0..7_usize {
            game.piles.get_mut(&PileType::Tableau(i)).unwrap().cards.clear();
        }
        game.piles.get_mut(&PileType::Waste).unwrap().cards.clear();
        // Fill foundation 0 with Clubs A–10, leaving only J/Q/K of Clubs
        // as plausible foundation moves; load the stock with cards that
        // can't land on the empty tableau (anything but a King) and can't
        // extend foundation 0 (anything but Jack of Clubs).
        let stock = game.piles.get_mut(&PileType::Stock).unwrap();
        stock.cards.clear();
        for r in [Rank::Two, Rank::Three, Rank::Four, Rank::Five] {
            stock.cards.push(Card { id: 100 + r as u32, suit: Suit::Hearts, rank: r, face_up: false });
        }
        let foundation_zero = game.piles.get_mut(&PileType::Foundation(0)).unwrap();
        for r in [
            Rank::Ace, Rank::Two, Rank::Three, Rank::Four, Rank::Five,
            Rank::Six, Rank::Seven, Rank::Eight, Rank::Nine, Rank::Ten,
        ] {
            foundation_zero.cards.push(Card { id: r as u32, suit: Suit::Clubs, rank: r, face_up: true });
        }
        assert!(
            !has_legal_moves(&game),
            "stock cards with no legal landing should count as softlock",
        );
    }

    #[test]
    fn has_legal_moves_returns_true_when_ace_can_go_to_foundation() {
        use solitaire_core::card::{Card, Rank, Suit};
        let mut game = GameState::new(1, DrawMode::DrawOne);

        // Empty stock and waste so draw is NOT available.
        game.piles.get_mut(&PileType::Stock).unwrap().cards.clear();
        game.piles.get_mut(&PileType::Waste).unwrap().cards.clear();

        // Clear all tableau and foundations, put Ace of Clubs on tableau 0.
        for slot in 0..4_u8 {
            game.piles.get_mut(&PileType::Foundation(slot)).unwrap().cards.clear();
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
        for slot in 0..4_u8 {
            game.piles.get_mut(&PileType::Foundation(slot)).unwrap().cards.clear();
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
            .write_message(NewGameRequestEvent { seed: None, mode: None, confirmed: false });
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
            .write_message(NewGameRequestEvent { seed: None, mode: None, confirmed: false });
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
            for slot in 0..4_u8 {
                gs.0.piles.get_mut(&PileType::Foundation(slot)).unwrap().cards.clear();
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
            for slot in 0..4_u8 {
                gs.0.piles.get_mut(&PileType::Foundation(slot)).unwrap().cards.clear();
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
        // The modal now uses real buttons instead of plain action-hint
        // text, so we assert on the button labels and their hotkey
        // chips rather than the prior "Press N…" / "Press G…" prose.
        assert!(
            texts.iter().any(|t| t == "New Game"),
            "primary action button must label 'New Game'; found: {texts:?}"
        );
        assert!(
            texts.iter().any(|t| t == "N"),
            "primary action must show its 'N' hotkey chip; found: {texts:?}"
        );
        assert!(
            texts.iter().any(|t| t == "Undo"),
            "secondary action button must label 'Undo'; found: {texts:?}"
        );
        assert!(
            texts.iter().any(|t| t == "U"),
            "secondary action must show its 'U' hotkey chip; found: {texts:?}"
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
            for slot in 0..4_u8 {
                gs.0.piles.get_mut(&PileType::Foundation(slot)).unwrap().cards.clear();
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
        app.world_mut().resource_mut::<Messages<NewGameRequestEvent>>().clear();

        // Simulate Escape press.
        {
            let mut input = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            input.clear();
            input.press(KeyCode::Escape);
        }
        app.update();

        // NewGameRequestEvent must have been fired.
        let events = app.world().resource::<Messages<NewGameRequestEvent>>();
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

        let events = app.world().resource::<Messages<InfoToastEvent>>();
        let mut reader = events.get_cursor();
        let fired: Vec<_> = reader.read(events).collect();
        assert_eq!(fired.len(), 1, "exactly one InfoToastEvent must fire on empty-stack undo");
        assert_eq!(
            fired[0].0,
            "Nothing to undo",
            "toast message must be 'Nothing to undo'"
        );
    }

    // -----------------------------------------------------------------------
    // Foundation-completion flourish — FoundationCompletedEvent firing logic
    // -----------------------------------------------------------------------

    /// Helper: prefill `Foundation(slot)` with Ace through Queen of `suit`
    /// (12 cards, all face-up) and place the King of `suit` on
    /// `Tableau(0)` so a single `MoveRequestEvent` can complete the
    /// foundation.
    fn seed_foundation_with_ace_through_queen(
        app: &mut App,
        slot: u8,
        suit: solitaire_core::card::Suit,
    ) {
        use solitaire_core::card::{Card, Rank};

        let ranks = [
            Rank::Ace, Rank::Two, Rank::Three, Rank::Four, Rank::Five, Rank::Six,
            Rank::Seven, Rank::Eight, Rank::Nine, Rank::Ten, Rank::Jack, Rank::Queen,
        ];
        let mut gs = app.world_mut().resource_mut::<GameStateResource>();
        let foundation = gs
            .0
            .piles
            .get_mut(&PileType::Foundation(slot))
            .expect("foundation slot must exist");
        foundation.cards.clear();
        for (i, &rank) in ranks.iter().enumerate() {
            foundation.cards.push(Card {
                id: 5_000 + i as u32 + (slot as u32) * 100,
                suit,
                rank,
                face_up: true,
            });
        }
        // Put the King on Tableau(0) so a single move can complete it.
        let t0 = gs.0.piles.get_mut(&PileType::Tableau(0)).unwrap();
        t0.cards.clear();
        t0.cards.push(Card {
            id: 6_000 + (slot as u32),
            suit,
            rank: Rank::King,
            face_up: true,
        });
    }

    /// Reading helper: collect every `FoundationCompletedEvent` written
    /// during the most recent `update()` so the test body can assert
    /// against count, slot, and suit.
    fn drain_foundation_events(app: &App) -> Vec<FoundationCompletedEvent> {
        let events = app
            .world()
            .resource::<Messages<FoundationCompletedEvent>>();
        let mut cursor = events.get_cursor();
        cursor.read(events).copied().collect()
    }

    /// When a King lands on a foundation that already holds Ace through
    /// Queen, exactly one `FoundationCompletedEvent` must fire and carry
    /// the matching slot + suit.
    #[test]
    fn foundation_completed_event_fires_when_king_lands() {
        use solitaire_core::card::Suit;

        let mut app = test_app(1);
        seed_foundation_with_ace_through_queen(&mut app, 2, Suit::Hearts);

        app.world_mut().write_message(MoveRequestEvent {
            from: PileType::Tableau(0),
            to: PileType::Foundation(2),
            count: 1,
        });
        app.update();

        let fired = drain_foundation_events(&app);
        assert_eq!(
            fired.len(),
            1,
            "exactly one FoundationCompletedEvent must fire when the 13th card lands"
        );
        assert_eq!(fired[0].slot, 2, "event slot must match the destination slot");
        assert_eq!(fired[0].suit, Suit::Hearts, "event suit must match the foundation suit");
    }

    /// Moving a card to a tableau pile must never produce a
    /// `FoundationCompletedEvent`, even if the source tableau happened
    /// to have been a King.
    #[test]
    fn foundation_completed_event_does_not_fire_for_non_foundation_moves() {
        use solitaire_core::card::{Card, Rank, Suit};

        let mut app = test_app(1);
        // Reset the world: clear stock + waste so a draw isn't possible,
        // empty all tableaux + foundations, then place a face-up King of
        // Spades on Tableau(0). Tableau(1) is empty, so the King can move
        // there legally.
        {
            let mut gs = app.world_mut().resource_mut::<GameStateResource>();
            gs.0.piles.get_mut(&PileType::Stock).unwrap().cards.clear();
            gs.0.piles.get_mut(&PileType::Waste).unwrap().cards.clear();
            for slot in 0..4_u8 {
                gs.0.piles.get_mut(&PileType::Foundation(slot)).unwrap().cards.clear();
            }
            for i in 0..7_usize {
                gs.0.piles.get_mut(&PileType::Tableau(i)).unwrap().cards.clear();
            }
            gs.0.piles.get_mut(&PileType::Tableau(0)).unwrap().cards.push(Card {
                id: 7_000,
                suit: Suit::Spades,
                rank: Rank::King,
                face_up: true,
            });
        }

        app.world_mut().write_message(MoveRequestEvent {
            from: PileType::Tableau(0),
            to: PileType::Tableau(1),
            count: 1,
        });
        app.update();

        let fired = drain_foundation_events(&app);
        assert!(
            fired.is_empty(),
            "FoundationCompletedEvent must not fire for non-foundation moves; got {fired:?}"
        );
    }

    /// At 12 cards on a foundation (Ace–Jack on the pile, Queen in
    /// flight), the event must NOT fire — the flourish is only for the
    /// final 13th completion.
    #[test]
    fn foundation_completed_event_does_not_fire_at_12_cards() {
        use solitaire_core::card::{Card, Rank, Suit};

        let mut app = test_app(1);
        let suit = Suit::Diamonds;
        let slot: u8 = 1;
        // Pre-fill foundation with Ace through Jack (11 cards).
        let pre_ranks = [
            Rank::Ace, Rank::Two, Rank::Three, Rank::Four, Rank::Five, Rank::Six,
            Rank::Seven, Rank::Eight, Rank::Nine, Rank::Ten, Rank::Jack,
        ];
        {
            let mut gs = app.world_mut().resource_mut::<GameStateResource>();
            let foundation = gs.0.piles.get_mut(&PileType::Foundation(slot)).unwrap();
            foundation.cards.clear();
            for (i, &rank) in pre_ranks.iter().enumerate() {
                foundation.cards.push(Card {
                    id: 8_000 + i as u32,
                    suit,
                    rank,
                    face_up: true,
                });
            }
            // Queen on Tableau(0) so a single move pushes the foundation
            // count to exactly 12 (still below the completion threshold).
            let t0 = gs.0.piles.get_mut(&PileType::Tableau(0)).unwrap();
            t0.cards.clear();
            t0.cards.push(Card {
                id: 8_900,
                suit,
                rank: Rank::Queen,
                face_up: true,
            });
        }

        app.world_mut().write_message(MoveRequestEvent {
            from: PileType::Tableau(0),
            to: PileType::Foundation(slot),
            count: 1,
        });
        app.update();

        // Sanity: the move actually landed (foundation has 12 cards now).
        let foundation_len = app
            .world()
            .resource::<GameStateResource>()
            .0
            .piles[&PileType::Foundation(slot)]
            .cards
            .len();
        assert_eq!(foundation_len, 12, "Queen must have landed on the foundation");

        let fired = drain_foundation_events(&app);
        assert!(
            fired.is_empty(),
            "FoundationCompletedEvent must not fire at 12 cards; got {fired:?}"
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
        app.world_mut().resource_mut::<Messages<InfoToastEvent>>().clear();

        app.world_mut().write_message(UndoRequestEvent);
        app.update();

        let events = app.world().resource::<Messages<InfoToastEvent>>();
        let mut reader = events.get_cursor();
        let fired: Vec<_> = reader.read(events).collect();
        assert!(
            fired.is_empty(),
            "no InfoToastEvent must fire on a successful undo"
        );
    }

    // -----------------------------------------------------------------------
    // Win-game replay recording
    //
    // The recording resource captures exactly the player-driven actions
    // that successfully advanced GameState. On GameWonEvent it freezes
    // into a Replay (with seed/mode/time/score metadata) and persists.
    // -----------------------------------------------------------------------

    /// Set up Tableau(0) with a face-up Ace of Clubs that can be moved
    /// to the empty Foundation(0) — gives us a single deterministic move
    /// to drive the recording without depending on the dealt layout.
    fn seed_single_legal_move(app: &mut App) {
        use solitaire_core::card::{Card, Rank, Suit};
        let mut gs = app.world_mut().resource_mut::<GameStateResource>();
        let t0 = gs.0.piles.get_mut(&PileType::Tableau(0)).unwrap();
        t0.cards.clear();
        t0.cards.push(Card {
            id: 999,
            suit: Suit::Clubs,
            rank: Rank::Ace,
            face_up: true,
        });
        let f0 = gs.0.piles.get_mut(&PileType::Foundation(0)).unwrap();
        f0.cards.clear();
    }

    /// Drive a fresh game through a draw + a tableau→foundation move,
    /// then assert the recording resource captured both, in order, with
    /// the correct shape.
    #[test]
    fn replay_records_moves_in_order() {
        let mut app = test_app(42);

        // Move 1: a draw against a non-empty stock.
        app.world_mut().write_message(DrawRequestEvent);
        app.update();

        // Move 2: a real card move from tableau to foundation.
        seed_single_legal_move(&mut app);
        app.world_mut().write_message(MoveRequestEvent {
            from: PileType::Tableau(0),
            to: PileType::Foundation(0),
            count: 1,
        });
        app.update();

        // Move 3: another draw.
        app.world_mut().write_message(DrawRequestEvent);
        app.update();

        let recording = app.world().resource::<RecordingReplay>();
        assert_eq!(
            recording.moves.len(),
            3,
            "recording must capture exactly the three successful actions",
        );
        assert!(
            matches!(recording.moves[0], ReplayMove::StockClick),
            "first entry must be StockClick, got {:?}",
            recording.moves[0],
        );
        match &recording.moves[1] {
            ReplayMove::Move { from, to, count } => {
                assert_eq!(*from, PileType::Tableau(0), "from pile must be Tableau(0)");
                assert_eq!(*to, PileType::Foundation(0), "to pile must be Foundation(0)");
                assert_eq!(*count, 1, "single-card move must have count 1");
            }
            other => panic!("second entry must be a Move, got {other:?}"),
        }
        assert!(
            matches!(recording.moves[2], ReplayMove::StockClick),
            "third entry must be StockClick, got {:?}",
            recording.moves[2],
        );
    }

    /// Invalid moves must not appear in the recording — the recording is
    /// "what successfully happened", not "what was requested".
    #[test]
    fn replay_does_not_record_rejected_moves() {
        let mut app = test_app(42);
        // Stock → Waste is InvalidDestination; the live engine rejects it.
        app.world_mut().write_message(MoveRequestEvent {
            from: PileType::Stock,
            to: PileType::Waste,
            count: 1,
        });
        app.update();

        let recording = app.world().resource::<RecordingReplay>();
        assert!(
            recording.moves.is_empty(),
            "rejected moves must not enter the recording, got {:?}",
            recording.moves,
        );
    }

    /// Undo intentionally does NOT enter the recording. The replay
    /// represents the canonical path the player took to win, not the
    /// missteps that were rolled back.
    #[test]
    fn replay_recording_skips_undo() {
        let mut app = test_app(42);
        app.world_mut().write_message(DrawRequestEvent);
        app.update();
        app.world_mut().write_message(UndoRequestEvent);
        app.update();

        let recording = app.world().resource::<RecordingReplay>();
        assert_eq!(
            recording.moves.len(),
            1,
            "only the draw is recorded; the undo does not erase it nor add a new entry",
        );
        assert!(matches!(recording.moves[0], ReplayMove::StockClick));
    }

    /// Starting a new game wipes the recording so the next deal begins
    /// with a clean buffer.
    #[test]
    fn replay_recording_clears_on_new_game() {
        let mut app = test_app(1);
        app.world_mut().write_message(DrawRequestEvent);
        app.update();
        assert_eq!(
            app.world().resource::<RecordingReplay>().moves.len(),
            1,
            "draw should have been recorded",
        );

        // Use `confirmed: true` so the request bypasses the
        // abandon-current-game modal (which fires when move_count > 0)
        // and goes straight to the new-game branch that clears the
        // recording. The modal-spawn path is exercised by other tests
        // in this module.
        app.world_mut().write_message(NewGameRequestEvent {
            seed: Some(2),
            mode: None,
            confirmed: true,
        });
        app.update();

        let recording = app.world().resource::<RecordingReplay>();
        assert!(
            recording.moves.is_empty(),
            "recording must be cleared on new-game start; got {:?}",
            recording.moves,
        );
    }

    /// On `GameWonEvent`, the recording is frozen into a `Replay` and
    /// persisted. We point `ReplayPath` at a temp file, fake a win, and
    /// load the file back to assert the metadata + move list match.
    #[test]
    fn replay_recording_freezes_into_replay_on_game_won() {
        use solitaire_data::load_latest_replay_from;

        let path = std::env::temp_dir().join("engine_test_replay_freeze.json");
        let _ = std::fs::remove_file(&path);

        let mut app = test_app(7654);
        app.insert_resource(ReplayPath(Some(path.clone())));

        // Push two recorded moves manually so we can verify they survive
        // the freeze/save round-trip without having to drive a real win.
        {
            let mut recording = app.world_mut().resource_mut::<RecordingReplay>();
            recording.moves.push(ReplayMove::StockClick);
            recording.moves.push(ReplayMove::Move {
                from: PileType::Waste,
                to: PileType::Tableau(2),
                count: 1,
            });
        }

        // Fire the win event the engine emits when the last foundation
        // completes — `record_replay_on_win` listens for it.
        app.world_mut().write_message(GameWonEvent {
            score: 4321,
            time_seconds: 250,
        });
        app.update();

        let loaded = load_latest_replay_from(&path)
            .expect("a winning replay must be persisted to ReplayPath");
        assert_eq!(loaded.seed, 7654, "seed must match the live game state");
        assert_eq!(loaded.draw_mode, DrawMode::DrawOne, "draw_mode must be captured");
        assert_eq!(loaded.final_score, 4321, "final_score must come from the win event");
        assert_eq!(loaded.time_seconds, 250, "time_seconds must come from the win event");
        assert_eq!(loaded.moves.len(), 2, "every recorded move must round-trip");
        assert!(matches!(loaded.moves[0], ReplayMove::StockClick));
        match &loaded.moves[1] {
            ReplayMove::Move { from, to, count } => {
                assert_eq!(*from, PileType::Waste);
                assert_eq!(*to, PileType::Tableau(2));
                assert_eq!(*count, 1);
            }
            other => panic!("second entry must be a Move, got {other:?}"),
        }

        let _ = std::fs::remove_file(&path);
    }

    /// `GameWonEvent` with an empty recording must NOT touch disk.
    /// Without this guard, parallel-plugin tests that synthesise
    /// win events for XP / streak / weekly-goal logic (without
    /// driving any actual moves) would clobber the developer's real
    /// replay file every time `cargo test` ran.
    #[test]
    fn replay_with_empty_recording_skips_save() {
        let path = std::env::temp_dir().join("engine_test_replay_empty_skip.json");
        let _ = std::fs::remove_file(&path);

        let mut app = test_app(1);
        app.insert_resource(ReplayPath(Some(path.clone())));
        // Recording is empty by default — fire a win event anyway.
        app.world_mut().write_message(GameWonEvent {
            score: 100,
            time_seconds: 30,
        });
        app.update();

        assert!(
            !path.exists(),
            "no replay must be written when recording is empty",
        );
    }
}
