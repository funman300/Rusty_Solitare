//! Bevy `AssetLoader` that rasterises an SVG into `bevy::image::Image`.
//!
//! The card-theme system (see `CARD_PLAN.md`) ships SVG sources both as
//! the embedded default theme and as user-supplied themes. Bevy 0.18 has
//! no built-in SVG support, so this loader bridges `usvg` (parser) +
//! `resvg` (renderer) + `tiny-skia` (CPU pixmap) to produce textures
//! that the rest of the engine consumes as plain `Handle<Image>` — no
//! awareness of vector graphics leaks past this boundary.
//!
//! Rasterisation happens once per (asset, settings) pair at load time.
//! Bevy's asset system caches the resulting `Image`, so the cost is paid
//! exactly once per theme switch, not per frame.
//!
//! # Settings
//!
//! Each `Handle<Image>` produced via this loader carries
//! [`SvgLoaderSettings`]. The most important field is `target_size` —
//! callers should specify the rasterisation resolution explicitly when
//! loading via `load_with_settings(...)`. The default of 512×768 is a
//! safe fallback that fits a typical 2:3 playing card.

use std::sync::{Arc, OnceLock};

use bevy::asset::io::Reader;
use bevy::asset::{AssetLoader, LoadContext, RenderAssetUsages};
use bevy::image::Image;
use bevy::math::UVec2;
use bevy::reflect::TypePath;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use usvg::fontdb;

/// Per-asset settings consumed by [`SvgLoader::load`].
///
/// `target_size` controls the rasterisation resolution. SVG content is
/// scaled uniformly to fit this box while preserving aspect ratio.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SvgLoaderSettings {
    /// Output texture dimensions in pixels.
    pub target_size: UVec2,
}

impl Default for SvgLoaderSettings {
    fn default() -> Self {
        // 512×768 is a 2:3 aspect at a resolution that stays sharp on
        // typical desktop windows where individual cards never exceed
        // ~250 px wide. Callers that need higher fidelity should
        // override via `load_with_settings`.
        Self {
            target_size: UVec2::new(512, 768),
        }
    }
}

/// Errors surfaced by [`SvgLoader::load`].
#[derive(Debug, Error)]
pub enum SvgLoaderError {
    /// The asset reader failed before the SVG bytes were consumed.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    /// `usvg` rejected the input as malformed or unsupported.
    #[error("svg parse: {0}")]
    Parse(#[from] usvg::Error),
    /// `tiny_skia::Pixmap::new` returned `None` — typically because the
    /// requested target_size is zero or absurdly large.
    #[error("could not allocate pixmap of size {0}x{1}")]
    PixmapAlloc(u32, u32),
}

/// `AssetLoader` registered for the `.svg` extension.
///
/// Stateless; safe to construct via `Default` and register once at
/// startup with `app.register_asset_loader(SvgLoader)`.
#[derive(Debug, Default, TypePath)]
pub struct SvgLoader;

impl AssetLoader for SvgLoader {
    type Asset = Image;
    type Settings = SvgLoaderSettings;
    type Error = SvgLoaderError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        settings: &Self::Settings,
        _load_context: &mut LoadContext<'_>,
    ) -> Result<Image, Self::Error> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;
        rasterize_svg(&bytes, settings.target_size)
    }

    fn extensions(&self) -> &[&str] {
        &["svg"]
    }
}

