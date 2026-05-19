//! Pure layout calculation — maps a window size to card size and pile positions.
//!
//! Bevy 2D uses a center-origin coordinate system: `(0, 0)` is the window
//! center, `+y` is up, `+x` is right.

use std::collections::HashMap;

use bevy::math::Vec2;
use bevy::prelude::{Resource, SystemSet};
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

/// Minimum window dimensions used as a layout floor.
///
/// `compute_layout` runs `window.max(MIN_WINDOW)` so a window smaller than this
/// on either axis is laid out as if it were at least this size. The floor
/// exists to guard against degenerate / divide-by-zero layouts on very small
/// surfaces (Bevy can briefly report 0-size windows during startup or after
/// minimisation on some compositors); it is not a "minimum supported playable
/// size" — desktop builds enforce that via `WindowResizeConstraints` set in
/// `solitaire_app::lib`.
///
/// The previous floor of 800×600 was set with desktop in mind and produced
/// the wrong behaviour on Android: a 360 dp phone got laid out as if it were
/// 800-wide, pushing the leftmost foundation past `-180` and the rightmost
/// tableau pile past `+180`, which clipped both at the visible viewport
/// edges (visible in the v0.22.3 hardware screenshot). 320×400 is below the
/// smallest reasonable phone (≈ 360×640) so every real device flows through
/// without clamping, while still being large enough that the layout math
/// produces non-degenerate card sizes.
pub const MIN_WINDOW: Vec2 = Vec2::new(320.0, 400.0);

/// Aspect ratio (height / width) of a standard playing card.
///
/// Matches the bundled hayeah/playing-cards-assets SVG dimensions
/// (167.087 × 242.667 → 1.4523). Pre-v0.11 the constant was 1.4,
/// which rendered the cards ~3.6 % squashed vertically.
const CARD_ASPECT: f32 = 1.4523;

/// Divisor used to derive the horizontal gap between columns from the card
/// width: `h_gap = card_width / H_GAP_DIVISOR`.
///
/// This constant also drives `card_width_width_based`:
///   total layout width = 7*card_width + 8*h_gap = card_width*(7 + 8/H_GAP_DIVISOR)
///   → card_width = window.x / (7 + 8/H_GAP_DIVISOR)
///
/// Desktop (H_GAP_DIVISOR = 4): card_width = window.x / 9  — existing behaviour.
/// Android (H_GAP_DIVISOR = 32): card_width = window.x / 7.25  — cards are ~10 %
///   wider than at divisor 8, with very tight gaps (~4 px) that are still visible
///   as a faint seam between columns. The primary readability boost on Android
///   comes from the `AndroidCornerLabel` overlay in `card_plugin`, but maximising
///   the physical card size helps too.
#[cfg(not(target_os = "android"))]
const H_GAP_DIVISOR: f32 = 4.0;
#[cfg(target_os = "android")]
const H_GAP_DIVISOR: f32 = 32.0;

/// Fraction of card height used as vertical padding between the top row and
/// the tableau row.
const VERTICAL_GAP_FRAC: f32 = 0.2;

/// Minimum fraction of card height used as vertical offset between face-up
/// tableau cards. Used for the height-based sizing candidate (worst-case
/// column must fit at this fraction). On desktop (height-limited) windows the
/// adaptive computation returns this value exactly; on portrait phones it
/// expands to fill available vertical space.
const TABLEAU_FAN_FRAC: f32 = 0.18;

/// Minimum fraction for face-down tableau cards. Scales proportionally with
/// the adaptive face-up fraction so hit-testing and rendering stay in sync.
///
/// Raised from 0.12 to 0.20 so face-down stacks on portrait phones show
/// enough of each card back to read as a meaningful stack rather than a
/// thin sliver. The ratio to TABLEAU_FAN_FRAC (0.80) is preserved by
/// the adaptive scaling in `compute_layout`.
const TABLEAU_FACEDOWN_FAN_FRAC: f32 = 0.14;

