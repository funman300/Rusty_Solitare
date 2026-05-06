//! Cursor-icon feedback (#31) and drag drop-target highlighting (#32).
//!
//! **Cursor icons** (`update_cursor_icon`)
//! - Cards are being dragged → `Grabbing` (closed hand)
//! - A UI `Button` entity is hovered (and no drag in progress) → `Pointer`
//!   (the hand-with-extended-index-finger icon). This telegraphs
//!   clickability for every modal button, HUD action, mode-launcher
//!   card, settings toggle, etc.
//! - Cursor hovers over a face-up draggable card → `Grab` (open hand)
//! - Otherwise → `Default` (arrow)
//!
//! Priority order: dragging > button-hover > card-hover > default. A
//! button-overlapping-a-card edge case favours `Pointer` because UI
//! elements take precedence over world-space cards; in practice
//! buttons are always on UI nodes and cards are sprites, so they
//! cannot occupy the same hit region simultaneously.
//!
//! **Drop-target highlights** (`update_drop_highlights`)
//! While a drag is in progress every `PileMarker` sprite is tinted:
//! - **Green** if the dragged stack can legally land there.
//! - **Default** (nearly transparent white) otherwise.
//!   The tint is cleared to default the frame the drag ends.
//!
//! **Drop-target overlays** (`update_drop_target_overlays`)
//! Pile markers sit *behind* the card stack, so on a tableau column with
//! any cards on it the green tint applied above is fully occluded. To
//! make legal targets unmistakable mid-drag, this system spawns a
//! translucent green rectangle plus four outline edges over every legal
//! destination pile. For tableau columns the overlay covers the full
//! visible fan (matching `input_plugin::pile_drop_rect`); for
//! foundations and empty tableaux it is card-sized. Overlays are
//! despawned the frame the drag ends or whenever the legal-target set
//! changes.

use bevy::prelude::*;
use bevy::window::{CursorIcon, PrimaryWindow, SystemCursorIcon};
use solitaire_core::game_state::{DrawMode, GameState};
use solitaire_core::pile::PileType;
use solitaire_core::rules::{can_place_on_foundation, can_place_on_tableau};

use crate::card_plugin::{RightClickHighlight, TABLEAU_FAN_FRAC};
use crate::layout::{Layout, LayoutResource};
use crate::resources::{DragState, GameStateResource};
use crate::table_plugin::PileMarker;
use crate::ui_theme::{
    DROP_TARGET_FILL, DROP_TARGET_OUTLINE, DROP_TARGET_OUTLINE_PX, Z_DROP_OVERLAY,
};

/// Semi-transparent white that `table_plugin` uses for idle pile markers.
/// Kept in sync with the `marker_colour` constant there.
const MARKER_DEFAULT: Color = Color::srgba(1.0, 1.0, 1.0, 0.08);

/// Green tint applied to pile markers that are valid drop targets during drag.
const MARKER_VALID: Color = Color::srgba(0.15, 0.85, 0.25, 0.55);

/// Marker component on a parent entity that owns one drop-target overlay
/// (a translucent fill plus four outline edges as children). The wrapped
/// `PileType` identifies which pile this overlay highlights, so test
/// queries and the despawn-on-target-change logic can filter by pile.
#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct DropTargetOverlay(pub PileType);

/// Renders a custom cursor sprite that follows the pointer and swaps to a grab-hand icon while a card drag is in progress.
pub struct CursorPlugin;

impl Plugin for CursorPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                update_cursor_icon,
                update_drop_highlights,
                update_drop_target_overlays,
            ),
        );
    }
}

// ---------------------------------------------------------------------------
// #31 — Cursor icon
// ---------------------------------------------------------------------------

/// Pure decision function for the cursor icon, separated from the Bevy
/// system so it can be unit-tested without `PrimaryWindow` /
/// `Camera` / `Time` plumbing.
///
/// Priority order (highest first):
/// 1. `is_dragging` → `Grabbing`
/// 2. `any_button_hovered` → `Pointer`
/// 3. `any_card_hovered` → `Grab`
/// 4. otherwise → `Default`
fn pick_cursor_icon(
    is_dragging: bool,
    any_button_hovered: bool,
    any_card_hovered: bool,
) -> SystemCursorIcon {
    if is_dragging {
        SystemCursorIcon::Grabbing
    } else if any_button_hovered {
        SystemCursorIcon::Pointer
    } else if any_card_hovered {
        SystemCursorIcon::Grab
    } else {
        SystemCursorIcon::Default
    }
}