/// Rasterises an SVG byte buffer into an `Image` of exactly
/// `target.x × target.y` pixels. Content is scaled uniformly to fit
/// while preserving aspect ratio; unused area is left transparent.
///
/// Exposed separately from the `AssetLoader` impl so callers (tests,
/// the Phase 7 zip importer's "is this a valid SVG?" check, future
/// thumbnail generators) can rasterise without going through the
/// asset graph.
pub fn rasterize_svg(svg_bytes: &[u8], target: UVec2) -> Result<Image, SvgLoaderError> {
    let opt = usvg::Options {
        fontdb: shared_fontdb(),
        ..Default::default()
    };
    let tree = usvg::Tree::from_data(svg_bytes, &opt)?;

    let svg_size = tree.size();
    let svg_w = svg_size.width();
    let svg_h = svg_size.height();

    let target_w = target.x as f32;
    let target_h = target.y as f32;

    // Scale-to-fit while preserving aspect — the smaller axis ratio wins
    // so the entire SVG is visible inside the target box.
    let scale = (target_w / svg_w).min(target_h / svg_h);

    let mut pixmap = tiny_skia::Pixmap::new(target.x, target.y)
        .ok_or(SvgLoaderError::PixmapAlloc(target.x, target.y))?;

    // Centre the scaled SVG inside the target box so any aspect-ratio
    // mismatch is balanced rather than pinned to the top-left corner.
    let dx = (target_w - svg_w * scale) * 0.5;
    let dy = (target_h - svg_h * scale) * 0.5;
    let transform = tiny_skia::Transform::from_scale(scale, scale).post_translate(dx, dy);

    resvg::render(&tree, transform, &mut pixmap.as_mut());

    Ok(Image::new(
        Extent3d {
            width: target.x,
            height: target.y,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        pixmap.take(),
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    ))
}

/// Returns a process-wide font database populated with the OS-installed
/// fonts the user has available. Initialised lazily on first SVG that
/// references text, then shared (via `Arc`) across every subsequent
/// rasterisation. `usvg::Options::default()` ships an empty `fontdb`,
/// so without this call any text glyph in an SVG renders with no font
/// match — the visible symptom on the bundled hayeah artwork is the
/// "No match for Arial font-family" warn spam plus glyphs that fall
/// through to whatever shape-only path usvg uses for missing fonts.
/// `load_system_fonts` is comparatively expensive (~50–200 ms on a
/// typical desktop) so we only pay it once for the lifetime of the
/// process, gated by `OnceLock`.
fn shared_fontdb() -> Arc<fontdb::Database> {
    static DB: OnceLock<Arc<fontdb::Database>> = OnceLock::new();
    DB.get_or_init(|| {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        Arc::new(db)
    })
    .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal but non-trivial SVG: yellow rectangle + dark circle.
    /// Embedded inline so tests have no filesystem dependencies. The
    /// `##` raw-string delimiter lets us inline `#`-prefixed hex colours.
    const TEST_SVG: &[u8] = br##"<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 200 300" width="200" height="300">
  <rect x="0" y="0" width="200" height="300" fill="#FFD23F"/>
  <circle cx="100" cy="150" r="80" fill="#1A0F2E"/>
</svg>"##;

    #[test]
    fn rasterizes_at_default_size() {
        let settings = SvgLoaderSettings::default();
        let image = rasterize_svg(TEST_SVG, settings.target_size).expect("rasterisation");
        assert_eq!(image.size().x, 512);
        assert_eq!(image.size().y, 768);
    }

    #[test]
    fn rasterizes_at_custom_size() {
        let image = rasterize_svg(TEST_SVG, UVec2::new(64, 96)).expect("rasterisation");
        assert_eq!(image.size().x, 64);
        assert_eq!(image.size().y, 96);
    }

    #[test]
    fn rejects_zero_dimension() {
        let err = rasterize_svg(TEST_SVG, UVec2::new(0, 100)).unwrap_err();
        assert!(matches!(err, SvgLoaderError::PixmapAlloc(0, 100)));
    }

    #[test]
    fn rejects_malformed_svg() {
        let err = rasterize_svg(b"not actually svg", UVec2::new(64, 96)).unwrap_err();
        assert!(matches!(err, SvgLoaderError::Parse(_)));
    }

    #[test]
    fn pixmap_data_is_rgba_with_target_byte_count() {
        let image =
            rasterize_svg(TEST_SVG, UVec2::new(32, 48)).expect("rasterisation");
        let pixels = image.data.as_ref().expect("rasterised image carries pixel data");
        // 32 × 48 × 4 (RGBA bytes) = 6144 bytes
        assert_eq!(pixels.len(), 32 * 48 * 4);
    }

    #[test]
    fn loader_advertises_svg_extension() {
        let loader = SvgLoader;
        assert_eq!(loader.extensions(), &["svg"]);
    }

    /// Compile-time guard that `SvgLoaderSettings` satisfies the trait
    /// bounds Bevy expects on `AssetLoader::Settings` — keeps the
    /// loader's `#[derive]` set honest if the upstream signature ever
    /// tightens.
    #[test]
    fn settings_satisfies_loader_bounds() {
        fn assert_loader_settings<T: Default + serde::Serialize + serde::de::DeserializeOwned>() {}
        assert_loader_settings::<SvgLoaderSettings>();
    }
}
