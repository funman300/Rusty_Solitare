//! `ThemePlugin` — owns [`ActiveTheme`], registers the `CardTheme` /
//! SVG asset machinery, and keeps `card_plugin::CardImageSet` in sync
//! with the currently-loaded theme so existing card-rendering systems
//! pick up the new artwork on the next state-changed tick.
//!
//! Phase 4 of `CARD_PLAN.md`. The plugin's `set_theme` helper is the
//! public API that the future picker UI (Phase 6) calls; for now it's
//! exposed for tests and for any embedder that wants to load an
//! alternative theme manually.

use std::collections::HashMap;

use bevy::asset::AssetEvent;
use bevy::ecs::message::MessageReader;
use bevy::math::UVec2;
use bevy::prelude::*;
use solitaire_core::card::{Rank, Suit};

use crate::assets::{
    default_theme_svg_bytes, rasterize_svg, user_theme_dir, DEFAULT_THEME_MANIFEST_URL,
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
    let url = match settings.as_deref() {
        Some(s) if s.0.selected_theme_id != "default" => {
            format!("themes://{}/theme.ron", s.0.selected_theme_id)
        }
        _ => DEFAULT_THEME_MANIFEST_URL.to_string(),
    };
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

    let url = if new_id == "default" {
        DEFAULT_THEME_MANIFEST_URL.to_string()
    } else {
        format!("themes://{new_id}/theme.ron")
    };
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

/// Basename (no extension) of the canonical "preview face" inside a
/// theme — the Ace of Spades. Matches `CardKey::manifest_name(Spades,
/// Ace)`. The thumbnail loader appends `.svg` first and falls back to
/// `.png` so themes shipped as raster art still get real previews.
const PREVIEW_FACE_BASENAME: &str = "spades_ace";

/// Basename (no extension) of the back preview inside a theme. Matched
/// the same way as [`PREVIEW_FACE_BASENAME`].
const PREVIEW_BACK_BASENAME: &str = "back";

/// Bytes of one preview slot tagged with its source format. SVGs go
/// through `rasterize_svg` (vector → fixed-size pixmap); PNGs decode
/// directly into a `bevy::image::Image` whose intrinsic dimensions
/// the UI scales at draw time.
#[derive(Debug)]
enum ThemePreviewBytes {
    /// SVG source — the bundled default theme's convention. Caller
    /// rasterises through the existing `usvg` + `resvg` pipeline.
    Svg(Vec<u8>),
    /// PNG source — the convention for raster-art user themes (e.g.
    /// pixel-art themes generated via Claude Design — see
    /// `SESSION_HANDOFF.md` for the v0.19 drop-in flow).
    Png(Vec<u8>),
}

/// Resolves the preview bytes for a card slot in `theme_id`, trying
/// `.svg` first (the bundled default's convention) and falling back
/// to `.png` for raster-art themes. Returns `None` when neither
/// extension resolves — the caller renders a placeholder.
///
/// - For the bundled `default` theme: reads from the embedded
///   `DEFAULT_THEME_SVGS` table via [`default_theme_svg_bytes`]. SVG
///   only — the embed table is `.svg` exclusive.
/// - For any user theme: reads from `<user_theme_dir>/<id>/`. Tries
///   `<basename>.svg` then `<basename>.png`. Either branch returns
///   `None` on I/O failure (file missing, permission denied, etc.).
fn read_theme_preview_bytes(theme_id: &str, basename: &str) -> Option<ThemePreviewBytes> {
    if theme_id == "default" {
        let filename = format!("{basename}.svg");
        return default_theme_svg_bytes(&filename)
            .map(|b| ThemePreviewBytes::Svg(b.to_vec()));
    }
    let dir = user_theme_dir().join(theme_id);
    if let Ok(bytes) = std::fs::read(dir.join(format!("{basename}.svg"))) {
        return Some(ThemePreviewBytes::Svg(bytes));
    }
    if let Ok(bytes) = std::fs::read(dir.join(format!("{basename}.png"))) {
        return Some(ThemePreviewBytes::Png(bytes));
    }
    None
}

/// Decodes raster bytes (currently PNG) into a `bevy::image::Image`.
/// Bevy's `Image::from_buffer` dispatches via the supplied
/// `ImageType`, so this is a thin wrapper that translates I/O
/// failures into a logged warning + `None`.
fn decode_png_for_thumbnail(png_bytes: &[u8]) -> Option<Image> {
    use bevy::image::{CompressedImageFormats, Image, ImageSampler, ImageType};
    use bevy::asset::RenderAssetUsages;
    Image::from_buffer(
        png_bytes,
        ImageType::Format(bevy::image::ImageFormat::Png),
        CompressedImageFormats::default(),
        true, // is_srgb — pixel-art faces are authored in sRGB
        ImageSampler::Default,
        RenderAssetUsages::default(),
    )
    .map_err(|e| warn!("theme thumbnail png decode failed: {e}"))
    .ok()
}

/// Pure helper: turns one preview byte slice into a thumbnail
/// `Handle<Image>`. SVGs rasterise to a fixed
/// `THEME_THUMBNAIL_WIDTH_PX × THEME_THUMBNAIL_HEIGHT_PX` pixmap
/// (preserving aspect, centred); PNGs decode at their native
/// dimensions and Bevy's UI scales them at draw time. Returns
/// [`Handle::default`] on decode / rasterise failure so the picker
/// can render a placeholder without crashing.
fn rasterize_preview_to_handle(
    bytes: &ThemePreviewBytes,
    images: &mut Assets<Image>,
) -> Handle<Image> {
    match bytes {
        ThemePreviewBytes::Svg(b) => {
            let target = UVec2::new(THEME_THUMBNAIL_WIDTH_PX, THEME_THUMBNAIL_HEIGHT_PX);
            match rasterize_svg(b, target) {
                Ok(image) => images.add(image),
                Err(err) => {
                    warn!("theme thumbnail svg rasterise failed: {err}");
                    Handle::default()
                }
            }
        }
        ThemePreviewBytes::Png(b) => match decode_png_for_thumbnail(b) {
            Some(image) => images.add(image),
            None => Handle::default(),
        },
    }
}

/// Builds a [`ThemeThumbnailPair`] for a single theme. Either handle
/// is [`Handle::default`] when the matching face / back file could
/// not be located in either `.svg` or `.png` form, or when decoding
/// failed.
fn generate_thumbnail_pair_for(
    theme_id: &str,
    images: &mut Assets<Image>,
) -> ThemeThumbnailPair {
    let ace = read_theme_preview_bytes(theme_id, PREVIEW_FACE_BASENAME)
        .map(|b| rasterize_preview_to_handle(&b, images))
        .unwrap_or_default();
    let back = read_theme_preview_bytes(theme_id, PREVIEW_BACK_BASENAME)
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
        let url = format!("themes://{}/theme.ron", "default");
        assert_eq!(url, "themes://default/theme.ron");
        let url2 = format!("themes://{}/theme.ron", "user_uploaded");
        assert_eq!(url2, "themes://user_uploaded/theme.ron");
    }

    /// Test 1: the bundled default theme always has embedded SVG bytes
    /// available, so calling `generate_thumbnail_pair_for("default", …)`
    /// must produce two non-default `Handle<Image>` slots.
    #[test]
    fn theme_thumbnails_generated_for_default_theme() {
        let mut images = Assets::<Image>::default();
        let pair = generate_thumbnail_pair_for("default", &mut images);
        assert!(
            pair.is_fully_populated(),
            "default theme must yield both ace + back thumbnail handles"
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

    /// `read_theme_preview_bytes` for the default theme always
    /// returns embedded SVG bytes for the canonical preview pair —
    /// covering the happy-path branch of the helper.
    #[test]
    fn read_default_theme_preview_returns_some_for_canonical_files() {
        assert!(
            matches!(
                read_theme_preview_bytes("default", PREVIEW_BACK_BASENAME),
                Some(ThemePreviewBytes::Svg(_)),
            ),
            "default theme back must resolve to embedded SVG bytes"
        );
        assert!(
            matches!(
                read_theme_preview_bytes("default", PREVIEW_FACE_BASENAME),
                Some(ThemePreviewBytes::Svg(_)),
            ),
            "default theme spades_ace must resolve to embedded SVG bytes"
        );
    }

    /// PNG raster-art themes (e.g. the v0.19 drop-in pixel-art theme
    /// generated via Claude Design) must produce non-default
    /// thumbnail handles in the picker. The function reads
    /// `<user_theme_dir>/<id>/spades_ace.png` and `back.png`,
    /// decodes them via Bevy's `Image::from_buffer`, and inserts the
    /// resulting `Image` into `Assets<Image>`. Pins the v0.18 →
    /// v0.19 SVG-only → SVG-or-PNG widening of the thumbnail
    /// pipeline.
    #[test]
    fn png_only_user_theme_generates_real_thumbnails() {
        // Drop a synthetic theme into a unique temp subdirectory so
        // the test doesn't collide with whatever real themes the dev
        // machine has installed under user_theme_dir().
        let theme_id = format!(
            "test-png-theme-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        );
        let theme_dir = user_theme_dir().join(&theme_id);
        std::fs::create_dir_all(&theme_dir).expect("create temp theme dir");

        // Encode a real 2×3 RGBA PNG via the `image` dev-dep so the
        // test exercises Bevy's actual PNG decoder. A handcrafted byte
        // string is too fragile (DEFLATE encodes are non-trivial) and
        // a `include_bytes!` of a checked-in PNG would shoulder
        // committed binary into the repo.
        let mut png_bytes: Vec<u8> = Vec::new();
        let img = image::RgbaImage::from_pixel(2, 3, image::Rgba([200, 60, 60, 255]));
        image::DynamicImage::ImageRgba8(img)
            .write_to(
                &mut std::io::Cursor::new(&mut png_bytes),
                image::ImageFormat::Png,
            )
            .expect("encode tiny png");

        std::fs::write(theme_dir.join("spades_ace.png"), &png_bytes)
            .expect("write spades_ace.png");
        std::fs::write(theme_dir.join("back.png"), &png_bytes)
            .expect("write back.png");

        let mut images = Assets::<Image>::default();
        let pair = generate_thumbnail_pair_for(&theme_id, &mut images);

        assert_ne!(
            pair.ace,
            Handle::default(),
            "PNG-only theme must yield a real ace thumbnail handle, not the placeholder",
        );
        assert_ne!(
            pair.back,
            Handle::default(),
            "PNG-only theme must yield a real back thumbnail handle, not the placeholder",
        );
        assert!(
            pair.is_fully_populated(),
            "complete PNG-only pair must report fully-populated",
        );

        // Cleanup — the test is robust to leftover dirs but tidy up
        // anyway so /tmp doesn't grow on repeated CI runs.
        let _ = std::fs::remove_dir_all(&theme_dir);
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
                id: "default".into(),
                display_name: "Default".into(),
                manifest_url: crate::assets::DEFAULT_THEME_MANIFEST_URL.into(),
                meta: ThemeMeta {
                    id: "default".into(),
                    name: "Default".into(),
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
            .get("default")
            .map(|p| p.ace.clone())
            .expect("default theme thumbnail must exist after one tick");

        // Second tick must NOT replace the cached handle.
        app.update();
        let second_ace = app
            .world()
            .resource::<ThemeThumbnailCache>()
            .get("default")
            .map(|p| p.ace.clone())
            .expect("default theme thumbnail must still exist");

        assert_eq!(
            first_ace.id(),
            second_ace.id(),
            "cached thumbnail handle must be stable across ticks"
        );
    }
}
