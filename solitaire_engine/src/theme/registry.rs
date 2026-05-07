//! Discovery and listing of available card themes.
//!
//! On startup the registry collects:
//!
//! - The bundled default theme — always present, served from
//!   `embedded://`.
//! - Every valid user-supplied theme found under
//!   [`crate::assets::user_theme_dir`] — one entry per immediate
//!   subdirectory whose `theme.ron` parses cleanly.
//!
//! The picker UI (Phase 6 acceptance: "dropping a valid theme folder
//! into the user themes dir makes it appear on next app start") reads
//! [`ThemeRegistry`] to populate its list of options.
//!
//! Per the plan, this only parses the `meta` block of each manifest —
//! we don't validate face/back paths here because (a) that work
//! already lives in [`super::manifest::ThemeManifest::validate`] and
//! [`super::loader::CardThemeLoader`], and (b) the registry should
//! surface entries quickly enough for a startup scan to feel free,
//! even with dozens of user themes installed.

use std::path::Path;

use bevy::prelude::{App, Plugin, Resource, Startup};
use serde::Deserialize;

use super::ThemeMeta;
use crate::assets::{user_theme_dir, DEFAULT_THEME_MANIFEST_URL};

/// One entry in the [`ThemeRegistry`] — the data the picker UI needs
/// to render a row and load the theme on selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeEntry {
    /// Stable identifier; matches `meta.id` from the manifest. For
    /// user themes this is also the directory name on disk; for the
    /// bundled default it is the literal string `"default"`.
    pub id: String,
    /// Human-readable label for the picker.
    pub display_name: String,
    /// Asset URL the picker passes to
    /// [`super::set_theme`] / `AssetServer::load`.
    pub manifest_url: String,
    /// The full meta block. Kept around so the picker can display
    /// author + version without a second round-trip through disk.
    pub meta: ThemeMeta,
}

/// Resource holding every theme available at app start.
///
/// The order is stable: default first, then user themes in the order
/// returned by [`std::fs::read_dir`] (filesystem-defined; usually
/// alphabetical on tested filesystems but not guaranteed by the OS).
#[derive(Resource, Debug, Default)]
pub struct ThemeRegistry {
    pub entries: Vec<ThemeEntry>,
}

impl ThemeRegistry {
    /// Returns the entry whose `id` matches, if any.
    pub fn find(&self, id: &str) -> Option<&ThemeEntry> {
        self.entries.iter().find(|e| e.id == id)
    }

    /// Iterator over every registered theme.
    pub fn iter(&self) -> impl Iterator<Item = &ThemeEntry> {
        self.entries.iter()
    }

    /// Number of registered themes (always ≥ 1 because the default
    /// entry is always inserted, even if user-theme discovery fails).
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True only when the default entry is missing — should never
    /// happen at runtime; provided for API completeness.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Bevy plugin that builds [`ThemeRegistry`] on startup.
pub struct ThemeRegistryPlugin;

impl Plugin for ThemeRegistryPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ThemeRegistry>()
            .add_systems(Startup, build_registry_on_startup);
    }
}

/// Reads `user_theme_dir()` and replaces the registry's contents with
/// the bundled default plus every valid user theme.
fn build_registry_on_startup(mut registry: bevy::ecs::system::ResMut<ThemeRegistry>) {
    *registry = build_registry(&user_theme_dir());
}

/// Pure helper: builds a registry given an explicit user-themes
/// directory. Tests pass a temp dir; production uses
/// [`user_theme_dir`].
pub fn build_registry(user_dir: &Path) -> ThemeRegistry {
    let mut entries = Vec::new();
    entries.push(default_entry());
    entries.extend(discover_user_themes(user_dir));
    ThemeRegistry { entries }
}

/// The bundled default theme entry — inserted unconditionally so the
/// picker always has at least one option.
fn default_entry() -> ThemeEntry {
    ThemeEntry {
        id: "default".to_string(),
        display_name: "Default".to_string(),
        manifest_url: DEFAULT_THEME_MANIFEST_URL.to_string(),
        meta: ThemeMeta {
            id: "default".to_string(),
            name: "Default".to_string(),
            author: "Solitaire Quest".to_string(),
            version: "1.0".to_string(),
            card_aspect: (2, 3),
            pixel_art: false,
        },
    }
}

