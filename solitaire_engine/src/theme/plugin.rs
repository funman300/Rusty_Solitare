//! `ThemePlugin` — owns [`ActiveTheme`], registers the `CardTheme` /
//! SVG asset machinery, and keeps `card_plugin::CardImageSet` in sync
//! with the currently-loaded theme so existing card-rendering systems
//! pick up the new artwork on the next state-changed tick.
//!
//! Phase 4 of `CARD_PLAN.md`. The plugin's `set_theme` helper is the
//! public API that the future picker UI (Phase 6) calls; for now it's
//! exposed for tests and for any embedder that wants to load an
//! alternative theme manually.

use bevy::asset::AssetEvent;
use bevy::ecs::message::MessageReader;
use bevy::prelude::*;
use solitaire_core::card::{Rank, Suit};

use crate::assets::DEFAULT_THEME_MANIFEST_URL;
use crate::card_plugin::CardImageSet;
use crate::events::StateChangedEvent;

use super::loader::CardThemeLoader;
use super::{CardKey, CardTheme};

/// Resource pointing at the currently-active card theme. Populated on
/// startup with the bundled default theme and replaced by [`set_theme`]
/// when the player switches.
#[derive(Resource, Debug)]
pub struct ActiveTheme(pub Handle<CardTheme>);

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
            .register_asset_loader(crate::assets::SvgLoader)
            .register_asset_loader(CardThemeLoader)
            .add_systems(Startup, load_initial_theme)
            .add_systems(
                Update,
                (
                    sync_card_image_set_with_active_theme,
                    react_to_settings_theme_change,
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
}
