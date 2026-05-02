//! Centralised UI design tokens — colours, typography, spacing, radius,
//! z-index hierarchy, and motion durations.
//!
//! Every UI surface (HUD, modals, popovers, toasts) reads from these
//! tokens instead of hardcoding hex codes or magic numbers. The audit
//! that produced this module found 40+ scattered colour literals, 12+
//! distinct font sizes, and 8+ hardcoded z-index values across the
//! engine; collapsing them into one source of truth keeps the visual
//! system coherent and makes future palette swaps a single-file change.
//!
//! Palette is "Midnight Purple + Balatro accent" — see the 2026-04-30
//! UX overhaul Phase 2 proposal for the rationale behind specific
//! values. The tokens are exposed as `pub const` so static contexts
//! (default colours on Sprite components, etc.) can use them; a future
//! `UiTheme` resource can layer runtime switching on top without
//! changing the constant API.

use bevy::color::Color;
use bevy::math::Vec2;
use bevy::prelude::Val;
use solitaire_data::AnimSpeed;

// ---------------------------------------------------------------------------
// Colours — Midnight Purple base with a Balatro-yellow primary accent.
// ---------------------------------------------------------------------------

/// Window backstop and the default text colour on top of `ACCENT_PRIMARY`.
/// Deep midnight purple, near-black. `#1A0F2E`.
pub const BG_BASE: Color = Color::srgb(0.102, 0.059, 0.180);

/// Elevated surface — modal cards, popover panels, button backgrounds.
/// One step lighter than `BG_BASE` so cards visually float above the
/// felt without needing real drop shadows. `#2D1B69`.
pub const BG_ELEVATED: Color = Color::srgb(0.176, 0.106, 0.412);

/// Hovered/highlighted surface — used on button hover and on the
/// currently-active row of a popover. `#3A2580`.
pub const BG_ELEVATED_HI: Color = Color::srgb(0.227, 0.145, 0.502);

/// Top elevation step — Secondary button hover, popover currently-
/// hovered row. One rung above `BG_ELEVATED_HI`. `#482F97`.
pub const BG_ELEVATED_TOP: Color = Color::srgb(0.282, 0.184, 0.592);

/// Pressed-button surface — `BG_ELEVATED` darkened ~15%. `#26155B`.
pub const BG_ELEVATED_PRESSED: Color = Color::srgb(0.149, 0.082, 0.357);

/// Uniform scrim under every modal. The audit found 0.60–0.92 alpha
/// drift across 11 overlay plugins; this single value replaces all of
/// them. `rgba(13, 7, 28, 0.85)`.
pub const SCRIM: Color = Color::srgba(0.051, 0.027, 0.110, 0.85);

/// Translucent fill for the top-of-window HUD band painted by
/// `hud_plugin::spawn_hud_band`. Same midnight-purple hue as `BG_BASE`,
/// but at 0.70 alpha so the green felt reads through subtly — enough
/// to mark the band as "UI" without feeling like a hard chrome strip.
/// `rgba(26, 15, 46, 0.70)`.
pub const BG_HUD_BAND: Color = Color::srgba(0.102, 0.059, 0.180, 0.70);

/// Primary text — warm off-white with a hint of purple to fit the
/// midnight palette without feeling clinical. `#F5F0FF`.
pub const TEXT_PRIMARY: Color = Color::srgb(0.961, 0.941, 1.000);

/// Secondary text — captions, hints, muted labels. Lavender-grey.
/// `#B5A8D5`.
pub const TEXT_SECONDARY: Color = Color::srgb(0.710, 0.659, 0.835);

/// Disabled text — greyed-out buttons, locked items. `#6B5F85`.
pub const TEXT_DISABLED: Color = Color::srgb(0.420, 0.373, 0.522);

/// Balatro-yellow primary accent — the loudest colour in the palette.
/// Reserved for primary actions (Confirm, Play Again), win states, and
/// "look here" callouts. `BG_BASE` text on top of this colour passes
/// AAA contrast. `#FFD23F`.
pub const ACCENT_PRIMARY: Color = Color::srgb(1.000, 0.824, 0.247);

/// Brightened `ACCENT_PRIMARY` for hover states on primary buttons.
/// Picks up saturation while keeping the same hue. `#FFE36B`.
pub const ACCENT_PRIMARY_HOVER: Color = Color::srgb(1.000, 0.890, 0.420);

