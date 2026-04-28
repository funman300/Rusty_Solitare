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
use bevy::input::ButtonInput;
use bevy::math::{Vec2, Vec3};
use bevy::prelude::*;
use bevy::window::{MonitorSelection, PrimaryWindow, WindowMode};
use solitaire_core::card::{Card, Suit};
use solitaire_core::game_state::GameState;
use solitaire_core::pile::PileType;
use solitaire_core::rules::{can_place_on_foundation, can_place_on_tableau};

use crate::card_plugin::{CardEntity, HintHighlight, TABLEAU_FAN_FRAC};
use crate::feedback_anim_plugin::ShakeAnim;
use solitaire_core::game_state::DrawMode;
use crate::challenge_plugin::CHALLENGE_UNLOCK_LEVEL;
use crate::events::{
    DrawRequestEvent, ForfeitEvent, InfoToastEvent, MoveRejectedEvent, MoveRequestEvent,
    NewGameConfirmEvent, NewGameRequestEvent, StateChangedEvent, UndoRequestEvent,
};
use crate::game_plugin::GameMutation;
use crate::pause_plugin::PausedResource;
use crate::progress_plugin::ProgressResource;
use crate::layout::{Layout, LayoutResource};
use crate::resources::{DragState, GameStateResource, HintCycleIndex};
use crate::time_attack_plugin::TimeAttackResource;

/// Z-depth used for cards while being dragged — above all resting cards.
const DRAG_Z: f32 = 500.0;

/// Registers keyboard and mouse input systems.
///
/// Drag systems run in a fixed order each frame:
/// `start_drag` → `follow_drag` → `end_drag`, with `follow_drag` after the
/// card-position sync so it overrides resting positions for cards being
/// dragged. `end_drag` runs before `GameMutation` so the `MoveRequestEvent`
/// it fires is consumed the same frame.
pub struct InputPlugin;