/// Updates the primary-window cursor icon based on drag state and hover.
fn update_cursor_icon(
    drag: Res<DragState>,
    windows: Query<(Entity, &Window), With<PrimaryWindow>>,
    cameras: Query<(&Camera, &GlobalTransform)>,
    layout: Option<Res<LayoutResource>>,
    game: Option<Res<GameStateResource>>,
    button_q: Query<&Interaction, With<Button>>,
    mut commands: Commands,
) {
    let Ok((win_entity, window)) = windows.single() else { return };

    let is_dragging = !drag.is_idle();

    // A UI button is "hovered" if any `Button` entity has its
    // `Interaction` set to `Hovered` or `Pressed`. We include
    // `Pressed` so the pointer icon stays visible while a click is
    // being held, matching browser behaviour.
    let any_button_hovered = button_q
        .iter()
        .any(|i| matches!(i, Interaction::Hovered | Interaction::Pressed));

    let any_card_hovered = if is_dragging || any_button_hovered {
        // No need to do the world-space hit test when a higher
        // priority branch already wins.
        false
    } else {
        (|| {
            let cursor = window.cursor_position()?;
            let (camera, cam_xf) = cameras.single().ok()?;
            let world = camera.viewport_to_world_2d(cam_xf, cursor).ok()?;
            let layout = layout.as_ref()?.0.clone();
            let game = game.as_ref()?;
            Some(cursor_over_draggable(world, &game.0, &layout))
        })()
        .unwrap_or(false)
    };

    let icon = pick_cursor_icon(is_dragging, any_button_hovered, any_card_hovered);
    commands.entity(win_entity).insert(CursorIcon::from(icon));
}

/// Returns `true` if `cursor` (world-space) is over any face-up draggable card.
fn cursor_over_draggable(cursor: Vec2, game: &GameState, layout: &Layout) -> bool {
    let piles = [
        PileType::Waste,
        PileType::Foundation(0),
        PileType::Foundation(1),
        PileType::Foundation(2),
        PileType::Foundation(3),
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
        let is_tableau = matches!(pile, PileType::Tableau(_));
        let base = layout.pile_positions[&pile];

        for (i, card) in pile_cards.cards.iter().enumerate().rev() {
            if !card.face_up {
                continue;
            }
            // Only the topmost card is draggable on non-tableau piles.
            if !is_tableau && i != pile_cards.cards.len() - 1 {
                continue;
            }
            let pos = tableau_or_stack_pos(game, layout, &pile, i, base, is_tableau);
            if point_in_rect(cursor, pos, layout.card_size) {
                return true;
            }
        }
    }
    false
}

// ---------------------------------------------------------------------------
// #32 — Drop-target highlighting
// ---------------------------------------------------------------------------

/// Tints pile-marker sprites green when they are valid drag destinations,
/// and restores the default colour when no drag is active.
/// Markers tagged with `RightClickHighlight` are skipped during the idle reset
/// so the right-click legal-destination highlight remains visible.
fn update_drop_highlights(
    drag: Res<DragState>,
    game: Option<Res<GameStateResource>>,
    mut markers: Query<(&PileMarker, &mut Sprite, Option<&RightClickHighlight>)>,
) {
    if drag.is_idle() {
        // Drag ended — restore markers that are not right-click-highlighted.
        for (_, mut sprite, rch) in &mut markers {
            if rch.is_none() {
                sprite.color = MARKER_DEFAULT;
            }
        }
        return;
    }

    let Some(game) = game else { return };

    // The first element of drag.cards is the bottom card that lands on the target.
    let Some(&bottom_id) = drag.cards.first() else { return };
    let bottom_card = game
        .0
        .piles
        .values()
        .flat_map(|p| p.cards.iter())
        .find(|c| c.id == bottom_id)
        .cloned();
    let Some(bottom_card) = bottom_card else { return };
    let drag_count = drag.cards.len();

    for (marker, mut sprite, _rch) in &mut markers {
        let valid = match &marker.0 {
            PileType::Foundation(slot) => {
                if drag_count != 1 {
                    false
                } else {
                    let pile = game.0.piles.get(&PileType::Foundation(*slot));
                    pile.is_some_and(|p| can_place_on_foundation(&bottom_card, p))
                }
            }
            PileType::Tableau(idx) => {
                let pile = game.0.piles.get(&PileType::Tableau(*idx));
                pile.is_some_and(|p| can_place_on_tableau(&bottom_card, p))
            }
            _ => false,
        };
        sprite.color = if valid { MARKER_VALID } else { MARKER_DEFAULT };
    }
}

