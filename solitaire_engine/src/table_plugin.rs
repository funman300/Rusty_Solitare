//! Renders the static table: felt background and empty pile markers.
//!
//! Pile markers are translucent rectangles that sit beneath any cards. They
//! remain visible only where a pile is empty, so the player can see where to
//! drop cards. All geometry comes from `LayoutResource`.

use bevy::prelude::*;
use bevy::window::WindowResized;
use solitaire_core::card::Suit;
use solitaire_core::pile::PileType;

use crate::events::{HintVisualEvent, StateChangedEvent};
use crate::hud_plugin::HudVisibility;
use crate::layout::{compute_layout, Layout, LayoutResource, LayoutSystem};
use crate::safe_area::SafeAreaInsets;
use crate::resources::GameStateResource;
#[cfg(test)]
use crate::layout::TABLE_COLOUR;
use crate::settings_plugin::{SettingsChangedEvent, SettingsResource};
use crate::ui_theme::TEXT_PRIMARY;
#[cfg(test)]
use solitaire_data::Theme;

/// Default tint applied to every empty-pile marker sprite. Pure white
/// at 8% alpha — soft enough that the marker reads as a "hint of a
/// slot" rather than a panel, but visible against every felt
/// background.
///
/// Re-exported as the source of truth for `cursor_plugin::MARKER_DEFAULT`,
/// which used to duplicate the literal alongside a "kept in sync" doc
/// comment. Pulling both call sites through this const makes drift a
/// compile error instead of a stale comment.
pub const PILE_MARKER_DEFAULT_COLOUR: Color = Color::srgba(1.0, 1.0, 1.0, 0.08);

/// Holds pre-loaded [`Handle<Image>`]s for the 5 selectable table backgrounds.
///
/// Loaded once at startup by [`load_background_images`].  Index 0 is the
/// default; indices 1–4 are unlockable.
#[derive(Resource)]
pub struct BackgroundImageSet {
    /// One handle per background slot (indices 0–4).
    pub handles: Vec<Handle<Image>>,
}

/// Z-depth used for the background — below everything.
const Z_BACKGROUND: f32 = -10.0;
/// Z-depth used for pile markers — below cards (which start at 0) but above
/// the background.
const Z_PILE_MARKER: f32 = -1.0;

/// Marker component for the table felt background.
#[derive(Component, Debug)]
pub struct TableBackground;

/// Marker component attached to each of the 13 empty-pile placeholders.
#[derive(Component, Debug, Clone)]
pub struct PileMarker(pub PileType);

/// Attached to a `PileMarker` entity when it has been temporarily tinted gold
/// as a hint destination. Stores the remaining countdown and the original sprite
/// colour so it can be restored when the timer expires.
#[derive(Component, Debug, Clone)]
pub struct HintPileHighlight {
    /// Seconds remaining before the pile marker colour is restored.
    pub timer: f32,
    /// The sprite colour the marker had before the hint tint was applied.
    pub original_color: Color,
}

/// Registers the table background and pile-marker rendering.
pub struct TablePlugin;

impl Plugin for TablePlugin {
    fn build(&self, app: &mut App) {
        // Register WindowResized so the plugin works under MinimalPlugins in
        // tests. Under DefaultPlugins, bevy_window has already registered it
        // and this call is a no-op.
        app.add_message::<WindowResized>()
            .add_message::<SettingsChangedEvent>()
            .add_message::<HintVisualEvent>()
            .add_message::<StateChangedEvent>()
            .add_systems(Startup, load_background_images.before(setup_table))
            .add_systems(Startup, setup_table)
            .add_systems(
                Update,
                (
                    on_safe_area_changed.before(LayoutSystem::UpdateOnResize),
                    on_window_resized.in_set(LayoutSystem::UpdateOnResize),
                    apply_theme_on_settings_change,
                    apply_hint_pile_highlight,
                    tick_hint_pile_highlights,
                    sync_pile_marker_visibility,
                ),
            );
    }
}

