//! Custom Bevy asset sources for the card-theme system.
//!
//! Two sources are wired up here:
//!
//! - **`embedded://`** — the bundled default theme. The default theme
//!   manifest (and, in later phases, every default-theme SVG) is
//!   compiled into the binary via `include_bytes!` and inserted into
//!   Bevy's [`EmbeddedAssetRegistry`] under a stable, pretty path.
//!   The default theme manifest is reachable as
//!   `embedded://solitaire_engine/assets/themes/default/theme.ron`.
//!
//! - **`themes://`** — user-supplied themes living in
//!   [`crate::assets::user_dir::user_theme_dir`]. Reads delegate to
//!   `FileAssetReader` rooted at that absolute path. If the directory
//!   doesn't exist yet (the common case on first run), the source
//!   still registers cleanly — individual reads simply return
//!   `NotFound` until the player drops a theme there, which is the
//!   correct behaviour for an empty user-themes directory.
//!
//! # Why two registration paths?
//!
//! Bevy treats the two sources differently:
//!
//! - **`themes://`** is a *new* source the engine doesn't know about.
//!   It must be registered with [`App::register_asset_source`] *before*
//!   `AssetPlugin` runs, because that plugin freezes the source list
//!   when it builds the `AssetServer`.
//!
//! - **`embedded://`** is already registered by `AssetPlugin` itself
//!   (via `EmbeddedAssetRegistry::register_source`). What we have to
//!   do is *populate* the registry with our default-theme files —
//!   exactly what Bevy's own `embedded_asset!` macro does. That
//!   population must happen *after* `AssetPlugin` runs, because
//!   `AssetPlugin::build` overwrites the `EmbeddedAssetRegistry`
//!   resource with a fresh empty one as part of its own setup.
//!
//! These two timing constraints can't be satisfied by a single
//! `Plugin::build` call, so the public API splits the work into:
//!
//! 1. [`register_theme_asset_sources`] — call *before* `DefaultPlugins`,
//!    typically immediately after `App::new()`. Registers `themes://`.
//! 2. [`AssetSourcesPlugin`] — add *after* `DefaultPlugins`. Populates
//!    the embedded default-theme files into Bevy's already-built
//!    `EmbeddedAssetRegistry`.
//!
//! Both must run for the card-theme system to function. The doc
//! comments on each call out the pairing so a future reader doesn't
//! accidentally drop one half.

use bevy::asset::io::embedded::EmbeddedAssetRegistry;
use bevy::asset::io::file::FileAssetReader;
use bevy::asset::io::AssetSourceBuilder;
use bevy::asset::AssetApp;
use bevy::prelude::*;

use crate::assets::user_dir::user_theme_dir;

/// `AssetSourceId` of the user-themes asset source. Use it as
/// `themes://<theme_id>/theme.ron` from any code that wants to load
/// from the user-themes directory.
pub const USER_THEMES: &str = "themes";

/// Stable embedded asset URL of the bundled Dark theme manifest.
///
/// Code that wants to load the embedded Dark theme — including
/// `ActiveTheme` initialisation — should use exactly this constant
/// rather than re-typing the URL inline.
pub const DARK_THEME_MANIFEST_URL: &str =
    "embedded://solitaire_engine/assets/themes/dark/theme.ron";

/// Path the embedded Dark-theme manifest registers under, relative
/// to the `embedded://` source root. Kept in lockstep with
/// [`DARK_THEME_MANIFEST_URL`] by the unit test
/// `dark_theme_url_constant_matches_embedded_path`.
const DARK_THEME_MANIFEST_PATH: &str = "solitaire_engine/assets/themes/dark/theme.ron";

/// Bytes of the bundled Dark theme manifest, embedded at compile time.
const DARK_THEME_MANIFEST_BYTES: &[u8] =
    include_bytes!("../../assets/themes/dark/theme.ron");

/// Stable embedded asset URL of the bundled Classic theme manifest.
pub const CLASSIC_THEME_MANIFEST_URL: &str =
    "embedded://solitaire_engine/assets/themes/classic/theme.ron";

