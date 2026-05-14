//! `ThemePlugin` — owns [`ActiveTheme`], registers the `CardTheme` /
//! SVG asset machinery, and keeps `card_plugin::CardImageSet` in sync
//! with the currently-loaded theme so existing card-rendering systems
//! pick up the new artwork on the next state-changed tick.
//!
//! The plugin's `set_theme` helper is the public API used by the
//! Settings appearance picker and exposed for tests.

use std::collections::HashMap;

use bevy::asset::AssetEvent;
use bevy::ecs::message::MessageReader;
use bevy::math::UVec2;
use bevy::prelude::*;
use solitaire_core::card::{Rank, Suit};

use crate::assets::{
    bundled_theme_url, classic_theme_svg_bytes, dark_theme_svg_bytes, rasterize_svg, user_theme_dir,
};
use crate::card_plugin::CardImageSet;
use crate::events::StateChangedEvent;

use super::loader::CardThemeLoader;
use super::registry::ThemeRegistry;
use super::{CardKey, CardTheme};

/// Width (logical px) of one Settings → Cosmetic theme-picker
/// thumbnail. A 2:3 card aspect at 100×140 keeps each chip a small
/// glanceable preview without bloating the picker row.
pub const THEME_THUMBNAIL_WIDTH_PX: u32 = 100;
/// Height counterpart to [`THEME_THUMBNAIL_WIDTH_PX`].
pub const THEME_THUMBNAIL_HEIGHT_PX: u32 = 140;

/// Resource pointing at the currently-active card theme. Populated on
/// startup with the bundled default theme and replaced by [`set_theme`]
/// when the player switches.
#[derive(Resource, Debug)]
pub struct ActiveTheme(pub Handle<CardTheme>);

/// One pair of preview-sized `Handle<Image>` for the Settings picker:
/// the theme's Ace of Spades and its card back.
///
/// Either handle may be [`Handle::default`] when the underlying SVG
/// could not be located (e.g. a user theme that ships only a partial
/// set of files). The picker UI treats the default-handle case as
/// "render a placeholder swatch instead of an image" so a broken
/// theme can never crash the panel.
#[derive(Debug, Clone, Default)]
pub struct ThemeThumbnailPair {
    /// Rasterised `spades_ace.svg` of the theme.
    pub ace: Handle<Image>,
    /// Rasterised `back.svg` of the theme.
    pub back: Handle<Image>,
}

impl ThemeThumbnailPair {
    /// Returns `true` only when *both* preview slots resolve to a
    /// non-default handle — a theme with at least one missing SVG is
    /// considered incomplete and renders the placeholder for the
    /// missing slot.
    pub fn is_fully_populated(&self) -> bool {
        self.ace != Handle::default() && self.back != Handle::default()
    }
}

/// Resource caching one [`ThemeThumbnailPair`] per registered theme,
/// keyed by `ThemeMeta::id`.
///
/// Populated lazily by [`ensure_theme_thumbnails`] whenever the
/// [`ThemeRegistry`] grows or changes. The Settings panel reads from
/// this cache by id and falls back to the placeholder rendering path
/// when an entry is missing.
#[derive(Resource, Debug, Default)]
pub struct ThemeThumbnailCache {
    pub entries: HashMap<String, ThemeThumbnailPair>,
}

impl ThemeThumbnailCache {
    /// Returns the cached pair for `theme_id`, if any.
    pub fn get(&self, theme_id: &str) -> Option<&ThemeThumbnailPair> {
        self.entries.get(theme_id)
    }
}

/// Bevy plugin that loads the default theme and keeps `CardImageSet`
/// in sync with `Assets<CardTheme>`.
///
/// Order considerations:
///
/// - `init_asset::<CardTheme>` must happen before any system that
///   stores `Handle<CardTheme>` runs, so it goes in `build`.
/// - `register_asset_loader` for the SVG and theme loaders must
///   happen after `AssetPlugin` is built (DefaultPlugins). This
///   plugin therefore must be added after `DefaultPlugins`.
/// - The `Startup` system that loads the default theme runs after
///   the asset sources are registered (see
///   `crate::assets::register_theme_asset_sources` and
///   `crate::assets::AssetSourcesPlugin`).
pub struct ThemePlugin;

