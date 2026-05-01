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
    populate_embedded_default_theme, register_theme_asset_sources, AssetSourcesPlugin,
    DEFAULT_THEME_MANIFEST_URL, USER_THEMES,
};
pub use svg_loader::{rasterize_svg, SvgLoader, SvgLoaderError, SvgLoaderSettings};
pub use user_dir::{set_user_theme_dir, user_theme_dir};
