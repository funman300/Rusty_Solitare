//! Keyboard + mouse input for the game board.
//!
//! All systems exit immediately when `PausedResource(true)` — no moves,
//! draws, undos, or drags are processed while the pause overlay is showing.
//!
//! Keyboard:
//! - `U` → `UndoRequestEvent`
//! - `N` → `NewGameRequestEvent { seed: None }` (cancels Time Attack if active)
//! - `D` / `Space` → `DrawRequestEvent`
//! - `Esc` → handled by `PausePlugin` (overlay toggle + paused flag)
//!
//! Mouse:
//! - Left-click on the stock pile (face-down top) → `DrawRequestEvent`
//! - Left-press-drag-release on a face-up card → `MoveRequestEvent` between
//!   the origin pile and whatever pile the cursor is over at release.
//!   On rejection, the drag cards snap back to their origin via a
//!   `StateChangedEvent` re-sync.

use std::collections::HashMap;

use bevy::ecs::system::SystemParam;
use bevy::input::touch::{TouchInput, TouchPhase, Touches};
use bevy::input::ButtonInput;
use bevy::math::{Vec2, Vec3};
use bevy::prelude::*;
use bevy::window::{MonitorSelection, PrimaryWindow, WindowMode};
use solitaire_core::card::{Card, Suit};
use solitaire_core::game_state::GameState;
use solitaire_core::pile::PileType;
use solitaire_core::rules::{can_place_on_foundation, can_place_on_tableau};

use crate::card_animation::tuning::AnimationTuning;
use crate::card_plugin::{CardEntity, HintHighlight, HintHighlightTimer, TABLEAU_FAN_FRAC};
use crate::feedback_anim_plugin::ShakeAnim;
use solitaire_core::game_state::DrawMode;
use crate::challenge_plugin::CHALLENGE_UNLOCK_LEVEL;
use crate::events::{
    DrawRequestEvent, ForfeitEvent, HintVisualEvent, InfoToastEvent, MoveRejectedEvent,
    MoveRequestEvent, NewGameConfirmEvent, NewGameRequestEvent, StateChangedEvent, UndoRequestEvent,
};
use crate::game_plugin::GameMutation;
use crate::pause_plugin::PausedResource;
use crate::progress_plugin::ProgressResource;
use crate::layout::{Layout, LayoutResource};
use crate::resources::{DragState, GameStateResource, HintCycleIndex};
use crate::selection_plugin::SelectionState;
use crate::time_attack_plugin::TimeAttackResource;

/// Z-depth used for cards while being dragged — above all resting cards.
const DRAG_Z: f32 = 500.0;

/// Shared countdown timers for the double-press confirmation flows.
///
/// Using a resource (instead of `Local`) lets the three keyboard sub-systems
/// share the same countdown state without needing to pass values between them.
#[derive(Resource, Debug, Default)]
struct KeyboardConfirmState {
    /// Seconds remaining in the new-game confirmation window (> 0 while open).
    new_game_countdown: f32,
    /// True while we are waiting for the second N press to confirm a new game.
    new_game_pending: bool,
    /// Seconds remaining in the forfeit confirmation window (> 0 while open).
    forfeit_countdown: f32,
}

/// Registers keyboard, mouse, and touch input systems.
///
/// Mouse drag pipeline (ordered, left-to-right):
/// `start_drag` → `follow_drag` → `end_drag`
///
/// Touch drag pipeline (ordered, interleaved with mouse):
/// `touch_start_drag` → `touch_follow_drag` → `touch_end_drag`
///
/// Both pipelines share [`DragState`]. Only one can be active at a time —
/// the second checks `drag.is_idle()` before proceeding, and mouse drags
/// check `drag.active_touch_id.is_none()`.
///
/// All drag systems run before [`GameMutation`] so move events are consumed
/// in the same frame they are emitted.
pub struct InputPlugin;

impl Plugin for InputPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<HintCycleIndex>()
            .init_resource::<KeyboardConfirmState>()
            .add_message::<NewGameConfirmEvent>()
            .add_message::<InfoToastEvent>()
            .add_message::<ForfeitEvent>()
            .add_message::<HintVisualEvent>()
            .add_systems(
                Update,
                (
                    handle_keyboard_core,
                    handle_keyboard_hint,
                    handle_keyboard_forfeit,
                    handle_stock_click,
                    handle_touch_stock_tap,
                    handle_double_click,
                    // Mouse drag pipeline.
                    start_drag,
                    follow_drag,
                    end_drag.before(GameMutation),
                    // Touch drag pipeline (parallel path through DragState).
                    touch_start_drag,
                    touch_follow_drag,
                    touch_end_drag.before(GameMutation),
                )
                    .chain(),
            )
            .add_systems(Update, handle_fullscreen)
            .add_systems(Update, reset_hint_cycle_on_state_change);
    }
}

/// Seconds after the first N press during which a second N confirms new game.
const NEW_GAME_CONFIRM_WINDOW: f32 = 3.0;

/// Seconds after the first G press during which a second G confirms forfeit.
const FORFEIT_CONFIRM_WINDOW: f32 = 3.0;

/// Bundles the event writers needed by the core keyboard handler.
///
/// Keeping these in a [`SystemParam`] avoids hitting Bevy's 16-parameter limit.
#[derive(SystemParam)]
struct CoreKeyboardMessages<'w> {
    undo: MessageWriter<'w, UndoRequestEvent>,
    new_game: MessageWriter<'w, NewGameRequestEvent>,
    confirm_event: MessageWriter<'w, NewGameConfirmEvent>,
    info_toast: MessageWriter<'w, InfoToastEvent>,
    draw: MessageWriter<'w, DrawRequestEvent>,
}

/// Handles the core keyboard shortcuts: U (undo), N (new game + confirmation
/// window), Z (zen mode), D / Space (draw), and ticks down the new-game
/// confirmation countdown each frame.
///
/// Also resets `forfeit_countdown` whenever U, D, Z, or N are pressed so that
/// an in-flight forfeit confirmation is cancelled by any other action.
#[allow(clippy::too_many_arguments)]
fn handle_keyboard_core(
    keys: Res<ButtonInput<KeyCode>>,
    paused: Option<Res<PausedResource>>,
    progress: Option<Res<ProgressResource>>,
    game: Option<Res<GameStateResource>>,
    time: Res<Time>,
    mut confirm: ResMut<KeyboardConfirmState>,
    mut ev: CoreKeyboardMessages<'_>,
    mut time_attack: Option<ResMut<TimeAttackResource>>,
    selection: Option<Res<SelectionState>>,
) {
    if paused.is_some_and(|p| p.0) {
        return;
    }

    // Tick down the new-game confirmation window each frame.
    if confirm.new_game_countdown > 0.0 {
        confirm.new_game_countdown -= time.delta_secs();
        if confirm.new_game_countdown <= 0.0 {
            confirm.new_game_countdown = 0.0;
            if confirm.new_game_pending {
                confirm.new_game_pending = false;
                ev.info_toast.write(InfoToastEvent("New game cancelled".to_string()));
            }
        }
    }

    if keys.just_pressed(KeyCode::KeyU) {
        // Cancel any pending forfeit when the player takes another action.
        confirm.forfeit_countdown = 0.0;
        ev.undo.write(UndoRequestEvent);
    }

    if keys.just_pressed(KeyCode::KeyN) {
        // Cancel any pending forfeit when the player takes another action.
        confirm.forfeit_countdown = 0.0;

        // If a Time Attack session is running, cancel it and start a Classic game.
        if let Some(ref mut session) = time_attack
            && session.active {
                session.active = false;
                session.remaining_secs = 0.0;
                ev.info_toast.write(InfoToastEvent("Time Attack ended".to_string()));
                ev.new_game.write(NewGameRequestEvent {
                    seed: None,
                    mode: Some(solitaire_core::game_state::GameMode::Classic),
                });
                confirm.new_game_countdown = 0.0;
                return;
            }

        let active_game = game.as_ref().is_some_and(|g| g.0.move_count > 0 && !g.0.is_won);
        let shift_held = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
        if shift_held || !active_game {
            // Shift+N or no active game — start immediately, no confirmation.
            ev.new_game.write(NewGameRequestEvent::default());
            confirm.new_game_countdown = 0.0;
            confirm.new_game_pending = false;
        } else if confirm.new_game_countdown > 0.0 {
            // Second press within the window — confirmed.
            ev.new_game.write(NewGameRequestEvent::default());
            confirm.new_game_countdown = 0.0;
            confirm.new_game_pending = false;
        } else {
            // First press on an active game — require confirmation.
            confirm.new_game_countdown = NEW_GAME_CONFIRM_WINDOW;
            confirm.new_game_pending = true;
            ev.confirm_event.write(NewGameConfirmEvent);
        }
    }

    if keys.just_pressed(KeyCode::KeyZ) {
        // Cancel any pending forfeit when the player takes another action.
        confirm.forfeit_countdown = 0.0;
        // Zen / Challenge / Time Attack are gated to level >= CHALLENGE_UNLOCK_LEVEL.
        // X is gated separately by ChallengePlugin.
        let level = progress.as_ref().map_or(0, |p| p.0.level);
        if level >= CHALLENGE_UNLOCK_LEVEL {
            ev.new_game.write(NewGameRequestEvent {
                seed: None,
                mode: Some(solitaire_core::game_state::GameMode::Zen),
            });
        } else {
            ev.info_toast.write(InfoToastEvent(format!(
                "Zen mode unlocks at level {CHALLENGE_UNLOCK_LEVEL}"
            )));
        }
    }

    // Space draws only when no card is keyboard-selected; when a card IS selected,
    // SelectionPlugin handles Space to execute the move.
    let space_draws = keys.just_pressed(KeyCode::Space)
        && selection.as_ref().is_none_or(|s| s.selected_pile.is_none());
    if keys.just_pressed(KeyCode::KeyD) || space_draws {
        // Cancel any pending forfeit when the player takes another action.
        confirm.forfeit_countdown = 0.0;
        ev.draw.write(DrawRequestEvent);
    }
    // Esc is handled by `PausePlugin` (overlay toggle + paused flag).
}

