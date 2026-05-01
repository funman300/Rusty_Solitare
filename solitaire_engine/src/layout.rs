//! Pure layout calculation — maps a window size to card size and pile positions.
//!
//! Bevy 2D uses a center-origin coordinate system: `(0, 0)` is the window
//! center, `+y` is up, `+x` is right.

use std::collections::HashMap;

use bevy::math::Vec2;
use bevy::prelude::{Resource, SystemSet};
use solitaire_core::card::Suit;
use solitaire_core::pile::PileType;

/// Schedule labels for layout-related systems so cross-plugin ordering is
/// explicit instead of relying on Bevy's automatic resource-conflict ordering
/// (which only forces non-parallel execution, not a particular order).
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LayoutSystem {
    /// The system that updates [`LayoutResource`], the table background, and
    /// pile markers in response to a `WindowResized` event. Card-snap systems
    /// (in `card_plugin`) run `.after(LayoutSystem::UpdateOnResize)` so they
    /// see the fresh layout.
    UpdateOnResize,
}

/// Minimum supported window dimensions. Layout is still computed below this
/// size but cards will be small.
pub const MIN_WINDOW: Vec2 = Vec2::new(800.0, 600.0);

/// Aspect ratio (height / width) of a standard playing card.
const CARD_ASPECT: f32 = 1.4;

/// Fraction of card height used as vertical padding between the top row and
/// the tableau row.
const VERTICAL_GAP_FRAC: f32 = 0.2;

/// Fraction of card height contributed by each additional face-up tableau card
/// when fanned. Mirrors `card_plugin::TABLEAU_FAN_FRAC` so layout sizing can
/// solve for a worst-case column without depending on `card_plugin`.
const TABLEAU_FAN_FRAC: f32 = 0.25;

/// Largest possible face-up tableau column in Klondike: a King down to an Ace
/// after every face-down card has flipped on column 7. Layout sizing must keep
/// this column inside the visible window.
const MAX_TABLEAU_CARDS: f32 = 13.0;

