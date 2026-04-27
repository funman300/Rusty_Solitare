//! Keyboard + mouse input for the game board.
//!
//! All systems exit immediately when `PausedResource(true)` — no moves,
//! draws, undos, or drags are processed while the pause overlay is showing.
//!
//! Keyboard:
//! - `U` → `UndoRequestEvent`
//! - `N` → `NewGameRequestEvent { seed: None }`
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

use bevy::input::ButtonInput;
use bevy::math::{Vec2, Vec3};
use bevy::prelude::*;
use bevy::window::{MonitorSelection, PrimaryWindow, WindowMode};
use solitaire_core::card::{Card, Suit};
use solitaire_core::game_state::GameState;
use solitaire_core::pile::PileType;
use solitaire_core::rules::{can_place_on_foundation, can_place_on_tableau};

use crate::card_plugin::{CardEntity, HintHighlight, TABLEAU_FAN_FRAC};
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
use crate::resources::{DragState, GameStateResource};

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
        app.add_event::<NewGameConfirmEvent>()
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
            .add_systems(Update, handle_fullscreen);
    }
}

/// Seconds after the first N press during which a second N confirms new game.
const NEW_GAME_CONFIRM_WINDOW: f32 = 3.0;

#[allow(clippy::too_many_arguments)]
fn handle_keyboard(
    keys: Res<ButtonInput<KeyCode>>,
    paused: Option<Res<PausedResource>>,
    progress: Option<Res<ProgressResource>>,
    game: Option<Res<GameStateResource>>,
    time: Res<Time>,
    mut confirm_countdown: Local<f32>,
    mut undo: EventWriter<UndoRequestEvent>,
    mut new_game: EventWriter<NewGameRequestEvent>,
    mut confirm_event: EventWriter<NewGameConfirmEvent>,
    mut info_toast: EventWriter<InfoToastEvent>,
    mut draw: EventWriter<DrawRequestEvent>,
    mut forfeit: EventWriter<ForfeitEvent>,
    mut commands: Commands,
    card_entities: Query<(Entity, &CardEntity, &Sprite)>,
    layout: Option<Res<LayoutResource>>,
) {
    if paused.is_some_and(|p| p.0) {
        return;
    }
    // Tick down any active confirmation window.
    if *confirm_countdown > 0.0 {
        *confirm_countdown -= time.delta_secs();
        if *confirm_countdown <= 0.0 {
            *confirm_countdown = 0.0;
        }
    }

    if keys.just_pressed(KeyCode::KeyU) {
        undo.send(UndoRequestEvent);
    }
    if keys.just_pressed(KeyCode::KeyN) {
        let active_game = game.as_ref().is_some_and(|g| g.0.move_count > 0 && !g.0.is_won);
        let shift_held = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
        if shift_held || !active_game {
            // Shift+N or no active game — start immediately, no confirmation.
            new_game.send(NewGameRequestEvent::default());
            *confirm_countdown = 0.0;
        } else if *confirm_countdown > 0.0 {
            // Second press within the window — confirmed.
            new_game.send(NewGameRequestEvent::default());
            *confirm_countdown = 0.0;
        } else {
            // First press on an active game — require confirmation.
            *confirm_countdown = NEW_GAME_CONFIRM_WINDOW;
            confirm_event.send(NewGameConfirmEvent);
        }
    }
    if keys.just_pressed(KeyCode::KeyZ) {
        // Zen / Challenge / Time Attack are gated to level >= CHALLENGE_UNLOCK_LEVEL.
        // X is gated separately by ChallengePlugin.
        let level = progress.as_ref().map_or(0, |p| p.0.level);
        if level >= CHALLENGE_UNLOCK_LEVEL {
            new_game.send(NewGameRequestEvent {
                seed: None,
                mode: Some(solitaire_core::game_state::GameMode::Zen),
            });
        } else {
            info_toast.send(InfoToastEvent(format!(
                "Zen mode unlocks at level {CHALLENGE_UNLOCK_LEVEL}"
            )));
        }
    }
    if keys.just_pressed(KeyCode::KeyD) || keys.just_pressed(KeyCode::Space) {
        draw.send(DrawRequestEvent);
    }
    // H — show a hint (highlight the source card of the best available move).
    if keys.just_pressed(KeyCode::KeyH) {
        if let Some(ref g) = game {
            if !g.0.is_won {
                if let Some(ref layout_res) = layout {
                    if let Some((from, _to, _count)) = find_hint(&g.0) {
                        // Find the top face-up card in the source pile.
                        let top_card_id = g.0.piles.get(&from)
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
                    } else {
                        info_toast.send(InfoToastEvent("No hints available".to_string()));
                    }
                }
            }
        }
    }
    // G — forfeit the current game (only when a game is actually in progress).
    if keys.just_pressed(KeyCode::KeyG) {
        let active_game = game.as_ref().is_some_and(|g| g.0.move_count > 0 && !g.0.is_won);
        if active_game {
            forfeit.send(ForfeitEvent);
        }
    }
    // Esc is handled by `PausePlugin` (overlay toggle + paused flag).
}

