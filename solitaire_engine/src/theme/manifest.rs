//! On-disk theme manifest schema (`.theme.ron`).
//!
//! A manifest is a single RON file that lists, for one card theme, the
//! display metadata plus the 52 face SVG paths and one back SVG path.
//! Paths are interpreted relative to the manifest file's directory so
//! the same manifest works whether the theme is bundled via
//! `embedded://`, dropped under `themes://`, or unpacked into a temp
//! dir during import validation.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::{CardKey, ThemeMeta, ThemeMetaError};

/// Raw deserialised manifest. Keys in `faces` use the canonical
/// [`CardKey::manifest_name`] string form (e.g. `"hearts_ace"`); the
/// loader converts to `HashMap<CardKey, _>` after validating that all
/// 52 entries are present.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThemeManifest {
    pub meta: ThemeMeta,
    pub back: PathBuf,
    pub faces: HashMap<String, PathBuf>,
}

/// Errors raised by [`ThemeManifest::validate`].
#[derive(Debug, Error)]
pub enum ManifestError {
    #[error("theme metadata invalid: {0}")]
    Meta(#[from] ThemeMetaError),
    #[error("manifest face key {key:?} is not a valid card name")]
    UnknownFaceKey { key: String },
    #[error("manifest is missing face entries: {missing:?}")]
    MissingFaces { missing: Vec<String> },
    #[error("manifest declares {duplicate} twice with different paths")]
    DuplicateFace { duplicate: String },
}

impl ThemeManifest {
    /// Parses the manifest's face map into a strongly-typed
    /// `HashMap<CardKey, PathBuf>`, surfacing precise errors for
    /// (a) keys that don't name a real card, (b) any of the 52 cards
    /// that the manifest forgot to list, and (c) duplicate keys (RON
    /// silently keeps the last value, which is brittle behaviour for
    /// a release). Also runs [`ThemeMeta::validate`] up front so
    /// metadata-level errors surface before path validation.
    pub fn validate(&self) -> Result<HashMap<CardKey, PathBuf>, ManifestError> {
        self.meta.validate()?;

        let mut faces: HashMap<CardKey, PathBuf> = HashMap::with_capacity(52);
        for (key_str, path) in &self.faces {
            let key = CardKey::parse_manifest_name(key_str).ok_or_else(|| {
                ManifestError::UnknownFaceKey {
                    key: key_str.clone(),
                }
            })?;
            if faces.insert(key, path.clone()).is_some() {
                return Err(ManifestError::DuplicateFace {
                    duplicate: key_str.clone(),
                });
            }
        }

        let missing: Vec<String> = CardKey::all()
            .filter(|k| !faces.contains_key(k))
            .map(CardKey::manifest_name)
            .collect();
        if !missing.is_empty() {
            return Err(ManifestError::MissingFaces { missing });
        }

        Ok(faces)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta() -> ThemeMeta {
        ThemeMeta {
            id: "default".into(),
            name: "Default".into(),
            author: "Solitaire Quest".into(),
            version: "1.0.0".into(),
            card_aspect: (2, 3),
        }
    }

    fn full_face_map() -> HashMap<String, PathBuf> {
        CardKey::all()
            .map(|k| (k.manifest_name(), PathBuf::from(format!("{}.svg", k.manifest_name()))))
            .collect()
    }

    #[test]
    fn complete_manifest_validates() {
        let m = ThemeManifest {
            meta: meta(),
            back: PathBuf::from("back.svg"),
            faces: full_face_map(),
        };
        let parsed = m.validate().expect("valid manifest");
        assert_eq!(parsed.len(), 52);
        for k in CardKey::all() {
            assert!(parsed.contains_key(&k), "{} missing", k.manifest_name());
        }
    }

    #[test]
    fn missing_face_is_rejected_with_a_named_list() {
        let mut faces = full_face_map();
        faces.remove("hearts_ace");
        faces.remove("spades_king");

        let m = ThemeManifest {
            meta: meta(),
            back: PathBuf::from("back.svg"),
            faces,
        };

        match m.validate() {
            Err(ManifestError::MissingFaces { missing }) => {
                assert!(missing.iter().any(|s| s == "hearts_ace"));
                assert!(missing.iter().any(|s| s == "spades_king"));
            }
            other => panic!("expected MissingFaces, got {other:?}"),
        }
    }

    #[test]
    fn unknown_face_key_is_rejected() {
        let mut faces = full_face_map();
        faces.insert("not_a_card".into(), PathBuf::from("nope.svg"));

        let m = ThemeManifest {
            meta: meta(),
            back: PathBuf::from("back.svg"),
            faces,
        };

        assert!(matches!(
            m.validate(),
            Err(ManifestError::UnknownFaceKey { key }) if key == "not_a_card"
        ));
    }

    #[test]
    fn invalid_meta_propagates() {
        let mut bad_meta = meta();
        bad_meta.id = "../escape".into();
        let m = ThemeManifest {
            meta: bad_meta,
            back: PathBuf::from("back.svg"),
            faces: full_face_map(),
        };
        assert!(matches!(m.validate(), Err(ManifestError::Meta(_))));
    }

    #[test]
    fn ron_round_trip_preserves_manifest() {
        let m = ThemeManifest {
            meta: meta(),
            back: PathBuf::from("back.svg"),
            faces: full_face_map(),
        };
        let serialised = ron::ser::to_string_pretty(
            &m,
            ron::ser::PrettyConfig::default(),
        )
        .expect("serde_ron");
        let parsed: ThemeManifest = ron::from_str(&serialised).expect("ron parse");
        assert_eq!(parsed.meta, m.meta);
        assert_eq!(parsed.back, m.back);
        assert_eq!(parsed.faces, m.faces);
    }
}
