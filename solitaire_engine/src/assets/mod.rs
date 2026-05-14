//! Asset-loading infrastructure for runtime SVG rasterisation and the
//! per-platform user-themes directory.
//!
//! Provides the SVG → `Image` loader and the `embedded://` / `themes://`
//! custom `AssetSource` implementations used by the theme system.

pub mod card_face_svg;
pub mod icon_svg;
pub mod sources;
pub mod svg_loader;
pub mod user_dir;

pub use sources::{
    bundled_theme_url, classic_theme_svg_bytes, dark_theme_svg_bytes,
    populate_embedded_classic_theme, populate_embedded_dark_theme, register_theme_asset_sources,
    AssetSourcesPlugin, CLASSIC_THEME_MANIFEST_URL, DARK_THEME_MANIFEST_URL, USER_THEMES,
};
pub use svg_loader::{rasterize_svg, SvgLoader, SvgLoaderError, SvgLoaderSettings};
pub use user_dir::{set_user_theme_dir, user_theme_dir};
