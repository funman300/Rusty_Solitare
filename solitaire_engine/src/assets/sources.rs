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

/// Stable embedded asset URL of the bundled default theme manifest.
///
/// Code that wants to load the embedded default — including the future
/// Phase 4 `ActiveTheme` initialisation — should use exactly this
/// constant rather than re-typing the URL inline. Changing where the
/// default theme lives in the asset graph then becomes a single-line
/// change in this file.
pub const DEFAULT_THEME_MANIFEST_URL: &str =
    "embedded://solitaire_engine/assets/themes/default/theme.ron";

/// Path the embedded default-theme manifest registers under, relative
/// to the `embedded://` source root. Kept in lockstep with
/// [`DEFAULT_THEME_MANIFEST_URL`] by the unit test
/// `default_theme_url_constant_matches_embedded_path`.
const DEFAULT_THEME_MANIFEST_PATH: &str = "solitaire_engine/assets/themes/default/theme.ron";

/// Bytes of the bundled default theme manifest. Embedded at compile
/// time via `include_bytes!` so the binary is self-contained even if
/// the workspace's `solitaire_engine/assets/` directory is absent at
/// runtime (e.g. when shipped to a player).
const DEFAULT_THEME_MANIFEST_BYTES: &[u8] =
    include_bytes!("../../assets/themes/default/theme.ron");

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
        populate_embedded_default_theme(app);
    }
}

/// Pushes every bundled default-theme file into the
/// [`EmbeddedAssetRegistry`] under its stable URL. Keeping this in a
/// free function (and not inside the `Plugin::build` body) means the
/// unit test below can exercise it without spinning up a full Bevy
/// `App` with `AssetPlugin`.
///
/// **Adding files to the bundled default theme** is a single edit
/// per file: add an `include_bytes!` constant that points at the file
/// under `solitaire_engine/assets/themes/default/`, then add a
/// matching `registry.insert_asset(...)` call here. Keep the
/// `asset_path` argument exactly the relative path that the manifest
/// references (e.g. `solitaire_engine/assets/themes/default/back.svg`).
pub fn populate_embedded_default_theme(app: &mut App) {
    let registry = app
        .world_mut()
        .get_resource_or_insert_with(EmbeddedAssetRegistry::default);

    // `full_path` is only consulted by the optional
    // `embedded_watcher` cargo feature (which we don't enable). Use
    // the manifest's logical workspace path so a future debugger
    // session sees a sensible source-of-truth string.
    registry.insert_asset(
        std::path::PathBuf::from(DEFAULT_THEME_MANIFEST_PATH),
        std::path::Path::new(DEFAULT_THEME_MANIFEST_PATH),
        DEFAULT_THEME_MANIFEST_BYTES,
    );
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

    /// `populate_embedded_default_theme` must work as a drop-in step
    /// regardless of whether `EmbeddedAssetRegistry` already exists,
    /// so it can be called both from `AssetSourcesPlugin::build`
    /// (after `AssetPlugin` initialised it) and from this test (which
    /// uses the resource's `get_resource_or_insert_with` fallback).
    #[test]
    fn populate_embedded_default_theme_runs_without_asset_plugin() {
        let mut app = App::new();
        populate_embedded_default_theme(&mut app);

        // Resource exists and has been inserted into.
        assert!(app
            .world()
            .get_resource::<EmbeddedAssetRegistry>()
            .is_some());
    }

    /// The bundled default theme stub must satisfy
    /// `ThemeManifest::validate` — otherwise the embedded source
    /// would register a manifest the loader will then reject at
    /// runtime.
    #[test]
    fn embedded_default_theme_manifest_validates() {
        use crate::theme::ThemeManifest;

        let manifest: ThemeManifest = ron::de::from_bytes(DEFAULT_THEME_MANIFEST_BYTES)
            .expect("default manifest must parse as RON");
        let faces = manifest
            .validate()
            .expect("default manifest must list all 52 faces");
        assert_eq!(faces.len(), 52);
    }

    /// Belt-and-braces: if anyone edits `DEFAULT_THEME_MANIFEST_PATH`
    /// without updating `DEFAULT_THEME_MANIFEST_URL` (or vice versa)
    /// the asset would register at one path and be loaded from
    /// another. Pin them together in the test suite so any drift
    /// fails CI.
    #[test]
    fn default_theme_url_constant_matches_embedded_path() {
        let url_tail = DEFAULT_THEME_MANIFEST_URL
            .strip_prefix("embedded://")
            .expect("default theme URL must use embedded:// scheme");
        assert_eq!(url_tail, DEFAULT_THEME_MANIFEST_PATH);
    }
}