/// Path the embedded Classic-theme manifest registers under, relative
/// to the `embedded://` source root. Kept in lockstep with
/// [`CLASSIC_THEME_MANIFEST_URL`] by the unit test
/// `classic_theme_url_constant_matches_embedded_path`.
const CLASSIC_THEME_MANIFEST_PATH: &str = "solitaire_engine/assets/themes/classic/theme.ron";

/// Bytes of the bundled Classic theme manifest, embedded at compile time.
const CLASSIC_THEME_MANIFEST_BYTES: &[u8] =
    include_bytes!("../../assets/themes/classic/theme.ron");

/// Generates a `(stable_path, bytes)` entry for one Dark-theme SVG.
macro_rules! embed_dark_svg {
    ($name:literal) => {
        (
            concat!("solitaire_engine/assets/themes/dark/", $name),
            include_bytes!(concat!("../../assets/themes/dark/", $name)) as &[u8],
        )
    };
}

/// Generates a `(stable_path, bytes)` entry for one Classic-theme SVG.
macro_rules! embed_classic_svg {
    ($name:literal) => {
        (
            concat!("solitaire_engine/assets/themes/classic/", $name),
            include_bytes!(concat!("../../assets/themes/classic/", $name)) as &[u8],
        )
    };
}

/// Every Dark-theme SVG file bundled into the binary.
const DARK_THEME_SVGS: &[(&str, &[u8])] = &[
    embed_dark_svg!("back.svg"),
    embed_dark_svg!("clubs_ace.svg"),
    embed_dark_svg!("clubs_2.svg"),
    embed_dark_svg!("clubs_3.svg"),
    embed_dark_svg!("clubs_4.svg"),
    embed_dark_svg!("clubs_5.svg"),
    embed_dark_svg!("clubs_6.svg"),
    embed_dark_svg!("clubs_7.svg"),
    embed_dark_svg!("clubs_8.svg"),
    embed_dark_svg!("clubs_9.svg"),
    embed_dark_svg!("clubs_10.svg"),
    embed_dark_svg!("clubs_jack.svg"),
    embed_dark_svg!("clubs_queen.svg"),
    embed_dark_svg!("clubs_king.svg"),
    embed_dark_svg!("diamonds_ace.svg"),
    embed_dark_svg!("diamonds_2.svg"),
    embed_dark_svg!("diamonds_3.svg"),
    embed_dark_svg!("diamonds_4.svg"),
    embed_dark_svg!("diamonds_5.svg"),
    embed_dark_svg!("diamonds_6.svg"),
    embed_dark_svg!("diamonds_7.svg"),
    embed_dark_svg!("diamonds_8.svg"),
    embed_dark_svg!("diamonds_9.svg"),
    embed_dark_svg!("diamonds_10.svg"),
    embed_dark_svg!("diamonds_jack.svg"),
    embed_dark_svg!("diamonds_queen.svg"),
    embed_dark_svg!("diamonds_king.svg"),
    embed_dark_svg!("hearts_ace.svg"),
    embed_dark_svg!("hearts_2.svg"),
    embed_dark_svg!("hearts_3.svg"),
    embed_dark_svg!("hearts_4.svg"),
    embed_dark_svg!("hearts_5.svg"),
    embed_dark_svg!("hearts_6.svg"),
    embed_dark_svg!("hearts_7.svg"),
    embed_dark_svg!("hearts_8.svg"),
    embed_dark_svg!("hearts_9.svg"),
    embed_dark_svg!("hearts_10.svg"),
    embed_dark_svg!("hearts_jack.svg"),
    embed_dark_svg!("hearts_queen.svg"),
    embed_dark_svg!("hearts_king.svg"),
    embed_dark_svg!("spades_ace.svg"),
    embed_dark_svg!("spades_2.svg"),
    embed_dark_svg!("spades_3.svg"),
    embed_dark_svg!("spades_4.svg"),
    embed_dark_svg!("spades_5.svg"),
    embed_dark_svg!("spades_6.svg"),
    embed_dark_svg!("spades_7.svg"),
    embed_dark_svg!("spades_8.svg"),
    embed_dark_svg!("spades_9.svg"),
    embed_dark_svg!("spades_10.svg"),
    embed_dark_svg!("spades_jack.svg"),
    embed_dark_svg!("spades_queen.svg"),
    embed_dark_svg!("spades_king.svg"),
];