/// Walks `user_dir`, treating every immediate subdirectory as a
/// candidate theme. A subdirectory contributes one entry if and only
/// if it contains a `theme.ron` whose `meta` block parses cleanly and
/// passes `ThemeMeta::validate`. Failed candidates are silently
/// skipped — broken themes don't poison discovery.
fn discover_user_themes(user_dir: &Path) -> Vec<ThemeEntry> {
    let mut out = Vec::new();
    let Ok(read) = std::fs::read_dir(user_dir) else {
        // Missing or unreadable user directory is the common case
        // before any theme is imported; treat it as "no themes" and
        // move on.
        return out;
    };

    for entry in read.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let manifest_path = path.join("theme.ron");
        if !manifest_path.is_file() {
            continue;
        }
        let Some(theme_entry) = read_meta_only(&manifest_path) else {
            continue;
        };
        out.push(theme_entry);
    }
    out
}

/// Partial deserialiser that only extracts `meta` from a theme
/// manifest. RON / serde silently skip unknown fields by default, so
/// this works against the full [`ThemeManifest`] schema without
/// having to load the 52 face paths or the back path.
#[derive(Deserialize)]
struct ManifestMetaOnly {
    meta: ThemeMeta,
}

/// Reads a single `theme.ron` and turns its `meta` block into a
/// [`ThemeEntry`]. Returns `None` for any I/O / parse / validation
/// failure — discovery is best-effort.
fn read_meta_only(manifest_path: &Path) -> Option<ThemeEntry> {
    let bytes = std::fs::read(manifest_path).ok()?;
    let parsed: ManifestMetaOnly = ron::de::from_bytes(&bytes).ok()?;
    parsed.meta.validate().ok()?;
    let id = parsed.meta.id.clone();
    let display_name = parsed.meta.name.clone();
    let manifest_url = format!("themes://{id}/theme.ron");
    Some(ThemeEntry {
        id,
        display_name,
        manifest_url,
        meta: parsed.meta,
    })
}