// ---------------------------------------------------------------------------
// Drop-target overlay sprites — render in front of cards, unlike the pile
// markers above which sit behind the stack.
// ---------------------------------------------------------------------------

/// Spawns / despawns translucent overlay sprites over every legal drop
/// target while a drag is in progress.
///
/// The overlay is a parent `Sprite` (the soft fill) with four child
/// `Sprite`s (top, bottom, left, right edges) that together form the
/// outline. A new parent is spawned whenever a target appears in the
/// valid set; a parent is despawned (with its children) whenever its
/// pile leaves the valid set or the drag ends.
///
/// Geometry mirrors `input_plugin::pile_drop_rect` exactly so the
/// highlighted region matches the actual drop hit-box.
fn update_drop_target_overlays(
    mut commands: Commands,
    drag: Res<DragState>,
    game: Option<Res<GameStateResource>>,
    layout: Option<Res<LayoutResource>>,
    overlays: Query<(Entity, &DropTargetOverlay)>,
) {
    // Drag idle → despawn every existing overlay and exit.
    if drag.is_idle() {
        for (entity, _) in &overlays {
            commands.entity(entity).despawn();
        }
        return;
    }

    let (Some(game), Some(layout)) = (game, layout) else {
        return;
    };

    // Resolve the bottom card of the dragged stack — same logic as
    // `update_drop_highlights` so rules can't drift between the marker
    // tint and the overlay.
    let Some(&bottom_id) = drag.cards.first() else {
        return;
    };
    let bottom_card = game
        .0
        .piles
        .values()
        .flat_map(|p| p.cards.iter())
        .find(|c| c.id == bottom_id)
        .cloned();
    let Some(bottom_card) = bottom_card else {
        return;
    };
    let drag_count = drag.cards.len();

    // Iterate the same pile list as `update_drop_highlights`. Stock and
    // Waste are excluded because they are never legal drop targets.
    let candidates = [
        PileType::Foundation(0),
        PileType::Foundation(1),
        PileType::Foundation(2),
        PileType::Foundation(3),
        PileType::Tableau(0),
        PileType::Tableau(1),
        PileType::Tableau(2),
        PileType::Tableau(3),
        PileType::Tableau(4),
        PileType::Tableau(5),
        PileType::Tableau(6),
    ];

    // Compute the new set of valid piles for this frame.
    let mut valid: Vec<PileType> = Vec::new();
    for pile in &candidates {
        let is_valid = match pile {
            PileType::Foundation(_) => {
                if drag_count != 1 {
                    false
                } else {
                    game.0
                        .piles
                        .get(pile)
                        .is_some_and(|p| can_place_on_foundation(&bottom_card, p))
                }
            }
            PileType::Tableau(_) => game
                .0
                .piles
                .get(pile)
                .is_some_and(|p| can_place_on_tableau(&bottom_card, p)),
            _ => false,
        };
        // Don't highlight the origin pile — dropping onto the source is
        // a no-op.
        if is_valid && drag.origin_pile.as_ref() != Some(pile) {
            valid.push(pile.clone());
        }
    }

    // Despawn overlays whose pile is no longer valid.
    for (entity, marker) in &overlays {
        if !valid.contains(&marker.0) {
            commands.entity(entity).despawn();
        }
    }

    // Spawn overlays for piles that are now valid but don't yet have one.
    let already_overlaid: Vec<PileType> = overlays
        .iter()
        .map(|(_, m)| m.0.clone())
        .filter(|p| valid.contains(p))
        .collect();

    for pile in valid {
        if already_overlaid.contains(&pile) {
            continue;
        }
        spawn_drop_target_overlay(&mut commands, &pile, &layout.0, &game.0);
    }
}