/// Every Classic-theme SVG file bundled into the binary.
const CLASSIC_THEME_SVGS: &[(&str, &[u8])] = &[
    embed_classic_svg!("back.svg"),
    embed_classic_svg!("clubs_ace.svg"),
    embed_classic_svg!("clubs_2.svg"),
    embed_classic_svg!("clubs_3.svg"),
    embed_classic_svg!("clubs_4.svg"),
    embed_classic_svg!("clubs_5.svg"),
    embed_classic_svg!("clubs_6.svg"),
    embed_classic_svg!("clubs_7.svg"),
    embed_classic_svg!("clubs_8.svg"),
    embed_classic_svg!("clubs_9.svg"),
    embed_classic_svg!("clubs_10.svg"),
    embed_classic_svg!("clubs_jack.svg"),
    embed_classic_svg!("clubs_queen.svg"),
    embed_classic_svg!("clubs_king.svg"),
    embed_classic_svg!("diamonds_ace.svg"),
    embed_classic_svg!("diamonds_2.svg"),
    embed_classic_svg!("diamonds_3.svg"),
    embed_classic_svg!("diamonds_4.svg"),
    embed_classic_svg!("diamonds_5.svg"),
    embed_classic_svg!("diamonds_6.svg"),
    embed_classic_svg!("diamonds_7.svg"),
    embed_classic_svg!("diamonds_8.svg"),
    embed_classic_svg!("diamonds_9.svg"),
    embed_classic_svg!("diamonds_10.svg"),
    embed_classic_svg!("diamonds_jack.svg"),
    embed_classic_svg!("diamonds_queen.svg"),
    embed_classic_svg!("diamonds_king.svg"),
    embed_classic_svg!("hearts_ace.svg"),
    embed_classic_svg!("hearts_2.svg"),
    embed_classic_svg!("hearts_3.svg"),
    embed_classic_svg!("hearts_4.svg"),
    embed_classic_svg!("hearts_5.svg"),
    embed_classic_svg!("hearts_6.svg"),
    embed_classic_svg!("hearts_7.svg"),
    embed_classic_svg!("hearts_8.svg"),
    embed_classic_svg!("hearts_9.svg"),
    embed_classic_svg!("hearts_10.svg"),
    embed_classic_svg!("hearts_jack.svg"),
    embed_classic_svg!("hearts_queen.svg"),
    embed_classic_svg!("hearts_king.svg"),
    embed_classic_svg!("spades_ace.svg"),
    embed_classic_svg!("spades_2.svg"),
    embed_classic_svg!("spades_3.svg"),
    embed_classic_svg!("spades_4.svg"),
    embed_classic_svg!("spades_5.svg"),
    embed_classic_svg!("spades_6.svg"),
    embed_classic_svg!("spades_7.svg"),
    embed_classic_svg!("spades_8.svg"),
    embed_classic_svg!("spades_9.svg"),
    embed_classic_svg!("spades_10.svg"),
    embed_classic_svg!("spades_jack.svg"),
    embed_classic_svg!("spades_queen.svg"),
    embed_classic_svg!("spades_king.svg"),
];

/// Registers asset sources that must be in place *before*
/// `AssetPlugin` is built.
///
/// In practice that means just `themes://`: `embedded://` is owned by
/// Bevy itself and is always registered by `AssetPlugin`. To finish
/// wiring up the embedded default theme, also add
/// [`AssetSourcesPlugin`] *after* `DefaultPlugins`.
///
/// Returns the `&mut App` so the call can be chained from the binary
/// entry point.
pub fn register_theme_asset_sources(app: &mut App) -> &mut App {
    let root = user_theme_dir();
    app.register_asset_source(
        USER_THEMES,
        AssetSourceBuilder::new(move || Box::new(FileAssetReader::new(root.clone()))),
    );
    app
}

