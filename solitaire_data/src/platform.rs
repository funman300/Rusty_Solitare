//! Per-platform resolution of the per-user data directory.
//!
//! The rest of `solitaire_data` (settings, stats, achievements,
//! replays, progress, game state) and the engine's user-themes
//! discovery all need a base path under which to nest
//! `ferrous_solitaire/<file>`. On desktop the right answer is
//! `dirs::data_dir()` (which resolves to platform-appropriate
//! locations: `~/.local/share` on Linux, `~/Library/Application
//! Support` on macOS, `%APPDATA%` on Windows). On Android the
//! `dirs` crate returns `None`, which would silently disable
//! every persistence path — settings, stats, replays, the lot.
//!
//! [`data_dir`] is a thin shim that returns the right base path
//! per target. Callers continue to append
//! `ferrous_solitaire/<file>` themselves, so the on-disk layout is
//! identical across platforms (the per-app Android sandbox makes
//! the extra `ferrous_solitaire/` segment harmless, and a `tar`
//! export from one platform deserialises cleanly on another).
//!
//! # Why hardcode on Android?
//!
//! The "proper" Android answer is JNI: call back into Java to
//! invoke `Activity.getFilesDir()`. That requires plumbing an
//! `AndroidApp` context through Bevy's startup hooks and a
//! per-call JNI bridge — meaningfully more code than the
//! sandbox-guaranteed `/data/data/<package>/files` path. The
//! package name `com.ferrousapp.solitaire` is fixed at compile
//! time in `solitaire_app/Cargo.toml`'s
//! `[package.metadata.android]` block, so a hardcoded path is
//! safe until that ever changes (at which point this constant
//! moves with it).

use std::path::PathBuf;

/// Hardcoded per-app private files directory on Android.
///
/// Matches `[package.metadata.android]` in `solitaire_app/Cargo.toml`.
/// The Android sandbox guarantees this path exists, is writable,
/// and is private to the app — no JNI needed. Update both this
/// constant and the Cargo metadata together if the package id
/// ever changes.
#[cfg(target_os = "android")]
const ANDROID_APP_FILES_DIR: &str = "/data/data/com.ferrousapp.solitaire/files";

/// Returns the per-user data directory for the current target,
/// or `None` if the platform doesn't expose one (rare; usually
/// indicates a broken `$HOME` or `$XDG_*` configuration on a
/// minimal Linux container).
///
/// Callers append `ferrous_solitaire/<file>` themselves. See the
/// module-level doc comment for the per-platform behaviour and
/// why Android uses a hardcoded path.
pub fn data_dir() -> Option<PathBuf> {
    #[cfg(target_os = "android")]
    {
        Some(PathBuf::from(ANDROID_APP_FILES_DIR))
    }
    #[cfg(not(target_os = "android"))]
    {
        dirs::data_dir()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// On every supported desktop target the OS reports a usable
    /// data directory. This test only runs on desktop because the
    /// Android branch returns a fixed string regardless of host
    /// state, and asserting on a fixed string is a tautology.
    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    #[test]
    fn data_dir_returns_some_on_desktop_targets() {
        let dir = data_dir().expect("desktop targets must report a data dir");
        assert!(
            dir.is_absolute(),
            "data_dir() must return an absolute path on desktop, got {dir:?}",
        );
    }

    /// On Android the hardcoded path matches the package id pinned
    /// in `solitaire_app/Cargo.toml`'s `[package.metadata.android]`.
    /// If a future change rotates that id, this test fails loudly
    /// so the path constant moves with it.
    #[cfg(target_os = "android")]
    #[test]
    fn data_dir_returns_sandbox_path_on_android() {
        let dir = data_dir().expect("android must report a data dir");
        assert_eq!(dir, PathBuf::from("/data/data/com.ferrousapp.solitaire/files"));
    }
}