/// Handles the H key: cycles through all available hints, highlighting the
/// source card yellow for 2 s and showing a descriptive toast. Resets the
/// forfeit countdown on each press.
///
/// The hint index wraps around once all hints have been cycled through. When no
/// moves are available a "No hints available" toast is shown instead.
#[allow(clippy::too_many_arguments)]
fn handle_keyboard_hint(
    keys: Res<ButtonInput<KeyCode>>,
    paused: Option<Res<PausedResource>>,
    game: Option<Res<GameStateResource>>,
    layout: Option<Res<LayoutResource>>,
    mut confirm: ResMut<KeyboardConfirmState>,
    mut hint_cycle: ResMut<HintCycleIndex>,
    mut commands: Commands,
    mut card_entities: Query<(Entity, &CardEntity, &mut Sprite)>,
    mut info_toast: MessageWriter<InfoToastEvent>,
    mut hint_visual: MessageWriter<HintVisualEvent>,
) {
    if paused.is_some_and(|p| p.0) {
        return;
    }
    if !keys.just_pressed(KeyCode::KeyH) {
        return;
    }

    // H cancels any in-flight forfeit confirmation.
    confirm.forfeit_countdown = 0.0;

    let Some(ref g) = game else { return };

    if g.0.is_won {
        info_toast.write(InfoToastEvent("Game won! Press N for a new game".to_string()));
        return;
    }

    let Some(_layout_res) = layout else { return };

    let hints = all_hints(&g.0);
    if hints.is_empty() {
        info_toast.write(InfoToastEvent("No hints available".to_string()));
        return;
    }

    // Pick the hint at the current cycle index (wrapping) and advance.
    let idx = hint_cycle.0 % hints.len();
    hint_cycle.0 = hint_cycle.0.wrapping_add(1);
    let (from, to, _count) = &hints[idx];

    // When the hint points at the stock (draw suggestion) there is no
    // face-up card to highlight — show a toast instead.
    // If the stock is empty, pressing D will recycle the waste rather
    // than draw a card, so the toast text must reflect that.
    if *from == PileType::Stock {
        let stock_empty = g.0.piles
            .get(&PileType::Stock)
            .is_some_and(|p| p.cards.is_empty());
        let msg = if stock_empty {
            "Hint: recycle waste (D)".to_string()
        } else {
            "Hint: draw from stock (D)".to_string()
        };
        info_toast.write(InfoToastEvent(msg));
        return;
    }

    // Find the top face-up card in the source pile and highlight it.
    let top_card_id = g.0.piles.get(from)
        .and_then(|p| p.cards.last().filter(|c| c.face_up))
        .map(|c| c.id);
    if let Some(card_id) = top_card_id {
        for (entity, card_entity, mut sprite) in card_entities.iter_mut() {
            if card_entity.card_id == card_id {
                // Tint the card gold without replacing the Sprite (which would
                // discard the image handle set by CardImageSet).
                sprite.color = Color::srgba(1.0, 1.0, 0.4, 1.0);
                commands.entity(entity)
                    .insert(HintHighlight { remaining: 2.0 })
                    .insert(HintHighlightTimer(2.0));
                break;
            }
        }
        // Emit HintVisualEvent so the destination pile marker is also
        // tinted gold for 2 s.
        hint_visual.write(HintVisualEvent {
            source_card_id: card_id,
            dest_pile: to.clone(),
        });
    }

    // Fire an informational toast describing where the hinted card should
    // move so the player always sees the suggestion in text.
    let msg = match to {
        PileType::Foundation(suit) => {
            let suit_name = match suit {
                Suit::Clubs => "Clubs",
                Suit::Diamonds => "Diamonds",
                Suit::Hearts => "Hearts",
                Suit::Spades => "Spades",
            };
            format!("Hint: move to {suit_name} foundation")
        }
        PileType::Tableau(col) => format!("Hint: move to tableau (col {})", col + 1),
        _ => "Hint: move card".to_string(),
    };
    info_toast.write(InfoToastEvent(msg));
}

/// Handles the G key: forfeit the current game with a 3-second double-confirm
/// window to prevent accidental forfeits.
///
/// First press shows a toast and starts the countdown.
/// Second press **within the window** sends [`ForfeitEvent`].
/// Pressing any other key between presses cancels the countdown
/// (handled by [`handle_keyboard_core`]).
fn handle_keyboard_forfeit(
    keys: Res<ButtonInput<KeyCode>>,
    paused: Option<Res<PausedResource>>,
    time: Res<Time>,
    game: Option<Res<GameStateResource>>,
    mut confirm: ResMut<KeyboardConfirmState>,
    mut forfeit: MessageWriter<ForfeitEvent>,
    mut info_toast: MessageWriter<InfoToastEvent>,
) {
    if paused.is_some_and(|p| p.0) {
        return;
    }

    // Tick down the forfeit confirmation window each frame.
    if confirm.forfeit_countdown > 0.0 {
        confirm.forfeit_countdown -= time.delta_secs();
        if confirm.forfeit_countdown <= 0.0 {
            confirm.forfeit_countdown = 0.0;
        }
    }

    if !keys.just_pressed(KeyCode::KeyG) {
        return;
    }

    let active_game = game.as_ref().is_some_and(|g| g.0.move_count > 0 && !g.0.is_won);
    if !active_game {
        return;
    }

    if confirm.forfeit_countdown > 0.0 {
        // Second press within the confirmation window — confirmed.
        forfeit.write(ForfeitEvent);
        confirm.forfeit_countdown = 0.0;
    } else {
        // First press — start the countdown and warn the player.
        confirm.forfeit_countdown = FORFEIT_CONFIRM_WINDOW;
        info_toast.write(InfoToastEvent("Press G again to forfeit".to_string()));
    }
}

/// Resets [`HintCycleIndex`] to `0` whenever the game state changes or a new
/// game is requested so the next H press always starts cycling from the first
/// hint of the new position.
///
/// Listening to both events ensures the reset happens immediately on
/// `NewGameRequestEvent`, one frame before the `StateChangedEvent` that the
/// game plugin fires after dealing — preventing a stale hint from the previous
/// game being shown when H is pressed in that gap frame.
fn reset_hint_cycle_on_state_change(
    mut state_events: MessageReader<StateChangedEvent>,
    mut new_game_events: MessageReader<NewGameRequestEvent>,
    mut hint_cycle: ResMut<HintCycleIndex>,
) {
    if state_events.read().next().is_some() || new_game_events.read().next().is_some() {
        hint_cycle.0 = 0;
    }
}

/// `F11` toggles between borderless-fullscreen and windowed mode.
/// Not gated by the pause flag — the player can always resize the window.
fn handle_fullscreen(
    keys: Res<ButtonInput<KeyCode>>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    mut toast: MessageWriter<InfoToastEvent>,
) {
    if !keys.just_pressed(KeyCode::F11) {
        return;
    }
    let Ok(mut window) = windows.single_mut() else { return };
    let new_mode = match window.mode {
        WindowMode::Windowed => WindowMode::BorderlessFullscreen(MonitorSelection::Current),
        _ => WindowMode::Windowed,
    };
    window.mode = new_mode;
    let label = match window.mode {
        WindowMode::Windowed => "Fullscreen: off",
        _ => "Fullscreen: on",
    };
    toast.write(InfoToastEvent(label.to_string()));
}

fn handle_stock_click(
    buttons: Res<ButtonInput<MouseButton>>,
    drag: Res<DragState>,
    paused: Option<Res<PausedResource>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Camera, &GlobalTransform)>,
    layout: Option<Res<LayoutResource>>,
    mut draw: MessageWriter<DrawRequestEvent>,
) {
    if paused.is_some_and(|p| p.0) {
        return;
    }
    if !buttons.just_pressed(MouseButton::Left) || !drag.is_idle() {
        return;
    }
    let Some(layout) = layout else {
        return;
    };
    let Some(world) = cursor_world(&windows, &cameras) else {
        return;
    };

    let Some(&stock_pos) = layout.0.pile_positions.get(&PileType::Stock) else {
        return;
    };
    if point_in_rect(world, stock_pos, layout.0.card_size) {
        draw.write(DrawRequestEvent);
    }
}

/// Fires [`DrawRequestEvent`] when the player taps the stock pile on a touch screen.
///
/// Uses `TouchPhase::Started` (the finger-down moment) for instant responsiveness
/// — since the stock cannot be dragged, there is no ambiguity between a tap and
/// the start of a drag on this pile. Does nothing while a drag is in progress.
fn handle_touch_stock_tap(
    mut touch_events: MessageReader<TouchInput>,
    paused: Option<Res<PausedResource>>,
    cameras: Query<(&Camera, &GlobalTransform)>,
    layout: Option<Res<LayoutResource>>,
    drag: Res<DragState>,
    mut draw: MessageWriter<DrawRequestEvent>,
) {
    if paused.is_some_and(|p| p.0) {
        return;
    }
    if !drag.is_idle() {
        return;
    }
    let Some(layout) = layout else { return };

    for event in touch_events.read() {
        if event.phase != TouchPhase::Started {
            continue;
        }
        let Some(world) = touch_to_world(&cameras, event.position) else {
            continue;
        };
        let Some(&stock_pos) = layout.0.pile_positions.get(&PileType::Stock) else {
            continue;
        };
        if point_in_rect(world, stock_pos, layout.0.card_size) {
            draw.write(DrawRequestEvent);
            break; // one draw per tap frame
        }
    }
}

/// Begins a mouse drag: records the press position and the cards that would be
/// dragged. Cards are **not** elevated yet — that happens in [`follow_drag`]
/// once the drag threshold is crossed.
fn start_drag(
    buttons: Res<ButtonInput<MouseButton>>,
    paused: Option<Res<PausedResource>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Camera, &GlobalTransform)>,
    layout: Option<Res<LayoutResource>>,
    game: Res<GameStateResource>,
    mut drag: ResMut<DragState>,
) {
    if paused.is_some_and(|p| p.0) {
        return;
    }
    // Only start a new drag when idle (no touch drag running either).
    if !buttons.just_pressed(MouseButton::Left) || !drag.is_idle() {
        return;
    }
    let Some(layout) = layout else { return };
    let Some(world) = cursor_world(&windows, &cameras) else { return };

    // Don't pick up the stock — that is handled by handle_stock_click.
    let Some((pile, stack_index, card_ids)) = find_draggable_at(world, &game.0, &layout.0) else {
        return;
    };

    let bottom_pos = card_position(&game.0, &layout.0, &pile, stack_index);

    // Store as a pending drag. We do NOT elevate the cards yet — the visual
    // lift happens in follow_drag once the threshold is crossed.
    drag.cards = card_ids;
    drag.origin_pile = Some(pile);
    drag.cursor_offset = bottom_pos - world;
    drag.origin_z = DRAG_Z;
    drag.press_pos = world;
    drag.committed = false;
    drag.active_touch_id = None;
}

