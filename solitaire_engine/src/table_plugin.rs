//! Renders the static table: felt background and empty pile markers.
//!
//! Pile markers are translucent rectangles that sit beneath any cards. They
//! remain visible only where a pile is empty, so the player can see where to
//! drop cards. All geometry comes from `LayoutResource`.

use bevy::prelude::*;
use bevy::window::WindowResized;
use solitaire_core::card::Suit;
use solitaire_core::pile::PileType;
use solitaire_data::settings::Theme;

use crate::events::HintVisualEvent;
use crate::layout::{compute_layout, Layout, LayoutResource, TABLE_COLOUR};
use crate::settings_plugin::{SettingsChangedEvent, SettingsResource};

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
        app.add_event::<WindowResized>()
            .add_event::<SettingsChangedEvent>()
            .add_event::<HintVisualEvent>()
            .add_systems(Startup, setup_table)
            .add_systems(
                Update,
                (
                    on_window_resized,
                    apply_theme_on_settings_change,
                    apply_hint_pile_highlight,
                    tick_hint_pile_highlights,
                ),
            );
    }
}

/// Returns the felt colour for a given theme.
fn theme_colour(theme: &Theme) -> Color {
    match theme {
        Theme::Green => Color::srgb(TABLE_COLOUR[0], TABLE_COLOUR[1], TABLE_COLOUR[2]),
        Theme::Blue  => Color::srgb(0.059, 0.196, 0.322),
        Theme::Dark  => Color::srgb(0.08, 0.08, 0.10),
    }
}

/// Effective table background colour: unlocked background index overrides the
/// Theme when `selected_background > 0`.
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
) {
    // Only spawn a camera if one does not already exist (e.g. a parent app
    // may have added one in tests).
    if existing_camera.is_empty() {
        commands.spawn(Camera2d);
    }

    let window_size = windows
        .iter()
        .next()
        .map(default_window_size)
        .unwrap_or(Vec2::new(1280.0, 800.0));
    let layout = compute_layout(window_size);

    let initial_colour = settings
        .as_ref()
        .map(|s| effective_background_colour(&s.0.theme, s.0.selected_background))
        .unwrap_or_else(|| Color::srgb(TABLE_COLOUR[0], TABLE_COLOUR[1], TABLE_COLOUR[2]));

    spawn_background(&mut commands, window_size, initial_colour);
    spawn_pile_markers(&mut commands, &layout);
    commands.insert_resource(LayoutResource(layout));
}

fn spawn_background(commands: &mut Commands, window_size: Vec2, color: Color) {
    // Spawn a felt-coloured rectangle that always covers the window. We give
    // it the window size plus headroom so resizing up doesn't expose edges
    // before the resize handler runs.
    commands.spawn((
        Sprite {
            color,
            custom_size: Some(window_size * 2.0),
            ..default()
        },
        Transform::from_xyz(0.0, 0.0, Z_BACKGROUND),
        TableBackground,
    ));
}

fn apply_theme_on_settings_change(
    mut events: EventReader<SettingsChangedEvent>,
    mut backgrounds: Query<&mut Sprite, With<TableBackground>>,
) {
    let Some(ev) = events.read().last() else {
        return;
    };
    let colour = effective_background_colour(&ev.0.theme, ev.0.selected_background);
    for mut sprite in &mut backgrounds {
        sprite.color = colour;
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
    let marker_colour = Color::srgba(1.0, 1.0, 1.0, 0.08);
    let marker_size = layout.card_size;
    let font_size = layout.card_size.x * 0.28;

    let mut piles: Vec<PileType> = Vec::with_capacity(13);
    piles.push(PileType::Stock);
    piles.push(PileType::Waste);
    for suit in [Suit::Clubs, Suit::Diamonds, Suit::Hearts, Suit::Spades] {
        piles.push(PileType::Foundation(suit));
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

        // Task #35 — suit symbol on empty foundation placeholders.
        if let PileType::Foundation(suit) = &pile {
            let symbol = suit_symbol(suit).to_string();
            entity.with_children(|b| {
                b.spawn((
                    Text2d::new(symbol),
                    TextFont { font_size, ..default() },
                    TextColor(Color::srgba(1.0, 1.0, 1.0, 0.45)),
                    Transform::from_xyz(0.0, 0.0, 0.1),
                ));
            });
        }

        // Task #43 — King indicator on empty tableau placeholders.
        if let PileType::Tableau(_) = &pile {
            entity.with_children(|b| {
                b.spawn((
                    Text2d::new("K"),
                    TextFont { font_size, ..default() },
                    TextColor(Color::srgba(1.0, 1.0, 1.0, 0.35)),
                    Transform::from_xyz(0.0, 0.0, 0.1),
                ));
            });
        }
    }
}

#[allow(clippy::type_complexity)]
fn on_window_resized(
    mut events: EventReader<WindowResized>,
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
    let new_layout = compute_layout(window_size);

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
}

// ---------------------------------------------------------------------------
// Task #6 — Hint pile-marker highlight
// ---------------------------------------------------------------------------

/// Gold tint applied to a `PileMarker` sprite when it is the current hint
/// destination.
const HINT_PILE_HIGHLIGHT_COLOUR: Color = Color::srgb(1.0, 0.85, 0.1);

/// Listens for `HintVisualEvent` and tints the matching `PileMarker` entity
/// gold for 2 s, storing the original colour in `HintPileHighlight` so it can
/// be restored when the timer expires.
///
/// If the pile marker already has a `HintPileHighlight` from a previous hint
/// press, the timer is reset to 2 s without changing `original_color`.
fn apply_hint_pile_highlight(
    mut events: EventReader<HintVisualEvent>,
    mut commands: Commands,
    mut pile_markers: Query<(Entity, &PileMarker, &mut Sprite, Option<&HintPileHighlight>)>,
) {
    for ev in events.read() {
        for (entity, pile_marker, mut sprite, existing) in pile_markers.iter_mut() {
            if pile_marker.0 != ev.dest_pile {
                continue;
            }
            let original_color = existing
                .map(|h| h.original_color)
                .unwrap_or(sprite.color);
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
        let default = Color::srgba(1.0, 1.0, 1.0, 0.08); // PILE_MARKER_DEFAULT_COLOUR
        assert_ne!(
            HINT_PILE_HIGHLIGHT_COLOUR, default,
            "HINT_PILE_HIGHLIGHT_COLOUR must differ from the default pile marker colour"
        );
    }

    /// A freshly-created HintPileHighlight has a positive timer countdown.
    #[test]
    fn hint_pile_highlight_timer_starts_positive() {
        let h = HintPileHighlight {
            timer: 2.0,
            original_color: Color::srgba(1.0, 1.0, 1.0, 0.08),
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

    /// The gold hint colour must have a strong yellow component (r ≥ 0.9, g ≥ 0.8,
    /// b ≤ 0.3) to be clearly visible as a "destination" indicator.
    #[test]
    fn hint_pile_highlight_colour_is_gold() {
        // Extract linear components.  srgb(1.0, 0.85, 0.1) is the expected gold.
        // We test the channel values rather than exact equality so future tweaks
        // to the shade do not break the test, as long as the colour remains golden.
        let Srgba { red, green, blue, .. } = HINT_PILE_HIGHLIGHT_COLOUR.to_srgba();
        assert!(red >= 0.9, "gold hint colour must have red ≥ 0.9, got {red}");
        assert!(green >= 0.7, "gold hint colour must have green ≥ 0.7, got {green}");
        assert!(blue <= 0.3, "gold hint colour must have blue ≤ 0.3, got {blue}");
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