/// Bevy `Plugin` that pushes the bundled default theme files into
/// [`EmbeddedAssetRegistry`].
///
/// Add this *after* `DefaultPlugins`. It pairs with
/// [`register_theme_asset_sources`] (which has to run *before*
/// `DefaultPlugins`); both are required for the card-theme system to
/// function. See the module-level doc comment for why the work has to
/// be split across two calls.
///
/// To bundle additional default-theme files (the SVG art slated to
/// land in a later phase), edit [`populate_embedded_default_theme`].
#[derive(Debug, Default)]
pub struct AssetSourcesPlugin;

impl Plugin for AssetSourcesPlugin {
    fn build(&self, app: &mut App) {
        populate_embedded_dark_theme(app);
        populate_embedded_classic_theme(app);
    }
}

/// Returns the embedded SVG bytes for a single Dark-theme file
/// (e.g. `"back.svg"` or `"spades_ace.svg"`), or `None` when the
/// filename is not bundled.
///
/// The thumbnail generator uses this to rasterise preview-sized art
/// without going through Bevy's async asset graph.
pub fn dark_theme_svg_bytes(filename: &str) -> Option<&'static [u8]> {
    let suffix = format!("/{filename}");
    DARK_THEME_SVGS
        .iter()
        .find(|(path, _)| path.ends_with(&suffix))
        .map(|(_, bytes)| *bytes)
}

/// Returns the manifest URL for a bundled (non-user) theme by id, or
/// `None` if `id` belongs to a user theme that lives under `themes://`.
///
/// Callers that need to resolve a theme URL without access to
/// [`crate::theme::ThemeRegistry`] (e.g. Startup systems where registry
/// ordering isn't guaranteed) should use this instead of constructing
/// the URL manually.
pub fn bundled_theme_url(id: &str) -> Option<&'static str> {
    match id {
        "dark" => Some(DARK_THEME_MANIFEST_URL),
        "classic" => Some(CLASSIC_THEME_MANIFEST_URL),
        _ => None,
    }
}

/// Returns the embedded SVG bytes for a single Classic-theme file
/// (e.g. `"back.svg"` or `"spades_ace.svg"`), or `None` when the
/// filename is not bundled.
pub fn classic_theme_svg_bytes(filename: &str) -> Option<&'static [u8]> {
    let suffix = format!("/{filename}");
    CLASSIC_THEME_SVGS
        .iter()
        .find(|(path, _)| path.ends_with(&suffix))
        .map(|(_, bytes)| *bytes)
}

/// Pushes every bundled Classic-theme file into the
/// [`EmbeddedAssetRegistry`] under its stable URL.
pub fn populate_embedded_classic_theme(app: &mut App) {
    let registry = app
        .world_mut()
        .get_resource_or_insert_with(EmbeddedAssetRegistry::default);

    registry.insert_asset(
        std::path::PathBuf::from(CLASSIC_THEME_MANIFEST_PATH),
        std::path::Path::new(CLASSIC_THEME_MANIFEST_PATH),
        CLASSIC_THEME_MANIFEST_BYTES,
    );

    for (path, bytes) in CLASSIC_THEME_SVGS {
        registry.insert_asset(
            std::path::PathBuf::from(*path),
            std::path::Path::new(*path),
            *bytes,
        );
    }
}