/// Computes the `(centre, size)` of the drop-target overlay for a pile.
///
/// Mirrors `input_plugin::pile_drop_rect` — for tableau columns with two
/// or more cards the rectangle extends downward to cover the full fan;
/// for everything else it is card-sized. Replicated here rather than
/// imported because `pile_drop_rect` is private to `input_plugin` and
/// this overlay is the only other consumer.
fn drop_overlay_rect(pile: &PileType, layout: &Layout, game: &GameState) -> (Vec2, Vec2) {
    let centre = layout.pile_positions[pile];
    if matches!(pile, PileType::Tableau(_)) {
        let card_count = game.piles.get(pile).map_or(0, |p| p.cards.len());
        if card_count > 1 {
            let fan = -layout.card_size.y * TABLEAU_FAN_FRAC;
            let bottom_card_centre_y = centre.y + fan * (card_count - 1) as f32;
            let top_edge = centre.y + layout.card_size.y / 2.0;
            let bottom_edge = bottom_card_centre_y - layout.card_size.y / 2.0;
            let span_height = top_edge - bottom_edge;
            let new_centre_y = (top_edge + bottom_edge) / 2.0;
            return (
                Vec2::new(centre.x, new_centre_y),
                Vec2::new(layout.card_size.x, span_height),
            );
        }
    }
    (centre, layout.card_size)
}