/// Vertical pixel band reserved at the top of the play area for the HUD
/// (action buttons, Score / Moves / Timer readouts). The card grid starts
/// below this band so the HUD doesn't bleed into the play surface.
///
/// 64 px comfortably fits the action button bar (~32 px tall) plus the
/// Score/Moves text line plus padding, with a few pixels of breathing room.
/// The matching translucent background is painted by `hud_plugin::spawn_hud_band`.
pub const HUD_BAND_HEIGHT: f32 = 64.0;

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
/// - `card_width` is the smaller of:
///   - `window.x / 9.0` — seven tableau columns with eight gaps (two outer
///     margins + six inner). This is the limiter on landscape windows.
///   - the height-based candidate that keeps a worst-case fanned tableau
///     column (13 face-up cards, see [`MAX_TABLEAU_CARDS`]) inside the
///     window with a bottom margin equal to `h_gap`. Limiter on tall/narrow
///     windows.
/// - `card_height = card_width * 1.4`.
/// - Horizontal gap `h_gap = card_width / 4.0`.
/// - Top row (stock, waste, 4 foundations) aligns with tableau columns
///   0, 1, 3, 4, 5, 6 — column 2 is intentionally empty to separate the
///   waste/stock cluster from the foundations.
pub fn compute_layout(window: Vec2) -> Layout {
    let window = window.max(MIN_WINDOW);

    // Width-based candidate (existing behaviour): 7 cards + 8 h_gaps = 9*card_width.
    let card_width_width_based = window.x / 9.0;

    // Height-based candidate. The vertical budget below the top row must hold
    // a worst-case fanned tableau column plus a bottom margin equal to h_gap.
    //
    // Letting w = card_width and h = w * CARD_ASPECT, the vertical layout is:
    //   top edge of window     = +window.y / 2
    //   top of top-row card    = window.y/2 - HUD_BAND_HEIGHT - h_gap     (HUD reserve + h_gap top margin)
    //   centre of top-row card = window.y/2 - HUD_BAND_HEIGHT - h_gap - h/2
    //   centre of tableau card = top centre - h - vertical_gap            (vertical_gap = VERTICAL_GAP_FRAC * h)
    //   bottom of last fanned  = tableau_centre + h/2 - fan_factor * h
    //                           where fan_factor = 1 + (MAX_TABLEAU_CARDS - 1) * TABLEAU_FAN_FRAC
    //   bottom of window       = -window.y / 2; require bottom-of-fanned >= -window.y/2 + h_gap
    //
    // Substituting h_gap = w/4 and h = CARD_ASPECT * w and solving for the
    // largest w that fits gives:
    //   (window.y - HUD_BAND_HEIGHT) = w * (0.5 + (1 + fan_factor + VERTICAL_GAP_FRAC) * CARD_ASPECT)
    let fan_factor = 1.0 + (MAX_TABLEAU_CARDS - 1.0) * TABLEAU_FAN_FRAC;
    let height_denom = 0.5 + (1.0 + fan_factor + VERTICAL_GAP_FRAC) * CARD_ASPECT;
    let card_width_height_based = (window.y - HUD_BAND_HEIGHT).max(0.0) / height_denom;

    let card_width = card_width_width_based.min(card_width_height_based);
    let card_height = card_width * CARD_ASPECT;
    let card_size = Vec2::new(card_width, card_height);

    let h_gap = card_width / 4.0;
    // Total occupied width = 7*card_width + 8*h_gap = 9*card_width. When card
    // sizing is height-limited (tall/narrow windows), this is smaller than
    // window.x, so the grid is centred horizontally; otherwise side_margin
    // collapses to h_gap and the geometry matches the original width-based
    // layout exactly.
    let total_grid_width = 9.0 * card_width;
    let side_margin = (window.x - total_grid_width) / 2.0 + h_gap;
    let left_edge = -window.x / 2.0;
    let col_x = |col: usize| -> f32 {
        left_edge + side_margin + card_width / 2.0 + (col as f32) * (card_width + h_gap)
    };

    let vertical_gap = card_height * VERTICAL_GAP_FRAC;
    let top_y = window.y / 2.0 - HUD_BAND_HEIGHT - h_gap - card_height / 2.0;
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

    /// HUD band reservation: the top edge of every top-row card must sit
    /// at least `HUD_BAND_HEIGHT` pixels below the top of the window so
    /// the action button bar / score readout has its own visual band
    /// instead of bleeding into the play surface.
    #[test]
    fn top_row_clears_hud_band() {
        let window = Vec2::new(1280.0, 800.0);
        let layout = compute_layout(window);
        let stock_y = layout.pile_positions[&PileType::Stock].y;
        let card_top = stock_y + layout.card_size.y / 2.0;
        let band_bottom = window.y / 2.0 - HUD_BAND_HEIGHT;
        assert!(
            card_top <= band_bottom,
            "top of stock card ({card_top}) must sit below the HUD band ({band_bottom})",
        );
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
    fn short_wide_window_constrains_card_width_via_height() {
        // Short wide window: vertical budget is the bottleneck, so card_width
        // must be strictly smaller than the naive window.x / 9 candidate to
        // keep a worst-case 13-card column inside the window. (Most desktop
        // monitors fall into this regime — e.g. 1280x800, 1920x1080.)
        let window = Vec2::new(2560.0, 1080.0);
        let layout = compute_layout(window);
        let width_based = window.x / 9.0;
        assert!(
            layout.card_size.x < width_based,
            "expected height to be the limiter (card_width {} should be < width-based candidate {})",
            layout.card_size.x,
            width_based
        );
    }

    #[test]
    fn tall_narrow_window_keeps_width_based_sizing() {
        // Tall narrow window: there's plenty of vertical budget, so width is
        // the bottleneck and card_width matches the legacy window.x / 9
        // derivation exactly.
        let window = Vec2::new(900.0, 1600.0);
        let layout = compute_layout(window);
        let width_based = window.x / 9.0;
        assert!(
            (layout.card_size.x - width_based).abs() < 1e-3,
            "expected width-based sizing (card_width {} should equal {})",
            layout.card_size.x,
            width_based
        );
    }

    #[test]
    fn worst_case_tableau_fits_vertically_on_default_resolution() {
        // Default app resolution (see solitaire_app/src/main.rs).
        let window = Vec2::new(1280.0, 800.0);
        let layout = compute_layout(window);
        let tableau_y = layout.pile_positions[&PileType::Tableau(6)].y;
        let card_h = layout.card_size.y;
        // Bottom edge of the 13th fanned face-up card.
        let bottom_edge = tableau_y - 12.0 * card_h * TABLEAU_FAN_FRAC - card_h / 2.0;
        // Bottom of the visible window with the same h_gap-sized margin used at
        // the top.
        let h_gap = layout.card_size.x / 4.0;
        let window_bottom_with_margin = -window.y / 2.0 + h_gap;
        assert!(
            bottom_edge >= window_bottom_with_margin - 1e-3,
            "worst-case tableau bottom {bottom_edge} overflows window margin {window_bottom_with_margin}"
        );
    }

    #[test]
    fn worst_case_tableau_fits_vertically_on_full_hd() {
        // The bug originally reproduced at 1920x1080. Lock in a regression test.
        let window = Vec2::new(1920.0, 1080.0);
        let layout = compute_layout(window);
        let tableau_y = layout.pile_positions[&PileType::Tableau(6)].y;
        let card_h = layout.card_size.y;
        let bottom_edge = tableau_y - 12.0 * card_h * TABLEAU_FAN_FRAC - card_h / 2.0;
        let h_gap = layout.card_size.x / 4.0;
        let window_bottom_with_margin = -window.y / 2.0 + h_gap;
        assert!(
            bottom_edge >= window_bottom_with_margin - 1e-3,
            "worst-case tableau bottom {bottom_edge} overflows window margin {window_bottom_with_margin}"
        );
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