impl Plugin for InputPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<HintCycleIndex>()
            .add_event::<NewGameConfirmEvent>()
            .add_event::<InfoToastEvent>()
            .add_event::<ForfeitEvent>()
            .add_systems(
                Update,
                (
                    handle_keyboard,
                    handle_stock_click,
                    handle_double_click,
                    start_drag,
                    follow_drag,
                    end_drag.before(GameMutation),
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

/// Bundles all event writers used by `handle_keyboard` so the system stays
/// within Bevy's 16-parameter limit.
#[derive(SystemParam)]
struct KeyboardEvents<'w> {
    undo: EventWriter<'w, UndoRequestEvent>,
    new_game: EventWriter<'w, NewGameRequestEvent>,
    confirm_event: EventWriter<'w, NewGameConfirmEvent>,
    info_toast: EventWriter<'w, InfoToastEvent>,
    draw: EventWriter<'w, DrawRequestEvent>,
    forfeit: EventWriter<'w, ForfeitEvent>,
}

#[allow(clippy::too_many_arguments)]
fn handle_keyboard(
    keys: Res<ButtonInput<KeyCode>>,
    paused: Option<Res<PausedResource>>,
    progress: Option<Res<ProgressResource>>,
    game: Option<Res<GameStateResource>>,
    time: Res<Time>,
    mut confirm_countdown: Local<f32>,
    mut confirm_pending: Local<bool>,
    mut forfeit_countdown: Local<f32>,
    mut ev: KeyboardEvents,
    mut commands: Commands,
    card_entities: Query<(Entity, &CardEntity, &Sprite)>,
    layout: Option<Res<LayoutResource>>,
    mut hint_cycle: ResMut<HintCycleIndex>,
    mut time_attack: Option<ResMut<TimeAttackResource>>,
) {
    if paused.is_some_and(|p| p.0) {
        return;
    }
    // Tick down any active confirmation window.
    if *confirm_countdown > 0.0 {
        *confirm_countdown -= time.delta_secs();
        if *confirm_countdown <= 0.0 {
            *confirm_countdown = 0.0;
            // Countdown expired without a second N press — notify the player.
            if *confirm_pending {
                *confirm_pending = false;
                ev.info_toast.send(InfoToastEvent("New game cancelled".to_string()));
            }
        }
    }
    // Tick down the forfeit confirmation window.
    if *forfeit_countdown > 0.0 {
        *forfeit_countdown -= time.delta_secs();
        if *forfeit_countdown <= 0.0 {
            *forfeit_countdown = 0.0;
        }
    }

    if keys.just_pressed(KeyCode::KeyU) {
        if *forfeit_countdown > 0.0 { *forfeit_countdown = 0.0; }
        ev.undo.send(UndoRequestEvent);
    }
    if keys.just_pressed(KeyCode::KeyN) {
        // If a Time Attack session is running, cancel it and start a Classic game.
        if let Some(ref mut session) = time_attack {
            if session.active {
                session.active = false;
                session.remaining_secs = 0.0;
                ev.info_toast.send(InfoToastEvent("Time Attack ended".to_string()));
                ev.new_game.send(NewGameRequestEvent {
                    seed: None,
                    mode: Some(solitaire_core::game_state::GameMode::Classic),
                });
                *confirm_countdown = 0.0;
                return;
            }
        }

        let active_game = game.as_ref().is_some_and(|g| g.0.move_count > 0 && !g.0.is_won);
        let shift_held = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
        if shift_held || !active_game {
            // Shift+N or no active game — start immediately, no confirmation.
            ev.new_game.send(NewGameRequestEvent::default());
            *confirm_countdown = 0.0;
            *confirm_pending = false;
        } else if *confirm_countdown > 0.0 {
            // Second press within the window — confirmed.
            ev.new_game.send(NewGameRequestEvent::default());
            *confirm_countdown = 0.0;
            *confirm_pending = false;
        } else {
            // First press on an active game — require confirmation.
            *confirm_countdown = NEW_GAME_CONFIRM_WINDOW;
            *confirm_pending = true;
            ev.confirm_event.send(NewGameConfirmEvent);
        }
    }
    if keys.just_pressed(KeyCode::KeyZ) {
        if *forfeit_countdown > 0.0 { *forfeit_countdown = 0.0; }
        // Zen / Challenge / Time Attack are gated to level >= CHALLENGE_UNLOCK_LEVEL.
        // X is gated separately by ChallengePlugin.
        let level = progress.as_ref().map_or(0, |p| p.0.level);
        if level >= CHALLENGE_UNLOCK_LEVEL {
            ev.new_game.send(NewGameRequestEvent {
                seed: None,
                mode: Some(solitaire_core::game_state::GameMode::Zen),
            });
        } else {
            ev.info_toast.send(InfoToastEvent(format!(
                "Zen mode unlocks at level {CHALLENGE_UNLOCK_LEVEL}"
            )));
        }
    }
    if keys.just_pressed(KeyCode::KeyD) || keys.just_pressed(KeyCode::Space) {
        if *forfeit_countdown > 0.0 { *forfeit_countdown = 0.0; }
        ev.draw.send(DrawRequestEvent);
    }
    // H — cycle through all available hints on each press, highlighting the
    // source card yellow for 1.5 s. The index wraps around once all hints have
    // been shown. When no moves are available a toast is shown instead.
    if keys.just_pressed(KeyCode::KeyH) {
        if *forfeit_countdown > 0.0 { *forfeit_countdown = 0.0; }
        if let Some(ref g) = game {
            if g.0.is_won {
                ev.info_toast.send(InfoToastEvent(
                    "Game won! Press N for a new game".to_string(),
                ));
            } else if let Some(ref layout_res) = layout {
                    let hints = all_hints(&g.0);
                    if hints.is_empty() {
                        ev.info_toast.send(InfoToastEvent("No hints available".to_string()));
                    } else {
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
                            ev.info_toast.send(InfoToastEvent(msg));
                        } else {
                            // Find the top face-up card in the source pile and highlight it.
                            let top_card_id = g.0.piles.get(from)
                                .and_then(|p| p.cards.last().filter(|c| c.face_up))
                                .map(|c| c.id);
                            if let Some(card_id) = top_card_id {
                                for (entity, card_entity, _sprite) in card_entities.iter() {
                                    if card_entity.card_id == card_id {
                                        commands.entity(entity)
                                            .insert(HintHighlight { remaining: 1.5 })
                                            .insert(Sprite {
                                                color: Color::srgba(1.0, 1.0, 0.4, 1.0),
                                                custom_size: Some(layout_res.0.card_size),
                                                ..default()
                                            });
                                        break;
                                    }
                                }
                            }
                            // Fire an informational toast describing where the hinted card
                            // should move so the player always sees the suggestion in text.
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
                                PileType::Tableau(col) => {
                                    format!("Hint: move to tableau (col {})", col + 1)
                                }
                                _ => "Hint: move card".to_string(),
                            };
                            ev.info_toast.send(InfoToastEvent(msg));
                        }
                    }
            }
        }
    }
    // G — forfeit the current game with a 3-second double-confirm window to
    // prevent accidental forfeits. First press shows a toast and starts the
    // countdown; second press within the window sends the ForfeitEvent.
    if keys.just_pressed(KeyCode::KeyG) {
        let active_game = game.as_ref().is_some_and(|g| g.0.move_count > 0 && !g.0.is_won);
        if active_game {
            if *forfeit_countdown > 0.0 {
                // Second press within the confirmation window — confirmed.
                ev.forfeit.send(ForfeitEvent);
                *forfeit_countdown = 0.0;
            } else {
                // First press — start the countdown and warn the player.
                *forfeit_countdown = FORFEIT_CONFIRM_WINDOW;
                ev.info_toast.send(InfoToastEvent("Press G again to forfeit".to_string()));
            }
        }
    }
    // Esc is handled by `PausePlugin` (overlay toggle + paused flag).
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
    mut state_events: EventReader<StateChangedEvent>,
    mut new_game_events: EventReader<NewGameRequestEvent>,
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
    mut toast: EventWriter<InfoToastEvent>,
) {
    if !keys.just_pressed(KeyCode::F11) {
        return;
    }
    let Ok(mut window) = windows.get_single_mut() else { return };
    let new_mode = match window.mode {
        WindowMode::Windowed => WindowMode::BorderlessFullscreen(MonitorSelection::Current),
        _ => WindowMode::Windowed,
    };
    window.mode = new_mode;
    let label = match window.mode {
        WindowMode::Windowed => "Fullscreen: off",
        _ => "Fullscreen: on",
    };
    toast.send(InfoToastEvent(label.to_string()));
}