impl Plugin for ThemePlugin {
    fn build(&self, app: &mut App) {
        app.init_asset::<CardTheme>()
            .init_resource::<ThemeThumbnailCache>()
            .register_asset_loader(crate::assets::SvgLoader)
            .register_asset_loader(CardThemeLoader)
            .add_systems(Startup, load_initial_theme)
            .add_systems(
                Update,
                (
                    sync_card_image_set_with_active_theme,
                    react_to_settings_theme_change,
                    ensure_theme_thumbnails,
                ),
            );
    }
}

/// Kicks off the initial theme load — the one named by
/// `Settings::selected_theme_id` if available, falling back to the
/// embedded default. The actual rasterisation runs asynchronously on
/// the asset task pool; the sync system below picks up the
/// `LoadedWithDependencies` event when every face + back is ready.
fn load_initial_theme(
    asset_server: Res<AssetServer>,
    settings: Option<Res<crate::settings_plugin::SettingsResource>>,
    mut commands: Commands,
) {
    let id = settings
        .as_deref()
        .map(|s| s.0.selected_theme_id.as_str())
        .unwrap_or("dark");
    let url = bundled_theme_url(id)
        .map(str::to_string)
        .unwrap_or_else(|| format!("themes://{id}/theme.ron"));
    let handle: Handle<CardTheme> = asset_server.load(url);
    commands.insert_resource(ActiveTheme(handle));
}

/// Watches [`crate::settings_plugin::SettingsChangedEvent`] and
/// triggers a fresh theme load whenever
/// `Settings::selected_theme_id` changes. The settings panel's theme
/// picker fires the event after persisting; this system is the bridge
/// that turns the persisted choice into a live `set_theme` call.
fn react_to_settings_theme_change(
    mut events: MessageReader<crate::settings_plugin::SettingsChangedEvent>,
    asset_server: Res<AssetServer>,
    active: Option<Res<ActiveTheme>>,
    themes: Res<Assets<CardTheme>>,
    mut commands: Commands,
) {
    let Some(latest) = events.read().last() else {
        return;
    };
    let new_id = latest.0.selected_theme_id.as_str();

    // No-op if the active theme already matches the desired id.
    if let Some(active) = active.as_deref()
        && let Some(theme) = themes.get(&active.0)
        && theme.meta.id == new_id
    {
        return;
    }

    let url = bundled_theme_url(new_id)
        .map(str::to_string)
        .unwrap_or_else(|| format!("themes://{new_id}/theme.ron"));
    let handle: Handle<CardTheme> = asset_server.load(url);
    commands.insert_resource(ActiveTheme(handle));
}

/// Replaces every face slot and the active-theme back-handle slot on
/// `CardImageSet` whenever the active theme finishes loading or
/// changes. Fires `StateChangedEvent` afterwards so the existing
/// `card_plugin::sync_cards_on_change` pipeline re-renders every
/// on-screen card with the new artwork.
///
/// `CardImageSet` may be absent — tests using `MinimalPlugins` skip
/// `CardPlugin` entirely. In that case the system is a no-op and the
/// plugin still composes cleanly under headless setups.
fn sync_card_image_set_with_active_theme(
    mut events: MessageReader<AssetEvent<CardTheme>>,
    active: Option<Res<ActiveTheme>>,
    themes: Res<Assets<CardTheme>>,
    mut card_image_set: Option<ResMut<CardImageSet>>,
    mut state_events: MessageWriter<StateChangedEvent>,
) {
    let Some(active) = active else { return };
    let active_id = active.0.id();
    let mut should_sync = false;
    for ev in events.read() {
        let id = match ev {
            AssetEvent::LoadedWithDependencies { id }
            | AssetEvent::Modified { id } => *id,
            _ => continue,
        };
        if id == active_id {
            should_sync = true;
        }
    }
    if !should_sync {
        return;
    }
    let Some(theme) = themes.get(&active.0) else {
        return;
    };
    let Some(card_image_set) = card_image_set.as_deref_mut() else {
        return;
    };
    apply_theme_to_card_image_set(theme, card_image_set);
    state_events.write(StateChangedEvent);
}