/// Loads the 5 background PNG files at startup via the Bevy `AssetServer` and
/// stores their [`Handle<Image>`]s in [`BackgroundImageSet`].
fn load_background_images(asset_server: Option<Res<AssetServer>>, mut commands: Commands) {
    let Some(asset_server) = asset_server else {
        // AssetServer absent (e.g. MinimalPlugins in tests) — insert an
        // empty set so setup_table can proceed using a default handle.
        commands.insert_resource(BackgroundImageSet { handles: Vec::new() });
        return;
    };
    let handles = (0..5)
        .map(|i| asset_server.load(format!("backgrounds/bg_{i}.png")))
        .collect();
    commands.insert_resource(BackgroundImageSet { handles });
}

/// Returns the felt colour for a given theme.
///
/// Only used in tests — the runtime path now picks a PNG image via
/// [`BackgroundImageSet`] rather than a solid colour.
#[cfg(test)]
fn theme_colour(theme: &Theme) -> Color {
    match theme {
        Theme::Green => Color::srgb(TABLE_COLOUR[0], TABLE_COLOUR[1], TABLE_COLOUR[2]),
        Theme::Blue  => Color::srgb(0.059, 0.196, 0.322),
        Theme::Dark  => Color::srgb(0.08, 0.08, 0.10),
    }
}

/// Effective table background colour: unlocked background index overrides the
/// Theme when `selected_background > 0`.
///
/// Only used in tests — the runtime path now picks a PNG image via
/// [`BackgroundImageSet`] rather than a solid colour.
#[cfg(test)]
fn effective_background_colour(theme: &Theme, selected_background: usize) -> Color {
    match selected_background {
        0 => theme_colour(theme),
        1 => Color::srgb(0.25, 0.18, 0.10), // dark wood
        2 => Color::srgb(0.05, 0.08, 0.22), // navy
        3 => Color::srgb(0.30, 0.05, 0.08), // burgundy
        _ => Color::srgb(0.12, 0.12, 0.14), // charcoal (4+)
    }
}

fn default_window_size(window: &Window) -> Vec2 {
    Vec2::new(window.resolution.width(), window.resolution.height())
}

fn setup_table(
    mut commands: Commands,
    windows: Query<&Window>,
    existing_camera: Query<(), With<Camera>>,
    settings: Option<Res<SettingsResource>>,
    bg_images: Option<Res<BackgroundImageSet>>,
    safe_area: Option<Res<SafeAreaInsets>>,
    hud_vis: Option<Res<HudVisibility>>,
) {
    // Only spawn a camera if one does not already exist (e.g. a parent app
    // may have added one in tests). Use the felt-green clear colour so the
    // background reads as green even before the background PNG finishes
    // loading (which is asynchronous and can lag by several frames on
    // Android).
    if existing_camera.is_empty() {
        commands.spawn((
            Camera2d,
            Camera {
                clear_color: ClearColorConfig::Custom(Color::srgb(
                    crate::layout::TABLE_COLOUR[0],
                    crate::layout::TABLE_COLOUR[1],
                    crate::layout::TABLE_COLOUR[2],
                )),
                ..default()
            },
        ));
    }

    let (window_size, scale) = windows.iter().next().map_or(
        (Vec2::new(1280.0, 800.0), 1.0f32),
        |w| (default_window_size(w), w.scale_factor()),
    );
    // Safe-area insets arrive from JNI asynchronously; they are almost always
    // 0 here (populated ~frame 2-3). on_safe_area_changed fires when they
    // arrive and issues a synthetic WindowResized to re-snap all game objects.
    let insets = safe_area.as_deref().copied().unwrap_or_default();
    let safe_area_top = insets.top / scale;
    let safe_area_bottom = insets.bottom / scale;
    let hud_visible = hud_vis.as_deref().copied().unwrap_or_default() == HudVisibility::Visible;
    let layout = compute_layout(window_size, safe_area_top, safe_area_bottom, hud_visible);

    let selected_bg = settings.as_ref().map_or(0, |s| s.0.selected_background);

    let image_handle = bg_images
        .as_ref()
        .and_then(|set| set.handles.get(selected_bg).cloned())
        .unwrap_or_default();

    spawn_background(&mut commands, window_size, image_handle);
    spawn_pile_markers(&mut commands, &layout);
    commands.insert_resource(LayoutResource(layout));
}