/// Spawns one overlay parent (fill) plus four edge sprites (outline) at
/// the appropriate world position for `pile`.
fn spawn_drop_target_overlay(
    commands: &mut Commands,
    pile: &PileType,
    layout: &Layout,
    game: &GameState,
) {
    let (centre, size) = drop_overlay_rect(pile, layout, game);
    let edge = DROP_TARGET_OUTLINE_PX;

    commands
        .spawn((
            Sprite {
                color: DROP_TARGET_FILL,
                custom_size: Some(size),
                ..default()
            },
            Transform::from_xyz(centre.x, centre.y, Z_DROP_OVERLAY),
            DropTargetOverlay(pile.clone()),
        ))
        .with_children(|parent| {
            // Top edge.
            parent.spawn((
                Sprite {
                    color: DROP_TARGET_OUTLINE,
                    custom_size: Some(Vec2::new(size.x, edge)),
                    ..default()
                },
                Transform::from_xyz(0.0, size.y / 2.0 - edge / 2.0, 0.01),
            ));
            // Bottom edge.
            parent.spawn((
                Sprite {
                    color: DROP_TARGET_OUTLINE,
                    custom_size: Some(Vec2::new(size.x, edge)),
                    ..default()
                },
                Transform::from_xyz(0.0, -size.y / 2.0 + edge / 2.0, 0.01),
            ));
            // Left edge.
            parent.spawn((
                Sprite {
                    color: DROP_TARGET_OUTLINE,
                    custom_size: Some(Vec2::new(edge, size.y)),
                    ..default()
                },
                Transform::from_xyz(-size.x / 2.0 + edge / 2.0, 0.0, 0.01),
            ));
            // Right edge.
            parent.spawn((
                Sprite {
                    color: DROP_TARGET_OUTLINE,
                    custom_size: Some(Vec2::new(edge, size.y)),
                    ..default()
                },
                Transform::from_xyz(size.x / 2.0 - edge / 2.0, 0.0, 0.01),
            ));
        });
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn tableau_or_stack_pos(
    game: &GameState,
    layout: &Layout,
    pile: &PileType,
    index: usize,
    base: Vec2,
    is_tableau: bool,
) -> Vec2 {
    if is_tableau {
        Vec2::new(
            base.x,
            base.y - layout.card_size.y * TABLEAU_FAN_FRAC * (index as f32),
        )
    } else if matches!(pile, PileType::Waste) && game.draw_mode == DrawMode::DrawThree {
        let pile_len = game.piles.get(pile).map_or(0, |p| p.cards.len());
        let visible_start = pile_len.saturating_sub(3);
        let slot = index.saturating_sub(visible_start) as f32;
        Vec2::new(base.x + slot * layout.card_size.x * 0.28, base.y)
    } else {
        base
    }
}

fn point_in_rect(point: Vec2, center: Vec2, size: Vec2) -> bool {
    let half = size / 2.0;
    point.x >= center.x - half.x
        && point.x <= center.x + half.x
        && point.y >= center.y - half.y
        && point.y <= center.y + half.y
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn point_in_rect_center_is_inside() {
        assert!(point_in_rect(Vec2::ZERO, Vec2::ZERO, Vec2::new(10.0, 10.0)));
    }

    #[test]
    fn point_in_rect_edge_is_inside() {
        assert!(point_in_rect(
            Vec2::new(5.0, 5.0),
            Vec2::ZERO,
            Vec2::new(10.0, 10.0)
        ));
    }

    #[test]
    fn point_in_rect_outside() {
        assert!(!point_in_rect(
            Vec2::new(6.0, 0.0),
            Vec2::ZERO,
            Vec2::new(10.0, 10.0)
        ));
    }

    #[test]
    fn marker_valid_and_default_colours_are_distinct() {
        // Regression guard — ensure these constants haven't been accidentally
        // set to the same value.
        assert_ne!(
            format!("{MARKER_VALID:?}"),
            format!("{MARKER_DEFAULT:?}")
        );
    }

    // -----------------------------------------------------------------------
    // pick_cursor_icon priority-order tests
    // -----------------------------------------------------------------------

    #[test]
    fn cursor_picks_grabbing_when_dragging_overrides_button_hover() {
        // Dragging always wins regardless of button or card hover state.
        assert!(matches!(
            pick_cursor_icon(true, true, true),
            SystemCursorIcon::Grabbing
        ));
        assert!(matches!(
            pick_cursor_icon(true, false, false),
            SystemCursorIcon::Grabbing
        ));
    }

    #[test]
    fn cursor_picks_pointer_when_button_hovered_and_no_drag() {
        // Button hover beats card hover when not dragging.
        assert!(matches!(
            pick_cursor_icon(false, true, false),
            SystemCursorIcon::Pointer
        ));
        assert!(matches!(
            pick_cursor_icon(false, true, true),
            SystemCursorIcon::Pointer
        ));
    }

    #[test]
    fn cursor_picks_grab_when_card_hovered_and_no_button() {
        // Card hover wins only when no drag and no button-hover.
        assert!(matches!(
            pick_cursor_icon(false, false, true),
            SystemCursorIcon::Grab
        ));
    }

    #[test]
    fn cursor_picks_default_when_nothing_hovered() {
        assert!(matches!(
            pick_cursor_icon(false, false, false),
            SystemCursorIcon::Default
        ));
    }

    #[test]
    fn cursor_over_draggable_returns_false_for_empty_game() {
        use solitaire_core::game_state::{DrawMode, GameState};
        use crate::layout::compute_layout;

        let game = GameState::new(42, DrawMode::DrawOne);
        let layout = compute_layout(Vec2::new(1280.0, 800.0));
        // A cursor far off-screen should never hit anything.
        assert!(!cursor_over_draggable(Vec2::new(-9999.0, -9999.0), &game, &layout));
    }

    // -----------------------------------------------------------------------
    // Drop-target overlay tests
    // -----------------------------------------------------------------------

    use crate::layout::compute_layout;
    use solitaire_core::card::{Card, Rank, Suit};
    use solitaire_core::game_state::{DrawMode, GameMode, GameState};

    /// Builds an `App` with `MinimalPlugins` and the overlay system
    /// registered, plus the resources the system needs. Callers
    /// customise `GameStateResource` and `DragState` after construction.
    fn overlay_test_app(game: GameState) -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(GameStateResource(game))
            .insert_resource(LayoutResource(compute_layout(Vec2::new(1280.0, 800.0))))
            .insert_resource(DragState::default())
            .add_systems(Update, update_drop_target_overlays);
        app
    }

    /// Replaces the top card of a tableau pile with a fresh face-up
    /// card. Used to make a specific tableau column accept a chosen
    /// drag stack.
    fn set_tableau_top(game: &mut GameState, idx: usize, card: Card) {
        let pile = game
            .piles
            .get_mut(&PileType::Tableau(idx))
            .expect("tableau pile exists");
        pile.cards.clear();
        pile.cards.push(card);
    }

    /// Inserts a single face-up dragged card into the waste pile and
    /// configures `DragState` so the overlay system treats it as the
    /// active drag.
    fn begin_drag_with(app: &mut App, dragged: Card) {
        // Place the dragged card on the waste pile (origin).
        {
            let mut game = app.world_mut().resource_mut::<GameStateResource>();
            let waste = game
                .0
                .piles
                .get_mut(&PileType::Waste)
                .expect("waste pile exists");
            waste.cards.clear();
            waste.cards.push(dragged.clone());
        }
        let mut drag = app.world_mut().resource_mut::<DragState>();
        drag.cards = vec![dragged.id];
        drag.origin_pile = Some(PileType::Waste);
        drag.committed = true;
    }

    #[test]
    fn drop_target_overlay_spawns_for_valid_tableau_during_drag() {
        // 5 of Hearts (red, rank 5) on top of Tableau(2)'s 6 of Spades
        // (black, rank 6) — alternating colour, one rank lower → legal.
        let mut game = GameState::new_with_mode(7, DrawMode::DrawOne, GameMode::Classic);
        set_tableau_top(
            &mut game,
            2,
            Card { id: 9001, suit: Suit::Spades, rank: Rank::Six, face_up: true },
        );
        let dragged = Card { id: 9002, suit: Suit::Hearts, rank: Rank::Five, face_up: true };

        let mut app = overlay_test_app(game);
        begin_drag_with(&mut app, dragged);

        app.update();

        let overlays: Vec<PileType> = app
            .world_mut()
            .query::<&DropTargetOverlay>()
            .iter(app.world())
            .map(|o| o.0.clone())
            .collect();
        assert!(
            overlays.contains(&PileType::Tableau(2)),
            "expected Tableau(2) to be highlighted as a legal drop target, got {overlays:?}"
        );
    }

    #[test]
    fn drop_target_overlay_does_not_spawn_for_invalid_destination() {
        // 5 of Spades (black) onto Tableau(2)'s 6 of Clubs (also black)
        // — same colour family, illegal. Tableau(2) must NOT be
        // highlighted.
        let mut game = GameState::new_with_mode(7, DrawMode::DrawOne, GameMode::Classic);
        set_tableau_top(
            &mut game,
            2,
            Card { id: 9101, suit: Suit::Clubs, rank: Rank::Six, face_up: true },
        );
        let dragged = Card { id: 9102, suit: Suit::Spades, rank: Rank::Five, face_up: true };

        let mut app = overlay_test_app(game);
        begin_drag_with(&mut app, dragged);

        app.update();

        let overlays: Vec<PileType> = app
            .world_mut()
            .query::<&DropTargetOverlay>()
            .iter(app.world())
            .map(|o| o.0.clone())
            .collect();
        assert!(
            !overlays.contains(&PileType::Tableau(2)),
            "Tableau(2) must not be highlighted for an illegal drop, got {overlays:?}"
        );
    }

    #[test]
    fn drop_target_overlays_despawn_on_drag_end() {
        // Set up a scenario that produces at least one valid overlay,
        // confirm it spawns, then clear the drag and confirm every
        // overlay is despawned.
        let mut game = GameState::new_with_mode(7, DrawMode::DrawOne, GameMode::Classic);
        set_tableau_top(
            &mut game,
            2,
            Card { id: 9201, suit: Suit::Spades, rank: Rank::Six, face_up: true },
        );
        let dragged = Card { id: 9202, suit: Suit::Hearts, rank: Rank::Five, face_up: true };

        let mut app = overlay_test_app(game);
        begin_drag_with(&mut app, dragged);
        app.update();

        let count_during_drag = app
            .world_mut()
            .query::<&DropTargetOverlay>()
            .iter(app.world())
            .count();
        assert!(
            count_during_drag >= 1,
            "expected ≥1 overlay during drag, got {count_during_drag}"
        );

        // End the drag — every overlay should despawn next frame.
        app.world_mut().resource_mut::<DragState>().clear();
        app.update();

        let count_after_drag = app
            .world_mut()
            .query::<&DropTargetOverlay>()
            .iter(app.world())
            .count();
        assert_eq!(
            count_after_drag, 0,
            "all overlays must despawn when the drag ends"
        );
    }
}