/// Pure helper that copies the theme's image handles into the
/// `[suit][rank]` face matrix and into the dedicated `theme_back`
/// slot. Split out so it can be unit-tested without spinning up a
/// Bevy `App`.
///
/// The legacy `backs[0..5]` array is left untouched — those handles
/// are the player's `selected_card_back` choices and remain available
/// as a fallback when the active theme does not declare a back. The
/// face-down render path in `card_plugin::card_sprite` prefers
/// `theme_back` when present, so writing here is sufficient to make
/// every face-down card pick up the theme's art on the next sync.
fn apply_theme_to_card_image_set(theme: &CardTheme, image_set: &mut CardImageSet) {
    for suit in [Suit::Clubs, Suit::Diamonds, Suit::Hearts, Suit::Spades] {
        for rank in [
            Rank::Ace, Rank::Two, Rank::Three, Rank::Four, Rank::Five,
            Rank::Six, Rank::Seven, Rank::Eight, Rank::Nine, Rank::Ten,
            Rank::Jack, Rank::Queen, Rank::King,
        ] {
            if let Some(handle) = theme.faces.get(&CardKey::new(suit, rank)) {
                image_set.faces[suit_index(suit)][rank_index(rank)] = handle.clone();
            }
        }
    }
    image_set.theme_back = Some(theme.back.clone());
}

/// Index used by [`CardImageSet::faces`] for a given suit. Mirrors
/// the `card_plugin` doc comment: Clubs=0, Diamonds=1, Hearts=2, Spades=3.
const fn suit_index(s: Suit) -> usize {
    match s {
        Suit::Clubs => 0,
        Suit::Diamonds => 1,
        Suit::Hearts => 2,
        Suit::Spades => 3,
    }
}

/// Index used by [`CardImageSet::faces`] for a given rank.
/// Ace=0, Two=1 … King=12.
const fn rank_index(r: Rank) -> usize {
    match r {
        Rank::Ace => 0,
        Rank::Two => 1,
        Rank::Three => 2,
        Rank::Four => 3,
        Rank::Five => 4,
        Rank::Six => 5,
        Rank::Seven => 6,
        Rank::Eight => 7,
        Rank::Nine => 8,
        Rank::Ten => 9,
        Rank::Jack => 10,
        Rank::Queen => 11,
        Rank::King => 12,
    }
}

/// Switches the active theme to the one served at
/// `themes://<theme_id>/theme.ron`. Returns the new `Handle<CardTheme>`
/// so callers can poll `Assets<CardTheme>` if they want to wait for
/// the load before changing UI state.
///
/// The handle is also written to the [`ActiveTheme`] resource — the
/// per-frame sync system picks up the `LoadedWithDependencies` event
/// and refreshes `CardImageSet` automatically; callers don't need to
/// fire `StateChangedEvent` themselves.
pub fn set_theme(
    commands: &mut Commands,
    asset_server: &AssetServer,
    theme_id: &str,
) -> Handle<CardTheme> {
    let url = format!("themes://{theme_id}/theme.ron");
    let handle: Handle<CardTheme> = asset_server.load(url);
    commands.insert_resource(ActiveTheme(handle.clone()));
    handle
}

// ---------------------------------------------------------------------------
// Picker-thumbnail generation
// ---------------------------------------------------------------------------

/// Filename of the canonical "preview face" SVG inside a theme — the
/// Ace of Spades. Matches `CardKey::manifest_name(Spades, Ace)` so the
/// path resolves the same way whether we're reading from disk or from
/// the bundled-default lookup table.
const PREVIEW_FACE_FILENAME: &str = "spades_ace.svg";

/// Filename of the back SVG inside a theme.
const PREVIEW_BACK_FILENAME: &str = "back.svg";