/// Warm magenta secondary accent — celebratory states (achievement
/// unlocked, streak milestones). Used sparingly so it stays special.
/// `#FF6B9D`.
pub const ACCENT_SECONDARY: Color = Color::srgb(1.000, 0.420, 0.616);

/// Success — foundation completion, valid drop tint, sync OK. `#4ADE80`.
pub const STATE_SUCCESS: Color = Color::srgb(0.290, 0.871, 0.502);

/// Warning — penalty signal. **Both** Undo and Recycle counters use
/// this when non-zero (the audit found these were inconsistent — Undos
/// amber, Recycles white). `#FFA94D`.
pub const STATE_WARNING: Color = Color::srgb(1.000, 0.663, 0.302);

/// Danger — rejection shake, illegal placement, sync error. `#F77272`.
pub const STATE_DANGER: Color = Color::srgb(0.969, 0.447, 0.447);

/// Info — daily-challenge constraint, draw-cycle indicator. `#6BBBFF`.
pub const STATE_INFO: Color = Color::srgb(0.420, 0.733, 1.000);

/// Soft fill colour for the drop-target overlay shown over every legal
/// destination pile while the player is dragging a card. Same green hue
/// as `STATE_SUCCESS` (`#4ADE80`) so the visual language stays
/// consistent, but at 10 % alpha so the underlying card faces remain
/// fully readable through the wash.
pub const DROP_TARGET_FILL: Color = Color::srgba(0.290, 0.871, 0.502, 0.10);

/// Outline colour for the drop-target overlay. Matches the
/// `STATE_SUCCESS` hue at 75 % alpha so the rectangle border reads
/// unmistakably against both the felt and stacked card faces without
/// drowning the cards themselves.
pub const DROP_TARGET_OUTLINE: Color = Color::srgba(0.290, 0.871, 0.502, 0.75);

/// Thickness of the drop-target outline edges, in world-space pixels.
pub const DROP_TARGET_OUTLINE_PX: f32 = 3.0;

/// Sprite-space `Transform.z` for drop-target overlay entities. Sits
/// well above any static card (top stack z is `~1.04`) but well below
/// the lifted dragged stack (`DRAG_Z = 500.0` in `input_plugin`) so the
/// overlay never occludes the card the player is holding. Distinct from
/// the i32 `Z_*` UI-Node tokens above — those are `ZIndex` values for
/// `bevy::ui`, while this is a 2D `Sprite` z coordinate.
pub const Z_DROP_OVERLAY: f32 = 50.0;

/// Background colour of the stock-pile remaining-count chip.
///
/// Reuses `BG_ELEVATED_HI` so the chip reads as one rung above the
/// translucent stock pile marker without introducing a new palette
/// value. The badge sits on the stock corner so the player knows how
/// many cards remain before a recycle.
pub const STOCK_BADGE_BG: Color = BG_ELEVATED_HI;

/// Foreground (text) colour of the stock-pile remaining-count chip.
///
/// `ACCENT_PRIMARY` keeps the chip readable against the elevated
/// purple background and matches the Balatro accent already used for
/// other "look here" callouts.
pub const STOCK_BADGE_FG: Color = ACCENT_PRIMARY;

/// Sprite-space `Transform.z` for the stock-pile remaining-count chip.
///
/// Sits above the stock pile marker (`Z_PILE_MARKER` = `-1`) and any
/// face-down stock cards (which start at `0`), but well below
/// [`Z_DROP_OVERLAY`] (`50.0`) so the green drop-target wash always
/// renders on top while a card is being dragged. Like `Z_DROP_OVERLAY`,
/// this is a 2D `Sprite` z coordinate, not a `bevy::ui` `ZIndex`.
pub const Z_STOCK_BADGE: f32 = 30.0;

// ---------------------------------------------------------------------------
// Card drop-shadow — the subtle dark halo painted beneath every card so the
// play surface reads as physical instead of a flat collage of stickers. Idle
// values are deliberately low-contrast (small offset, ~25% alpha) so resting
// cards feel grounded without competing with focus rings or drop overlays.
// Drag values are slightly stronger (further offset, ~40% alpha, larger
// halo) so the dragged stack visually "lifts" off the felt.
// ---------------------------------------------------------------------------

/// RGB base for the per-card drop shadow. Always neutral black — never
/// suit-tinted — so the shadow never carries colour information that a
/// colour-blind player would rely on to identify a card. Alpha is applied
/// separately via [`CARD_SHADOW_ALPHA_IDLE`] / [`CARD_SHADOW_ALPHA_DRAG`].
pub const CARD_SHADOW_COLOR: Color = Color::srgb(0.0, 0.0, 0.0);