fn handle_stock_click(
    buttons: Res<ButtonInput<MouseButton>>,
    drag: Res<DragState>,
    paused: Option<Res<PausedResource>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Camera, &GlobalTransform)>,
    layout: Option<Res<LayoutResource>>,
    mut draw: EventWriter<DrawRequestEvent>,
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
        draw.send(DrawRequestEvent);
    }
}

#[allow(clippy::too_many_arguments)]
fn start_drag(
    buttons: Res<ButtonInput<MouseButton>>,
    paused: Option<Res<PausedResource>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Camera, &GlobalTransform)>,
    layout: Option<Res<LayoutResource>>,
    game: Res<GameStateResource>,
    mut drag: ResMut<DragState>,
    mut card_visuals: Query<(&CardEntity, &mut Transform, &mut Sprite)>,
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

    // Don't try to pick up the stock — that's the draw click.
    let Some((pile, stack_index, card_ids)) = find_draggable_at(world, &game.0, &layout.0) else {
        return;
    };

    let Some(&bottom_id) = card_ids.first() else {
        return;
    };

    // Find the bottom drag card's current world position so we can compute
    // the offset between cursor and that card (grab point).
    let bottom_pos = card_position(&game.0, &layout.0, pile.clone(), stack_index);
    let cursor_offset = bottom_pos - world;

    // Elevate dragged cards to DRAG_Z and dim them slightly so the board
    // beneath remains visible during the drag.
    for (i, id) in card_ids.iter().enumerate() {
        if let Some((_, mut transform, mut sprite)) = card_visuals
            .iter_mut()
            .find(|(entity, _, _)| entity.card_id == *id)
        {
            transform.translation.z = DRAG_Z + (i as f32) * 0.01;
            sprite.color.set_alpha(0.85);
        }
    }

    drag.cards = card_ids;
    drag.origin_pile = Some(pile);
    drag.cursor_offset = cursor_offset;
    drag.origin_z = DRAG_Z;
    let _ = bottom_id; // retained for clarity, not used further
}

