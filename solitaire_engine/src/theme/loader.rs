//! `AssetLoader` for `.theme.ron` manifests.
//!
//! Reads the manifest, validates structurally (52 faces, sane meta),
//! then schedules each referenced SVG via [`crate::assets::SvgLoader`]
//! at the resolution implied by `meta.card_aspect`. The resulting
//! `Handle<Image>`s are stored on the [`super::CardTheme`] asset, so
//! Bevy's asset dependency graph keeps each face alive for as long as
//! the theme is alive.

use std::collections::HashMap;

use bevy::asset::io::Reader;
use bevy::asset::{AssetLoader, AssetPath, LoadContext, ParseAssetPathError};
use bevy::reflect::TypePath;
use thiserror::Error;

use crate::assets::SvgLoaderSettings;

use super::manifest::{ManifestError, ThemeManifest};
use super::{CardKey, CardTheme};

/// Default rasterisation height when the manifest's `card_aspect`
/// implies a 2:3 card. 768 px tall × 512 px wide stays sharp on
/// any reasonable desktop window. Mobile viewports may want larger;
/// the per-load settings hook in `SvgLoader` stays available for
/// future overrides.
const DEFAULT_CARD_HEIGHT_PX: u32 = 768;

/// Errors raised by [`CardThemeLoader::load`].
#[derive(Debug, Error)]
pub enum CardThemeLoaderError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("manifest parse (RON): {0}")]
    Parse(#[from] ron::error::SpannedError),
    #[error("manifest validation: {0}")]
    Validation(#[from] ManifestError),
    /// `AssetPath::resolve_embed` rejected a manifest-relative path.
    /// Almost always means the manifest contains an absolute path or
    /// a surface that includes a custom asset source the manifest
    /// shouldn't be reaching across.
    #[error("could not resolve asset path: {0}")]
    PathResolve(#[from] ParseAssetPathError),
}

/// `AssetLoader` registered for the `.theme.ron` extension.
#[derive(Debug, Default, TypePath)]
pub struct CardThemeLoader;

impl AssetLoader for CardThemeLoader {
    type Asset = CardTheme;
    type Settings = ();
    type Error = CardThemeLoaderError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &Self::Settings,
        load_context: &mut LoadContext<'_>,
    ) -> Result<CardTheme, Self::Error> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;
        let manifest: ThemeManifest = ron::de::from_bytes(&bytes)?;

        // Surfaces metadata + face-completeness errors with named
        // diagnostics before we touch the asset graph.
        let face_paths = manifest.validate()?;
        let target = target_size_from_aspect(manifest.meta.card_aspect);

        // Clone the manifest's own asset path so we can compose
        // sibling paths via `AssetPath::resolve` without holding an
        // immutable borrow of `load_context` while we mutably borrow
        // it via `.loader()`.
        let manifest_path: AssetPath<'static> = load_context.path().clone();

        // `resolve_embed` is the RFC 1808 sibling-resolution method:
        // the last segment of the base path (the manifest filename) is
        // stripped before concatenation, so `themes/foo/theme.ron` +
        // `hearts_4.svg` resolves to `themes/foo/hearts_4.svg`. Plain
        // `resolve` would concatenate, giving `themes/foo/theme.ron/hearts_4.svg`,
        // which is never what manifest-relative references mean.
        let back_path = manifest_path.resolve_embed(&path_to_str(&manifest.back))?;
        let face_full: Vec<(CardKey, AssetPath<'static>)> = face_paths
            .iter()
            .map(|(k, p)| {
                manifest_path
                    .resolve_embed(&path_to_str(p))
                    .map(|ap| (*k, ap))
            })
            .collect::<Result<_, _>>()?;

        let mut faces = HashMap::with_capacity(face_full.len());
        for (key, full_path) in face_full {
            let handle = load_context
                .loader()
                .with_settings(move |s: &mut SvgLoaderSettings| s.target_size = target)
                .load(full_path);
            faces.insert(key, handle);
        }
        let back = load_context
            .loader()
            .with_settings(move |s: &mut SvgLoaderSettings| s.target_size = target)
            .load(back_path);

        Ok(CardTheme {
            meta: manifest.meta,
            faces,
            back,
        })
    }

    fn extensions(&self) -> &[&str] {
        &["theme.ron"]
    }
}

/// `AssetPath::resolve` takes `&str`; manifest paths are `PathBuf`.
/// Lossy is acceptable here because manifest paths must be plain ASCII
/// for cross-platform asset resolution to behave consistently.
fn path_to_str(p: &std::path::Path) -> String {
    p.to_string_lossy().into_owned()
}

/// Translates `card_aspect` into the SVG rasteriser's target pixel
/// size. Height is held constant at [`DEFAULT_CARD_HEIGHT_PX`]; width
/// is derived to preserve the aspect, with a minimum of 1 px so a
/// degenerate-but-validated aspect doesn't produce a 0-width pixmap.
fn target_size_from_aspect(aspect: (u32, u32)) -> bevy::math::UVec2 {
    let (num, denom) = aspect;
    let width = ((DEFAULT_CARD_HEIGHT_PX as u64 * num as u64) / denom as u64).max(1) as u32;
    bevy::math::UVec2::new(width, DEFAULT_CARD_HEIGHT_PX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_size_2_to_3_yields_512_by_768() {
        assert_eq!(
            target_size_from_aspect((2, 3)),
            bevy::math::UVec2::new(512, 768)
        );
    }

    #[test]
    fn target_size_handles_non_standard_aspect() {
        // 3:4 → wider card.
        let v = target_size_from_aspect((3, 4));
        assert_eq!(v.y, DEFAULT_CARD_HEIGHT_PX);
        assert_eq!(v.x, 576);
    }

    #[test]
    fn target_size_clamps_to_at_least_1px_wide() {
        // 1:10000 would otherwise round to zero.
        let v = target_size_from_aspect((1, 10_000));
        assert!(v.x >= 1);
    }

    #[test]
    fn loader_advertises_theme_ron_extension() {
        let loader = CardThemeLoader;
        assert_eq!(loader.extensions(), &["theme.ron"]);
    }
}