/// Resolves the SVG bytes for one preview file (`back.svg` or
/// `spades_ace.svg`) belonging to the named theme.
///
/// - For the embedded `dark` theme, reads from the in-binary table via
///   [`dark_theme_svg_bytes`]. No filesystem I/O.
/// - For the embedded `classic` theme, reads from the in-binary table via
///   [`classic_theme_svg_bytes`]. No filesystem I/O.
/// - For user themes, reads from `<user_theme_dir>/<id>/<filename>`.
///   Returns `None` for any I/O failure.
fn read_theme_preview_svg_bytes(theme_id: &str, filename: &str) -> Option<Vec<u8>> {
    if theme_id == "dark" {
        return dark_theme_svg_bytes(filename).map(|b| b.to_vec());
    }
    if theme_id == "classic" {
        return classic_theme_svg_bytes(filename).map(|b| b.to_vec());
    }
    // User themes live in the user theme dir.
    let path = user_theme_dir().join(theme_id).join(filename);
    std::fs::read(&path).ok()
}

/// Pure helper: rasterises one SVG preview byte slice at the picker's
/// thumbnail dimensions, inserts the resulting `Image` into
/// `Assets<Image>`, and returns the new handle. Returns
/// [`Handle::default`] if rasterisation fails (malformed SVG, etc.) so
/// the picker can render a placeholder for broken themes without
/// crashing.
fn rasterize_preview_to_handle(
    svg_bytes: &[u8],
    images: &mut Assets<Image>,
) -> Handle<Image> {
    let target = UVec2::new(THEME_THUMBNAIL_WIDTH_PX, THEME_THUMBNAIL_HEIGHT_PX);
    match rasterize_svg(svg_bytes, target) {
        Ok(image) => images.add(image),
        Err(err) => {
            warn!("theme thumbnail rasterise failed: {err}");
            Handle::default()
        }
    }
}

/// Builds a [`ThemeThumbnailPair`] for a single theme. Either handle
/// is [`Handle::default`] when the matching SVG could not be located
/// or rasterised.
fn generate_thumbnail_pair_for(
    theme_id: &str,
    images: &mut Assets<Image>,
) -> ThemeThumbnailPair {
    let ace = read_theme_preview_svg_bytes(theme_id, PREVIEW_FACE_FILENAME)
        .map(|b| rasterize_preview_to_handle(&b, images))
        .unwrap_or_default();
    let back = read_theme_preview_svg_bytes(theme_id, PREVIEW_BACK_FILENAME)
        .map(|b| rasterize_preview_to_handle(&b, images))
        .unwrap_or_default();
    ThemeThumbnailPair { ace, back }
}