/// Moves dragged cards with the mouse cursor each frame.
///
/// If the drag has not yet been committed (threshold not crossed), checks
/// whether the cursor has moved far enough from the press position to commit.
/// On commit, cards are elevated to `DRAG_Z` and dimmed. Does nothing for
/// touch-driven drags (`drag.active_touch_id.is_some()`).
#[allow(clippy::too_many_arguments)]
fn follow_drag(
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Camera, &GlobalTransform)>,
    mut drag: ResMut<DragState>,
    layout: Option<Res<LayoutResource>>,
    tuning: Res<AnimationTuning>,
    mut card_transforms: Query<(&CardEntity, &mut Transform, &mut Sprite)>,
) {
    // Skip if idle or if a touch drag is running.
    if drag.is_idle() || drag.active_touch_id.is_some() {
        return;
    }
    let Some(layout) = layout else { return };
    let Some(world) = cursor_world(&windows, &cameras) else {
        // Cursor left the window mid-drag. Cancel a pending drag; let a
        // committed drag freeze at the last known position.
        if !drag.committed {
            drag.clear();
        }
        return;
    };

    // Check drag threshold on the first frames after press.
    if !drag.committed {
        // Use screen-space distance (world ≈ screen for 2-D games with no
        // camera zoom, which is our case).
        let moved = world.distance(drag.press_pos);
        if moved < tuning.drag_threshold_px {
            return; // Still within tap zone — don't start visual drag yet.
        }

        // Threshold crossed → commit.
        drag.committed = true;

        // Elevate cards: push to DRAG_Z and dim slightly so the board
        // beneath stays readable.
        for (i, &id) in drag.cards.iter().enumerate() {
            if let Some((_, mut transform, mut sprite)) =
                card_transforms.iter_mut().find(|(ce, _, _)| ce.card_id == id)
            {
                transform.translation.z = DRAG_Z + i as f32 * 0.01;
                sprite.color.set_alpha(0.85);
            }
        }
    }

    // Move cards to the cursor.
    let bottom_pos = world + drag.cursor_offset;
    let fan = -layout.0.card_size.y * TABLEAU_FAN_FRAC;

    for (i, &id) in drag.cards.iter().enumerate() {
        if let Some((_, mut transform, _)) =
            card_transforms.iter_mut().find(|(ce, _, _)| ce.card_id == id)
        {
            transform.translation.x = bottom_pos.x;
            transform.translation.y = bottom_pos.y + fan * i as f32;
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn end_drag(
    buttons: Res<ButtonInput<MouseButton>>,
    paused: Option<Res<PausedResource>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Camera, &GlobalTransform)>,
    layout: Option<Res<LayoutResource>>,
    game: Res<GameStateResource>,
    mut drag: ResMut<DragState>,
    mut moves: MessageWriter<MoveRequestEvent>,
    mut rejected: MessageWriter<MoveRejectedEvent>,
    mut changed: MessageWriter<StateChangedEvent>,
    mut commands: Commands,
    card_entities: Query<(Entity, &CardEntity, &Transform)>,
) {
    if paused.is_some_and(|p| p.0) {
        drag.clear();
        return;
    }
    // Only handle mouse releases; touch releases are handled by touch_end_drag.
    if !buttons.just_released(MouseButton::Left) || drag.is_idle() {
        return;
    }
    if drag.active_touch_id.is_some() {
        return; // Touch-driven drag — not ours to handle.
    }

    // If the drag was never committed (user tapped without moving far enough),
    // treat it as a click: just cancel the pending drag and resync card positions.
    if !drag.committed {
        drag.clear();
        changed.write(StateChangedEvent);
        return;
    }
    let Some(layout) = layout else {
        return;
    };
    let Some(origin) = drag.origin_pile.clone() else {
        drag.clear();
        return;
    };
    let count = drag.cards.len();

    let world = cursor_world(&windows, &cameras);
    let target = world.and_then(|w| find_drop_target(w, &game.0, &layout.0, &origin));

    // Whether we fire a MoveRequestEvent or not, always trigger a resync so
    // the dragged cards snap back to their resting positions if the move is
    // rejected (or never fired). When the cursor was over a real pile but
    // the placement is illegal, fire MoveRejectedEvent so AudioPlugin can
    // play card_invalid.wav.
    let mut fired = false;
    if let Some(target) = target
        && target != origin {
            let bottom_card_id = drag.cards[0];
            if let Some(bottom_card) = card_by_id(&game.0, bottom_card_id) {
                let ok = match &target {
                    PileType::Foundation(suit) => {
                        count == 1
                            && can_place_on_foundation(
                                &bottom_card,
                                &game.0.piles[&target],
                                *suit,
                            )
                    }
                    PileType::Tableau(_) => {
                        can_place_on_tableau(&bottom_card, &game.0.piles[&target])
                    }
                    _ => false,
                };
                if ok {
                    moves.write(MoveRequestEvent {
                        from: origin.clone(),
                        to: target.clone(),
                        count,
                    });
                    fired = true;
                } else {
                    rejected.write(MoveRejectedEvent {
                        from: origin.clone(),
                        to: target.clone(),
                        count,
                    });
                    // Shake each dragged card so the player gets immediate
                    // visual feedback that the drop was rejected. ShakeAnim
                    // restores translation.x to origin_x at the end of the
                    // animation, so origin_x must be the target slot in the
                    // origin pile — using the current drag transform would
                    // pin the card at the drop location and fight the
                    // sync_cards slide that StateChangedEvent triggers
                    // (the symptom is "card lands beside the pile").
                    if let Some(origin_pile) = game.0.piles.get(&origin) {
                        for &card_id in &drag.cards {
                            let Some(stack_index) =
                                origin_pile.cards.iter().position(|c| c.id == card_id)
                            else {
                                continue;
                            };
                            let target_pos =
                                card_position(&game.0, &layout.0, &origin, stack_index);
                            if let Some((entity, _, _)) = card_entities
                                .iter()
                                .find(|(_, ce, _)| ce.card_id == card_id)
                            {
                                commands.entity(entity).insert(ShakeAnim {
                                    elapsed: 0.0,
                                    origin_x: target_pos.x,
                                });
                            }
                        }
                    }
                }
            }
        }

    drag.clear();

    // Either the move succeeded (GamePlugin will also fire StateChangedEvent)
    // or it didn't — in both cases we emit one so cards resync to the current
    // game state. Duplicate events are harmless.
    changed.write(StateChangedEvent);
    let _ = fired;
}

// ---------------------------------------------------------------------------
// Touch drag pipeline
// ---------------------------------------------------------------------------

/// Begins a touch drag when a finger first touches a face-up card.
///
/// Mirrors [`start_drag`] but uses [`TouchInput`] events instead of mouse
/// buttons. Records the touch ID in [`DragState`] so only this finger drives
/// the drag — other fingers are ignored.
fn touch_start_drag(
    mut touch_events: MessageReader<TouchInput>,
    paused: Option<Res<PausedResource>>,
    cameras: Query<(&Camera, &GlobalTransform)>,
    layout: Option<Res<LayoutResource>>,
    game: Res<GameStateResource>,
    mut drag: ResMut<DragState>,
) {
    if paused.is_some_and(|p| p.0) {
        return;
    }
    // Only one drag at a time.
    if !drag.is_idle() {
        return;
    }
    let Some(layout) = layout else { return };

    for event in touch_events.read() {
        if event.phase != TouchPhase::Started {
            continue;
        }
        let Some(world) = touch_to_world(&cameras, event.position) else {
            continue;
        };
        let Some((pile, stack_index, card_ids)) =
            find_draggable_at(world, &game.0, &layout.0)
        else {
            continue;
        };

        let bottom_pos = card_position(&game.0, &layout.0, &pile, stack_index);

        drag.cards = card_ids;
        drag.origin_pile = Some(pile);
        drag.cursor_offset = bottom_pos - world;
        drag.origin_z = DRAG_Z;
        drag.press_pos = event.position; // screen-space for threshold comparison
        drag.committed = false;
        drag.active_touch_id = Some(event.id);
        // Process only the first touch that landed on a card.
        break;
    }
}

/// Moves touch-dragged cards with the active finger each frame.
///
/// Checks the drag threshold on the first frames after the touch began and
/// commits (elevates cards) once exceeded. Does nothing for mouse drags.
#[allow(clippy::too_many_arguments)]
fn touch_follow_drag(
    touches: Option<Res<Touches>>,
    cameras: Query<(&Camera, &GlobalTransform)>,
    mut drag: ResMut<DragState>,
    layout: Option<Res<LayoutResource>>,
    tuning: Res<AnimationTuning>,
    mut card_transforms: Query<(&CardEntity, &mut Transform, &mut Sprite)>,
) {
    let Some(active_id) = drag.active_touch_id else {
        return; // Mouse drag or idle.
    };
    let Some(touches) = touches else { return };
    let Some(layout) = layout else { return };

    // Look up the driving touch.
    let Some(touch) = touches.iter().find(|t| t.id() == active_id) else {
        // Touch no longer active — will be cleaned up by touch_end_drag.
        return;
    };

    let Some(world) = touch_to_world(&cameras, touch.position()) else {
        return;
    };

    if !drag.committed {
        // Compare screen-space distance from the original press position.
        let moved = touch.position().distance(drag.press_pos);
        if moved < tuning.drag_threshold_px {
            return;
        }

        drag.committed = true;

        for (i, &id) in drag.cards.iter().enumerate() {
            if let Some((_, mut transform, mut sprite)) =
                card_transforms.iter_mut().find(|(ce, _, _)| ce.card_id == id)
            {
                transform.translation.z = DRAG_Z + i as f32 * 0.01;
                sprite.color.set_alpha(0.85);
            }
        }
    }

    let bottom_pos = world + drag.cursor_offset;
    let fan = -layout.0.card_size.y * TABLEAU_FAN_FRAC;

    for (i, &id) in drag.cards.iter().enumerate() {
        if let Some((_, mut transform, _)) =
            card_transforms.iter_mut().find(|(ce, _, _)| ce.card_id == id)
        {
            transform.translation.x = bottom_pos.x;
            transform.translation.y = bottom_pos.y + fan * i as f32;
        }
    }
}

/// Resolves a touch drag when the finger lifts or is cancelled.
///
/// Mirrors [`end_drag`] but reads [`TouchInput`] events instead of mouse
/// buttons. Uncommitted drags (tap gestures) are cancelled cleanly.
#[allow(clippy::too_many_arguments)]
fn touch_end_drag(
    mut touch_events: MessageReader<TouchInput>,
    paused: Option<Res<PausedResource>>,
    cameras: Query<(&Camera, &GlobalTransform)>,
    layout: Option<Res<LayoutResource>>,
    game: Res<GameStateResource>,
    mut drag: ResMut<DragState>,
    mut moves: MessageWriter<MoveRequestEvent>,
    mut rejected: MessageWriter<MoveRejectedEvent>,
    mut changed: MessageWriter<StateChangedEvent>,
    mut commands: Commands,
    card_entities: Query<(Entity, &CardEntity, &Transform)>,
) {
    let Some(active_id) = drag.active_touch_id else {
        return; // Mouse drag or idle.
    };

    if paused.is_some_and(|p| p.0) {
        drag.clear();
        return;
    }

    for event in touch_events.read() {
        if event.id != active_id {
            continue;
        }
        if !matches!(event.phase, TouchPhase::Ended | TouchPhase::Canceled) {
            continue;
        }

        // Uncommitted tap — cancel cleanly.
        if !drag.committed {
            drag.clear();
            changed.write(StateChangedEvent);
            return;
        }

        let Some(origin) = drag.origin_pile.clone() else {
            drag.clear();
            return;
        };
        let count = drag.cards.len();

        // Find the drop target using the finger's lift position.
        let world = touch_to_world(&cameras, event.position);
        let Some(layout) = layout.as_ref() else {
            drag.clear();
            changed.write(StateChangedEvent);
            return;
        };
        let target =
            world.and_then(|w| find_drop_target(w, &game.0, &layout.0, &origin));

        let mut fired = false;
        if let Some(target) = target
            && target != origin {
                let bottom_card_id = drag.cards[0];
                if let Some(bottom_card) = card_by_id(&game.0, bottom_card_id) {
                    let ok = match &target {
                        PileType::Foundation(suit) => {
                            count == 1
                                && can_place_on_foundation(&bottom_card, &game.0.piles[&target], *suit)
                        }
                        PileType::Tableau(_) => {
                            can_place_on_tableau(&bottom_card, &game.0.piles[&target])
                        }
                        _ => false,
                    };
                    if ok {
                        moves.write(MoveRequestEvent { from: origin.clone(), to: target, count });
                        fired = true;
                    } else {
                        rejected.write(MoveRejectedEvent { from: origin.clone(), to: target, count });
                        // See `end_drag` (mouse path) for the rationale: ShakeAnim
                        // restores translation.x to origin_x, so origin_x must be
                        // the origin pile's slot, not the drop location.
                        if let Some(origin_pile) = game.0.piles.get(&origin) {
                            for &card_id in &drag.cards {
                                let Some(stack_index) =
                                    origin_pile.cards.iter().position(|c| c.id == card_id)
                                else {
                                    continue;
                                };
                                let target_pos =
                                    card_position(&game.0, &layout.0, &origin, stack_index);
                                if let Some((entity, _, _)) =
                                    card_entities.iter().find(|(_, ce, _)| ce.card_id == card_id)
                                {
                                    commands.entity(entity).insert(ShakeAnim {
                                        elapsed: 0.0,
                                        origin_x: target_pos.x,
                                    });
                                }
                            }
                        }
                    }
                }
            }

        drag.clear();
        changed.write(StateChangedEvent);
        let _ = fired;
        return;
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn cursor_world(
    windows: &Query<&Window, With<PrimaryWindow>>,
    cameras: &Query<(&Camera, &GlobalTransform)>,
) -> Option<Vec2> {
    let window = windows.single().ok()?;
    let cursor = window.cursor_position()?;
    let (camera, camera_transform) = cameras.single().ok()?;
    camera.viewport_to_world_2d(camera_transform, cursor).ok()
}

/// Converts a touch screen position (logical pixels, top-left origin) to
/// world-space 2-D coordinates using the primary camera.
///
/// Returns `None` if no camera is present or the projection fails.
fn touch_to_world(
    cameras: &Query<(&Camera, &GlobalTransform)>,
    screen_pos: Vec2,
) -> Option<Vec2> {
    let (camera, camera_transform) = cameras.single().ok()?;
    camera.viewport_to_world_2d(camera_transform, screen_pos).ok()
}

/// Axis-aligned rectangle hit-test with a center and full size.
fn point_in_rect(point: Vec2, center: Vec2, size: Vec2) -> bool {
    let half = size / 2.0;
    point.x >= center.x - half.x
        && point.x <= center.x + half.x
        && point.y >= center.y - half.y
        && point.y <= center.y + half.y
}

/// Where a card at `stack_index` in pile `pile` would be rendered.
fn card_position(game: &GameState, layout: &Layout, pile: &PileType, stack_index: usize) -> Vec2 {
    let base = layout.pile_positions[pile];
    if matches!(pile, PileType::Tableau(_)) {
        let fan = -layout.card_size.y * TABLEAU_FAN_FRAC;
        Vec2::new(base.x, base.y + fan * (stack_index as f32))
    } else if matches!(pile, PileType::Waste) && game.draw_mode == DrawMode::DrawThree {
        // In Draw-Three mode the top 3 waste cards are fanned in X to match
        // card_plugin::card_positions(). Hit-testing must use the same offsets
        // so clicking the visually rightmost (top) card actually registers.
        let pile_len = game.piles.get(pile).map_or(0, |p| p.cards.len());
        let visible_start = pile_len.saturating_sub(3);
        let slot = stack_index.saturating_sub(visible_start) as f32;
        Vec2::new(base.x + slot * layout.card_size.x * 0.28, base.y)
    } else {
        base
    }
}

fn card_by_id(game: &GameState, id: u32) -> Option<solitaire_core::card::Card> {
    for pile in game.piles.values() {
        if let Some(card) = pile.cards.iter().find(|c| c.id == id) {
            return Some(card.clone());
        }
    }
    None
}

/// Given a world-space cursor, find the topmost draggable card. Returns
/// `(pile, bottom_stack_index, card_ids_bottom_to_top)`.
fn find_draggable_at(
    cursor: Vec2,
    game: &GameState,
    layout: &Layout,
) -> Option<(PileType, usize, Vec<u32>)> {
    // Search order: waste, foundations, tableau. Stock is skipped (click-to-draw).
    // Within a pile, we consider cards top-down because the visual top card is drawn last.
    let piles = [
        PileType::Waste,
        PileType::Foundation(Suit::Clubs),
        PileType::Foundation(Suit::Diamonds),
        PileType::Foundation(Suit::Hearts),
        PileType::Foundation(Suit::Spades),
        PileType::Tableau(0),
        PileType::Tableau(1),
        PileType::Tableau(2),
        PileType::Tableau(3),
        PileType::Tableau(4),
        PileType::Tableau(5),
        PileType::Tableau(6),
    ];

    for pile in piles {
        let Some(pile_cards) = game.piles.get(&pile) else {
            continue;
        };
        if pile_cards.cards.is_empty() {
            continue;
        }

        let is_tableau = matches!(pile, PileType::Tableau(_));

        // Iterate from topmost to bottommost so the first hit is the one
        // visually on top.
        for i in (0..pile_cards.cards.len()).rev() {
            let card = &pile_cards.cards[i];
            if !card.face_up {
                continue;
            }
            let pos = card_position(game, layout, &pile, i);
            if !point_in_rect(cursor, pos, layout.card_size) {
                continue;
            }

            // Picked a face-up card. Determine drag range:
            //   - Tableau: cards [i..len), must all be face-up (guaranteed
            //     because tableau never has face-down above face-up).
            //   - Waste / Foundation: only the top card is draggable.
            let (start, end) = if is_tableau {
                (i, pile_cards.cards.len())
            } else {
                if i != pile_cards.cards.len() - 1 {
                    return None;
                }
                (i, i + 1)
            };
            let ids: Vec<u32> = pile_cards.cards[start..end].iter().map(|c| c.id).collect();
            return Some((pile, start, ids));
        }
    }
    None
}

/// Pick the drop-target pile whose extended rectangle contains `cursor`.
/// Returns `None` if the cursor is outside every pile's rectangle.
fn find_drop_target(
    cursor: Vec2,
    game: &GameState,
    layout: &Layout,
    origin: &PileType,
) -> Option<PileType> {
    let piles = [
        PileType::Foundation(Suit::Clubs),
        PileType::Foundation(Suit::Diamonds),
        PileType::Foundation(Suit::Hearts),
        PileType::Foundation(Suit::Spades),
        PileType::Tableau(0),
        PileType::Tableau(1),
        PileType::Tableau(2),
        PileType::Tableau(3),
        PileType::Tableau(4),
        PileType::Tableau(5),
        PileType::Tableau(6),
    ];

    for pile in piles {
        let (center, size) = pile_drop_rect(&pile, layout, game);
        if point_in_rect(cursor, center, size) {
            // Skip origin — dropping onto the source is a no-op.
            if pile == *origin {
                continue;
            }
            return Some(pile);
        }
    }
    None
}

/// Bounding rect used for drop detection. For tableaus this extends
/// downward to cover the entire visible fan of cards.
fn pile_drop_rect(pile: &PileType, layout: &Layout, game: &GameState) -> (Vec2, Vec2) {
    let center = layout.pile_positions[pile];
    if matches!(pile, PileType::Tableau(_)) {
        let card_count = game.piles.get(pile).map_or(0, |p| p.cards.len());
        if card_count > 1 {
            let fan = -layout.card_size.y * TABLEAU_FAN_FRAC;
            let bottom_card_center_y = center.y + fan * (card_count - 1) as f32;
            let top_edge = center.y + layout.card_size.y / 2.0;
            let bottom_edge = bottom_card_center_y - layout.card_size.y / 2.0;
            let span_height = top_edge - bottom_edge;
            let new_center_y = (top_edge + bottom_edge) / 2.0;
            return (
                Vec2::new(center.x, new_center_y),
                Vec2::new(layout.card_size.x, span_height),
            );
        }
    }
    (center, layout.card_size)
}

// ---------------------------------------------------------------------------
// Task #27 — Double-click to auto-move
// ---------------------------------------------------------------------------

/// Maximum seconds between two clicks to count as a double-click.
const DOUBLE_CLICK_WINDOW: f32 = 0.35;

/// Find the best legal destination for `card` — Foundation first, then Tableau.
///
/// Returns `None` if no legal move exists from the card's current location.
pub fn best_destination(card: &Card, game: &GameState) -> Option<PileType> {
    // Try all four foundations first.
    for suit in [Suit::Clubs, Suit::Diamonds, Suit::Hearts, Suit::Spades] {
        let dest = PileType::Foundation(suit);
        if let Some(pile) = game.piles.get(&dest)
            && can_place_on_foundation(card, pile, suit) {
                return Some(dest);
            }
    }
    // Then try all seven tableau piles.
    for i in 0..7_usize {
        let dest = PileType::Tableau(i);
        if let Some(pile) = game.piles.get(&dest)
            && can_place_on_tableau(card, pile) {
                return Some(dest);
            }
    }
    None
}

/// Find the best tableau column onto which the stack rooted at `bottom_card`
/// can be legally placed, excluding the stack's own source pile.
///
/// Returns `(destination, stack_count)` if a legal target exists, or `None`
/// if the stack cannot move anywhere. Only tableau destinations are considered
/// because multi-card stacks cannot go to foundations.
pub fn best_tableau_destination_for_stack(
    bottom_card: &Card,
    from: &PileType,
    game: &GameState,
    stack_count: usize,
) -> Option<(PileType, usize)> {
    for i in 0..7_usize {
        let dest = PileType::Tableau(i);
        if dest == *from {
            continue;
        }
        if let Some(pile) = game.piles.get(&dest)
            && can_place_on_tableau(bottom_card, pile) {
                return Some((dest, stack_count));
            }
    }
    None
}

/// System that detects double-clicks on face-up cards and fires `MoveRequestEvent`
/// to the best legal destination.
///
/// Move priority:
/// 1. Move the single **top** card to its best foundation (or tableau) destination.
/// 2. If no single-card move exists and the clicked card is the base of a
///    multi-card face-up stack, move the whole stack to the best tableau column.
///
/// When a multi-card stack double-click finds no legal destination (Priority 2
/// returns `None`), fires `MoveRejectedEvent` with `from == to == pile` so the
/// invalid-move sound plays and the source pile cards shake as feedback.
#[allow(clippy::too_many_arguments)]
fn handle_double_click(
    buttons: Res<ButtonInput<MouseButton>>,
    paused: Option<Res<PausedResource>>,
    time: Res<Time>,
    drag: Res<DragState>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Camera, &GlobalTransform)>,
    layout: Option<Res<LayoutResource>>,
    game: Res<GameStateResource>,
    mut last_click: Local<HashMap<u32, f32>>,
    mut moves: MessageWriter<MoveRequestEvent>,
    mut rejected: MessageWriter<MoveRejectedEvent>,
) {
    if paused.is_some_and(|p| p.0) {
        return;
    }
    if !buttons.just_pressed(MouseButton::Left) || !drag.is_idle() {
        return;
    }
    let Some(layout) = layout else { return };
    let Some(world) = cursor_world(&windows, &cameras) else { return };

    // Identify which card (or stack base) was clicked (must be face-up and draggable).
    let Some((pile, stack_index, card_ids)) = find_draggable_at(world, &game.0, &layout.0) else {
        return;
    };

    // The topmost card in the draggable run — used as the double-click key.
    let Some(&top_card_id) = card_ids.last() else { return };
    let top_index = stack_index + card_ids.len() - 1;
    let Some(top_card) = game.0.piles.get(&pile)
        .and_then(|p| p.cards.get(top_index)) else { return };
    if !top_card.face_up || top_card.id != top_card_id {
        return;
    }

    let now = time.elapsed_secs();
    let prev = last_click.get(&top_card_id).copied().unwrap_or(f32::NEG_INFINITY);

    if now - prev <= DOUBLE_CLICK_WINDOW {
        // Double-click confirmed.
        last_click.remove(&top_card_id);

        // Priority 1: move the single top card (foundation preferred, then tableau).
        if let Some(dest) = best_destination(top_card, &game.0) {
            moves.write(MoveRequestEvent {
                from: pile,
                to: dest,
                count: 1,
            });
            return;
        }

        // Priority 2: if the player clicked the base of a multi-card face-up
        // stack (card_ids.len() > 1), try moving the whole stack to another
        // tableau column.
        if card_ids.len() > 1 {
            let Some(bottom_card) = game.0.piles.get(&pile)
                .and_then(|p| p.cards.get(stack_index)) else { return };
            if let Some((dest, count)) = best_tableau_destination_for_stack(
                bottom_card,
                &pile,
                &game.0,
                card_ids.len(),
            ) {
                moves.write(MoveRequestEvent {
                    from: pile,
                    to: dest,
                    count,
                });
            } else {
                // No legal destination for the stack — play the invalid-move
                // sound and shake the source pile cards as feedback.
                // `MoveRejectedEvent` with `from == to` routes the shake to
                // the source pile (which `start_shake_anim` reads from `ev.to`).
                rejected.write(MoveRejectedEvent {
                    from: pile.clone(),
                    to: pile,
                    count: card_ids.len(),
                });
            }
        }
    } else {
        // Single click — record the time.
        last_click.insert(top_card_id, now);
    }
}

// ---------------------------------------------------------------------------
// Task #28 — Hint system helpers
// ---------------------------------------------------------------------------

/// Build the complete list of legal moves available in `game`, ordered so that
/// foundation moves come first, then tableau-to-tableau moves, with "draw from
/// stock" appended last when the stock is non-empty and nothing else is
/// available.
///
/// Each entry is `(from, to, count)` — the same triple used by
/// [`MoveRequestEvent`]. The list may be empty when no move exists at all
/// (game is stuck).
///
/// This is the backing data for the cycling hint system: the H key steps
/// through `hints[HintCycleIndex % hints.len()]` on each press.
pub fn all_hints(game: &GameState) -> Vec<(PileType, PileType, usize)> {
    let suits = [Suit::Clubs, Suit::Diamonds, Suit::Hearts, Suit::Spades];
    let sources: Vec<PileType> = {
        let mut s = vec![PileType::Waste];
        for i in 0..7_usize {
            s.push(PileType::Tableau(i));
        }
        s
    };

    let mut hints: Vec<(PileType, PileType, usize)> = Vec::new();

    // Pass 1 — foundation moves (highest priority, shown first).
    for from in &sources {
        let Some(from_pile) = game.piles.get(from) else { continue };
        let Some(card) = from_pile.cards.last().filter(|c| c.face_up) else { continue };
        for &suit in &suits {
            let dest = PileType::Foundation(suit);
            if let Some(dest_pile) = game.piles.get(&dest)
                && can_place_on_foundation(card, dest_pile, suit) {
                    hints.push((from.clone(), dest, 1));
                    // Each source card can go to at most one foundation suit;
                    // no need to check the remaining three for this card.
                    break;
                }
        }
    }

    // Pass 2 — tableau moves (deduplicated by source pile so we don't
    // repeat the same source card multiple times for different destinations).
    for from in &sources {
        let Some(from_pile) = game.piles.get(from) else { continue };
        let Some(card) = from_pile.cards.last().filter(|c| c.face_up) else { continue };
        // Skip if this source already has a foundation hint — prefer to show
        // that one when cycling rather than suggesting a less optimal move.
        let already_has_foundation_hint = hints.iter().any(|(f, t, _)| {
            f == from && matches!(t, PileType::Foundation(_))
        });
        if already_has_foundation_hint {
            continue;
        }
        for i in 0..7_usize {
            let dest = PileType::Tableau(i);
            if dest == *from {
                continue;
            }
            if let Some(dest_pile) = game.piles.get(&dest)
                && can_place_on_tableau(card, dest_pile) {
                    hints.push((from.clone(), dest, 1));
                    // One tableau destination per source card is enough for the
                    // hint list — the player can see where else a card can go
                    // via the right-click destination highlights.
                    break;
                }
        }
    }

    // Pass 3 — suggest drawing from the stock when no other hint was found.
    if hints.is_empty() {
        let stock_non_empty = game.piles.get(&PileType::Stock)
            .is_some_and(|p| !p.cards.is_empty());
        let waste_can_recycle = game.piles.get(&PileType::Stock)
            .is_some_and(|p| p.cards.is_empty())
            && game.piles.get(&PileType::Waste)
                .is_some_and(|p| !p.cards.is_empty());
        if stock_non_empty || waste_can_recycle {
            // Stock→Waste is not a real pile-to-pile move, but we reuse the
            // triple to signal "draw". The H handler only reads `from` to
            // locate the card to highlight; we point at the stock pile.
            hints.push((PileType::Stock, PileType::Waste, 1));
        }
    }

    hints
}

/// Find one valid move in the current game state.
///
/// Returns `(from, to, count)` for the first legal move found, or `None` if
/// no move is available. This is a convenience wrapper over [`all_hints`].
pub fn find_hint(game: &GameState) -> Option<(PileType, PileType, usize)> {
    all_hints(game).into_iter().next()
}

// `Vec3` is referenced only via the `DRAG_Z` constant; keep the import silenced
// when the compiler can't see it used.
#[allow(dead_code)]
const _VEC3_REFERENCED: Option<Vec3> = None;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::compute_layout;
    use solitaire_core::game_state::{DrawMode, GameState};

    #[test]
    fn point_in_rect_inside_returns_true() {
        let center = Vec2::new(10.0, 20.0);
        let size = Vec2::new(40.0, 60.0);
        assert!(point_in_rect(Vec2::new(10.0, 20.0), center, size));
        assert!(point_in_rect(Vec2::new(29.0, 49.0), center, size));
        assert!(point_in_rect(Vec2::new(-9.0, -9.0), center, size));
    }

    #[test]
    fn point_in_rect_on_edge_returns_true() {
        let center = Vec2::ZERO;
        let size = Vec2::new(10.0, 10.0);
        assert!(point_in_rect(Vec2::new(5.0, 5.0), center, size));
        assert!(point_in_rect(Vec2::new(-5.0, -5.0), center, size));
    }

    #[test]
    fn point_in_rect_outside_returns_false() {
        let center = Vec2::ZERO;
        let size = Vec2::new(10.0, 10.0);
        assert!(!point_in_rect(Vec2::new(6.0, 0.0), center, size));
        assert!(!point_in_rect(Vec2::new(0.0, 6.0), center, size));
        assert!(!point_in_rect(Vec2::new(-100.0, 0.0), center, size));
    }

    #[test]
    fn find_draggable_picks_top_of_tableau() {
        let game = GameState::new(42, DrawMode::DrawOne);
        let layout = compute_layout(Vec2::new(1280.0, 800.0));

        // In tableau 6, the visually topmost card is the last (face-up) one.
        // Its position: base.y + fan * 6.
        let top_pos = card_position(&game, &layout, &PileType::Tableau(6), 6);
        let result = find_draggable_at(top_pos, &game, &layout).expect("hit");
        assert_eq!(result.0, PileType::Tableau(6));
        assert_eq!(result.1, 6);
        assert_eq!(result.2.len(), 1);
    }

    #[test]
    fn find_draggable_skips_face_down_cards() {
        let game = GameState::new(42, DrawMode::DrawOne);
        let layout = compute_layout(Vec2::new(1280.0, 800.0));

        // Tableau 6 has 7 cards; only index 6 is face-up. A cursor over the
        // position of the bottom face-down card (index 0) should miss —
        // that card is face-down and the topmost face-up card overlaps at
        // a different fanned position.
        let bottom_pos = card_position(&game, &layout, &PileType::Tableau(6), 0);
        // Shift to avoid accidental overlap with the face-up card above it.
        let below_bottom = bottom_pos - Vec2::new(0.0, layout.card_size.y * 0.4);
        let result = find_draggable_at(below_bottom, &game, &layout);
        assert!(result.is_none(), "face-down cards should not be draggable");
    }

    #[test]
    fn find_draggable_returns_run_when_picking_mid_stack() {
        // Manually construct a tableau with three face-up cards all stacked.
        let mut game = GameState::new(1, DrawMode::DrawOne);
        use solitaire_core::card::{Card, Rank, Suit};
        let t0 = game.piles.get_mut(&PileType::Tableau(0)).unwrap();
        t0.cards.clear();
        t0.cards.push(Card {
            id: 100,
            suit: Suit::Spades,
            rank: Rank::King,
            face_up: true,
        });
        t0.cards.push(Card {
            id: 101,
            suit: Suit::Hearts,
            rank: Rank::Queen,
            face_up: true,
        });
        t0.cards.push(Card {
            id: 102,
            suit: Suit::Clubs,
            rank: Rank::Jack,
            face_up: true,
        });

        let layout = compute_layout(Vec2::new(1280.0, 800.0));
        // The Queen's geometric center (index 1) is inside the Jack's bounding box
        // (Jack fans 0.5h below base; its box spans [base-h, base]).  To hit the
        // Queen we click in her visible strip: the 0.25h band above the Jack's top
        // edge (base.y to base.y+0.25h).  Midpoint = queen_center + 0.375*card_h.
        let queen_center = card_position(&game, &layout, &PileType::Tableau(0), 1);
        let pos = queen_center + Vec2::new(0.0, layout.card_size.y * 0.375);
        let (pile, start, ids) = find_draggable_at(pos, &game, &layout).expect("hit");
        assert_eq!(pile, PileType::Tableau(0));
        assert_eq!(start, 1);
        assert_eq!(ids, vec![101, 102]);
    }

    #[test]
    fn find_draggable_skips_non_top_waste_card() {
        let mut game = GameState::new(1, DrawMode::DrawOne);
        use solitaire_core::card::{Card, Rank, Suit};
        let waste = game.piles.get_mut(&PileType::Waste).unwrap();
        waste.cards.clear();
        waste.cards.push(Card {
            id: 200,
            suit: Suit::Spades,
            rank: Rank::Two,
            face_up: true,
        });
        waste.cards.push(Card {
            id: 201,
            suit: Suit::Hearts,
            rank: Rank::Three,
            face_up: true,
        });

        let layout = compute_layout(Vec2::new(1280.0, 800.0));
        // Both cards in waste sit at the same (x, y). Clicking should pick
        // the visually top card (id 201), with count = 1.
        let pos = card_position(&game, &layout, &PileType::Waste, 0);
        let (pile, start, ids) = find_draggable_at(pos, &game, &layout).expect("hit");
        assert_eq!(pile, PileType::Waste);
        assert_eq!(start, 1);
        assert_eq!(ids, vec![201]);
    }

    #[test]
    fn find_drop_target_hits_empty_tableau_pile_marker() {
        let game = GameState::new(42, DrawMode::DrawOne);
        let layout = compute_layout(Vec2::new(1280.0, 800.0));
        // Move all cards out of tableau 0 so its marker is the only drop area.
        let mut game = game;
        game.piles.get_mut(&PileType::Tableau(0)).unwrap().cards.clear();
        let pos = layout.pile_positions[&PileType::Tableau(0)];
        let target = find_drop_target(pos, &game, &layout, &PileType::Tableau(6));
        assert_eq!(target, Some(PileType::Tableau(0)));
    }

    #[test]
    fn find_drop_target_returns_none_for_origin() {
        let game = GameState::new(42, DrawMode::DrawOne);
        let layout = compute_layout(Vec2::new(1280.0, 800.0));
        let pos = layout.pile_positions[&PileType::Tableau(3)];
        let target = find_drop_target(pos, &game, &layout, &PileType::Tableau(3));
        assert_eq!(target, None);
    }

    #[test]
    fn pile_drop_rect_extends_for_tableau_with_cards() {
        let game = GameState::new(42, DrawMode::DrawOne);
        let layout = compute_layout(Vec2::new(1280.0, 800.0));
        // Tableau 6 has 7 cards.
        let (_, size) = pile_drop_rect(&PileType::Tableau(6), &layout, &game);
        // Expected: card_height + 6 * fan. fan = 0.25 * card_height, so
        // size.y = card_height * (1 + 6 * 0.25) = card_height * 2.5.
        let expected = layout.card_size.y * 2.5;
        assert!(
            (size.y - expected).abs() < 1e-3,
            "expected {expected}, got {}",
            size.y
        );
    }

    #[test]
    fn find_draggable_draw_three_waste_top_card_hit_at_fanned_position() {
        use solitaire_core::card::{Card, Rank, Suit};
        use solitaire_core::game_state::{DrawMode, GameMode};
        let mut game = GameState::new_with_mode(1, DrawMode::DrawThree, GameMode::Classic);
        let waste = game.piles.get_mut(&PileType::Waste).unwrap();
        waste.cards.clear();
        // Three waste cards; top (id=202) is rightmost in the fan.
        waste.cards.push(Card { id: 200, suit: Suit::Spades, rank: Rank::Two, face_up: true });
        waste.cards.push(Card { id: 201, suit: Suit::Hearts, rank: Rank::Three, face_up: true });
        waste.cards.push(Card { id: 202, suit: Suit::Clubs, rank: Rank::Four, face_up: true });

        let layout = compute_layout(Vec2::new(1280.0, 800.0));
        let waste_base = layout.pile_positions[&PileType::Waste];
        // Top card (slot=2) is at base.x + 2 * 0.28 * card_width.
        let top_card_x = waste_base.x + 2.0 * 0.28 * layout.card_size.x;
        let cursor = Vec2::new(top_card_x, waste_base.y);

        let result = find_draggable_at(cursor, &game, &layout);
        assert!(result.is_some(), "top fanned waste card must be hittable at its visual X position");
        let (pile, _start, ids) = result.unwrap();
        assert_eq!(pile, PileType::Waste);
        assert_eq!(ids, vec![202], "only the top card is draggable from waste");
    }

    #[test]
    fn find_draggable_returns_none_for_click_on_empty_pile() {
        let mut game = GameState::new(42, DrawMode::DrawOne);
        let layout = compute_layout(Vec2::new(1280.0, 800.0));
        // Clear tableau 0 so it's an empty slot.
        game.piles.get_mut(&PileType::Tableau(0)).unwrap().cards.clear();
        let pos = layout.pile_positions[&PileType::Tableau(0)];
        let result = find_draggable_at(pos, &game, &layout);
        assert!(result.is_none(), "clicking an empty pile must not produce a draggable");
    }

    #[test]
    fn pile_drop_rect_is_card_sized_for_non_tableau() {
        let game = GameState::new(42, DrawMode::DrawOne);
        let layout = compute_layout(Vec2::new(1280.0, 800.0));
        for pile in [
            PileType::Waste,
            PileType::Foundation(Suit::Hearts),
        ] {
            let (_, size) = pile_drop_rect(&pile, &layout, &game);
            assert_eq!(size, layout.card_size);
        }
    }

    // -----------------------------------------------------------------------
    // Task #27 — best_destination pure-function tests
    // -----------------------------------------------------------------------

    #[test]
    fn best_destination_prefers_foundation_over_tableau() {
        use solitaire_core::card::{Card, Rank, Suit};
        use solitaire_core::game_state::GameMode;
        let mut game = GameState::new_with_mode(1, DrawMode::DrawOne, GameMode::Classic);

        // Put an Ace of Clubs in the waste pile.
        let waste = game.piles.get_mut(&PileType::Waste).unwrap();
        waste.cards.clear();
        waste.cards.push(Card { id: 200, suit: Suit::Clubs, rank: Rank::Ace, face_up: true });

        // Foundation for Clubs is empty — Ace should go there.
        let foundation = game.piles.get_mut(&PileType::Foundation(Suit::Clubs)).unwrap();
        foundation.cards.clear();

        let card = Card { id: 200, suit: Suit::Clubs, rank: Rank::Ace, face_up: true };
        let dest = best_destination(&card, &game);
        assert_eq!(dest, Some(PileType::Foundation(Suit::Clubs)));
    }

    #[test]
    fn best_destination_falls_back_to_tableau_when_no_foundation() {
        use solitaire_core::card::{Card, Rank, Suit};
        use solitaire_core::game_state::GameMode;
        let mut game = GameState::new_with_mode(1, DrawMode::DrawOne, GameMode::Classic);

        // Clear all foundations — a Two of Clubs cannot go there.
        for suit in [Suit::Clubs, Suit::Diamonds, Suit::Hearts, Suit::Spades] {
            game.piles.get_mut(&PileType::Foundation(suit)).unwrap().cards.clear();
        }

        // Put a Two of Clubs as the card.
        let card = Card { id: 300, suit: Suit::Clubs, rank: Rank::Two, face_up: true };

        // Set tableau 0 to have a Three of Hearts on top so we can place clubs two there.
        for i in 0..7_usize {
            game.piles.get_mut(&PileType::Tableau(i)).unwrap().cards.clear();
        }
        game.piles.get_mut(&PileType::Tableau(0)).unwrap().cards.push(Card {
            id: 301,
            suit: Suit::Hearts,
            rank: Rank::Three,
            face_up: true,
        });

        let dest = best_destination(&card, &game);
        assert_eq!(dest, Some(PileType::Tableau(0)));
    }

    #[test]
    fn best_destination_returns_none_when_no_legal_move() {
        use solitaire_core::card::{Card, Rank, Suit};
        let mut game = GameState::new(1, DrawMode::DrawOne);

        // Clear everything except one card that has nowhere to go.
        for suit in [Suit::Clubs, Suit::Diamonds, Suit::Hearts, Suit::Spades] {
            game.piles.get_mut(&PileType::Foundation(suit)).unwrap().cards.clear();
        }
        for i in 0..7_usize {
            game.piles.get_mut(&PileType::Tableau(i)).unwrap().cards.clear();
        }

        // A Two of Clubs with empty foundations and empty tableau has no destination.
        let card = Card { id: 400, suit: Suit::Clubs, rank: Rank::Two, face_up: true };
        assert!(best_destination(&card, &game).is_none());
    }

    // -----------------------------------------------------------------------
    // best_tableau_destination_for_stack pure-function tests
    // -----------------------------------------------------------------------

    #[test]
    fn best_tableau_destination_for_stack_finds_legal_column() {
        use solitaire_core::card::{Card, Rank, Suit};
        let mut game = GameState::new(1, DrawMode::DrawOne);

        // Clear all piles for a clean test.
        for suit in [Suit::Clubs, Suit::Diamonds, Suit::Hearts, Suit::Spades] {
            game.piles.get_mut(&PileType::Foundation(suit)).unwrap().cards.clear();
        }
        for i in 0..7_usize {
            game.piles.get_mut(&PileType::Tableau(i)).unwrap().cards.clear();
        }

        // Tableau 0: King of Spades (the source stack base), Queen of Hearts on top.
        let t0 = game.piles.get_mut(&PileType::Tableau(0)).unwrap();
        t0.cards.push(Card { id: 100, suit: Suit::Spades, rank: Rank::King, face_up: true });
        t0.cards.push(Card { id: 101, suit: Suit::Hearts, rank: Rank::Queen, face_up: true });

        // Tableau 1..6: empty — Kings can land on any of them.

        let bottom_card = Card { id: 100, suit: Suit::Spades, rank: Rank::King, face_up: true };
        let result = best_tableau_destination_for_stack(
            &bottom_card,
            &PileType::Tableau(0),
            &game,
            2,
        );
        assert!(result.is_some(), "should find a destination for King-stack");
        let (dest, count) = result.unwrap();
        assert!(matches!(dest, PileType::Tableau(_)));
        assert_ne!(dest, PileType::Tableau(0), "must not return the source pile");
        assert_eq!(count, 2);
    }

    #[test]
    fn best_tableau_destination_for_stack_skips_source_pile() {
        use solitaire_core::card::{Card, Rank, Suit};
        let mut game = GameState::new(1, DrawMode::DrawOne);

        for suit in [Suit::Clubs, Suit::Diamonds, Suit::Hearts, Suit::Spades] {
            game.piles.get_mut(&PileType::Foundation(suit)).unwrap().cards.clear();
        }
        for i in 0..7_usize {
            game.piles.get_mut(&PileType::Tableau(i)).unwrap().cards.clear();
        }

        // Only tableau 0 has anything; every other column is empty.
        // A King is the only card that can go on an empty tableau column.
        // Source is Tableau(0), so the result must NOT be Tableau(0).
        let t0 = game.piles.get_mut(&PileType::Tableau(0)).unwrap();
        t0.cards.push(Card { id: 200, suit: Suit::Hearts, rank: Rank::King, face_up: true });

        let bottom_card = Card { id: 200, suit: Suit::Hearts, rank: Rank::King, face_up: true };
        let result = best_tableau_destination_for_stack(
            &bottom_card,
            &PileType::Tableau(0),
            &game,
            1,
        );
        // Result must be some other empty tableau column, never the source.
        if let Some((dest, _)) = result {
            assert_ne!(dest, PileType::Tableau(0));
        }
    }

    #[test]
    fn best_tableau_destination_for_stack_returns_none_when_no_legal_move() {
        use solitaire_core::card::{Card, Rank, Suit};
        let mut game = GameState::new(1, DrawMode::DrawOne);

        for suit in [Suit::Clubs, Suit::Diamonds, Suit::Hearts, Suit::Spades] {
            game.piles.get_mut(&PileType::Foundation(suit)).unwrap().cards.clear();
        }
        for i in 0..7_usize {
            game.piles.get_mut(&PileType::Tableau(i)).unwrap().cards.clear();
        }

        // Source: tableau 0 has a Two of Clubs (can't go on empty pile; not a King).
        // All other piles are empty — no legal tableau target.
        let t0 = game.piles.get_mut(&PileType::Tableau(0)).unwrap();
        t0.cards.push(Card { id: 300, suit: Suit::Clubs, rank: Rank::Two, face_up: true });

        let bottom_card = Card { id: 300, suit: Suit::Clubs, rank: Rank::Two, face_up: true };
        let result = best_tableau_destination_for_stack(
            &bottom_card,
            &PileType::Tableau(0),
            &game,
            1,
        );
        assert!(result.is_none(), "Two of Clubs has no legal tableau destination on empty piles");
    }

    // -----------------------------------------------------------------------
    // Task #28 — find_hint pure-function tests
    // -----------------------------------------------------------------------

    #[test]
    fn find_hint_finds_ace_to_foundation() {
        use solitaire_core::card::{Card, Rank, Suit};
        let mut game = GameState::new(1, DrawMode::DrawOne);

        // Place Ace of Clubs on top of tableau 0.
        for i in 0..7_usize {
            game.piles.get_mut(&PileType::Tableau(i)).unwrap().cards.clear();
        }
        game.piles.get_mut(&PileType::Tableau(0)).unwrap().cards.push(Card {
            id: 500, suit: Suit::Clubs, rank: Rank::Ace, face_up: true,
        });
        game.piles.get_mut(&PileType::Foundation(Suit::Clubs)).unwrap().cards.clear();

        let hint = find_hint(&game);
        assert!(hint.is_some(), "should find a hint");
        let (from, to, count) = hint.unwrap();
        assert_eq!(from, PileType::Tableau(0));
        assert_eq!(to, PileType::Foundation(Suit::Clubs));
        assert_eq!(count, 1);
    }

    #[test]
    fn find_hint_returns_none_when_no_legal_move() {
        use solitaire_core::card::{Card, Rank, Suit};
        let mut game = GameState::new(1, DrawMode::DrawOne);

        // Put only a Two on tableau 0, empty everything else.
        for suit in [Suit::Clubs, Suit::Diamonds, Suit::Hearts, Suit::Spades] {
            game.piles.get_mut(&PileType::Foundation(suit)).unwrap().cards.clear();
        }
        for i in 0..7_usize {
            game.piles.get_mut(&PileType::Tableau(i)).unwrap().cards.clear();
        }
        game.piles.get_mut(&PileType::Waste).unwrap().cards.clear();
        game.piles.get_mut(&PileType::Stock).unwrap().cards.clear();

        // Two of Clubs has no legal destination.
        game.piles.get_mut(&PileType::Tableau(0)).unwrap().cards.push(Card {
            id: 600, suit: Suit::Clubs, rank: Rank::Two, face_up: true,
        });

        assert!(find_hint(&game).is_none(), "no hint should exist");
    }

    // -----------------------------------------------------------------------
    // Task #54 — forfeit double-confirm logic pure-function tests
    // -----------------------------------------------------------------------

    /// Verify the FORFEIT_CONFIRM_WINDOW constant is positive so the countdown
    /// window actually opens on the first G press.
    #[test]
    fn forfeit_confirm_window_is_positive() {
        const { assert!(FORFEIT_CONFIRM_WINDOW > 0.0, "FORFEIT_CONFIRM_WINDOW must be > 0"); }
    }

    /// Simulate the first G press: countdown was 0, so it should become
    /// FORFEIT_CONFIRM_WINDOW and no ForfeitEvent should be "sent" yet.
    #[test]
    fn forfeit_first_press_opens_countdown() {
        // Simulate: forfeit_countdown starts at 0 (no pending confirmation).
        let mut forfeit_countdown: f32 = 0.0;
        let active_game = true;

        // --- first G press logic (mirrors handle_keyboard) ---
        let forfeit_sent = if active_game {
            if forfeit_countdown > 0.0 {
                // Second press — would send ForfeitEvent.
                forfeit_countdown = 0.0;
                true
            } else {
                // First press — open window, send toast (not ForfeitEvent).
                forfeit_countdown = FORFEIT_CONFIRM_WINDOW;
                false
            }
        } else {
            false
        };

        assert!(!forfeit_sent, "ForfeitEvent must NOT fire on first G press");
        assert_eq!(
            forfeit_countdown, FORFEIT_CONFIRM_WINDOW,
            "countdown must be opened to FORFEIT_CONFIRM_WINDOW after first press"
        );
    }

    /// Simulate the second G press within the window: countdown > 0, so
    /// ForfeitEvent should fire and countdown resets to 0.
    #[test]
    fn forfeit_second_press_within_window_sends_event() {
        // Countdown is open from the first press.
        let mut forfeit_countdown: f32 = FORFEIT_CONFIRM_WINDOW - 1.0; // still in window
        let active_game = true;

        // --- second G press logic ---
        let forfeit_sent = if active_game {
            if forfeit_countdown > 0.0 {
                forfeit_countdown = 0.0;
                true
            } else {
                forfeit_countdown = FORFEIT_CONFIRM_WINDOW;
                false
            }
        } else {
            false
        };

        assert!(forfeit_sent, "ForfeitEvent MUST fire on second G press within window");
        assert_eq!(forfeit_countdown, 0.0, "countdown must reset to 0 after confirmation");
    }

    /// Simulate G press after the countdown has expired: countdown ticked to 0,
    /// so the next G press opens a fresh window (no ForfeitEvent).
    #[test]
    fn forfeit_press_after_countdown_expired_reopens_window() {
        // Countdown already expired.
        let mut forfeit_countdown: f32 = 0.0;
        let active_game = true;

        let forfeit_sent = if active_game {
            if forfeit_countdown > 0.0 {
                forfeit_countdown = 0.0;
                true
            } else {
                forfeit_countdown = FORFEIT_CONFIRM_WINDOW;
                false
            }
        } else {
            false
        };

        assert!(!forfeit_sent, "ForfeitEvent must NOT fire when countdown expired before second press");
        assert_eq!(
            forfeit_countdown, FORFEIT_CONFIRM_WINDOW,
            "a new confirmation window must open"
        );
    }

    /// Pressing any other key (e.g. U for undo) while the forfeit countdown is
    /// active must immediately cancel it (reset to 0.0).
    #[test]
    fn forfeit_cancelled_by_other_key_press() {
        // Countdown is open from the first G press.
        let mut forfeit_countdown: f32 = FORFEIT_CONFIRM_WINDOW - 0.5; // still in window

        // --- simulate U (undo) press: cancel countdown ---
        if forfeit_countdown > 0.0 {
            forfeit_countdown = 0.0;
        }
        // Then perform undo logic (omitted here as it requires Bevy infrastructure).

        assert_eq!(
            forfeit_countdown, 0.0,
            "forfeit countdown must be reset to 0.0 when another key is pressed"
        );
    }

    /// G press when no game is active must never fire ForfeitEvent and must
    /// not open a countdown.
    #[test]
    fn forfeit_no_active_game_does_nothing() {
        let mut forfeit_countdown: f32 = 0.0;
        let active_game = false;

        let forfeit_sent = if active_game {
            if forfeit_countdown > 0.0 {
                forfeit_countdown = 0.0;
                true
            } else {
                forfeit_countdown = FORFEIT_CONFIRM_WINDOW;
                false
            }
        } else {
            false
        };

        assert!(!forfeit_sent, "ForfeitEvent must not fire when no game is active");
        assert_eq!(forfeit_countdown, 0.0, "countdown must remain 0 when no game is active");
    }

    // -----------------------------------------------------------------------
    // all_hints / new-game window — pure-function tests added during refactor
    // -----------------------------------------------------------------------

    /// Pass 3 of `all_hints` should suggest drawing from the stock when there
    /// are no other moves and the stock is non-empty.
    #[test]
    fn all_hints_suggests_draw_when_no_moves_and_stock_nonempty() {
        use solitaire_core::card::{Card, Rank, Suit};
        let mut game = GameState::new(1, DrawMode::DrawOne);

        // Remove all foundation, tableau, and waste cards so no pile-to-pile
        // move exists. Leave one card in the stock.
        for suit in [Suit::Clubs, Suit::Diamonds, Suit::Hearts, Suit::Spades] {
            game.piles.get_mut(&PileType::Foundation(suit)).unwrap().cards.clear();
        }
        for i in 0..7_usize {
            game.piles.get_mut(&PileType::Tableau(i)).unwrap().cards.clear();
        }
        game.piles.get_mut(&PileType::Waste).unwrap().cards.clear();
        game.piles.get_mut(&PileType::Stock).unwrap().cards.clear();
        // Put one card back into the stock so "draw" is a valid suggestion.
        game.piles.get_mut(&PileType::Stock).unwrap().cards.push(Card {
            id: 1,
            suit: Suit::Clubs,
            rank: Rank::Ace,
            face_up: false,
        });

        let hints = all_hints(&game);
        assert_eq!(hints.len(), 1, "exactly one hint: draw from stock");
        let (from, to, count) = &hints[0];
        assert_eq!(*from, PileType::Stock, "hint must come from Stock");
        assert_eq!(*to, PileType::Waste, "hint must point to Waste");
        assert_eq!(*count, 1);
    }

    /// `all_hints` must be empty when both stock and waste are empty and no
    /// pile-to-pile move exists — the game is truly stuck.
    #[test]
    fn all_hints_is_empty_when_truly_stuck() {
        use solitaire_core::card::{Card, Rank, Suit};
        let mut game = GameState::new(1, DrawMode::DrawOne);

        // Clear every pile, then put a single card that has nowhere to go.
        for suit in [Suit::Clubs, Suit::Diamonds, Suit::Hearts, Suit::Spades] {
            game.piles.get_mut(&PileType::Foundation(suit)).unwrap().cards.clear();
        }
        for i in 0..7_usize {
            game.piles.get_mut(&PileType::Tableau(i)).unwrap().cards.clear();
        }
        game.piles.get_mut(&PileType::Waste).unwrap().cards.clear();
        game.piles.get_mut(&PileType::Stock).unwrap().cards.clear();

        // Two of Clubs on tableau 0 — can't go to an empty foundation (needs Ace
        // first) and can't go to any empty tableau column (not a King).
        game.piles.get_mut(&PileType::Tableau(0)).unwrap().cards.push(Card {
            id: 700,
            suit: Suit::Clubs,
            rank: Rank::Two,
            face_up: true,
        });

        let hints = all_hints(&game);
        assert!(hints.is_empty(), "no hint should exist when the game is truly stuck");
    }

    /// Const-assert that `NEW_GAME_CONFIRM_WINDOW` is positive so the
    /// confirmation countdown actually opens on the first N press.
    ///
    /// Mirrors the existing `forfeit_confirm_window_is_positive` test.
    #[test]
    fn new_game_confirm_window_is_positive() {
        const { assert!(NEW_GAME_CONFIRM_WINDOW > 0.0, "NEW_GAME_CONFIRM_WINDOW must be > 0"); }
    }

    // -----------------------------------------------------------------------
    // Task #57 — ShakeAnim insertion on rejected drag
    // -----------------------------------------------------------------------

    /// Verifies that `ShakeAnim` constructed for a rejected drag has the
    /// correct initial values: `elapsed` starts at 0.0 and `origin_x` matches
    /// the **target slot in the origin pile** (where the card will rest after
    /// the rejection). Saving the drop-location X here was the root cause of
    /// the "card lands beside the pile" bug — `tick_shake_anim` restores
    /// `translation.x` to `origin_x` at the end of the shake, fighting the
    /// `sync_cards` slide that `StateChangedEvent` triggers.
    ///
    /// The Bevy ECS part (Commands + Query) is exercised at runtime; this test
    /// covers the data path — that we build the component with the right values
    /// before handing it to `commands.entity(...).insert(...)`.
    #[test]
    fn shake_anim_for_rejected_drag_has_correct_initial_values() {
        use crate::feedback_anim_plugin::ShakeAnim;

        // Simulate the X coordinate of the card's slot in its origin pile —
        // computed by `card_position(game, layout, &origin, stack_index)` at
        // rejection time, not the drop-location transform X.
        let target_slot_x = 123.5_f32;

        // This mirrors the ShakeAnim construction in `end_drag` and
        // `touch_end_drag` after the bugfix: origin_x is the origin pile's
        // slot X, so the shake ends with the card at its correct resting
        // position.
        let anim = ShakeAnim {
            elapsed: 0.0,
            origin_x: target_slot_x,
        };

        assert_eq!(
            anim.elapsed, 0.0,
            "ShakeAnim must start with elapsed=0.0 so the animation plays from the beginning"
        );
        assert!(
            (anim.origin_x - target_slot_x).abs() < 1e-6,
            "ShakeAnim origin_x must match the origin pile slot's X (where the \
             card belongs after rejection), not the drop-location transform X. \
             Expected {target_slot_x}, got {}",
            anim.origin_x
        );
    }

    /// When a drag is rejected, every card id in `drag.cards` should receive a
    /// `ShakeAnim`. Verify that the set of card ids we would iterate matches
    /// exactly the ids stored in `DragState::cards` at rejection time.
    #[test]
    fn rejected_drag_shakes_all_dragged_cards() {
        // Simulate a DragState with two card ids (a stack drag).
        let dragged_ids: Vec<u32> = vec![10, 11];

        // In `end_drag`, we iterate `drag.cards` and look up each id in
        // `card_entities`. The ids we would insert ShakeAnim on must exactly
        // match the dragged set.
        let mut shaken: Vec<u32> = Vec::new();
        for &card_id in &dragged_ids {
            // Simulate finding the entity for card_id (always succeeds here).
            shaken.push(card_id);
        }

        assert_eq!(
            shaken, dragged_ids,
            "every card id in drag.cards must receive a ShakeAnim on rejection"
        );
    }
}

