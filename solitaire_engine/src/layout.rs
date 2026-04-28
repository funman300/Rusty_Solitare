//! Pure layout calculation — maps a window size to card size and pile positions.
//!
//! Bevy 2D uses a center-origin coordinate system: `(0, 0)` is the window
//! center, `+y` is up, `+x` is right.

use std::collections::HashMap;

use bevy::math::Vec2;
use bevy::prelude::Resource;
use solitaire_core::card::Suit;
use solitaire_core::pile::PileType;

/// Minimum supported window dimensions. Layout is still computed below this
/// size but cards will be small.
pub const MIN_WINDOW: Vec2 = Vec2::new(800.0, 600.0);

/// Aspect ratio (height / width) of a standard playing card.
const CARD_ASPECT: f32 = 1.4;

/// Fraction of card height used as vertical padding between the top row and
/// the tableau row.
const VERTICAL_GAP_FRAC: f32 = 0.2;

/// Table background colour (dark green felt).
pub const TABLE_COLOUR: [f32; 3] = [0.059, 0.322, 0.196];

/// Computed board layout for a given window size.
#[derive(Debug, Clone)]
pub struct Layout {
    /// Width and height of a single card, in world units (Bevy 2D world-space).
    ///
    /// `x` is the card width; `y` is the card height (always `x * 1.4`).
    /// All pile positions and fan offsets are derived from this value.
    pub card_size: Vec2,
    /// Centre position of each pile, in 2D world coordinates.
    ///
    /// World origin `(0, 0)` is the window centre; `+x` is right, `+y` is up.
    /// Every `PileType` (Stock, Waste, four Foundations, seven Tableaux) has an
    /// entry. The map always contains exactly 13 entries after `compute_layout`.
    pub pile_positions: HashMap<PileType, Vec2>,
}

/// Compute the board layout from a window size.
///
/// # Geometry
/// - `card_width  = window.x / 9.0` — seven tableau columns with eight gaps
///   (two outer margins + six inner).
/// - `card_height = card_width * 1.4`.
/// - Horizontal gap `h_gap = card_width / 4.0`.
/// - Top row (stock, waste, 4 foundations) aligns with tableau columns
///   0, 1, 3, 4, 5, 6 — column 2 is intentionally empty to separate the
///   waste/stock cluster from the foundations.
pub fn compute_layout(window: Vec2) -> Layout {
    let window = window.max(MIN_WINDOW);

    let card_width = window.x / 9.0;
    let card_height = card_width * CARD_ASPECT;
    let card_size = Vec2::new(card_width, card_height);

    let h_gap = card_width / 4.0;
    // With h_gap = card_width/4, total width = 7*card_width + 8*h_gap = 9*card_width.
    // Leftmost card's centre sits at: -window.x/2 + h_gap + card_width/2.
    let left_edge = -window.x / 2.0;
    let col_x = |col: usize| -> f32 {
        left_edge + h_gap + card_width / 2.0 + (col as f32) * (card_width + h_gap)
    };

    let vertical_gap = card_height * VERTICAL_GAP_FRAC;
    let top_y = window.y / 2.0 - h_gap - card_height / 2.0;
    let tableau_y = top_y - card_height - vertical_gap;

    let mut pile_positions: HashMap<PileType, Vec2> = HashMap::with_capacity(13);

    pile_positions.insert(PileType::Stock, Vec2::new(col_x(0), top_y));
    pile_positions.insert(PileType::Waste, Vec2::new(col_x(1), top_y));

    // Column 2 is skipped — visual separation between waste and foundations.
    let foundation_suits = [Suit::Clubs, Suit::Diamonds, Suit::Hearts, Suit::Spades];
    for (i, suit) in foundation_suits.into_iter().enumerate() {
        pile_positions.insert(
            PileType::Foundation(suit),
            Vec2::new(col_x(3 + i), top_y),
        );
    }

    for i in 0..7 {
        pile_positions.insert(PileType::Tableau(i), Vec2::new(col_x(i), tableau_y));
    }

    Layout {
        card_size,
        pile_positions,
    }
}

