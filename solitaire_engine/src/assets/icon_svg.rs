//! SVG builder for the Ferrous Solitaire application icon.
//!
//! Renders the project's signature `▌RS` Terminal mark (the same
//! cursor-block + monogram pair used on the splash boot-screen and
//! card backs) on a dark `#151515` background with a 1 px brick-red
//! border. Square aspect, authored in a 64-unit logical box and
//! scaled at the rasterisation site.
//!
//! Reads at every size from 16 px taskbar tile to 1024 px macOS
//! Retina icon — the high-contrast cursor block carries the
//! recognition load and the smaller `RS` letters sit beneath as
//! a secondary recognition cue.
//!
//! Same SVG-to-PNG pipeline as `card_face_svg` — `icon_generator`
//! example rasterises this at multiple target sizes and writes
//! into `assets/icon/`. The `icon_svg_pin` integration test hashes
//! rasterised RGBA bytes to guard against `usvg`/`resvg` drift.

use bevy::math::UVec2;

/// Default rasterisation target — single canonical size used by the
/// runtime `Window::icon` wiring. The generator example emits
/// additional sizes (16, 32, 48, 64, 128, 256, 512, 1024) for the
/// Linux hicolor hierarchy and for downstream `.ico` / `.icns`
/// packaging.
pub const TARGET: UVec2 = UVec2::new(256, 256);

/// Every size the `icon_generator` example emits. Covers Linux
/// hicolor (16, 24, 32, 48, 64, 128, 256, 512), Windows `.ico`
/// targets (16, 32, 48, 256), and macOS `.icns` targets (16, 32,
/// 64, 128, 256, 512, 1024).
pub const ICON_SIZES: &[u32] = &[16, 24, 32, 48, 64, 128, 256, 512, 1024];

const BG: &str = "#151515"; // BG_BASE
const ACCENT: &str = "#a54242"; // ACCENT_PRIMARY brick red
const FG: &str = "#d0d0d0"; // TEXT_PRIMARY

/// Build the icon SVG. Square aspect, 64 logical units per side.
pub fn icon_svg() -> String {
    // Layout in a 64×64 logical box:
    //   border:        1 logical unit, brick-red, inset 0.5 to
    //                  centre the stroke inside the pixmap.
    //   corner radius: 6 units (~9 % of side, scales smoothly down
    //                  to 16 px where it disappears into pixel grid).
    //   `▌` cursor:    18 px tall, 6 px wide, brick-red, centred
    //                  horizontally, sitting on a baseline at y=40
    //                  so there's room for `RS` beneath it.
    //   `RS` mark:     14 px FiraMono Bold at y=58, foreground gray,
    //                  letter-spaced for readability at small sizes.
    //
    // The `▌` glyph is U+258C (LEFT HALF BLOCK) — same character the
    // splash and card-back monogram use, rendered upright at icon
    // scale. FiraMono carries this at usable size (verified by the
    // splash + card-back rendering), so `<text>` is safe here unlike
    // the suit glyphs.
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64" viewBox="0 0 64 64">
  <rect x="0.5" y="0.5" width="63" height="63" rx="6" ry="6"
        fill="{BG}" stroke="{ACCENT}" stroke-width="1"/>

  <!-- Centred ▌ cursor block at y=22..40, brick-red. -->
  <rect x="29" y="22" width="6" height="18" fill="{ACCENT}"/>

  <!-- RS monogram beneath, foreground gray. text-anchor=middle so
       the letterforms balance around the cursor block above. -->
  <text x="32" y="56" font-family="Fira Mono" font-size="14" font-weight="700"
        fill="{FG}" text-anchor="middle" letter-spacing="1">RS</text>
</svg>"##
    )
}