/// Alpha for the resting-state card shadow. Low enough that 52 stacked
/// shadows do not darken the felt into a uniform smear, high enough that
/// each card reads as separated from the surface.
pub const CARD_SHADOW_ALPHA_IDLE: f32 = 0.25;

/// Alpha for the lifted/dragged card shadow. Stronger than the idle value
/// so the dragged stack visibly "casts more shadow" while the player holds
/// it above the table.
pub const CARD_SHADOW_ALPHA_DRAG: f32 = 0.40;

/// World-space pixel offset of the resting-state card shadow relative to
/// its parent card centre. Down-and-right matches a soft top-left light
/// source — the same convention used by the elevated-surface tones in the
/// rest of the palette.
pub const CARD_SHADOW_OFFSET_IDLE: Vec2 = Vec2::new(2.0, -3.0);

/// World-space pixel offset of the lifted/dragged card shadow. Roughly
/// double the idle offset so the parallax reads as "the card is further
/// from the table".
pub const CARD_SHADOW_OFFSET_DRAG: Vec2 = Vec2::new(4.0, -6.0);

/// Padding in pixels added to each axis of the card size when sizing the
/// resting-state shadow sprite. The shadow extends slightly past every
/// edge of the card so the dark border reads as a halo rather than a
/// matte rectangle behind the card.
pub const CARD_SHADOW_PADDING_IDLE: Vec2 = Vec2::new(4.0, 4.0);

/// Padding added to the card size when sizing the lifted/dragged shadow.
/// A slightly larger halo at the drag state reinforces the "lifted off
/// the felt" cue alongside the deeper offset and higher alpha.
pub const CARD_SHADOW_PADDING_DRAG: Vec2 = Vec2::new(8.0, 8.0);

/// Local `Transform.z` for the shadow child sprite, relative to its
/// parent `CardEntity`. Slightly negative so the shadow always renders
/// below the card itself even though it shares the parent's world z.
pub const CARD_SHADOW_LOCAL_Z: f32 = -0.05;

/// Subtle border — default popover, card, and idle button outline.
pub const BORDER_SUBTLE: Color = Color::srgba(0.647, 0.549, 1.000, 0.12);

/// Strong border — hover outline, focused button, active popover.
pub const BORDER_STRONG: Color = Color::srgba(0.647, 0.549, 1.000, 0.30);

/// 2 px ring drawn around the focused interactive element. Balatro yellow
/// (matches `ACCENT_PRIMARY`) at 85% alpha so the ring stays legible
/// against both elevated surfaces and the modal scrim backdrop.
pub const FOCUS_RING: Color = Color::srgba(1.0, 0.823, 0.247, 0.85);

// ---------------------------------------------------------------------------
// Typography scale (px) — 5 rungs replace the prior
// 14/15/16/17/18/22/26/28/30/32/40/48 jungle. All UI uses FiraMono via
// `FontResource`; sizes carry the hierarchy.
// ---------------------------------------------------------------------------

/// Display titles — Home, Win Summary, Onboarding header. 40 px.
pub const TYPE_DISPLAY: f32 = 40.0;

/// Modal / overlay headers. 26 px.
pub const TYPE_HEADLINE: f32 = 26.0;

/// Primary HUD numbers, button labels, body copy that needs weight.
/// 18 px.
pub const TYPE_BODY_LG: f32 = 18.0;

/// Secondary HUD, body copy, list items. 14 px.
pub const TYPE_BODY: f32 = 14.0;

/// Hotkey-hint chips, microcopy, dates. 11 px.
pub const TYPE_CAPTION: f32 = 11.0;

// ---------------------------------------------------------------------------
// Spacing scale (px) — 4-multiple rungs. Every padding, margin, and gap
// in the engine snaps to one of these.
// ---------------------------------------------------------------------------

/// 4 px — inline padding, chip gap.
pub const SPACE_1: f32 = 4.0;
/// 8 px — default gap between row items.
pub const SPACE_2: f32 = 8.0;
/// 12 px — standard button padding-X, action row gap.
pub const SPACE_3: f32 = 12.0;
/// 16 px — section gap inside a modal.
pub const SPACE_4: f32 = 16.0;
/// 24 px — modal card outer padding.
pub const SPACE_5: f32 = 24.0;
/// 32 px — block separator inside large overlays.
pub const SPACE_6: f32 = 32.0;
/// 48 px — outer modal margin from the window edge.
pub const SPACE_7: f32 = 48.0;

