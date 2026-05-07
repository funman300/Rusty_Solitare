//! Card-theme asset type.
//!
//! A `CardTheme` is a self-contained set of 52 face images plus one back
//! image, addressable by `CardKey`. Themes are loaded from RON manifests
//! (`.theme.ron`) by [`CardThemeLoader`]; the loader rasterises every
//! referenced SVG via [`crate::assets::SvgLoader`] and binds the
//! resulting `Handle<Image>` to its `CardKey`.
//!
//! The runtime card-rendering systems read the active theme through
//! [`crate::theme::ActiveTheme`] (added in Phase 4) and look up
//! `theme.faces.get(&card_key)` per render. They never store image
//! handles directly on card entities, so a theme switch propagates on
//! the next frame without re-spawning anything.

pub mod importer;
pub mod loader;
pub mod manifest;
pub mod plugin;
pub mod registry;

use std::collections::HashMap;

use bevy::asset::{Asset, Handle};
use bevy::image::Image;
use bevy::reflect::TypePath;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use solitaire_core::card::{Rank, Suit};

pub use importer::{import_theme, import_theme_into, ImportError, ThemeId};
pub use loader::{CardThemeLoader, CardThemeLoaderError};
pub use manifest::ThemeManifest;
pub use plugin::{
    ensure_theme_thumbnails, set_theme, ActiveTheme, ThemePlugin, ThemeThumbnailCache,
    ThemeThumbnailPair, THEME_THUMBNAIL_HEIGHT_PX, THEME_THUMBNAIL_WIDTH_PX,
};
pub use registry::{
    build_registry, refresh_registry, ThemeEntry, ThemeRegistry, ThemeRegistryPlugin,
};

/// Hashable lookup key into [`CardTheme::faces`].
///
/// Distinct from `solitaire_core::Card`: the core type carries an `id`
/// and a `face_up` flag that vary per deal, neither of which is
/// relevant to image lookup. `CardKey` is just the (suit, rank) pair
/// that uniquely identifies which artwork to draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CardKey {
    pub suit: Suit,
    pub rank: Rank,
}

impl CardKey {
    /// Constructs a key from a `(suit, rank)` pair.
    pub const fn new(suit: Suit, rank: Rank) -> Self {
        Self { suit, rank }
    }

    /// Iterator over all 52 valid keys, in suit-major / rank-ascending order.
    /// Used to enumerate the manifest's required entries.
    pub fn all() -> impl Iterator<Item = CardKey> {
        const SUITS: [Suit; 4] = [Suit::Clubs, Suit::Diamonds, Suit::Hearts, Suit::Spades];
        const RANKS: [Rank; 13] = [
            Rank::Ace, Rank::Two, Rank::Three, Rank::Four, Rank::Five, Rank::Six,
            Rank::Seven, Rank::Eight, Rank::Nine, Rank::Ten, Rank::Jack, Rank::Queen,
            Rank::King,
        ];
        SUITS
            .into_iter()
            .flat_map(|s| RANKS.into_iter().map(move |r| CardKey::new(s, r)))
    }

    /// Canonical manifest-key string: `"{suit}_{rank}"` lowercase.
    /// e.g. `"hearts_ace"`, `"spades_10"`, `"clubs_king"`.
    pub fn manifest_name(self) -> String {
        format!("{}_{}", suit_token(self.suit), rank_token(self.rank))
    }

    /// Inverse of [`CardKey::manifest_name`]. Accepts the canonical
    /// `"{suit}_{rank}"` form. Returns `None` for any other shape so
    /// the manifest loader surfaces a clear error message instead of
    /// silently picking wrong defaults.
    pub fn parse_manifest_name(s: &str) -> Option<CardKey> {
        let (suit_part, rank_part) = s.split_once('_')?;
        Some(CardKey::new(parse_suit(suit_part)?, parse_rank(rank_part)?))
    }
}

fn suit_token(s: Suit) -> &'static str {
    match s {
        Suit::Clubs => "clubs",
        Suit::Diamonds => "diamonds",
        Suit::Hearts => "hearts",
        Suit::Spades => "spades",
    }
}

fn rank_token(r: Rank) -> &'static str {
    match r {
        Rank::Ace => "ace",
        Rank::Two => "2",
        Rank::Three => "3",
        Rank::Four => "4",
        Rank::Five => "5",
        Rank::Six => "6",
        Rank::Seven => "7",
        Rank::Eight => "8",
        Rank::Nine => "9",
        Rank::Ten => "10",
        Rank::Jack => "jack",
        Rank::Queen => "queen",
        Rank::King => "king",
    }
}

fn parse_suit(s: &str) -> Option<Suit> {
    match s {
        "clubs" => Some(Suit::Clubs),
        "diamonds" => Some(Suit::Diamonds),
        "hearts" => Some(Suit::Hearts),
        "spades" => Some(Suit::Spades),
        _ => None,
    }
}

fn parse_rank(s: &str) -> Option<Rank> {
    match s {
        "ace" => Some(Rank::Ace),
        "2" => Some(Rank::Two),
        "3" => Some(Rank::Three),
        "4" => Some(Rank::Four),
        "5" => Some(Rank::Five),
        "6" => Some(Rank::Six),
        "7" => Some(Rank::Seven),
        "8" => Some(Rank::Eight),
        "9" => Some(Rank::Nine),
        "10" => Some(Rank::Ten),
        "jack" => Some(Rank::Jack),
        "queen" => Some(Rank::Queen),
        "king" => Some(Rank::King),
        _ => None,
    }
}