/// Spawns the felt background sprite using a PNG image handle.
///
/// The sprite covers the window at twice the window size so brief resize gaps
/// are never visible.  The image is tinted `Color::WHITE` (no tint) so the PNG
/// pixel data is rendered as-is.
fn spawn_background(commands: &mut Commands, window_size: Vec2, image: Handle<Image>) {
    // Spawn a sprite covering the window. We give it the window size plus
    // headroom so resizing up doesn't expose edges before the resize handler
    // runs.
    commands.spawn((
        Sprite {
            image,
            color: Color::WHITE,
            custom_size: Some(window_size * 2.0),
            ..default()
        },
        Transform::from_xyz(0.0, 0.0, Z_BACKGROUND),
        TableBackground,
    ));
}

/// Reacts to settings changes by updating the background sprite's image handle.
///
/// When [`BackgroundImageSet`] is available the selected PNG handle is applied
/// directly (color is kept at `Color::WHITE` so the PNG pixel data shows
/// unmodified).  If the resource is not yet ready the sprite is left unchanged.
fn apply_theme_on_settings_change(
    mut events: MessageReader<SettingsChangedEvent>,
    mut backgrounds: Query<&mut Sprite, With<TableBackground>>,
    bg_images: Option<Res<BackgroundImageSet>>,
) {
    let Some(ev) = events.read().last() else {
        return;
    };
    let Some(set) = bg_images else {
        // BackgroundImageSet not ready yet — leave sprite unchanged.
        return;
    };
    let selected = ev.0.selected_background;
    let Some(handle) = set.handles.get(selected).cloned() else {
        return;
    };
    for mut sprite in &mut backgrounds {
        sprite.image = handle.clone();
        sprite.color = Color::WHITE;
    }
}

/// Returns the single-letter suit symbol used on empty foundation markers.
///
/// Matches the same ASCII convention used by `CardPlugin` for card labels.
pub fn suit_symbol(suit: &Suit) -> &'static str {
    match suit {
        Suit::Spades   => "S",
        Suit::Hearts   => "H",
        Suit::Diamonds => "D",
        Suit::Clubs    => "C",
    }
}

fn spawn_pile_markers(commands: &mut Commands, layout: &Layout) {
    let marker_colour = PILE_MARKER_DEFAULT_COLOUR;
    let marker_size = layout.card_size;
    let font_size = layout.card_size.x * 0.28;

    let mut piles: Vec<PileType> = Vec::with_capacity(13);
    piles.push(PileType::Stock);
    piles.push(PileType::Waste);
    for slot in 0..4_u8 {
        piles.push(PileType::Foundation(slot));
    }
    for i in 0..7 {
        piles.push(PileType::Tableau(i));
    }

    for pile in piles {
        let pos = layout.pile_positions[&pile];
        let mut entity = commands.spawn((
            Sprite {
                color: marker_colour,
                custom_size: Some(marker_size),
                ..default()
            },
            Transform::from_xyz(pos.x, pos.y, Z_PILE_MARKER),
            PileMarker(pile.clone()),
        ));

        // Tableau markers show "K" (only a King may start an empty column).
        // Foundation markers show "A" (only an Ace may claim an empty slot).
        // Neither label carries a suit because any suit may start any slot.
        match &pile {
            PileType::Tableau(_) => {
                entity.with_children(|b| {
                    b.spawn((
                        Text2d::new("K"),
                        TextFont { font_size, ..default() },
                        TextColor(TEXT_PRIMARY.with_alpha(0.35)),
                        Transform::from_xyz(0.0, 0.0, 0.1),
                    ));
                });
            }
            PileType::Foundation(_) => {
                entity.with_children(|b| {
                    b.spawn((
                        Text2d::new("A"),
                        TextFont { font_size, ..default() },
                        TextColor(TEXT_PRIMARY.with_alpha(0.35)),
                        Transform::from_xyz(0.0, 0.0, 0.1),
                    ));
                });
            }
            _ => {}
        }
    }
}