/// `Val::Px` form of `SPACE_1`, for ergonomic Node construction.
pub const VAL_SPACE_1: Val = Val::Px(SPACE_1);
/// `Val::Px` form of `SPACE_2`.
pub const VAL_SPACE_2: Val = Val::Px(SPACE_2);
/// `Val::Px` form of `SPACE_3`.
pub const VAL_SPACE_3: Val = Val::Px(SPACE_3);
/// `Val::Px` form of `SPACE_4`.
pub const VAL_SPACE_4: Val = Val::Px(SPACE_4);
/// `Val::Px` form of `SPACE_5`.
pub const VAL_SPACE_5: Val = Val::Px(SPACE_5);
/// `Val::Px` form of `SPACE_6`.
pub const VAL_SPACE_6: Val = Val::Px(SPACE_6);
/// `Val::Px` form of `SPACE_7`.
pub const VAL_SPACE_7: Val = Val::Px(SPACE_7);

// ---------------------------------------------------------------------------
// Border radius (px)
// ---------------------------------------------------------------------------

/// 4 px — hotkey chip, inline pill.
pub const RADIUS_SM: f32 = 4.0;
/// 8 px — buttons, popover panels.
pub const RADIUS_MD: f32 = 8.0;
/// 16 px — modal cards.
pub const RADIUS_LG: f32 = 16.0;

// ---------------------------------------------------------------------------
// Z-index hierarchy — replaces 8+ scattered magic numbers across plugins
// (background, pile markers, HUD, several overlay tiers, win cascade,
// toasts). Documented order:
//
//   background  →  pile markers  →  cards  →  HUD  →  HUD popovers  →
//   modal scrim → modal panel  →  pause  →  onboarding  →
//   win cascade  →  toasts (always on top)
// ---------------------------------------------------------------------------

pub const Z_BACKGROUND: i32 = -10;
pub const Z_PILE_MARKER: i32 = -1;
/// Base layer for HUD readouts (top-left).
pub const Z_HUD: i32 = 50;
/// Action bar + popovers — above HUD readouts so dropdowns can overlap.
pub const Z_HUD_TOP: i32 = 60;
pub const Z_MODAL_SCRIM: i32 = 200;
pub const Z_MODAL_PANEL: i32 = 210;
/// Pause overlay outranks normal modals — pausing should always be on top.
pub const Z_PAUSE: i32 = 220;
/// Confirmation dialog stacked on top of the pause overlay (e.g. the
/// forfeit-confirm modal launched from the pause modal). Sits above
/// `Z_PAUSE` so the dialog is always visible over the paused state.
pub const Z_PAUSE_DIALOG: i32 = 225;
pub const Z_ONBOARDING: i32 = 230;
/// Z-layer for the keyboard focus indicator. Sits one rung above the
/// topmost modal layer (`Z_ONBOARDING`) so the ring is never occluded by
/// a modal card's hover state, while staying below the win cascade and
/// transient toasts that are allowed to overlay everything else.
pub const Z_FOCUS_RING: i32 = 240;
/// Win cascade sits between modals and toasts so the celebration plays
/// over a paused / mid-modal screen.
pub const Z_WIN_CASCADE: i32 = 300;
/// Toasts always render above everything else.
pub const Z_TOAST: i32 = 400;

// ---------------------------------------------------------------------------
// Motion — durations in seconds at `AnimSpeed::Normal`. `Fast` halves
// every value, `Instant` zeroes them. Use `scaled_duration` to apply.
// ---------------------------------------------------------------------------

/// Card slide during gameplay — tweened with `MotionCurve::SmoothSnap`
/// (ease-out-cubic). 180 ms; bumped from the old 150 ms because cubic
/// feels slower at endpoints — the perceived speed is unchanged.
pub const MOTION_SLIDE_SECS: f32 = 0.18;

/// Settle bounce on placement — only the moved card, not every top
/// card on every state change. 180 ms.
pub const MOTION_SETTLE_SECS: f32 = 0.18;

/// Shake on rejected drop — tightened from 300 ms; frequency drops to
/// 35 rad/s to match the new settle bounce so the two feedback signals
/// no longer feel discordant. 250 ms.
pub const MOTION_SHAKE_SECS: f32 = 0.25;

