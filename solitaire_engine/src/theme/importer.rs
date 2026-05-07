//! Theme zip-archive importer.
//!
//! Phase 7 of the card-theme system (see `CARD_PLAN.md`). Players ship
//! and install third-party themes as a single `.zip` containing a
//! `theme.ron` manifest at the archive root plus the 52 face SVGs and
//! one back SVG referenced by that manifest. [`import_theme`] is the
//! one-shot entry point: it opens the zip, validates structurally,
//! then atomically unpacks into [`crate::assets::user_theme_dir`] —
//! never touching the user's themes directory unless every check
//! passes.
//!
//! # Safety guarantees
//!
//! - **Hard size cap.** The total declared archive size must not
//!   exceed 20 MB. The cap is checked from the central directory
//!   (i.e. before any extraction) so a zip-bomb cannot run us out of
//!   memory or disk just by pretending to be small.
//! - **Zip-slip immune.** Every entry path is normalised and rejected
//!   if it contains `..`, an absolute prefix, or any non-`Normal`
//!   component. We never trust the OS to clamp `..` for us.
//! - **Manifest-driven.** Only paths the manifest declares are
//!   extracted; every face/back path is also rasterised through
//!   [`crate::assets::rasterize_svg`] as a structural validity check
//!   so corrupt SVGs surface here, not in the asset graph.
//! - **Atomic install.** Extraction goes to a sibling temp directory
//!   that's renamed into place only after every byte has been
//!   written. A failed import leaves the user themes dir untouched.
//! - **No id collision.** If the manifest's `meta.id` already names a
//!   directory in the user themes root, we abort before writing.
//!   Replacing an existing theme is a deliberate user action handled
//!   by Phase 6's registry, not a side effect of importing.
//!
//! # Testing hook
//!
//! Tests target [`import_theme_into`] directly so they can pass a
//! `tempfile::TempDir` as the destination root. The public
//! [`import_theme`] is a thin wrapper that resolves the destination
//! via [`crate::assets::user_theme_dir`].

use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};

use thiserror::Error;

use bevy::math::UVec2;

use crate::assets::{rasterize_svg, user_theme_dir, SvgLoaderError};

use super::manifest::{ManifestError, ThemeManifest};
use super::ThemeMetaError;

/// Hard cap on the *uncompressed* total of all archive entries. Set
/// generously high relative to a realistic 53-SVG theme (~1–2 MB at
/// most for vector content) but firmly below anything we'd risk
/// extracting blind.
pub const MAX_ARCHIVE_BYTES: u64 = 20 * 1024 * 1024;

/// Tiny rasterisation target used purely to validate SVG structure.
/// The actual asset loader picks a real size at load time; we just
/// need `usvg` + `resvg` to accept the bytes here.
const SVG_VALIDATION_SIZE: UVec2 = UVec2::new(64, 96);

/// Filename of the manifest at the archive root. Must match what
/// `CardThemeLoader::extensions()` advertises so the same artefact
/// works for both import and load.
const MANIFEST_NAME: &str = "theme.ron";

/// Strongly-typed wrapper around a successfully imported theme's
/// manifest id. Consumers (notably the Phase 6 registry refresh)
/// receive this so they can route directly to the new theme without
/// re-parsing the manifest off disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeId(pub String);

impl ThemeId {
    /// Borrow the underlying id as `&str` for path joining and lookups.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Errors surfaced by [`import_theme`] / [`import_theme_into`].
///
/// Each variant pinpoints exactly which check rejected the archive
/// — the importer never lumps unrelated failures into a single
/// generic error so the caller can render a precise message in the UI.
#[derive(Debug, Error)]
pub enum ImportError {
    /// The zip file could not be opened or its central directory was
    /// unreadable. `zip::result::ZipError` covers both I/O failures
    /// and structurally corrupt archives.
    #[error("could not open zip archive: {0}")]
    OpenArchive(#[from] zip::result::ZipError),

    /// Filesystem failure outside of zip parsing (creating the temp
    /// dir, writing extracted bytes, the final rename, etc).
    #[error("io error: {0}")]
    Io(#[from] io::Error),

    /// The archive's declared total uncompressed size exceeds
    /// [`MAX_ARCHIVE_BYTES`]. Checked *before* extraction.
    #[error(
        "archive declares {total} uncompressed bytes, exceeds the {limit}-byte limit"
    )]
    Oversized { total: u64, limit: u64 },