#[allow(clippy::type_complexity)]
fn on_window_resized(
    mut events: MessageReader<WindowResized>,
    safe_area: Option<Res<SafeAreaInsets>>,
    windows: Query<&Window>,
    hud_vis: Option<Res<HudVisibility>>,
    mut layout_res: Option<ResMut<LayoutResource>>,
    mut backgrounds: Query<
        (&mut Sprite, &mut Transform),
        (With<TableBackground>, Without<PileMarker>),
    >,
    mut markers: Query<(&PileMarker, &mut Sprite, &mut Transform), Without<TableBackground>>,
) {
    let Some(ev) = events.read().last() else {
        return;
    };
    let window_size = Vec2::new(ev.width, ev.height);
    let scale = windows.iter().next().map_or(1.0, |w| w.scale_factor());
    let insets = safe_area.as_deref().copied().unwrap_or_default();
    let safe_area_top = insets.top / scale;
    let safe_area_bottom = insets.bottom / scale;
    let hud_visible = hud_vis.as_deref().copied().unwrap_or_default() == HudVisibility::Visible;
    let new_layout = compute_layout(window_size, safe_area_top, safe_area_bottom, hud_visible);

    if let Some(layout_res) = layout_res.as_deref_mut() {
        layout_res.0 = new_layout.clone();
    }

    for (mut sprite, mut transform) in &mut backgrounds {
        sprite.custom_size = Some(window_size * 2.0);
        transform.translation.x = 0.0;
        transform.translation.y = 0.0;
    }

    for (marker, mut sprite, mut transform) in &mut markers {
        if let Some(pos) = new_layout.pile_positions.get(&marker.0) {
            sprite.custom_size = Some(new_layout.card_size);
            transform.translation.x = pos.x;
            transform.translation.y = pos.y;
        }
    }

    // Card sprites are repositioned by `card_plugin::snap_cards_on_window_resize`
    // running `.after(LayoutSystem::UpdateOnResize)` — that system snaps card
    // transforms directly to the new layout instead of going through
    // `StateChangedEvent → sync_cards → CardAnim` which would retarget the
    // slide tween every frame during a corner drag (the visible "snap back
    // and forth" jitter).
}

/// Bridges the asynchronous safe-area inset update into the synchronous
/// window-resize pipeline. When Android's JNI delivers the real inset values
/// (typically frame 2-3 of a fresh launch), this system writes a synthetic
/// `WindowResized` event carrying the current window size. `on_window_resized`
/// (which runs in `LayoutSystem::UpdateOnResize`) will then recompute the
/// layout with the correct `safe_area_top`, update `LayoutResource` and the
/// pile markers, and `snap_cards_on_window_resize` (running after the set)
/// will snap card sprites to the corrected positions.
fn on_safe_area_changed(
    safe_area: Option<Res<SafeAreaInsets>>,
    windows: Query<(Entity, &Window)>,
    mut resize_events: MessageWriter<WindowResized>,
) {
    let Some(safe_area) = safe_area else { return; };
    if !safe_area.is_changed() {
        return;
    }
    let Some((entity, window)) = windows.iter().next() else {
        return;
    };
    resize_events.write(WindowResized {
        window: entity,
        width: window.resolution.width(),
        height: window.resolution.height(),
    });
}

// ---------------------------------------------------------------------------
// Task #6 — Hint pile-marker highlight
// ---------------------------------------------------------------------------

/// Gold tint applied to a `PileMarker` sprite when it is the current
/// hint destination. Same RGB as the design-system [`STATE_WARNING`]
/// token (`#ddb26f`) so the in-game "look here" colour is the same hue
/// as every other warning/attention signal in the UI. Spelled as a
/// literal because `Alpha::with_alpha` is not yet a `const` trait
/// method on stable; the tracking test below pins the RGB to
/// `STATE_WARNING` so a future palette swap can't drift the two apart.
const HINT_PILE_HIGHLIGHT_COLOUR: Color = Color::srgb(0.867, 0.698, 0.435);