/// Shake angular frequency in rad/s.
pub const MOTION_SHAKE_OMEGA: f32 = 35.0;

/// Duration of the smooth return tween when a drag is rejected by an
/// invalid drop target. Short enough to feel snappy but long enough to
/// read as motion rather than a teleport.
pub const MOTION_DRAG_REJECT_SECS: f32 = 0.15;

/// Card flip — half-time per phase (squash + grow). 100 ms each =
/// 200 ms total. Pair with a ±8° Z-rotation at the midpoint for a 3D
/// feel without 3D rendering.
pub const MOTION_FLIP_HALF_SECS: f32 = 0.10;

/// Per-card stagger on the new-game deal animation — centre value;
/// each card gets ±10% jitter applied at deal time so the deal feels
/// organic instead of mechanical. 40 ms.
pub const MOTION_DEAL_STAGGER_SECS: f32 = 0.04;

/// Deal slide duration with an `ease-out` curve and a 40 ms
/// scale-pop on land so cards "arrive" instead of just stopping.
/// 280 ms.
pub const MOTION_DEAL_SLIDE_SECS: f32 = 0.28;

/// Win cascade per-card stagger — slightly slower than the prior
/// 50 ms for a more theatrical feel. 60 ms.
pub const MOTION_CASCADE_STAGGER_SECS: f32 = 0.06;

/// Win cascade per-card slide — uses `MotionCurve::Expressive`
/// (overshoot) plus ±15° Z-rotation. 500 ms.
pub const MOTION_CASCADE_SLIDE_SECS: f32 = 0.50;

/// Screen shake on win — wider and longer than the old 0.6 s / 8 px.
/// 800 ms.
pub const MOTION_WIN_SHAKE_SECS: f32 = 0.80;

/// Peak displacement of the win screen shake. 12 px.
pub const MOTION_WIN_SHAKE_AMPLITUDE: f32 = 12.0;

/// Toast in — scale-from-0.92-to-1.0 fade-in. 200 ms.
pub const MOTION_TOAST_IN_SECS: f32 = 0.20;

/// Toast out — fade. 250 ms.
pub const MOTION_TOAST_OUT_SECS: f32 = 0.25;

/// Modal in/out — scale-from-0.96-to-1.0 + scrim fade. 220 ms.
pub const MOTION_MODAL_SECS: f32 = 0.22;

/// Button hover/press colour blend — short, snappy. 100 ms.
pub const MOTION_BUTTON_BLEND_SECS: f32 = 0.10;

/// Score-pulse — when score increases by ≥ 50, briefly scale the
/// readout 1.0 → 1.1 → 1.0. 250 ms.
pub const MOTION_SCORE_PULSE_SECS: f32 = 0.25;

/// Foundation-completion flourish — when a King lands on a foundation
/// pile (Ace → King, 13 cards), briefly scale the King card 1.0 →
/// [`FOUNDATION_FLOURISH_PEAK_SCALE`] → 1.0 and tint the matching
/// `PileMarker` gold. 400 ms.
pub const MOTION_FOUNDATION_FLOURISH_SECS: f32 = 0.4;

/// Peak scale magnification reached at the midpoint of the
/// foundation-completion flourish. The triangular curve climbs from
/// 1.0 at `t=0` to this value at `t=0.5` and back to 1.0 at `t=1.0`.
pub const FOUNDATION_FLOURISH_PEAK_SCALE: f32 = 1.15;

/// Loading-ellipsis cycle — `.`/`..`/`...` toggles every step.
/// 400 ms.
pub const MOTION_LOADING_TICK_SECS: f32 = 0.40;

/// Hover delay before a tooltip appears, in seconds. Long enough that
/// players gliding the cursor across the HUD don't see flicker; short
/// enough that "stop and read" feels responsive. Not run through
/// [`scaled_duration`] — `AnimSpeed` controls gameplay motion, not the
/// hover-discoverability budget for help text.
pub const MOTION_TOOLTIP_DELAY_SECS: f32 = 0.5;

/// Total visible duration of the splash screen overlay, in seconds.
/// Composed of a fade-in, a hold, and a fade-out — see
/// [`MOTION_SPLASH_FADE_SECS`] for the per-edge fade budget. Not run
/// through [`scaled_duration`]: the splash is a one-shot brand beat at
/// app start, not gameplay motion that should track `AnimSpeed`.
pub const MOTION_SPLASH_TOTAL_SECS: f32 = 1.6;

