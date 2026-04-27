//! Keyboard + mouse input for the game board.
//!
//! Keyboard:
//! - `U` → `UndoRequestEvent`
//! - `N` → `NewGameRequestEvent { seed: None }`
//! - `D` → `DrawRequestEvent`
//! - `Esc` → handled by `PausePlugin` (overlay toggle + paused flag)
//!
//! Mouse:
//! - Left-click on the stock pile (face-down top) → `DrawRequestEvent`
//! - Left-press-drag-release on a face-up card → `MoveRequestEvent` between
//!   the origin pile and whatever pile the cursor is over at release.
//!   On rejection, the drag cards snap back to their origin via a
//!   `StateChangedEvent` re-sync.

use bevy::input::ButtonInput;
use bevy::math::{Vec2, Vec3};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use solitaire_core::card::Suit;
use solitaire_core::game_state::GameState;
use solitaire_core::pile::PileType;
use solitaire_core::rules::{can_place_on_foundation, can_place_on_tableau};

use crate::card_plugin::{CardEntity, TABLEAU_FAN_FRAC};
use crate::challenge_plugin::CHALLENGE_UNLOCK_LEVEL;
use crate::events::{
    DrawRequestEvent, InfoToastEvent, MoveRejectedEvent, MoveRequestEvent, NewGameConfirmEvent,
    NewGameRequestEvent, StateChangedEvent, UndoRequestEvent,
};
use crate::game_plugin::GameMutation;
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
            .add_systems(
                Update,
                (
                    handle_keyboard,
                    handle_stock_click,
                    start_drag,
                    follow_drag,
                    end_drag.before(GameMutation),
                )
                    .chain(),
            );
    }
}

/// Seconds after the first N press during which a second N confirms new game.
const NEW_GAME_CONFIRM_WINDOW: f32 = 3.0;

#[allow(clippy::too_many_arguments)]
fn handle_keyboard(
    keys: Res<ButtonInput<KeyCode>>,
    progress: Option<Res<ProgressResource>>,
    game: Option<Res<crate::resources::GameStateResource>>,
    time: Res<Time>,
    mut confirm_countdown: Local<f32>,
    mut undo: EventWriter<UndoRequestEvent>,
    mut new_game: EventWriter<NewGameRequestEvent>,
    mut confirm_event: EventWriter<NewGameConfirmEvent>,
    mut info_toast: EventWriter<InfoToastEvent>,
    mut draw: EventWriter<DrawRequestEvent>,
) {
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
        if !active_game {
            // No active game — start immediately.
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
    if keys.just_pressed(KeyCode::KeyD) {
        draw.send(DrawRequestEvent);
    }
    // Esc is handled by `PausePlugin` (overlay toggle + paused flag).
}

fn handle_stock_click(
    buttons: Res<ButtonInput<MouseButton>>,
    drag: Res<DragState>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Camera, &GlobalTransform)>,
    layout: Option<Res<LayoutResource>>,
    mut draw: EventWriter<DrawRequestEvent>,
) {
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

fn start_drag(
    buttons: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Camera, &GlobalTransform)>,
    layout: Option<Res<LayoutResource>>,
    game: Res<GameStateResource>,
    mut drag: ResMut<DragState>,
    mut card_transforms: Query<(&CardEntity, &mut Transform)>,
) {
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
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Camera, &GlobalTransform)>,
    layout: Option<Res<LayoutResource>>,
    game: Res<GameStateResource>,
    mut drag: ResMut<DragState>,
    mut moves: EventWriter<MoveRequestEvent>,
    mut rejected: EventWriter<MoveRejectedEvent>,
    mut changed: EventWriter<StateChangedEvent>,
) {
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
    } else {
        let _ = game;
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
}

// `Vec3` is referenced only via the `DRAG_Z` constant; keep the import silenced
// when the compiler can't see it used.
#[allow(dead_code)]
const _VEC3_REFERENCED: Option<Vec3> = None;