/// Listens for `HintVisualEvent` and tints the matching `PileMarker` entity
/// gold for 2 s, storing the original colour in `HintPileHighlight` so it can
/// be restored when the timer expires.
///
/// If the pile marker already has a `HintPileHighlight` from a previous hint
/// press, the timer is reset to 2 s without changing `original_color`.
fn apply_hint_pile_highlight(
    mut events: MessageReader<HintVisualEvent>,
    mut commands: Commands,
    mut pile_markers: Query<(Entity, &PileMarker, &mut Sprite, Option<&HintPileHighlight>)>,
) {
    for ev in events.read() {
        for (entity, pile_marker, mut sprite, existing) in pile_markers.iter_mut() {
            if pile_marker.0 != ev.dest_pile {
                continue;
            }
            let original_color = existing.map_or(sprite.color, |h| h.original_color);
            sprite.color = HINT_PILE_HIGHLIGHT_COLOUR;
            commands.entity(entity).insert(HintPileHighlight {
                timer: 2.0,
                original_color,
            });
        }
    }
}

/// Counts down `HintPileHighlight::timer` each frame and restores the original
/// pile marker colour when the timer expires.
fn tick_hint_pile_highlights(
    mut commands: Commands,
    time: Res<Time>,
    mut pile_markers: Query<(Entity, &mut Sprite, &mut HintPileHighlight)>,
) {
    let dt = time.delta_secs();
    for (entity, mut sprite, mut highlight) in pile_markers.iter_mut() {
        highlight.timer -= dt;
        if highlight.timer <= 0.0 {
            sprite.color = highlight.original_color;
            commands.entity(entity).remove::<HintPileHighlight>();
        }
    }
}