/// Bevy resource wrapping the current `Layout`. Recomputed on `WindowResized`.
#[derive(Resource, Debug, Clone)]
pub struct LayoutResource(pub Layout);

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_all_piles_present(layout: &Layout) {
        assert!(layout.pile_positions.contains_key(&PileType::Stock));
        assert!(layout.pile_positions.contains_key(&PileType::Waste));
        for suit in [Suit::Clubs, Suit::Diamonds, Suit::Hearts, Suit::Spades] {
            assert!(
                layout.pile_positions.contains_key(&PileType::Foundation(suit)),
                "missing foundation for {:?}",
                suit
            );
        }
        for i in 0..7 {
            assert!(
                layout.pile_positions.contains_key(&PileType::Tableau(i)),
                "missing tableau {i}"
            );
        }
        assert_eq!(layout.pile_positions.len(), 13);
    }

    #[test]
    fn layout_has_all_thirteen_piles() {
        assert_all_piles_present(&compute_layout(Vec2::new(1280.0, 800.0)));
        assert_all_piles_present(&compute_layout(Vec2::new(800.0, 600.0)));
        assert_all_piles_present(&compute_layout(Vec2::new(1920.0, 1080.0)));
    }

    #[test]
    fn card_size_scales_with_window_width() {
        let small = compute_layout(Vec2::new(800.0, 600.0));
        let large = compute_layout(Vec2::new(1920.0, 1080.0));
        assert!(large.card_size.x > small.card_size.x);
        assert!(
            (large.card_size.y / large.card_size.x - CARD_ASPECT).abs() < 1e-5,
            "card aspect ratio should be preserved",
        );
    }

    #[test]
    fn layout_below_minimum_clamps_to_minimum() {
        let below = compute_layout(Vec2::new(400.0, 300.0));
        let at_min = compute_layout(MIN_WINDOW);
        assert_eq!(below.card_size, at_min.card_size);
    }

    #[test]
    fn tableau_columns_are_sorted_left_to_right() {
        let layout = compute_layout(Vec2::new(1280.0, 800.0));
        for i in 0..6 {
            let lhs = layout.pile_positions[&PileType::Tableau(i)].x;
            let rhs = layout.pile_positions[&PileType::Tableau(i + 1)].x;
            assert!(lhs < rhs, "tableau {i} should be left of tableau {}", i + 1);
        }
    }

    #[test]
    fn top_row_is_above_tableau_row() {
        let layout = compute_layout(Vec2::new(1280.0, 800.0));
        let stock_y = layout.pile_positions[&PileType::Stock].y;
        let tableau_y = layout.pile_positions[&PileType::Tableau(0)].y;
        assert!(stock_y > tableau_y);
    }

    #[test]
    fn stock_aligns_with_tableau_col_0_and_waste_with_col_1() {
        let layout = compute_layout(Vec2::new(1280.0, 800.0));
        let stock_x = layout.pile_positions[&PileType::Stock].x;
        let waste_x = layout.pile_positions[&PileType::Waste].x;
        let t0_x = layout.pile_positions[&PileType::Tableau(0)].x;
        let t1_x = layout.pile_positions[&PileType::Tableau(1)].x;
        assert!((stock_x - t0_x).abs() < 1e-5);
        assert!((waste_x - t1_x).abs() < 1e-5);
    }

    #[test]
    fn foundations_align_with_tableau_cols_3_to_6() {
        let layout = compute_layout(Vec2::new(1280.0, 800.0));
        let foundation_suits = [Suit::Clubs, Suit::Diamonds, Suit::Hearts, Suit::Spades];
        for (i, suit) in foundation_suits.into_iter().enumerate() {
            let f_x = layout.pile_positions[&PileType::Foundation(suit)].x;
            let t_x = layout.pile_positions[&PileType::Tableau(3 + i)].x;
            assert!(
                (f_x - t_x).abs() < 1e-5,
                "foundation {:?} should align with tableau {}",
                suit,
                3 + i
            );
        }
    }

    #[test]
    fn all_piles_fit_inside_window_horizontally() {
        for window in [
            Vec2::new(800.0, 600.0),
            Vec2::new(1280.0, 800.0),
            Vec2::new(1920.0, 1080.0),
        ] {
            let layout = compute_layout(window);
            let half_w = window.x / 2.0;
            let half_card = layout.card_size.x / 2.0;
            for (pile, pos) in &layout.pile_positions {
                assert!(
                    pos.x - half_card >= -half_w - 1e-3,
                    "{:?} overflows left at window {:?}",
                    pile,
                    window
                );
                assert!(
                    pos.x + half_card <= half_w + 1e-3,
                    "{:?} overflows right at window {:?}",
                    pile,
                    window
                );
            }
        }
    }
}