/// Fade-in and fade-out duration of the splash overlay, in seconds.
/// The hold time is `MOTION_SPLASH_TOTAL_SECS - 2 * MOTION_SPLASH_FADE_SECS`.
/// Mirroring fade-in and fade-out keeps the curve symmetric so the brand
/// beat reads as a single dissolve instead of two separate animations.
pub const MOTION_SPLASH_FADE_SECS: f32 = 0.3;

// ---------------------------------------------------------------------------
// Z-index — tooltip layer
// ---------------------------------------------------------------------------

/// Z-layer for tooltips. Sits one rung above the focus ring so a
/// tooltip rendered over a focused button is never occluded by the
/// button's outline. Still below `Z_WIN_CASCADE` and `Z_TOAST` so the
/// celebration and notification layers stay on top.
pub const Z_TOOLTIP: i32 = Z_FOCUS_RING + 10;

/// Z-layer for the launch splash overlay. The splash owns the entire
/// viewport for ~1.6 s before fading out, so it sits above every other
/// UI rung — including `Z_TOAST` — to guarantee the brand beat is
/// never occluded by a stray toast or tooltip. Neither toasts nor the
/// win cascade can fire during the splash window in practice (no game
/// has run yet, no toast queue has dispatched), but the relative order
/// is kept tidy in case a future feature schedules either at startup.
pub const Z_SPLASH: i32 = Z_TOAST + 100;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Scales a `MOTION_*_SECS` value by the player's animation-speed
/// preference. `Normal` × 1.0, `Fast` × 0.5, `Instant` → 0.
///
/// Pass any duration constant from this module through this helper
/// before handing it to a tween. The audit found that only slide and
/// cascade respected `AnimSpeed`; toasts, deal stagger, shake, and
/// settle were hardcoded. Routing every duration through this function
/// fixes that.
pub fn scaled_duration(secs: f32, speed: AnimSpeed) -> f32 {
    match speed {
        AnimSpeed::Normal => secs,
        AnimSpeed::Fast => secs * 0.5,
        AnimSpeed::Instant => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every spacing rung is a positive multiple of 4 — keeps the scale
    /// honest if someone tweaks values later.
    #[test]
    fn spacing_scale_is_a_4_multiple_geometric_progression() {
        for v in [SPACE_1, SPACE_2, SPACE_3, SPACE_4, SPACE_5, SPACE_6, SPACE_7] {
            assert!(v > 0.0, "spacing tokens must be positive");
            assert!(
                (v.rem_euclid(4.0)).abs() < f32::EPSILON,
                "spacing token {v} must be a 4-multiple"
            );
        }
    }

    /// Type scale is monotonically decreasing display → caption.
    #[test]
    fn type_scale_is_monotonically_decreasing() {
        let scale = [TYPE_DISPLAY, TYPE_HEADLINE, TYPE_BODY_LG, TYPE_BODY, TYPE_CAPTION];
        for window in scale.windows(2) {
            assert!(
                window[0] > window[1],
                "type scale must be monotonically decreasing: {} should be > {}",
                window[0],
                window[1]
            );
        }
    }

    /// Z-index hierarchy is monotonically increasing through documented
    /// layers, so a future add-a-layer change can't accidentally land
    /// in the wrong slot.
    #[test]
    fn z_index_hierarchy_is_monotonically_increasing() {
        let layers = [
            Z_BACKGROUND,
            Z_PILE_MARKER,
            Z_HUD,
            Z_HUD_TOP,
            Z_MODAL_SCRIM,
            Z_MODAL_PANEL,
            Z_PAUSE,
            Z_PAUSE_DIALOG,
            Z_ONBOARDING,
            Z_FOCUS_RING,
            Z_TOOLTIP,
            Z_WIN_CASCADE,
            Z_TOAST,
            Z_SPLASH,
        ];
        for window in layers.windows(2) {
            assert!(
                window[0] < window[1],
                "z-index hierarchy must be monotonically increasing: {} should be < {}",
                window[0],
                window[1]
            );
        }
    }

    #[test]
    fn scaled_duration_matches_anim_speed() {
        assert!((scaled_duration(0.18, AnimSpeed::Normal) - 0.18).abs() < f32::EPSILON);
        assert!((scaled_duration(0.18, AnimSpeed::Fast) - 0.09).abs() < f32::EPSILON);
        assert_eq!(scaled_duration(0.18, AnimSpeed::Instant), 0.0);
    }
}