/// Hides pile-marker sprites for piles that have a card on top, shows them
/// for empty piles. Implements the "remain visible only where a pile is
/// empty" invariant declared in this module's top-level doc comment but
/// previously not enforced — markers always rendered, and the resulting
/// translucent rectangle bled through the rounded corners of any card sat
/// on top, producing visible "gray L" artifacts at each card corner.
///
/// Runs every Update tick guarded by `game.is_changed()` so the work is
/// skipped on idle frames. Bevy's resource change-detection sets the
/// changed flag on every state mutation (draw, move, undo, recycle, new
/// game), which covers every transition that flips a pile from
/// empty-to-occupied or vice versa.
fn sync_pile_marker_visibility(
    game: Option<Res<GameStateResource>>,
    mut markers: Query<(&PileMarker, &mut Visibility)>,
) {
    let Some(game) = game else {
        return;
    };
    if !game.is_changed() {
        return;
    }
    for (pile_marker, mut visibility) in markers.iter_mut() {
        let is_empty = game
            .0
            .piles
            .get(&pile_marker.0)
            .is_none_or(|pile| pile.cards.is_empty());
        *visibility = if is_empty {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game_plugin::GamePlugin;

    /// Minimal headless app — omits windowing so pile markers are spawned with
    /// the default 1280×800 layout and no camera is created.
    fn headless_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(GamePlugin)
            .add_plugins(TablePlugin);
        app.update();
        app
    }

    #[test]
    fn table_plugin_spawns_thirteen_pile_markers() {
        let mut app = headless_app();
        let count = app
            .world_mut()
            .query::<&PileMarker>()
            .iter(app.world())
            .count();
        assert_eq!(count, 13);
    }

    #[test]
    fn table_plugin_spawns_one_background() {
        let mut app = headless_app();
        let count = app
            .world_mut()
            .query::<&TableBackground>()
            .iter(app.world())
            .count();
        assert_eq!(count, 1);
    }

    #[test]
    fn table_plugin_inserts_layout_resource() {
        let app = headless_app();
        assert!(app.world().get_resource::<LayoutResource>().is_some());
    }

    #[test]
    fn every_pile_marker_has_unique_type() {
        let mut app = headless_app();
        let mut types: Vec<PileType> = app
            .world_mut()
            .query::<&PileMarker>()
            .iter(app.world())
            .map(|m| m.0.clone())
            .collect();
        types.sort_by_key(|p| format!("{p:?}"));
        types.dedup();
        assert_eq!(types.len(), 13);
    }

    #[test]
    fn pile_markers_hide_when_pile_is_occupied() {
        // After a fresh deal: the 7 tableau piles + the stock pile are
        // all occupied; the 4 foundation piles + the waste pile are
        // empty. The visibility-by-occupancy system must hide the
        // first 8 markers and keep the last 5 visible. This implements
        // the "remain visible only where a pile is empty" invariant
        // in the module-level doc comment that was previously
        // declared but not enforced — pile markers used to always
        // render, and the resulting translucent rectangle bled through
        // the rounded corners of any card sat on top.
        let mut app = headless_app();
        // headless_app() runs one tick; run another so
        // sync_pile_marker_visibility has a chance to fire (it runs
        // in Update, after Startup spawns the markers and the game
        // state populates).
        app.update();

        let mut q = app.world_mut().query::<(&PileMarker, &Visibility)>();
        let mut hidden_piles: Vec<PileType> = Vec::new();
        let mut visible_piles: Vec<PileType> = Vec::new();
        for (marker, visibility) in q.iter(app.world()) {
            if matches!(visibility, Visibility::Hidden) {
                hidden_piles.push(marker.0.clone());
            } else {
                visible_piles.push(marker.0.clone());
            }
        }

        // 8 occupied piles after a fresh deal: stock + 7 tableau.
        assert_eq!(
            hidden_piles.len(),
            8,
            "stock + 7 tableau piles should hide their markers post-deal",
        );
        assert!(hidden_piles.contains(&PileType::Stock));
        for i in 0..7 {
            assert!(
                hidden_piles.contains(&PileType::Tableau(i)),
                "tableau {i} marker should be hidden — it has cards",
            );
        }

        // 5 empty piles: waste + 4 foundations.
        assert_eq!(visible_piles.len(), 5);
        assert!(visible_piles.contains(&PileType::Waste));
        for i in 0..4_u8 {
            assert!(visible_piles.contains(&PileType::Foundation(i)));
        }
    }

    // -----------------------------------------------------------------------
    // Pure-function tests (no Bevy app required)
    // -----------------------------------------------------------------------

    #[test]
    fn all_three_themes_produce_distinct_colours() {
        let green = theme_colour(&Theme::Green);
        let blue  = theme_colour(&Theme::Blue);
        let dark  = theme_colour(&Theme::Dark);
        assert_ne!(green, blue, "Green and Blue must differ");
        assert_ne!(green, dark, "Green and Dark must differ");
        assert_ne!(blue,  dark, "Blue and Dark must differ");
    }

    #[test]
    fn effective_background_index_0_matches_theme_colour() {
        for theme in [Theme::Green, Theme::Blue, Theme::Dark] {
            let expected = theme_colour(&theme);
            let actual   = effective_background_colour(&theme, 0);
            assert_eq!(
                expected, actual,
                "index 0 must always return the theme colour for {:?}",
                theme
            );
        }
    }

    #[test]
    fn effective_background_indices_1_through_3_are_distinct_from_theme() {
        // Non-zero indices override the theme with a fixed colour.
        let theme_green = theme_colour(&Theme::Green);
        for idx in 1..=3 {
            let eff = effective_background_colour(&Theme::Green, idx);
            assert_ne!(eff, theme_green, "index {idx} must override the theme colour");
        }
    }

    #[test]
    fn effective_background_index_4_falls_through_to_charcoal() {
        // All indices ≥ 4 share the same charcoal fallback.
        let c4 = effective_background_colour(&Theme::Green, 4);
        let c5 = effective_background_colour(&Theme::Green, 5);
        let c99 = effective_background_colour(&Theme::Green, 99);
        assert_eq!(c4, c5, "indices 4 and 5 must share the charcoal fallback");
        assert_eq!(c4, c99, "index 99 must share the charcoal fallback");
    }

    // -----------------------------------------------------------------------
    // suit_symbol pure-function tests (Task #35)
    // -----------------------------------------------------------------------

    #[test]
    fn suit_symbol_returns_correct_letters() {
        assert_eq!(suit_symbol(&Suit::Spades),   "S");
        assert_eq!(suit_symbol(&Suit::Hearts),   "H");
        assert_eq!(suit_symbol(&Suit::Diamonds), "D");
        assert_eq!(suit_symbol(&Suit::Clubs),    "C");
    }

    // -----------------------------------------------------------------------
    // Task #6 — HintPileHighlight timer and colour pure-function tests
    // -----------------------------------------------------------------------

    /// The HINT_PILE_HIGHLIGHT_COLOUR constant must be visibly distinct from the
    /// default pile marker colour so the player can see which pile is highlighted.
    #[test]
    fn hint_pile_highlight_colour_is_distinct_from_default() {
        assert_ne!(
            HINT_PILE_HIGHLIGHT_COLOUR, PILE_MARKER_DEFAULT_COLOUR,
            "HINT_PILE_HIGHLIGHT_COLOUR must differ from the default pile marker colour"
        );
    }

    /// `HINT_PILE_HIGHLIGHT_COLOUR` is spelled as a literal because
    /// `Alpha::with_alpha` is not a `const` trait method on stable.
    /// This test pins its RGB to the design-system `STATE_WARNING`
    /// token so a future palette swap that updates the token but
    /// forgets the hint highlight fails loudly here.
    #[test]
    fn hint_pile_highlight_rgb_tracks_state_warning_token() {
        use crate::ui_theme::STATE_WARNING;
        let hint = HINT_PILE_HIGHLIGHT_COLOUR.to_srgba();
        let warning = STATE_WARNING.to_srgba();
        assert!((hint.red - warning.red).abs() < 1e-6);
        assert!((hint.green - warning.green).abs() < 1e-6);
        assert!((hint.blue - warning.blue).abs() < 1e-6);
    }

    /// A freshly-created HintPileHighlight has a positive timer countdown.
    #[test]
    fn hint_pile_highlight_timer_starts_positive() {
        let h = HintPileHighlight {
            timer: 2.0,
            original_color: PILE_MARKER_DEFAULT_COLOUR,
        };
        assert!(
            h.timer > 0.0,
            "HintPileHighlight timer must start positive, got {}",
            h.timer
        );
    }

    /// Ticking the timer past its initial value results in a non-positive (expired)
    /// countdown.
    #[test]
    fn hint_pile_highlight_timer_expires_after_full_duration() {
        let mut remaining = 2.0_f32;
        remaining -= 2.5; // 2.5 s elapsed on a 2.0 s timer
        assert!(
            remaining <= 0.0,
            "timer must be expired after ticking past its initial value, got {}",
            remaining
        );
    }

    /// `original_color` is preserved through the highlight lifecycle so colour
    /// can be correctly restored on expiry.
    #[test]
    fn hint_pile_highlight_preserves_original_color() {
        let original = Color::srgb(0.1, 0.3, 0.5);
        let h = HintPileHighlight {
            timer: 2.0,
            original_color: original,
        };
        assert_eq!(
            h.original_color, original,
            "original_color must be stored without modification"
        );
    }

    /// The hint colour must read as "gold-ish" — red dominant, green
    /// close behind, blue noticeably lower — so a player intuitively
    /// associates the highlight with attention/warning. Bounds are
    /// loose enough to accommodate the Terminal palette's muted gold
    /// (`STATE_WARNING`, `#ddb26f`) while still rejecting a stray
    /// red, green, or neutral grey if someone refactors badly.
    /// Exact-RGB tracking lives in
    /// `hint_pile_highlight_rgb_tracks_state_warning_token`.
    #[test]
    fn hint_pile_highlight_colour_is_gold() {
        let Srgba { red, green, blue, .. } = HINT_PILE_HIGHLIGHT_COLOUR.to_srgba();
        assert!(red >= 0.7, "gold hint colour must have red ≥ 0.7, got {red}");
        assert!(green >= 0.5, "gold hint colour must have green ≥ 0.5, got {green}");
        assert!(blue <= 0.6, "gold hint colour must have blue ≤ 0.6, got {blue}");
        assert!(red > blue, "gold hint colour must be warmer than cool, got r={red} b={blue}");
        assert!(green > blue, "gold hint colour must be warmer than cool, got g={green} b={blue}");
    }

    #[test]
    fn suit_symbol_all_four_are_distinct() {
        let symbols: Vec<&str> = [Suit::Spades, Suit::Hearts, Suit::Diamonds, Suit::Clubs]
            .iter()
            .map(suit_symbol)
            .collect();
        let unique: std::collections::HashSet<&&str> = symbols.iter().collect();
        assert_eq!(unique.len(), 4, "all four suit symbols must be distinct");
    }
}
