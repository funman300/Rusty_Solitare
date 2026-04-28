//! Cursor-icon feedback (#31) and drag drop-target highlighting (#32).
//!
//! **Cursor icons** (`update_cursor_icon`)
//! - Cards are being dragged → `Grabbing` (closed hand)
//! - Cursor hovers over a face-up draggable card → `Grab` (open hand)
//! - Otherwise → `Default` (arrow)
//!
//! **Drop-target highlights** (`update_drop_highlights`)
//! While a drag is in progress every `PileMarker` sprite is tinted:
//! - **Green** if the dragged stack can legally land there.
//! - **Default** (nearly transparent white) otherwise.
//!   The tint is cleared to default the frame the drag ends.

use bevy::prelude::*;
use bevy::window::{PrimaryWindow, SystemCursorIcon};
use bevy::winit::cursor::CursorIcon;
use solitaire_core::card::Suit;
use solitaire_core::game_state::{DrawMode, GameState};
use solitaire_core::pile::PileType;
use solitaire_core::rules::{can_place_on_foundation, can_place_on_tableau};

use crate::card_plugin::{RightClickHighlight, TABLEAU_FAN_FRAC};
use crate::layout::{Layout, LayoutResource};
use crate::resources::{DragState, GameStateResource};
use crate::table_plugin::PileMarker;

/// Semi-transparent white that `table_plugin` uses for idle pile markers.
/// Kept in sync with the `marker_colour` constant there.
const MARKER_DEFAULT: Color = Color::srgba(1.0, 1.0, 1.0, 0.08);

/// Green tint applied to pile markers that are valid drop targets during drag.
const MARKER_VALID: Color = Color::srgba(0.15, 0.85, 0.25, 0.55);

pub struct CursorPlugin;

impl Plugin for CursorPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (update_cursor_icon, update_drop_highlights));
    }
}

// ---------------------------------------------------------------------------
// #31 — Cursor icon
// ---------------------------------------------------------------------------

/// Updates the primary-window cursor icon based on drag state and hover.
fn update_cursor_icon(
    drag: Res<DragState>,
    windows: Query<(Entity, &Window), With<PrimaryWindow>>,
    cameras: Query<(&Camera, &GlobalTransform)>,
    layout: Option<Res<LayoutResource>>,
    game: Option<Res<GameStateResource>>,
    mut commands: Commands,
) {
    let Ok((win_entity, window)) = windows.get_single() else { return };

    if !drag.is_idle() {
        commands
            .entity(win_entity)
            .insert(CursorIcon::from(SystemCursorIcon::Grabbing));
        return;
    }

    let hovering = (|| {
        let cursor = window.cursor_position()?;
        let (camera, cam_xf) = cameras.get_single().ok()?;
        let world = camera.viewport_to_world_2d(cam_xf, cursor).ok()?;
        let layout = layout.as_ref()?.0.clone();
        let game = game.as_ref()?;
        Some(cursor_over_draggable(world, &game.0, &layout))
    })()
    .unwrap_or(false);

    commands.entity(win_entity).insert(CursorIcon::from(if hovering {
        SystemCursorIcon::Grab
    } else {
        SystemCursorIcon::Default
    }));
}

/// Returns `true` if `cursor` (world-space) is over any face-up draggable card.
fn cursor_over_draggable(cursor: Vec2, game: &GameState, layout: &Layout) -> bool {
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
            PileType::Foundation(suit) => {
                if drag_count != 1 {
                    false
                } else {
                    let pile = game.0.piles.get(&PileType::Foundation(*suit));
                    pile.is_some_and(|p| can_place_on_foundation(&bottom_card, p, *suit))
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
    use solitaire_core::card::{Card, Rank};

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

    #[test]
    fn cursor_over_draggable_returns_false_for_empty_game() {
        use solitaire_core::game_state::{DrawMode, GameState};
        use crate::layout::compute_layout;

        let game = GameState::new(42, DrawMode::DrawOne);
        let layout = compute_layout(Vec2::new(1280.0, 800.0));
        // A cursor far off-screen should never hit anything.
        assert!(!cursor_over_draggable(Vec2::new(-9999.0, -9999.0), &game, &layout));
    }
}