/// Human-facing metadata stored in every theme manifest. Surfaces in
/// the future picker UI (Phase 6) and is preserved on disk so the
/// importer (Phase 7) can validate that two themes don't collide on
/// `id`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThemeMeta {
    /// Unique opaque identifier — also the directory name on disk.
    /// Must be filesystem-safe (no path separators); the importer
    /// enforces this.
    pub id: String,
    /// Display name shown in the picker.
    pub name: String,
    /// Author attribution (free-form text).
    pub author: String,
    /// Version string (free-form, but conventionally semver).
    pub version: String,
    /// Card aspect ratio as `(numerator, denominator)`. The SVG
    /// rasteriser uses this to choose a target size that preserves
    /// the artwork's intended proportions when the player resizes the
    /// window. Standard playing cards are 2:3.
    pub card_aspect: (u32, u32),
}

/// Errors surfaced by [`ThemeMeta::validate`].
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ThemeMetaError {
    #[error("theme id is empty")]
    EmptyId,
    #[error("theme id contains a path separator: {0:?}")]
    PathSeparatorInId(String),
    #[error("card_aspect denominator is zero")]
    ZeroDenominator,
    #[error("card_aspect numerator is zero")]
    ZeroNumerator,
}

impl ThemeMeta {
    /// Validates surface invariants. The importer (Phase 7) calls this
    /// before unpacking a zip into the user-themes directory so it
    /// can reject ill-formed manifests early without filesystem side
    /// effects.
    pub fn validate(&self) -> Result<(), ThemeMetaError> {
        if self.id.is_empty() {
            return Err(ThemeMetaError::EmptyId);
        }
        if self.id.contains('/') || self.id.contains('\\') {
            return Err(ThemeMetaError::PathSeparatorInId(self.id.clone()));
        }
        if self.card_aspect.0 == 0 {
            return Err(ThemeMetaError::ZeroNumerator);
        }
        if self.card_aspect.1 == 0 {
            return Err(ThemeMetaError::ZeroDenominator);
        }
        Ok(())
    }
}

/// A loaded card theme — 52 face images + 1 back image + metadata.
///
/// `faces` is keyed by [`CardKey`]; every key produced by
/// `CardKey::all()` is guaranteed to be present (the loader rejects
/// manifests that miss any of the 52 entries).
#[derive(Asset, TypePath, Debug)]
pub struct CardTheme {
    pub meta: ThemeMeta,
    pub faces: HashMap<CardKey, Handle<Image>>,
    pub back: Handle<Image>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_yields_52_unique_keys() {
        let keys: Vec<CardKey> = CardKey::all().collect();
        assert_eq!(keys.len(), 52);
        let unique: std::collections::HashSet<CardKey> = keys.iter().copied().collect();
        assert_eq!(unique.len(), 52);
    }

    #[test]
    fn manifest_name_round_trips_for_every_card() {
        for key in CardKey::all() {
            let name = key.manifest_name();
            assert_eq!(
                CardKey::parse_manifest_name(&name),
                Some(key),
                "round-trip failed for {name}"
            );
        }
    }

    #[test]
    fn manifest_name_examples() {
        assert_eq!(
            CardKey::new(Suit::Hearts, Rank::Ace).manifest_name(),
            "hearts_ace"
        );
        assert_eq!(
            CardKey::new(Suit::Spades, Rank::Ten).manifest_name(),
            "spades_10"
        );
        assert_eq!(
            CardKey::new(Suit::Clubs, Rank::King).manifest_name(),
            "clubs_king"
        );
    }

    #[test]
    fn parse_manifest_name_rejects_garbage() {
        assert!(CardKey::parse_manifest_name("nope").is_none());
        assert!(CardKey::parse_manifest_name("hearts").is_none());
        assert!(CardKey::parse_manifest_name("hearts_").is_none());
        assert!(CardKey::parse_manifest_name("_ace").is_none());
        assert!(CardKey::parse_manifest_name("hearts_15").is_none());
        assert!(CardKey::parse_manifest_name("HEARTS_ACE").is_none());
    }

    #[test]
    fn theme_meta_validates_well_formed() {
        let meta = ThemeMeta {
            id: "default".into(),
            name: "Default".into(),
            author: "Solitaire Quest".into(),
            version: "1.0.0".into(),
            card_aspect: (2, 3),
        };
        assert_eq!(meta.validate(), Ok(()));
    }

    #[test]
    fn theme_meta_rejects_empty_id() {
        let meta = ThemeMeta {
            id: String::new(),
            name: "x".into(),
            author: "x".into(),
            version: "x".into(),
            card_aspect: (2, 3),
        };
        assert_eq!(meta.validate(), Err(ThemeMetaError::EmptyId));
    }

    #[test]
    fn theme_meta_rejects_path_separator_in_id() {
        let meta = ThemeMeta {
            id: "../etc/passwd".into(),
            name: "x".into(),
            author: "x".into(),
            version: "x".into(),
            card_aspect: (2, 3),
        };
        assert!(matches!(
            meta.validate(),
            Err(ThemeMetaError::PathSeparatorInId(_))
        ));
    }

    #[test]
    fn theme_meta_rejects_zero_aspect_components() {
        let mut meta = ThemeMeta {
            id: "x".into(),
            name: "x".into(),
            author: "x".into(),
            version: "x".into(),
            card_aspect: (0, 3),
        };
        assert_eq!(meta.validate(), Err(ThemeMetaError::ZeroNumerator));
        meta.card_aspect = (2, 0);
        assert_eq!(meta.validate(), Err(ThemeMetaError::ZeroDenominator));
    }
}