/// Largest possible face-up tableau column in Klondike: a King down to an Ace
/// after every face-down card has flipped on column 7. Layout sizing must keep
/// this column inside the visible window.
const MAX_TABLEAU_CARDS: f32 = 13.0;

/// Vertical pixel band reserved at the top of the play area for the HUD
/// (action buttons, Score / Moves / Timer readouts). The card grid starts
/// below this band so the HUD doesn't bleed into the play surface.
///
/// Desktop: 64 px fits the score/moves/time + mode badge rows.
/// Android: 112 px — the HUD column has 4 flex tiers with 3 inter-tier
/// gaps (4 px each) plus a SPACE_2 = 8 px top offset. With empty tiers
/// still contributing gap height in Bevy's flex layout, the actual HUD
/// height can reach ~80 px before the grid starts; 112 px gives ~28 px
/// of clearance between the HUD bottom and the top card edge, preventing
/// the overlap seen with the previous 80 px value.
#[cfg(not(target_os = "android"))]
pub const HUD_BAND_HEIGHT: f32 = 64.0;
#[cfg(target_os = "android")]
pub const HUD_BAND_HEIGHT: f32 = 112.0;

/// Height of the bottom action-bar (the row of ≡ ← || ? ! M + buttons).
///
/// The action bar sits *above* the OS gesture/navigation zone, so it is NOT
/// covered by `safe_area_bottom`. `compute_layout` adds this constant to
/// `safe_area_bottom` before computing the height-based card-size candidate
/// and the available tableau height, ensuring the deepest fanned column
/// never scrolls behind the button row.
///
/// Derivation (Android): `min_height 44 px` buttons
///   + `padding.top 8 px` + `padding.bottom 8 px` outer bar padding = **60 px**.
///
/// Desktop: no persistent bottom bar, so 0.
#[cfg(not(target_os = "android"))]
const BOTTOM_BAR_HEIGHT: f32 = 0.0;
#[cfg(target_os = "android")]
const BOTTOM_BAR_HEIGHT: f32 = 60.0;

/// Table background colour (dark green felt).
pub const TABLE_COLOUR: [f32; 3] = [0.059, 0.322, 0.196];

/// Computed board layout for a given window size.
#[derive(Debug, Clone)]
pub struct Layout {
    /// Width and height of a single card, in world units (Bevy 2D world-space).
    ///
    /// `x` is the card width; `y` is the card height (`x * CARD_ASPECT`).
    /// All pile positions and fan offsets are derived from this value.
    pub card_size: Vec2,
    /// Centre position of each pile, in 2D world coordinates.
    ///
    /// World origin `(0, 0)` is the window centre; `+x` is right, `+y` is up.
    /// Every `PileType` (Stock, Waste, four Foundations, seven Tableaux) has an
    /// entry. The map always contains exactly 13 entries after `compute_layout`.
    pub pile_positions: HashMap<PileType, Vec2>,
    /// Per-step vertical offset fraction for face-up tableau cards, as a
    /// fraction of `card_size.y`. On height-limited (desktop) windows this
    /// equals `TABLEAU_FAN_FRAC` (0.18); on width-limited (portrait phone)
    /// windows it expands to fill the available vertical space so the tableau
    /// stretches to the bottom of the screen. Card rendering (`card_plugin`)
    /// and hit testing (`input_plugin`) both read from this field so they
    /// stay in sync.
    pub tableau_fan_frac: f32,
    /// Per-step vertical offset fraction for face-down tableau cards, as a
    /// fraction of `card_size.y`. Scales proportionally with `tableau_fan_frac`
    /// (ratio preserved from `TABLEAU_FACEDOWN_FAN_FRAC / TABLEAU_FAN_FRAC`).
    pub tableau_facedown_fan_frac: f32,
    /// Vertical pixel budget available for tableau fan steps — the distance
    /// from the top edge of the first tableau card to the bottom margin, in
    /// logical pixels. Used by `card_plugin::update_tableau_fan_frac` to
    /// recompute `tableau_fan_frac` dynamically based on the actual max
    /// face-up column depth after each game state change.
    pub available_tableau_height: f32,
}