/// Refreshes [`ThemeRegistry`] in place — call after a successful
/// [`super::import_theme`] so the new theme is visible in the picker
/// without restarting the app.
pub fn refresh_registry(registry: &mut ThemeRegistry, user_dir: &Path) {
    *registry = build_registry(user_dir);
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn write_manifest(dir: &Path, id: &str, name: &str) {
        let manifest = format!(
            r#"(
    meta: (
        id: "{id}",
        name: "{name}",
        author: "tester",
        version: "1.0.0",
        card_aspect: (2, 3),
    ),
    back: "back.svg",
    faces: {{}},
)"#
        );
        fs::write(dir.join("theme.ron"), manifest).unwrap();
    }

    fn write_full_manifest(dir: &Path, id: &str, name: &str) {
        // A complete manifest with the 52 face entries and back.
        // Only used when a test specifically wants the full schema;
        // most discovery tests use the meta-only stub via
        // write_manifest above because the meta-only deserialiser
        // ignores the rest of the file anyway.
        let mut faces = String::new();
        for key in crate::theme::CardKey::all() {
            let mn = key.manifest_name();
            faces.push_str(&format!("        \"{mn}\": \"{mn}.svg\",\n"));
        }
        let manifest = format!(
            r#"(
    meta: (
        id: "{id}",
        name: "{name}",
        author: "tester",
        version: "1.0.0",
        card_aspect: (2, 3),
    ),
    back: "back.svg",
    faces: {{
{faces}    }},
)"#
        );
        fs::write(dir.join("theme.ron"), manifest).unwrap();
    }

    #[test]
    fn empty_user_dir_yields_only_the_default_entry() {
        let tmp = tempfile::tempdir().unwrap();
        let registry = build_registry(tmp.path());
        assert_eq!(registry.len(), 1);
        assert_eq!(registry.entries[0].id, "default");
    }

    #[test]
    fn nonexistent_user_dir_still_yields_default() {
        let registry = build_registry(Path::new(
            "/definitely/not/a/real/path/should/not/panic",
        ));
        assert_eq!(registry.len(), 1);
        assert_eq!(registry.entries[0].id, "default");
    }

    #[test]
    fn user_theme_with_valid_manifest_appears_in_registry() {
        let tmp = tempfile::tempdir().unwrap();
        let theme_dir = tmp.path().join("midnight");
        fs::create_dir_all(&theme_dir).unwrap();
        write_manifest(&theme_dir, "midnight", "Midnight");

        let registry = build_registry(tmp.path());
        assert_eq!(registry.len(), 2);
        let entry = registry.find("midnight").expect("midnight registered");
        assert_eq!(entry.display_name, "Midnight");
        assert_eq!(entry.manifest_url, "themes://midnight/theme.ron");
    }

    #[test]
    fn full_manifest_also_works_via_meta_only_parser() {
        // The meta-only deserialiser must tolerate the full ThemeManifest
        // schema without complaining about unknown fields.
        let tmp = tempfile::tempdir().unwrap();
        let theme_dir = tmp.path().join("full");
        fs::create_dir_all(&theme_dir).unwrap();
        write_full_manifest(&theme_dir, "full", "Full");

        let registry = build_registry(tmp.path());
        assert!(registry.find("full").is_some());
    }

    #[test]
    fn malformed_manifest_is_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let theme_dir = tmp.path().join("broken");
        fs::create_dir_all(&theme_dir).unwrap();
        fs::write(theme_dir.join("theme.ron"), "this is not valid ron").unwrap();

        // Plus a valid theme so we can confirm one bad apple doesn't
        // poison discovery.
        let good_dir = tmp.path().join("good");
        fs::create_dir_all(&good_dir).unwrap();
        write_manifest(&good_dir, "good", "Good Theme");

        let registry = build_registry(tmp.path());
        assert!(registry.find("broken").is_none());
        assert!(registry.find("good").is_some());
    }

    #[test]
    fn manifest_with_invalid_meta_is_skipped() {
        // id with a path separator violates ThemeMeta::validate.
        let tmp = tempfile::tempdir().unwrap();
        let theme_dir = tmp.path().join("escape");
        fs::create_dir_all(&theme_dir).unwrap();
        write_manifest(&theme_dir, "../etc/passwd", "Evil");

        let registry = build_registry(tmp.path());
        assert_eq!(registry.len(), 1, "escape attempt must not register");
        assert_eq!(registry.entries[0].id, "default");
    }

    #[test]
    fn directory_without_theme_ron_is_ignored() {
        let tmp = tempfile::tempdir().unwrap();
        let lonely = tmp.path().join("no-manifest-here");
        fs::create_dir_all(&lonely).unwrap();
        fs::write(lonely.join("readme.md"), "wrong filename").unwrap();

        let registry = build_registry(tmp.path());
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn find_returns_none_for_unknown_id() {
        let registry = build_registry(Path::new("/nonexistent"));
        assert!(registry.find("definitely-not-a-theme").is_none());
    }

    #[test]
    fn refresh_replaces_existing_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let mut registry = ThemeRegistry::default();
        registry.entries.push(ThemeEntry {
            id: "stale".into(),
            display_name: "Stale".into(),
            manifest_url: "themes://stale/theme.ron".into(),
            meta: ThemeMeta {
                id: "stale".into(),
                name: "Stale".into(),
                author: "x".into(),
                version: "x".into(),
                card_aspect: (2, 3),
                pixel_art: false,
            },
        });

        refresh_registry(&mut registry, tmp.path());

        assert_eq!(registry.len(), 1);
        assert_eq!(registry.entries[0].id, "default");
        assert!(registry.find("stale").is_none());
    }

    #[test]
    fn default_entry_url_matches_embedded_constant() {
        // Ensures the picker always gets a URL it can hand to the
        // asset server for the bundled theme.
        let entry = default_entry();
        assert_eq!(entry.manifest_url, DEFAULT_THEME_MANIFEST_URL);
    }
}