    /// No `theme.ron` at the archive root.
    #[error("archive does not contain `theme.ron` at the root")]
    MissingManifest,

    /// `theme.ron` present but couldn't be parsed as RON.
    #[error("manifest parse (RON): {0}")]
    ManifestParse(#[from] ron::error::SpannedError),

    /// Manifest parsed but failed structural validation. Wraps the
    /// 52-faces / unknown-key / duplicate-key diagnostics from
    /// [`super::manifest`].
    #[error("manifest validation: {0}")]
    Validation(#[from] ManifestError),

    /// Manifest's `meta` block failed validation in isolation. Kept
    /// distinct from the above so callers can branch on metadata-only
    /// problems (id shape, aspect zero, etc.) when wanted.
    #[error("manifest meta: {0}")]
    Meta(#[from] ThemeMetaError),

    /// A face or back path declared in the manifest is not present in
    /// the zip's entry list.
    #[error("manifest references file not present in archive: {missing}")]
    MissingFile { missing: String },

    /// One of the referenced SVGs failed to rasterise — `usvg`
    /// rejected it, the bytes were truncated, etc.
    #[error("invalid SVG content for {path}: {source}")]
    InvalidSvg {
        path: String,
        #[source]
        source: SvgLoaderError,
    },

    /// A zip entry's normalised path escapes the archive root (zip
    /// slip): contains `..`, is absolute, or names something other
    /// than a normal file component.
    #[error("zip-slip path traversal attempt: {path}")]
    ZipSlip { path: String },

    /// The manifest's `meta.id` already names a directory under the
    /// user themes root.
    #[error("a theme with id {id:?} is already installed")]
    IdCollision { id: String },
}

/// Imports a theme zip into the per-platform user themes directory
/// resolved by [`crate::assets::user_theme_dir`].
///
/// Returns the imported theme's manifest id on success. The Phase 6
/// registry is responsible for refreshing its in-memory list — this
/// function only writes to disk.
///
/// See the module-level docs for the full safety contract.
pub fn import_theme(zip_path: &Path) -> Result<ThemeId, ImportError> {
    import_theme_into(zip_path, &user_theme_dir())
}

/// Same as [`import_theme`] but takes the destination root explicitly.
///
/// Tests use this directly with a `tempfile::TempDir` so they can
/// exercise the full extraction path without touching the global
/// [`crate::assets::user_dir::set_user_theme_dir`] override.
pub fn import_theme_into(
    zip_path: &Path,
    target_root: &Path,
) -> Result<ThemeId, ImportError> {
    let file = File::open(zip_path)?;
    let mut archive = zip::ZipArchive::new(file)?;

    enforce_archive_size_limit(&mut archive)?;
    enforce_zip_slip_safe(&mut archive)?;

    // Parse + validate the manifest in-memory. Anything below this
    // line works against a known-good `ThemeManifest`.
    let manifest = read_manifest(&mut archive)?;
    let face_paths = manifest.validate()?;

    // Confirm every referenced SVG is in the archive AND structurally
    // valid before we commit to writing anything to disk.
    let mut required: Vec<PathBuf> = face_paths.values().cloned().collect();
    required.push(manifest.back.clone());
    for path in &required {
        let bytes = read_archive_entry(&mut archive, path)?;
        rasterize_svg(&bytes, SVG_VALIDATION_SIZE).map_err(|source| {
            ImportError::InvalidSvg {
                path: path.to_string_lossy().into_owned(),
                source,
            }
        })?;
    }

    let id = manifest.meta.id.clone();
    let final_dir = target_root.join(&id);
    if final_dir.exists() {
        return Err(ImportError::IdCollision { id });
    }

    // Stage the extraction in a sibling temp dir so a partial write
    // never reaches `final_dir`. We rename on success; on failure
    // we wipe the staging dir without ever touching `final_dir`.
    fs::create_dir_all(target_root)?;
    let staging = target_root.join(format!(".{id}.tmp"));
    if staging.exists() {
        // Leftover from a previous crashed import; safe to remove
        // because it never made the rename.
        fs::remove_dir_all(&staging)?;
    }
    fs::create_dir_all(&staging)?;

    let extract_result = (|| -> Result<(), ImportError> {
        // Always extract the manifest under its canonical name.
        write_archive_entry(&mut archive, MANIFEST_NAME, &staging)?;
        for path in &required {
            write_archive_entry_pathbuf(&mut archive, path, &staging)?;
        }
        Ok(())
    })();

    match extract_result {
        Ok(()) => match install_atomic(&staging, &final_dir) {
            Ok(()) => Ok(ThemeId(id)),
            Err(e) => {
                // Best-effort cleanup; preserve the original error.
                let _ = fs::remove_dir_all(&staging);
                Err(e)
            }
        },
        Err(e) => {
            let _ = fs::remove_dir_all(&staging);
            Err(e)
        }
    }
}

/// Sums every entry's declared uncompressed size and rejects archives
/// that overflow [`MAX_ARCHIVE_BYTES`]. Iterates the central
/// directory only — does not actually decompress anything.
fn enforce_archive_size_limit<R: io::Read + io::Seek>(
    archive: &mut zip::ZipArchive<R>,
) -> Result<(), ImportError> {
    let mut total: u64 = 0;
    for i in 0..archive.len() {
        let entry = archive.by_index_raw(i)?;
        total = total.saturating_add(entry.size());
        if total > MAX_ARCHIVE_BYTES {
            return Err(ImportError::Oversized {
                total,
                limit: MAX_ARCHIVE_BYTES,
            });
        }
    }
    Ok(())
}

/// Walks every entry name and rejects the archive if any path
/// (after normalisation) escapes its root. Catches `..`, absolute
/// paths, drive prefixes on Windows, and the awkward case where
/// `enclosed_name` returns `None` because the entry is suspicious.
fn enforce_zip_slip_safe<R: io::Read + io::Seek>(
    archive: &mut zip::ZipArchive<R>,
) -> Result<(), ImportError> {
    for i in 0..archive.len() {
        let entry = archive.by_index_raw(i)?;
        let name = entry.name().to_owned();
        let normalised = entry
            .enclosed_name()
            .ok_or_else(|| ImportError::ZipSlip { path: name.clone() })?;
        if !is_safe_relative_path(&normalised) {
            return Err(ImportError::ZipSlip { path: name });
        }
    }
    Ok(())
}

/// True iff `p` is a relative path consisting only of `Normal`
/// components — no root, no prefix, no `.` or `..`. The zip crate's
/// `enclosed_name` already strips most attacks; this is belt and
/// braces.
fn is_safe_relative_path(p: &Path) -> bool {
    if p.is_absolute() {
        return false;
    }
    p.components()
        .all(|c| matches!(c, Component::Normal(_)))
}

/// Reads `theme.ron` from the archive root and parses it.
///
/// Errors:
/// - [`ImportError::MissingManifest`] when the archive has no
///   `theme.ron` entry at its root.
/// - [`ImportError::ManifestParse`] when the bytes don't form valid
///   RON for `ThemeManifest`.
fn read_manifest<R: io::Read + io::Seek>(
    archive: &mut zip::ZipArchive<R>,
) -> Result<ThemeManifest, ImportError> {
    // We can't use `?` directly across `by_name` because a missing
    // file is a domain-level error here, not a generic ZipError.
    let bytes = match archive.by_name(MANIFEST_NAME) {
        Ok(mut entry) => {
            let mut buf = Vec::with_capacity(entry.size() as usize);
            entry.read_to_end(&mut buf)?;
            buf
        }
        Err(zip::result::ZipError::FileNotFound) => {
            return Err(ImportError::MissingManifest);
        }
        Err(e) => return Err(ImportError::OpenArchive(e)),
    };

    let manifest: ThemeManifest = ron::de::from_bytes(&bytes)?;
    Ok(manifest)
}

/// Reads a manifest-declared entry's bytes by `Path`, normalising the
/// separators so a manifest authored on Windows still resolves on
/// Unix and vice versa.
///
/// Returns [`ImportError::MissingFile`] when the archive has no entry
/// matching the path.
fn read_archive_entry<R: io::Read + io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    path: &Path,
) -> Result<Vec<u8>, ImportError> {
    let key = archive_key(path);
    let mut entry = match archive.by_name(&key) {
        Ok(e) => e,
        Err(zip::result::ZipError::FileNotFound) => {
            return Err(ImportError::MissingFile { missing: key });
        }
        Err(e) => return Err(ImportError::OpenArchive(e)),
    };
    let mut buf = Vec::with_capacity(entry.size() as usize);
    entry.read_to_end(&mut buf)?;
    Ok(buf)
}

/// Joins manifest-declared sub-paths with the archive's root using
/// forward slashes — zip uses `/` on every platform.
fn archive_key(path: &Path) -> String {
    path.components()
        .filter_map(|c| match c {
            Component::Normal(s) => s.to_str(),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// Extracts a single entry under the staging directory, creating any
/// parent directories as needed. The destination path is rebuilt from
/// the safe components we already vetted in
/// [`enforce_zip_slip_safe`], not from the raw entry name.
fn write_archive_entry<R: io::Read + io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    name: &str,
    staging: &Path,
) -> Result<(), ImportError> {
    let mut entry = match archive.by_name(name) {
        Ok(e) => e,
        Err(zip::result::ZipError::FileNotFound) => {
            return Err(ImportError::MissingFile {
                missing: name.to_owned(),
            });
        }
        Err(e) => return Err(ImportError::OpenArchive(e)),
    };
    let safe = entry
        .enclosed_name()
        .ok_or_else(|| ImportError::ZipSlip {
            path: name.to_owned(),
        })?;
    if !is_safe_relative_path(&safe) {
        return Err(ImportError::ZipSlip {
            path: name.to_owned(),
        });
    }
    let dest = staging.join(&safe);
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut out = File::create(&dest)?;
    io::copy(&mut entry, &mut out)?;
    Ok(())
}

/// Variant of [`write_archive_entry`] keyed by `Path` for the
/// manifest-declared face/back paths.
fn write_archive_entry_pathbuf<R: io::Read + io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    path: &Path,
    staging: &Path,
) -> Result<(), ImportError> {
    let key = archive_key(path);
    write_archive_entry(archive, &key, staging)
}

/// Promotes the staging directory to its final location.
///
/// `fs::rename` is atomic when both paths share a filesystem; if not
/// (e.g. `/tmp` on a tmpfs vs. the user data dir on disk) we fall
/// back to a recursive copy + remove so the import still completes.
fn install_atomic(staging: &Path, final_dir: &Path) -> Result<(), ImportError> {
    match fs::rename(staging, final_dir) {
        Ok(()) => Ok(()),
        Err(rename_err) => {
            // EXDEV (cross-device link) is the canonical case for
            // falling back; other rename errors are surfaced after
            // the fallback so the user sees the original cause if
            // copy also fails.
            copy_dir_recursive(staging, final_dir).map_err(|copy_err| {
                ImportError::Io(io::Error::new(
                    copy_err.kind(),
                    format!(
                        "cross-device install fallback failed: rename={rename_err}, copy={copy_err}"
                    ),
                ))
            })?;
            // Best-effort: if removing the staging dir fails the
            // import is still a success — the user has the theme.
            let _ = fs::remove_dir_all(staging);
            Ok(())
        }
    }
}

/// Plain recursive copy used by [`install_atomic`] when `rename`
/// can't cross filesystem boundaries.
fn copy_dir_recursive(src: &Path, dst: &Path) -> io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else {
            fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::HashMap;
    use std::io::Write;

    use tempfile::TempDir;
    use zip::write::SimpleFileOptions;
    use zip::CompressionMethod;

    use crate::theme::manifest::ThemeManifest;
    use crate::theme::{CardKey, ThemeMeta};

    /// Smallest non-trivial SVG that round-trips through `usvg` /
    /// `resvg`. Matches the fixture used by `svg_loader::tests`.
    const TEST_SVG: &[u8] = br##"<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 200 300" width="200" height="300">
  <rect x="0" y="0" width="200" height="300" fill="#FFD23F"/>
  <circle cx="100" cy="150" r="80" fill="#1A0F2E"/>
</svg>"##;

    fn meta(id: &str) -> ThemeMeta {
        ThemeMeta {
            id: id.to_owned(),
            name: "Test Theme".into(),
            author: "Tester".into(),
            version: "1.0.0".into(),
            card_aspect: (2, 3),
            pixel_art: false,
        }
    }

    /// Returns a manifest that maps every card and the back to a
    /// per-card SVG path inside the archive root.
    fn full_manifest(id: &str) -> ThemeManifest {
        let faces: HashMap<String, PathBuf> = CardKey::all()
            .map(|k| {
                let name = k.manifest_name();
                (name.clone(), PathBuf::from(format!("faces/{name}.svg")))
            })
            .collect();
        ThemeManifest {
            meta: meta(id),
            back: PathBuf::from("back.svg"),
            faces,
        }
    }

    /// Writes a zip archive at `zip_path` containing every entry in
    /// `entries` (path → bytes). Uses `Stored` so tests don't pull
    /// the deflate codepath into the assertion semantics.
    fn write_zip(zip_path: &Path, entries: &[(&str, Vec<u8>)]) {
        let file = File::create(zip_path).expect("create zip");
        let mut writer = zip::ZipWriter::new(file);
        let opts: SimpleFileOptions =
            SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        for (name, bytes) in entries {
            writer.start_file(*name, opts).expect("start file");
            writer.write_all(bytes).expect("write entry");
        }
        writer.finish().expect("finish zip");
    }

    /// Builds a complete, valid theme zip at `zip_path` with the
    /// given manifest id.
    fn write_valid_zip(zip_path: &Path, id: &str) {
        let manifest = full_manifest(id);
        let manifest_ron = ron::ser::to_string_pretty(
            &manifest,
            ron::ser::PrettyConfig::default(),
        )
        .expect("ron serialise");
        let mut entries: Vec<(String, Vec<u8>)> = Vec::with_capacity(54);
        entries.push((MANIFEST_NAME.to_owned(), manifest_ron.into_bytes()));
        entries.push(("back.svg".to_owned(), TEST_SVG.to_vec()));
        for k in CardKey::all() {
            entries.push((
                format!("faces/{}.svg", k.manifest_name()),
                TEST_SVG.to_vec(),
            ));
        }
        let entries_ref: Vec<(&str, Vec<u8>)> =
            entries.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
        write_zip(zip_path, &entries_ref);
    }

    #[test]
    fn valid_zip_imports_and_extracts_files() {
        let scratch = TempDir::new().expect("scratch");
        let zip_path = scratch.path().join("good.zip");
        write_valid_zip(&zip_path, "fancy");

        let target = TempDir::new().expect("target");
        let id = import_theme_into(&zip_path, target.path()).expect("import succeeds");

        assert_eq!(id.as_str(), "fancy");
        let installed = target.path().join("fancy");
        assert!(installed.is_dir(), "theme dir should exist");
        assert!(installed.join("theme.ron").is_file(), "manifest extracted");
        assert!(installed.join("back.svg").is_file(), "back extracted");
        assert!(
            installed.join("faces/hearts_ace.svg").is_file(),
            "face extracted"
        );
    }

    #[test]
    fn missing_manifest_is_rejected() {
        let scratch = TempDir::new().expect("scratch");
        let zip_path = scratch.path().join("no_manifest.zip");
        // No `theme.ron` at root, but plenty of other content.
        write_zip(
            &zip_path,
            &[
                ("back.svg", TEST_SVG.to_vec()),
                ("faces/hearts_ace.svg", TEST_SVG.to_vec()),
            ],
        );

        let target = TempDir::new().expect("target");
        let err = import_theme_into(&zip_path, target.path()).expect_err("expected error");
        assert!(
            matches!(err, ImportError::MissingManifest),
            "got {err:?}"
        );
        assert!(
            target.path().read_dir().unwrap().next().is_none(),
            "target untouched"
        );
    }

    #[test]
    fn manifest_with_only_51_faces_is_rejected() {
        let scratch = TempDir::new().expect("scratch");
        let zip_path = scratch.path().join("missing_face.zip");

        let mut manifest = full_manifest("incomplete");
        // Drop one face so validation surfaces MissingFaces.
        manifest.faces.remove("hearts_ace");
        let manifest_ron = ron::ser::to_string_pretty(
            &manifest,
            ron::ser::PrettyConfig::default(),
        )
        .expect("ron serialise");

        let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
        entries.push((MANIFEST_NAME.to_owned(), manifest_ron.into_bytes()));
        entries.push(("back.svg".to_owned(), TEST_SVG.to_vec()));
        for k in CardKey::all() {
            entries.push((
                format!("faces/{}.svg", k.manifest_name()),
                TEST_SVG.to_vec(),
            ));
        }
        let entries_ref: Vec<(&str, Vec<u8>)> =
            entries.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
        write_zip(&zip_path, &entries_ref);

        let target = TempDir::new().expect("target");
        let err = import_theme_into(&zip_path, target.path()).expect_err("expected error");
        match err {
            ImportError::Validation(ManifestError::MissingFaces { missing }) => {
                assert!(missing.iter().any(|s| s == "hearts_ace"));
            }
            other => panic!("expected MissingFaces, got {other:?}"),
        }
        assert!(
            target.path().read_dir().unwrap().next().is_none(),
            "target untouched"
        );
    }

    #[test]
    fn oversized_archive_is_rejected() {
        let scratch = TempDir::new().expect("scratch");
        let zip_path = scratch.path().join("huge.zip");

        // Compose a single entry whose declared uncompressed size
        // exceeds the cap. We use a real Vec here (compressed via
        // Stored) because zip's central directory mirrors the
        // payload size we actually wrote.
        let huge = vec![0u8; (MAX_ARCHIVE_BYTES + 1) as usize];
        write_zip(
            &zip_path,
            &[
                (MANIFEST_NAME, b"".to_vec()),
                ("filler.bin", huge),
            ],
        );

        let target = TempDir::new().expect("target");
        let err = import_theme_into(&zip_path, target.path()).expect_err("expected error");
        assert!(
            matches!(err, ImportError::Oversized { .. }),
            "got {err:?}"
        );
        assert!(
            target.path().read_dir().unwrap().next().is_none(),
            "target untouched"
        );
    }

    #[test]
    fn zip_slip_path_is_rejected() {
        let scratch = TempDir::new().expect("scratch");
        let zip_path = scratch.path().join("slip.zip");

        // Build the zip with a deliberately path-traversing entry
        // name. We bypass `write_zip`'s normal flow because the
        // SimpleFileOptions API will gladly accept any string.
        let file = File::create(&zip_path).expect("create zip");
        let mut writer = zip::ZipWriter::new(file);
        let opts: SimpleFileOptions =
            SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        writer
            .start_file("../etc/passwd", opts)
            .expect("start file");
        writer.write_all(b"not really").expect("write entry");
        writer.finish().expect("finish zip");

        let target = TempDir::new().expect("target");
        let err = import_theme_into(&zip_path, target.path()).expect_err("expected error");
        assert!(matches!(err, ImportError::ZipSlip { .. }), "got {err:?}");
        assert!(
            target.path().read_dir().unwrap().next().is_none(),
            "target untouched"
        );
    }

    #[test]
    fn manifest_referenced_face_missing_from_archive_is_rejected() {
        let scratch = TempDir::new().expect("scratch");
        let zip_path = scratch.path().join("missing_file.zip");

        // Manifest is well-formed and validates, but we omit one of
        // the SVGs from the archive to trigger the MissingFile path.
        let manifest = full_manifest("missing_file_theme");
        let manifest_ron = ron::ser::to_string_pretty(
            &manifest,
            ron::ser::PrettyConfig::default(),
        )
        .expect("ron serialise");

        let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
        entries.push((MANIFEST_NAME.to_owned(), manifest_ron.into_bytes()));
        entries.push(("back.svg".to_owned(), TEST_SVG.to_vec()));
        for k in CardKey::all() {
            // Skip hearts_ace.svg so its manifest entry has no
            // matching archive payload.
            if k.manifest_name() == "hearts_ace" {
                continue;
            }
            entries.push((
                format!("faces/{}.svg", k.manifest_name()),
                TEST_SVG.to_vec(),
            ));
        }
        let entries_ref: Vec<(&str, Vec<u8>)> =
            entries.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
        write_zip(&zip_path, &entries_ref);

        let target = TempDir::new().expect("target");
        let err = import_theme_into(&zip_path, target.path()).expect_err("expected error");
        match err {
            ImportError::MissingFile { missing } => {
                assert!(
                    missing.contains("hearts_ace"),
                    "missing path should mention hearts_ace, got {missing}"
                );
            }
            other => panic!("expected MissingFile, got {other:?}"),
        }
        assert!(
            !target.path().join("missing_file_theme").exists(),
            "target untouched"
        );
    }

    #[test]
    fn id_collision_with_existing_dir_is_rejected() {
        let scratch = TempDir::new().expect("scratch");
        let zip_path = scratch.path().join("collide.zip");
        write_valid_zip(&zip_path, "duplicate");

        let target = TempDir::new().expect("target");
        // Pre-populate the destination so the import path runs
        // straight into the IdCollision check.
        fs::create_dir_all(target.path().join("duplicate")).unwrap();

        let err = import_theme_into(&zip_path, target.path()).expect_err("expected error");
        match err {
            ImportError::IdCollision { id } => assert_eq!(id, "duplicate"),
            other => panic!("expected IdCollision, got {other:?}"),
        }
        // Existing dir must still be there, but no extracted files
        // should have been copied into it.
        let existing = target.path().join("duplicate");
        assert!(existing.is_dir());
        assert!(!existing.join("theme.ron").exists());
    }
}
