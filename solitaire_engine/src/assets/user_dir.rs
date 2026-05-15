//! Per-platform resolution of the user-themes directory.
//!
//! The path is determined exactly once and exposed via
//! [`user_theme_dir`]. The base directory comes from
//! [`solitaire_data::data_dir`] (desktop: `dirs::data_dir()`;
//! Android: the hardcoded `/data/data/<package>/files` sandbox
//! path). Mobile entry points may still override the path via
//! [`set_user_theme_dir`] when they need to point at a non-default
//! location (e.g. tests, custom AssetManager wiring).
//!
//! # Why panic instead of returning Result?
//!
//! User-theme resolution is bootstrap-time configuration, not game
//! logic, so per CLAUDE.md panics are acceptable here. Returning
//! `Result` would force every caller (the registry, the asset source,
//! the importer) to plumb an error through systems that have no
//! recovery path: there is no useful state to display if we can't
//! find the user themes directory at all.

use std::path::PathBuf;
use std::sync::OnceLock;

/// Override slot populated by mobile entry points (Android's
/// `android_main`, iOS's launch handler) before the Bevy `App` starts.
/// Desktop platforms ignore the override and fall through to
/// [`desktop_theme_dir`].
static USER_THEME_DIR_OVERRIDE: OnceLock<PathBuf> = OnceLock::new();

/// Sub-folder under `dirs::data_dir()` where the project keeps every
/// per-user file. Matches the existing convention used by
/// `solitaire_data` for `settings.json`, `stats.json`, etc.
const APP_DIR_NAME: &str = "ferrous_solitaire";

/// Sub-folder under [`APP_DIR_NAME`] dedicated to user themes.
const THEME_DIR_NAME: &str = "themes";

/// Sets the user-themes directory at runtime — escape hatch for
/// embedders or tests that need to override the platform default.
///
/// Returns `Err` containing the rejected path if the override has
/// already been set. The first caller wins and subsequent calls are
/// silently a no-op-with-feedback so a mis-configured embedder can't
/// flip the path mid-session.
///
/// Mostly unnecessary now that [`solitaire_data::data_dir`] handles
/// every supported target — the override is kept for tests and for
/// embedders that want a non-default location (e.g. a sandboxed
/// AssetManager root on a future iOS port).
pub fn set_user_theme_dir(path: PathBuf) -> Result<(), PathBuf> {
    USER_THEME_DIR_OVERRIDE.set(path)
}

/// Returns the absolute path of the user-themes directory on the
/// current platform.
///
/// # Panics
///
/// Panics if [`solitaire_data::data_dir`] returns `None`, which on
/// desktop indicates a broken `$HOME` / `$XDG_*` configuration.
/// Android always returns `Some`. The panic message names the
/// supported workaround ([`set_user_theme_dir`]).
pub fn user_theme_dir() -> PathBuf {
    if let Some(p) = USER_THEME_DIR_OVERRIDE.get() {
        return p.clone();
    }
    user_theme_dir_for(detected_platform_data_dir())
}

/// Composition helper that takes the platform data dir as input so the
/// pure path-joining behaviour is unit-testable without depending on
/// the user's actual `$HOME`.
fn user_theme_dir_for(data_dir: PathBuf) -> PathBuf {
    data_dir.join(APP_DIR_NAME).join(THEME_DIR_NAME)
}

/// Per-target-os resolution of the platform's data dir. Delegates
/// to [`solitaire_data::data_dir`] which encapsulates the
/// per-target shape (desktop: `dirs::data_dir()`; android: the
/// hardcoded `/data/data/<package>/files` sandbox path). Panics
/// only when the underlying resolver returns `None`, which on
/// desktop indicates a broken `$HOME` / `$XDG_*` configuration —
/// the panic message names the supported workaround.
fn detected_platform_data_dir() -> PathBuf {
    solitaire_data::data_dir().unwrap_or_else(|| {
        panic!(
            "user_theme_dir(): platform data directory is unavailable. \
             On Linux check $XDG_DATA_HOME or $HOME; on macOS / Windows \
             the OS reported no Application Support / AppData path. \
             As a workaround call solitaire_engine::assets::user_dir::\
             set_user_theme_dir() before App::run()."
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_theme_dir_for_appends_ferrous_solitaire_themes() {
        let dir = user_theme_dir_for(PathBuf::from("/tmp/data"));
        assert_eq!(
            dir,
            PathBuf::from("/tmp/data/ferrous_solitaire/themes"),
            "user dir must nest under ferrous_solitaire/themes"
        );
    }

    #[test]
    fn user_theme_dir_for_handles_empty_root() {
        let dir = user_theme_dir_for(PathBuf::new());
        assert_eq!(dir, PathBuf::from("ferrous_solitaire/themes"));
    }

    #[test]
    fn detected_data_dir_yields_a_path_with_a_parent() {
        // On every supported target the platform resolver
        // (`solitaire_data::data_dir`) returns a usable directory:
        // desktop targets via `dirs::data_dir()` (the test machine
        // already has a `$HOME` for it to discover), Android via
        // the hardcoded `/data/data/<package>/files` sandbox path.
        // We don't pin the exact value because it depends on the
        // user's `$HOME` on desktop, but it must at least be a
        // non-empty path with a parent component.
        let dir = detected_platform_data_dir();
        assert!(dir.parent().is_some(), "data dir {dir:?} should be absolute");
    }

    // The OnceLock-based override is intentionally NOT covered here:
    // setting it once would pollute every subsequent test in the
    // process that called `user_theme_dir()`. The override's
    // first-write-wins semantics come from `std::sync::OnceLock` which
    // is already well-tested upstream; the behaviour we add on top is
    // a trivial early-return that's covered by code review.
}