/// `F11` toggles between borderless-fullscreen and windowed mode.
/// Not gated by the pause flag — the player can always resize the window.
fn handle_fullscreen(
    keys: Res<ButtonInput<KeyCode>>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
) {
    if !keys.just_pressed(KeyCode::F11) {
        return;
    }
    let Ok(mut window) = windows.get_single_mut() else { return };
    window.mode = match window.mode {
        WindowMode::Windowed => WindowMode::BorderlessFullscreen(MonitorSelection::Current),
        _ => WindowMode::Windowed,
    };
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
    mut card_transforms: Query<(&CardEntity, &mut Transform)>,
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

    // Elevate dragged cards to DRAG_Z.
    for (i, id) in card_ids.iter().enumerate() {
        if let Some((_, mut transform)) = card_transforms
            .iter_mut()
            .find(|(entity, _)| entity.card_id == *id)
        {
            transform.translation.z = DRAG_Z + (i as f32) * 0.01;
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

/// System that detects double-clicks on face-up cards and fires `MoveRequestEvent`
/// to the best legal destination (foundation before tableau).
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
) {
    if paused.is_some_and(|p| p.0) {
        return;
    }
    if !buttons.just_pressed(MouseButton::Left) || !drag.is_idle() {
        return;
    }
    let Some(layout) = layout else { return };
    let Some(world) = cursor_world(&windows, &cameras) else { return };

    // Identify which card was clicked (must be face-up and draggable).
    let Some((pile, stack_index, card_ids)) = find_draggable_at(world, &game.0, &layout.0) else {
        return;
    };
    // Only auto-move a single card (top card of the stack).
    let Some(&top_card_id) = card_ids.last() else { return };
    // The top draggable card is at `stack_index + card_ids.len() - 1`.
    let top_index = stack_index + card_ids.len() - 1;
    let Some(card) = game.0.piles.get(&pile)
        .and_then(|p| p.cards.get(top_index)) else { return };

    if !card.face_up || card.id != top_card_id {
        return;
    }

    let now = time.elapsed_secs();
    let prev = last_click.get(&top_card_id).copied().unwrap_or(f32::NEG_INFINITY);

    if now - prev <= DOUBLE_CLICK_WINDOW {
        // Double-click detected — find and fire the best move.
        last_click.remove(&top_card_id);
        if let Some(dest) = best_destination(card, &game.0) {
            moves.send(MoveRequestEvent {
                from: pile,
                to: dest,
                count: 1,
            });
        }
    } else {
        // Single click — record the time.
        last_click.insert(top_card_id, now);
    }
}

// ---------------------------------------------------------------------------
// Task #28 — Hint system helpers
// ---------------------------------------------------------------------------

/// Find one valid move in the current game state.
///
/// Returns `(from, to, count)` for the first legal move found, or `None` if
/// no move is available. Sources checked: Waste top, then Tableau 0–6.
/// Destinations checked: all 4 Foundations, then all 7 Tableau piles.
pub fn find_hint(game: &GameState) -> Option<(PileType, PileType, usize)> {
    let sources: Vec<PileType> = {
        let mut s = vec![PileType::Waste];
        for i in 0..7_usize {
            s.push(PileType::Tableau(i));
        }
        s
    };

    let suits = [Suit::Clubs, Suit::Diamonds, Suit::Hearts, Suit::Spades];

    for from in &sources {
        let Some(from_pile) = game.piles.get(from) else { continue };
        let Some(card) = from_pile.cards.last().filter(|c| c.face_up) else { continue };

        // Check foundations.
        for &suit in &suits {
            let dest = PileType::Foundation(suit);
            if let Some(dest_pile) = game.piles.get(&dest) {
                if can_place_on_foundation(card, dest_pile, suit) {
                    return Some((from.clone(), dest, 1));
                }
            }
        }

        // Check tableau piles (skip the source pile itself).
        for i in 0..7_usize {
            let dest = PileType::Tableau(i);
            if dest == *from {
                continue;
            }
            if let Some(dest_pile) = game.piles.get(&dest) {
                if can_place_on_tableau(card, dest_pile) {
                    return Some((from.clone(), dest, 1));
                }
            }
        }
    }
    None
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
}

// `Vec3` is referenced only via the `DRAG_Z` constant; keep the import silenced
// when the compiler can't see it used.
#[allow(dead_code)]
const _VEC3_REFERENCED: Option<Vec3> = None;