/// Pushes every bundled Dark-theme file into the
/// [`EmbeddedAssetRegistry`] under its stable URL.
pub fn populate_embedded_dark_theme(app: &mut App) {
    let registry = app
        .world_mut()
        .get_resource_or_insert_with(EmbeddedAssetRegistry::default);

    registry.insert_asset(
        std::path::PathBuf::from(DARK_THEME_MANIFEST_PATH),
        std::path::Path::new(DARK_THEME_MANIFEST_PATH),
        DARK_THEME_MANIFEST_BYTES,
    );

    for (path, bytes) in DARK_THEME_SVGS {
        registry.insert_asset(
            std::path::PathBuf::from(*path),
            std::path::Path::new(*path),
            *bytes,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `register_theme_asset_sources` must register `themes://`
    /// without panicking, even when `user_theme_dir()` resolves to a
    /// directory that doesn't exist on disk — Bevy's
    /// `FileAssetReader` constructs lazily, so a missing root is
    /// fine.
    #[test]
    fn register_theme_asset_sources_inserts_themes_source() {
        let mut app = App::new();
        register_theme_asset_sources(&mut app);

        let mut sources = app
            .world_mut()
            .get_resource_or_init::<bevy::asset::io::AssetSourceBuilders>();
        assert!(
            sources.get_mut(USER_THEMES).is_some(),
            "themes:// source not registered"
        );
    }

    #[test]
    fn populate_embedded_dark_theme_runs_without_asset_plugin() {
        let mut app = App::new();
        populate_embedded_dark_theme(&mut app);
        assert!(app
            .world()
            .get_resource::<EmbeddedAssetRegistry>()
            .is_some());
    }

    #[test]
    fn embedded_dark_theme_manifest_validates() {
        use crate::theme::ThemeManifest;

        let manifest: ThemeManifest = ron::de::from_bytes(DARK_THEME_MANIFEST_BYTES)
            .expect("dark manifest must parse as RON");
        let faces = manifest
            .validate()
            .expect("dark manifest must list all 52 faces");
        assert_eq!(faces.len(), 52);
    }

    #[test]
    fn dark_theme_svg_bytes_finds_back_and_ace_of_spades() {
        assert!(
            dark_theme_svg_bytes("back.svg").is_some(),
            "dark theme must bundle a back.svg"
        );
        assert!(
            dark_theme_svg_bytes("spades_ace.svg").is_some(),
            "dark theme must bundle a spades_ace.svg"
        );
    }

    #[test]
    fn dark_theme_svg_bytes_returns_none_for_unknown_file() {
        assert!(dark_theme_svg_bytes("nope.svg").is_none());
        assert!(dark_theme_svg_bytes("").is_none());
    }

    #[test]
    fn dark_theme_url_constant_matches_embedded_path() {
        let url_tail = DARK_THEME_MANIFEST_URL
            .strip_prefix("embedded://")
            .expect("dark theme URL must use embedded:// scheme");
        assert_eq!(url_tail, DARK_THEME_MANIFEST_PATH);
    }

    #[test]
    fn populate_embedded_classic_theme_runs_without_asset_plugin() {
        let mut app = App::new();
        populate_embedded_classic_theme(&mut app);
        assert!(app
            .world()
            .get_resource::<EmbeddedAssetRegistry>()
            .is_some());
    }

    #[test]
    fn embedded_classic_theme_manifest_validates() {
        use crate::theme::ThemeManifest;

        let manifest: ThemeManifest = ron::de::from_bytes(CLASSIC_THEME_MANIFEST_BYTES)
            .expect("classic manifest must parse as RON");
        let faces = manifest
            .validate()
            .expect("classic manifest must list all 52 faces");
        assert_eq!(faces.len(), 52);
    }

    #[test]
    fn classic_theme_svg_bytes_finds_back_and_ace_of_spades() {
        assert!(
            classic_theme_svg_bytes("back.svg").is_some(),
            "classic theme must bundle a back.svg"
        );
        assert!(
            classic_theme_svg_bytes("spades_ace.svg").is_some(),
            "classic theme must bundle a spades_ace.svg"
        );
    }

    #[test]
    fn classic_theme_svg_bytes_returns_none_for_unknown_file() {
        assert!(classic_theme_svg_bytes("nope.svg").is_none());
        assert!(classic_theme_svg_bytes("").is_none());
    }

    #[test]
    fn classic_theme_url_constant_matches_embedded_path() {
        let url_tail = CLASSIC_THEME_MANIFEST_URL
            .strip_prefix("embedded://")
            .expect("classic theme URL must use embedded:// scheme");
        assert_eq!(url_tail, CLASSIC_THEME_MANIFEST_PATH);
    }
}