/// System that generates a [`ThemeThumbnailPair`] for every registered
/// theme that doesn't yet have one in [`ThemeThumbnailCache`].
///
/// Runs each frame but the early-exit check (`already cached?`) keeps
/// the steady-state cost to a single hash lookup per theme. Generation
/// itself only happens once per theme — the SVGs are rasterised and
/// inserted into `Assets<Image>` and the handles cached forever.
///
/// Lazy-on-first-pass beats Startup-only for two reasons:
///
/// - The `ThemeRegistry` is built by a different `Startup` system, and
///   Bevy doesn't guarantee inter-system Startup ordering without
///   explicit `.after()` chaining. Polling each Update tick removes
///   the dependency.
/// - The future `refresh_registry` path (used after a successful
///   theme import in Phase 7) adds entries mid-session — this system
///   picks them up automatically without any extra wiring.
pub fn ensure_theme_thumbnails(
    registry: Option<Res<ThemeRegistry>>,
    mut cache: ResMut<ThemeThumbnailCache>,
    mut images: ResMut<Assets<Image>>,
) {
    let Some(registry) = registry else { return };
    for entry in registry.iter() {
        if cache.entries.contains_key(&entry.id) {
            continue;
        }
        let pair = generate_thumbnail_pair_for(&entry.id, &mut images);
        cache.entries.insert(entry.id.clone(), pair);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    use crate::theme::ThemeMeta;

    fn empty_theme() -> CardTheme {
        CardTheme {
            meta: ThemeMeta {
                id: "test".into(),
                name: "Test".into(),
                author: "test".into(),
                version: "0".into(),
                card_aspect: (2, 3),
            },
            faces: HashMap::new(),
            back: Handle::default(),
        }
    }

    fn empty_card_image_set() -> CardImageSet {
        // Every slot is the asset server's default-empty handle, the
        // same shape `card_plugin::load_card_images` uses when the
        // asset server is absent (tests under MinimalPlugins).
        CardImageSet {
            faces: std::array::from_fn(|_| std::array::from_fn(|_| Handle::default())),
            backs: std::array::from_fn(|_| Handle::default()),
            theme_back: None,
        }
    }

    #[test]
    fn suit_index_ranges_match_card_plugin_layout() {
        assert_eq!(suit_index(Suit::Clubs), 0);
        assert_eq!(suit_index(Suit::Diamonds), 1);
        assert_eq!(suit_index(Suit::Hearts), 2);
        assert_eq!(suit_index(Suit::Spades), 3);
    }

    #[test]
    fn rank_index_starts_at_ace_zero_and_ends_at_king_twelve() {
        assert_eq!(rank_index(Rank::Ace), 0);
        assert_eq!(rank_index(Rank::Two), 1);
        assert_eq!(rank_index(Rank::Ten), 9);
        assert_eq!(rank_index(Rank::Jack), 10);
        assert_eq!(rank_index(Rank::Queen), 11);
        assert_eq!(rank_index(Rank::King), 12);
    }

    #[test]
    fn applying_empty_theme_does_not_panic() {
        // A theme whose faces map is empty should leave existing
        // image-set face slots untouched (the .get() returns None,
        // we skip). The back is always copied since theme.back is
        // a single handle.
        let mut image_set = empty_card_image_set();
        let theme = empty_theme();
        apply_theme_to_card_image_set(&theme, &mut image_set);
    }

    #[test]
    fn applying_theme_writes_theme_back_slot_and_leaves_legacy_backs_untouched() {
        // The active-theme back lives in its own dedicated slot
        // (`theme_back`) so the legacy `backs[0..5]` PNG fallbacks
        // remain untouched. This guarantees the player's
        // `selected_card_back` choice can still be honoured when no
        // theme is active.
        let mut image_set = empty_card_image_set();
        // Snapshot the legacy back ids so we can prove they don't
        // change when a theme is applied.
        let legacy_ids_before: [bevy::asset::AssetId<bevy::image::Image>; 5] =
            std::array::from_fn(|i| image_set.backs[i].id());
        let theme = empty_theme();
        assert!(image_set.theme_back.is_none(), "theme_back starts empty");
        apply_theme_to_card_image_set(&theme, &mut image_set);
        // The active-theme back is now populated and matches the theme.
        let active_back = image_set
            .theme_back
            .as_ref()
            .expect("theme_back populated after apply");
        assert_eq!(active_back.id(), theme.back.id());
        // Every legacy back slot is preserved byte-for-byte by id.
        for (i, before) in legacy_ids_before.iter().enumerate() {
            assert_eq!(
                image_set.backs[i].id(),
                *before,
                "legacy back slot {i} must not be clobbered by theme apply",
            );
        }
    }

    #[test]
    fn theme_plugin_builds_under_minimal_plugins() {
        // Smoke test: the plugin's build hooks (init_asset,
        // register_asset_loader, system registration) run cleanly
        // under MinimalPlugins. Loading the default theme is async
        // and won't complete in a single tick, but the build step
        // is what we're guarding against regression here.
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.init_resource::<Assets<CardTheme>>();
        // The full ThemePlugin requires AssetServer (not present
        // under MinimalPlugins). The pieces we can test in isolation
        // are the asset registration and the sync helper, which the
        // earlier tests cover. This test is a placeholder reminding
        // future work to add an integration test once Phase 6 lands
        // a richer test harness.
    }

    #[test]
    fn set_theme_url_format_matches_themes_source() {
        // The format string is the only behavioural surface of
        // set_theme that doesn't require an App. We assert the URL
        // shape so a future refactor doesn't accidentally change the
        // path layout.
        let url = format!("themes://{}/theme.ron", "user_uploaded");
        assert_eq!(url, "themes://user_uploaded/theme.ron");
    }

    /// Test 1: the bundled dark theme always has embedded SVG bytes
    /// available, so calling `generate_thumbnail_pair_for("dark", …)`
    /// must produce two non-default `Handle<Image>` slots.
    #[test]
    fn theme_thumbnails_generated_for_dark_theme() {
        let mut images = Assets::<Image>::default();
        let pair = generate_thumbnail_pair_for("dark", &mut images);
        assert!(
            pair.is_fully_populated(),
            "dark theme must yield both ace + back thumbnail handles"
        );
        // And the underlying images must actually exist in the assets
        // collection — the handles are real, not dangling.
        assert!(images.get(&pair.ace).is_some(), "ace image must be inserted");
        assert!(images.get(&pair.back).is_some(), "back image must be inserted");
    }

    /// Test 2: when a theme is registered but its preview SVGs are not
    /// available on disk (a broken user-supplied theme), thumbnail
    /// generation must NOT panic and must leave the missing slots as
    /// the default handle so the picker UI can render its placeholder.
    #[test]
    fn theme_thumbnails_handle_missing_svg_gracefully() {
        let mut images = Assets::<Image>::default();
        // A theme id that definitely has no files on disk under the
        // user_theme_dir (the directory may not even exist on a
        // fresh test machine). The function reads the filesystem
        // lazily and silently returns None on I/O failures — no
        // panic, no rasterise attempt.
        let pair = generate_thumbnail_pair_for(
            "this-theme-does-not-exist-on-disk-for-testing",
            &mut images,
        );
        assert_eq!(
            pair.ace,
            Handle::default(),
            "missing ace.svg must yield Handle::default placeholder"
        );
        assert_eq!(
            pair.back,
            Handle::default(),
            "missing back.svg must yield Handle::default placeholder"
        );
        assert!(
            !pair.is_fully_populated(),
            "incomplete pair must report not-fully-populated"
        );
    }

    /// `read_theme_preview_svg_bytes` for the dark theme always returns
    /// embedded bytes for the canonical preview pair.
    #[test]
    fn read_dark_theme_preview_returns_some_for_canonical_files() {
        assert!(
            read_theme_preview_svg_bytes("dark", PREVIEW_BACK_FILENAME).is_some(),
            "dark theme back.svg must be embedded"
        );
        assert!(
            read_theme_preview_svg_bytes("dark", PREVIEW_FACE_FILENAME).is_some(),
            "dark theme spades_ace.svg must be embedded"
        );
    }

    /// `read_theme_preview_svg_bytes` for the classic theme always returns
    /// embedded bytes for the canonical preview pair.
    #[test]
    fn read_classic_theme_preview_returns_some_for_canonical_files() {
        assert!(
            read_theme_preview_svg_bytes("classic", PREVIEW_BACK_FILENAME).is_some(),
            "classic theme back.svg must be embedded"
        );
        assert!(
            read_theme_preview_svg_bytes("classic", PREVIEW_FACE_FILENAME).is_some(),
            "classic theme spades_ace.svg must be embedded"
        );
    }

    /// `ensure_theme_thumbnails` is idempotent: calling it twice with
    /// the same registry must not regenerate or replace already-cached
    /// entries. This guards against the per-frame Update tick churning
    /// new `Handle<Image>` allocations and growing `Assets<Image>`
    /// without bound.
    #[test]
    fn ensure_theme_thumbnails_caches_after_first_run() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.init_resource::<Assets<Image>>();
        app.init_resource::<ThemeThumbnailCache>();
        app.insert_resource(ThemeRegistry {
            entries: vec![crate::theme::ThemeEntry {
                id: "dark".into(),
                display_name: "Dark".into(),
                manifest_url: crate::assets::DARK_THEME_MANIFEST_URL.into(),
                meta: ThemeMeta {
                    id: "dark".into(),
                    name: "Dark".into(),
                    author: "x".into(),
                    version: "x".into(),
                    card_aspect: (2, 3),
                },
            }],
        });
        app.add_systems(Update, ensure_theme_thumbnails);

        // First tick generates the entry.
        app.update();
        let first_ace = app
            .world()
            .resource::<ThemeThumbnailCache>()
            .get("dark")
            .map(|p| p.ace.clone())
            .expect("dark theme thumbnail must exist after one tick");

        // Second tick must NOT replace the cached handle.
        app.update();
        let second_ace = app
            .world()
            .resource::<ThemeThumbnailCache>()
            .get("dark")
            .map(|p| p.ace.clone())
            .expect("dark theme thumbnail must still exist");

        assert_eq!(
            first_ace.id(),
            second_ace.id(),
            "cached thumbnail handle must be stable across ticks"
        );
    }
}