/// Compute the board layout from a window size and safe-area insets.
///
/// `safe_area_top` and `safe_area_bottom` are the **logical-pixel** heights of
/// the OS-reserved regions at the top and bottom of the screen (status bar and
/// gesture / navigation bar on Android). Pass `0.0` on desktop or when the
/// inset is unknown. Android's `WindowInsets` API returns **physical** pixels;
/// callers must divide by `window.scale_factor()` before passing values here.
///
/// # Geometry
/// - `card_width` is the smaller of:
///   - `window.x / 9.0` — seven tableau columns with eight gaps (two outer
///     margins + six inner). This is the limiter on landscape windows.
///   - the height-based candidate that keeps a worst-case fanned tableau
///     column (13 face-up cards, see [`MAX_TABLEAU_CARDS`]) inside the
///     window with a bottom margin equal to `h_gap`. Limiter on tall/narrow
///     windows.
/// - `card_height = card_width * CARD_ASPECT` (1.4523, matches the
///   bundled hayeah card art's natural SVG dimensions).
/// - Horizontal gap `h_gap = card_width / 4.0`.
/// - Top row (stock, waste, 4 foundations) aligns with tableau columns
///   0, 1, 3, 4, 5, 6 — column 2 is intentionally empty to separate the
///   waste/stock cluster from the foundations.
pub fn compute_layout(window: Vec2, safe_area_top: f32, safe_area_bottom: f32, hud_visible: bool) -> Layout {
    let window = window.max(MIN_WINDOW);
    let band_h = if hud_visible { HUD_BAND_HEIGHT } else { 0.0 };

    // Width-based candidate: 7 cards + 8 h_gaps where h_gap = card_width/H_GAP_DIVISOR.
    // Total = card_width*(7 + 8/H_GAP_DIVISOR) = window.x  →  card_width = window.x/card_width_divisor.
    let card_width_divisor = 7.0 + 8.0 / H_GAP_DIVISOR;
    let card_width_width_based = window.x / card_width_divisor;

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
    // Reserve space for both the OS gesture/nav bar and the app's own action
    // bar, which sits above it and is invisible to safe_area_bottom.
    let effective_safe_bottom = safe_area_bottom + BOTTOM_BAR_HEIGHT;

    let fan_factor = 1.0 + (MAX_TABLEAU_CARDS - 1.0) * TABLEAU_FAN_FRAC;
    let height_denom = 0.5 + (1.0 + fan_factor + VERTICAL_GAP_FRAC) * CARD_ASPECT;
    let card_width_height_based = (window.y - safe_area_top - effective_safe_bottom - band_h).max(0.0) / height_denom;

    let card_width = card_width_width_based.min(card_width_height_based);
    let card_height = card_width * CARD_ASPECT;
    let card_size = Vec2::new(card_width, card_height);

    let h_gap = card_width / H_GAP_DIVISOR;
    // Total occupied width = 7*card_width + 8*h_gap = card_width_divisor*card_width.
    // When card sizing is height-limited (tall/narrow windows) this is smaller than
    // window.x and the grid is centred horizontally; otherwise side_margin collapses
    // to h_gap and the geometry fills the window exactly.
    let total_grid_width = card_width_divisor * card_width;
    let side_margin = (window.x - total_grid_width) / 2.0 + h_gap;
    let left_edge = -window.x / 2.0;
    let col_x = |col: usize| -> f32 {
        left_edge + side_margin + card_width / 2.0 + (col as f32) * (card_width + h_gap)
    };

    let vertical_gap = card_height * VERTICAL_GAP_FRAC;
    let top_y = window.y / 2.0 - safe_area_top - band_h - h_gap - card_height / 2.0;
    let tableau_y = top_y - card_height - vertical_gap;

    let mut pile_positions: HashMap<PileType, Vec2> = HashMap::with_capacity(13);

    pile_positions.insert(PileType::Stock, Vec2::new(col_x(0), top_y));
    pile_positions.insert(PileType::Waste, Vec2::new(col_x(1), top_y));

    // Column 2 is skipped — visual separation between waste and foundations.
    for slot in 0..4_u8 {
        pile_positions.insert(
            PileType::Foundation(slot),
            Vec2::new(col_x(3 + slot as usize), top_y),
        );
    }

    for i in 0..7 {
        pile_positions.insert(PileType::Tableau(i), Vec2::new(col_x(i), tableau_y));
    }

    // Adaptive tableau fan fraction. On height-limited (desktop) windows the
    // height-based sizing already ensures a worst-case 13-card column fits at
    // TABLEAU_FAN_FRAC (0.25), so the formula returns ≈0.25 and the clamp
    // keeps it there — no change from prior behaviour. On width-limited
    // (portrait phone) windows card_size is small and lots of vertical space
    // is unused; we solve for the fraction that exactly fills the available
    // space to the bottom margin.
    //
    // avail = distance from the top of the first tableau card to the bottom
    //         margin — i.e. the space available for 12 fan steps.
    let avail = (tableau_y - (-window.y / 2.0 + effective_safe_bottom + h_gap) - card_height / 2.0).max(0.0);
    let ideal_fan_frac = if card_height > 0.0 {
        avail / ((MAX_TABLEAU_CARDS - 1.0) * card_height)
    } else {
        TABLEAU_FAN_FRAC
    };
    // Never go below the desktop minimum — avoids shrinking the fan on
    // degenerate near-square windows where the formula might undershoot.
    let tableau_fan_frac = ideal_fan_frac.max(TABLEAU_FAN_FRAC);
    // Scale the face-down fraction proportionally so rendering and hit-testing
    // stay in sync (TABLEAU_FACEDOWN_FAN_FRAC / TABLEAU_FAN_FRAC = 0.48 ratio).
    let facedown_scale = TABLEAU_FACEDOWN_FAN_FRAC / TABLEAU_FAN_FRAC;
    let tableau_facedown_fan_frac = tableau_fan_frac * facedown_scale;

    Layout {
        card_size,
        pile_positions,
        tableau_fan_frac,
        tableau_facedown_fan_frac,
        available_tableau_height: avail,
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
        for slot in 0..4_u8 {
            assert!(
                layout.pile_positions.contains_key(&PileType::Foundation(slot)),
                "missing foundation slot {slot}",
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
        assert_all_piles_present(&compute_layout(Vec2::new(1280.0, 800.0), 0.0, 0.0, true));
        assert_all_piles_present(&compute_layout(Vec2::new(800.0, 600.0), 0.0, 0.0, true));
        assert_all_piles_present(&compute_layout(Vec2::new(1920.0, 1080.0), 0.0, 0.0, true));
    }

    #[test]
    fn card_size_scales_with_window_width() {
        let small = compute_layout(Vec2::new(800.0, 600.0), 0.0, 0.0, true);
        let large = compute_layout(Vec2::new(1920.0, 1080.0), 0.0, 0.0, true);
        assert!(large.card_size.x > small.card_size.x);
        assert!(
            (large.card_size.y / large.card_size.x - CARD_ASPECT).abs() < 1e-5,
            "card aspect ratio should be preserved",
        );
    }

    #[test]
    fn layout_below_minimum_clamps_to_minimum() {
        // 200×200 sits below the floor on both axes, so the clamp pulls each
        // axis up to MIN_WINDOW and the layout matches compute_layout(MIN_WINDOW, 0.0, 0.0, true).
        let below = compute_layout(Vec2::new(200.0, 200.0), 0.0, 0.0, true);
        let at_min = compute_layout(MIN_WINDOW, 0.0, 0.0, true);
        assert_eq!(below.card_size, at_min.card_size);
    }

    /// Regression for the v0.22.3 Android viewport-overflow bug. A typical
    /// portrait-phone viewport (360 dp × 800 dp) must produce a layout
    /// where every pile fits horizontally — i.e. card_width is derived
    /// from the actual window, not a clamped-up desktop floor.
    #[test]
    fn phone_portrait_layout_fits_horizontally() {
        let window = Vec2::new(360.0, 800.0);
        let layout = compute_layout(window, 0.0, 0.0, true);
        let half_w = window.x / 2.0;
        let half_card = layout.card_size.x / 2.0;
        for (pile, pos) in &layout.pile_positions {
            assert!(
                pos.x - half_card >= -half_w - 1e-3,
                "{:?} overflows left at portrait phone window {:?}",
                pile,
                window
            );
            assert!(
                pos.x + half_card <= half_w + 1e-3,
                "{:?} overflows right at portrait phone window {:?}",
                pile,
                window
            );
        }
    }

    #[test]
    fn tableau_columns_are_sorted_left_to_right() {
        let layout = compute_layout(Vec2::new(1280.0, 800.0), 0.0, 0.0, true);
        for i in 0..6 {
            let lhs = layout.pile_positions[&PileType::Tableau(i)].x;
            let rhs = layout.pile_positions[&PileType::Tableau(i + 1)].x;
            assert!(lhs < rhs, "tableau {i} should be left of tableau {}", i + 1);
        }
    }

    #[test]
    fn top_row_is_above_tableau_row() {
        let layout = compute_layout(Vec2::new(1280.0, 800.0), 0.0, 0.0, true);
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
        let layout = compute_layout(window, 0.0, 0.0, true);
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
        let layout = compute_layout(Vec2::new(1280.0, 800.0), 0.0, 0.0, true);
        let stock_x = layout.pile_positions[&PileType::Stock].x;
        let waste_x = layout.pile_positions[&PileType::Waste].x;
        let t0_x = layout.pile_positions[&PileType::Tableau(0)].x;
        let t1_x = layout.pile_positions[&PileType::Tableau(1)].x;
        assert!((stock_x - t0_x).abs() < 1e-5);
        assert!((waste_x - t1_x).abs() < 1e-5);
    }

    #[test]
    fn foundations_align_with_tableau_cols_3_to_6() {
        let layout = compute_layout(Vec2::new(1280.0, 800.0), 0.0, 0.0, true);
        for slot in 0..4_u8 {
            let f_x = layout.pile_positions[&PileType::Foundation(slot)].x;
            let t_x = layout.pile_positions[&PileType::Tableau(3 + slot as usize)].x;
            assert!(
                (f_x - t_x).abs() < 1e-5,
                "foundation slot {slot} should align with tableau {}",
                3 + slot as usize,
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
        let layout = compute_layout(window, 0.0, 0.0, true);
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
        // the bottleneck and card_width matches window.x / (7 + 8/H_GAP_DIVISOR).
        let window = Vec2::new(900.0, 1600.0);
        let layout = compute_layout(window, 0.0, 0.0, true);
        let width_based = window.x / (7.0 + 8.0 / H_GAP_DIVISOR);
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
        let layout = compute_layout(window, 0.0, 0.0, true);
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
        let layout = compute_layout(window, 0.0, 0.0, true);
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

    /// Portrait phone (width-limited) should expand the fan fraction beyond
    /// the desktop minimum so the tableau fills the available vertical space.
    #[test]
    fn portrait_phone_expands_tableau_fan_frac() {
        let desktop = compute_layout(Vec2::new(1280.0, 800.0), 0.0, 0.0, true);
        let phone = compute_layout(Vec2::new(360.0, 800.0), 0.0, 0.0, true);
        assert!(
            phone.tableau_fan_frac > desktop.tableau_fan_frac,
            "portrait phone fan_frac ({:.3}) should exceed desktop ({:.3})",
            phone.tableau_fan_frac,
            desktop.tableau_fan_frac,
        );
    }

    /// The expanded fan on a portrait phone must not overflow the visible
    /// window — the worst-case 13-card column must stay above the bottom margin.
    #[test]
    fn expanded_fan_fits_phone_viewport() {
        let window = Vec2::new(360.0, 800.0);
        let layout = compute_layout(window, 0.0, 0.0, true);
        let tableau_y = layout.pile_positions[&PileType::Tableau(0)].y;
        let card_h = layout.card_size.y;
        let h_gap = layout.card_size.x / 4.0;
        // Bottom of the 13th (worst-case) fanned face-up card.
        let bottom = tableau_y - 12.0 * layout.tableau_fan_frac * card_h - card_h / 2.0;
        let margin = -window.y / 2.0 + h_gap;
        assert!(
            bottom >= margin - 1e-3,
            "worst-case fan overflows phone viewport: bottom={bottom:.1} < margin={margin:.1}",
        );
    }

    /// Desktop (height-limited) must keep the minimum fan fraction so the
    /// existing worst-case-fits-vertically invariant is preserved.
    #[test]
    fn desktop_tableau_fan_frac_is_minimum() {
        let layout = compute_layout(Vec2::new(1280.0, 800.0), 0.0, 0.0, true);
        assert!(
            (layout.tableau_fan_frac - TABLEAU_FAN_FRAC).abs() < 1e-3,
            "desktop fan_frac should stay at minimum {TABLEAU_FAN_FRAC}, got {:.4}",
            layout.tableau_fan_frac,
        );
    }

    #[test]
    fn all_piles_fit_inside_window_horizontally() {
        for window in [
            Vec2::new(800.0, 600.0),
            Vec2::new(1280.0, 800.0),
            Vec2::new(1920.0, 1080.0),
        ] {
            let layout = compute_layout(window, 0.0, 0.0, true);
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

    /// A non-zero `safe_area_top` must shift both the top row and the tableau
    /// downward by the same amount — so the first card row stays below the
    /// status-bar band and the tableau tracks it proportionally.
    #[test]
    fn safe_area_top_shifts_top_row_downward() {
        let window = Vec2::new(360.0, 800.0);
        let without = compute_layout(window, 0.0, 0.0, true);
        let with_inset = compute_layout(window, 32.0, 0.0, true);
        let stock_no_inset = without.pile_positions[&PileType::Stock].y;
        let stock_with_inset = with_inset.pile_positions[&PileType::Stock].y;
        assert!(
            stock_with_inset < stock_no_inset,
            "safe_area_top=32 must shift stock pile down (y decreased): {} → {}",
            stock_no_inset,
            stock_with_inset,
        );
        assert!(
            (stock_no_inset - stock_with_inset - 32.0).abs() < 1e-3,
            "stock pile must shift by exactly safe_area_top (32 dp): delta was {:.3}",
            stock_no_inset - stock_with_inset,
        );
    }

    /// With a safe-area inset the card grid must still fit horizontally —
    /// safe_area_top only affects the vertical budget.
    #[test]
    fn safe_area_top_does_not_affect_horizontal_layout() {
        let window = Vec2::new(360.0, 800.0);
        let without = compute_layout(window, 0.0, 0.0, true);
        let with_inset = compute_layout(window, 32.0, 0.0, true);
        for pile in [
            PileType::Stock,
            PileType::Waste,
            PileType::Tableau(0),
            PileType::Tableau(6),
        ] {
            assert!(
                (without.pile_positions[&pile].x - with_inset.pile_positions[&pile].x).abs() < 1e-3,
                "{pile:?} x-position must not change with safe_area_top",
            );
        }
    }

    /// A bottom safe-area inset must shrink the tableau fan so the worst-case
    /// column stays above the gesture bar.
    #[test]
    fn safe_area_bottom_reduces_tableau_fan() {
        let window = Vec2::new(360.0, 800.0);
        let without = compute_layout(window, 0.0, 0.0, true);
        let with_inset = compute_layout(window, 0.0, 48.0, true);
        assert!(
            with_inset.tableau_fan_frac <= without.tableau_fan_frac,
            "safe_area_bottom=48 must not increase tableau_fan_frac: {:.4} → {:.4}",
            without.tableau_fan_frac,
            with_inset.tableau_fan_frac,
        );
        let card_h = with_inset.card_size.y;
        let tableau_y = with_inset.pile_positions[&PileType::Tableau(6)].y;
        let bottom_edge = tableau_y - 12.0 * card_h * with_inset.tableau_fan_frac - card_h / 2.0;
        let h_gap = with_inset.card_size.x / 4.0;
        let margin = -window.y / 2.0 + 48.0 + h_gap;
        assert!(
            bottom_edge >= margin - 1e-3,
            "worst-case tableau bottom {bottom_edge:.2} overflows gesture-bar margin {margin:.2}",
        );
    }

    /// Suspend → resume layout-consistency invariant.
    ///
    /// If the resume handler resets `SafeAreaInsets` to zero and then the JNI
    /// poller re-resolves the same values, `compute_layout` must produce an
    /// identical result to the fresh-launch layout.  This test also verifies
    /// that a layout computed with `safe_area_top = 0` (the brief window while
    /// insets haven't re-resolved after resume) differs visibly from the
    /// correct layout, confirming that the bug would manifest without the fix.
    #[test]
    fn suspend_resume_layout_matches_fresh_launch() {
        let window = Vec2::new(900.0, 2000.0);
        let safe_top = 27.0_f32;
        let safe_bottom = 110.0_f32;

        // Fresh-launch layout — insets known from startup.
        let fresh = compute_layout(window, safe_top, safe_bottom, true);

        // Layout computed during the brief post-resume window before insets
        // re-resolve (safe_area_top temporarily 0).
        let wrong = compute_layout(window, 0.0, safe_bottom, true);

        // Verify the "wrong" layout actually differs — the bug would push the
        // top card row upward by exactly safe_top pixels.
        let fresh_stock_y = fresh.pile_positions[&PileType::Stock].y;
        let wrong_stock_y = wrong.pile_positions[&PileType::Stock].y;
        // In Bevy's +y-is-up system, adding safe_area_top pushes the stock
        // downward (−y direction).  So wrong_stock_y > fresh_stock_y by safe_top.
        assert!(
            (wrong_stock_y - fresh_stock_y - safe_top).abs() < 1e-3,
            "wrong layout must displace stock upward by safe_top ({safe_top}): \
             fresh={fresh_stock_y:.2} wrong={wrong_stock_y:.2} delta={:.2}",
            wrong_stock_y - fresh_stock_y,
        );

        // After the poller re-resolves correct insets the layout must be
        // identical to the fresh-launch layout.
        let corrected = compute_layout(window, safe_top, safe_bottom, true);
        assert_eq!(
            corrected.card_size, fresh.card_size,
            "card size must be preserved after resume",
        );
        assert!(
            (corrected.pile_positions[&PileType::Stock].y - fresh_stock_y).abs() < 1e-3,
            "stock y must match fresh launch after resume: \
             corrected={:.2} fresh={fresh_stock_y:.2}",
            corrected.pile_positions[&PileType::Stock].y,
        );
        assert!(
            (corrected.pile_positions[&PileType::Stock].x
                - fresh.pile_positions[&PileType::Stock].x)
                .abs()
                < 1e-3,
            "stock x must be unchanged after resume",
        );
        // The HUD band top clearance (distance from window top to card top)
        // must match as well — this is the quantity directly visible in Bug 2.
        let card_top = |layout: &super::Layout| {
            layout.pile_positions[&PileType::Stock].y + layout.card_size.y / 2.0
        };
        assert!(
            (card_top(&corrected) - card_top(&fresh)).abs() < 1e-3,
            "top-of-card must match fresh launch after resume: \
             corrected={:.2} fresh={:.2}",
            card_top(&corrected),
            card_top(&fresh),
        );
    }

    /// safe_area_bottom must not affect horizontal positions.
    #[test]
    fn safe_area_bottom_does_not_affect_horizontal_layout() {
        let window = Vec2::new(360.0, 800.0);
        let without = compute_layout(window, 0.0, 0.0, true);
        let with_inset = compute_layout(window, 0.0, 48.0, true);
        for pile in [PileType::Stock, PileType::Tableau(0), PileType::Tableau(6)] {
            assert!(
                (without.pile_positions[&pile].x - with_inset.pile_positions[&pile].x).abs() < 1e-3,
                "{pile:?} x-position must not change with safe_area_bottom",
            );
        }
    }
}