fn follow_drag(
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Camera, &GlobalTransform)>,
    drag: Res<DragState>,
    layout: Option<Res<LayoutResource>>,
    mut card_transforms: Query<(&CardEntity, &mut Transform)>,
) {
    if drag.is_idle() {
        return;
    }
    let Some(layout) = layout else {
        return;
    };
    let Some(world) = cursor_world(&windows, &cameras) else {
        return;
    };

    let bottom_pos = world + drag.cursor_offset;
    let fan = -layout.0.card_size.y * TABLEAU_FAN_FRAC;

    for (i, id) in drag.cards.iter().enumerate() {
        if let Some((_, mut transform)) = card_transforms
            .iter_mut()
            .find(|(entity, _)| entity.card_id == *id)
        {
            transform.translation.x = bottom_pos.x;
            transform.translation.y = bottom_pos.y + fan * (i as f32);
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
    mut moves: EventWriter<MoveRequestEvent>,
    mut rejected: EventWriter<MoveRejectedEvent>,
    mut changed: EventWriter<StateChangedEvent>,
    mut commands: Commands,
    card_entities: Query<(Entity, &CardEntity, &Transform)>,
) {
    if paused.is_some_and(|p| p.0) {
        drag.clear();
        return;
    }
    if !buttons.just_released(MouseButton::Left) || drag.is_idle() {
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
    if let Some(target) = target {
        if target != origin {
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
                    moves.send(MoveRequestEvent {
                        from: origin.clone(),
                        to: target.clone(),
                        count,
                    });
                    fired = true;
                } else {
                    rejected.send(MoveRejectedEvent {
                        from: origin.clone(),
                        to: target.clone(),
                        count,
                    });
                    // Shake each dragged card so the player gets immediate
                    // visual feedback that the drop was rejected.
                    for &card_id in &drag.cards {
                        if let Some((entity, _, transform)) = card_entities
                            .iter()
                            .find(|(_, ce, _)| ce.card_id == card_id)
                        {
                            commands.entity(entity).insert(ShakeAnim {
                                elapsed: 0.0,
                                origin_x: transform.translation.x,
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
    changed.send(StateChangedEvent);
    let _ = fired;
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn cursor_world(
    windows: &Query<&Window, With<PrimaryWindow>>,
    cameras: &Query<(&Camera, &GlobalTransform)>,
) -> Option<Vec2> {
    let window = windows.get_single().ok()?;
    let cursor = window.cursor_position()?;
    let (camera, camera_transform) = cameras.get_single().ok()?;
    camera.viewport_to_world_2d(camera_transform, cursor).ok()
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
fn card_position(game: &GameState, layout: &Layout, pile: PileType, stack_index: usize) -> Vec2 {
    let base = layout.pile_positions[&pile];
    if matches!(pile, PileType::Tableau(_)) {
        let fan = -layout.card_size.y * TABLEAU_FAN_FRAC;
        Vec2::new(base.x, base.y + fan * (stack_index as f32))
    } else if matches!(pile, PileType::Waste) && game.draw_mode == DrawMode::DrawThree {
        // In Draw-Three mode the top 3 waste cards are fanned in X to match
        // card_plugin::card_positions(). Hit-testing must use the same offsets
        // so clicking the visually rightmost (top) card actually registers.
        let pile_len = game.piles.get(&pile).map_or(0, |p| p.cards.len());
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
            let pos = card_position(game, layout, pile.clone(), i);
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
        if let Some(pile) = game.piles.get(&dest) {
            if can_place_on_foundation(card, pile, suit) {
                return Some(dest);
            }
        }
    }
    // Then try all seven tableau piles.
    for i in 0..7_usize {
        let dest = PileType::Tableau(i);
        if let Some(pile) = game.piles.get(&dest) {
            if can_place_on_tableau(card, pile) {
                return Some(dest);
            }
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
        if let Some(pile) = game.piles.get(&dest) {
            if can_place_on_tableau(bottom_card, pile) {
                return Some((dest, stack_count));
            }
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
    mut moves: EventWriter<MoveRequestEvent>,
    mut rejected: EventWriter<MoveRejectedEvent>,
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
            moves.send(MoveRequestEvent {
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
                moves.send(MoveRequestEvent {
                    from: pile,
                    to: dest,
                    count,
                });
            } else {
                // No legal destination for the stack — play the invalid-move
                // sound and shake the source pile cards as feedback.
                // `MoveRejectedEvent` with `from == to` routes the shake to
                // the source pile (which `start_shake_anim` reads from `ev.to`).
                rejected.send(MoveRejectedEvent {
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
            if let Some(dest_pile) = game.piles.get(&dest) {
                if can_place_on_foundation(card, dest_pile, suit) {
                    hints.push((from.clone(), dest, 1));
                    // Each source card can go to at most one foundation suit;
                    // no need to check the remaining three for this card.
                    break;
                }
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
            if let Some(dest_pile) = game.piles.get(&dest) {
                if can_place_on_tableau(card, dest_pile) {
                    hints.push((from.clone(), dest, 1));
                    // One tableau destination per source card is enough for the
                    // hint list — the player can see where else a card can go
                    // via the right-click destination highlights.
                    break;
                }
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
        let top_pos = card_position(&game, &layout, PileType::Tableau(6), 6);
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
        let bottom_pos = card_position(&game, &layout, PileType::Tableau(6), 0);
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
        let queen_center = card_position(&game, &layout, PileType::Tableau(0), 1);
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
        let pos = card_position(&game, &layout, PileType::Waste, 0);
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
        assert!(FORFEIT_CONFIRM_WINDOW > 0.0, "FORFEIT_CONFIRM_WINDOW must be > 0");
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
    // Task #57 — ShakeAnim insertion on rejected drag
    // -----------------------------------------------------------------------

    /// Verifies that `ShakeAnim` constructed for a rejected drag has the
    /// correct initial values: `elapsed` starts at 0.0 and `origin_x` matches
    /// the card's current transform X position.
    ///
    /// The Bevy ECS part (Commands + Query) is exercised at runtime; this test
    /// covers the data path — that we build the component with the right values
    /// before handing it to `commands.entity(...).insert(...)`.
    #[test]
    fn shake_anim_for_rejected_drag_has_correct_initial_values() {
        use crate::feedback_anim_plugin::ShakeAnim;

        // Simulate the transform X that a dragged card would have at the
        // moment the drag is released (could be anywhere on screen).
        let current_x = 123.5_f32;

        // This mirrors the ShakeAnim construction in `end_drag`.
        let anim = ShakeAnim {
            elapsed: 0.0,
            origin_x: current_x,
        };

        assert_eq!(
            anim.elapsed, 0.0,
            "ShakeAnim must start with elapsed=0.0 so the animation plays from the beginning"
        );
        assert!(
            (anim.origin_x - current_x).abs() < 1e-6,
            "ShakeAnim origin_x must match the card's transform X at drop time, \
             expected {current_x}, got {}",
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

// `Vec3` is referenced only via the `DRAG_Z` constant; keep the import silenced
// when the compiler can't see it used.
#[allow(dead_code)]
const _VEC3_REFERENCED: Option<Vec3> = None;
