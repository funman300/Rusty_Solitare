//! Asset-loading infrastructure for runtime SVG rasterisation and the
//! per-platform user-themes directory.
//!
//! See `CARD_PLAN.md` for the full multi-phase implementation plan.
//! This module is the entry point for Phases 1 (SVG → `Image`) and 5
//! (user-themes directory). Phase 3 will extend it further with custom
//! `AssetSource` implementations for `embedded://` and `themes://`.

pub mod sources;
pub mod svg_loader;
pub mod user_dir;

pub use sources::{
    default_theme_svg_bytes, populate_embedded_default_theme,
    populate_embedded_rusty_pixel_theme, register_theme_asset_sources,
    rusty_pixel_theme_png_bytes, AssetSourcesPlugin, DEFAULT_THEME_MANIFEST_URL,
    RUSTY_PIXEL_THEME_MANIFEST_URL, USER_THEMES,
};
pub use svg_loader::{rasterize_svg, SvgLoader, SvgLoaderError, SvgLoaderSettings};
pub use user_dir::{set_user_theme_dir, user_theme_dir};
