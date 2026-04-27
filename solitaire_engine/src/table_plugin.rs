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

/// Registers the table background and pile-marker rendering.
pub struct TablePlugin;

impl Plugin for TablePlugin {
    fn build(&self, app: &mut App) {
        // Register WindowResized so the plugin works under MinimalPlugins in
        // tests. Under DefaultPlugins, bevy_window has already registered it
        // and this call is a no-op.
        app.add_event::<WindowResized>()
            .add_event::<SettingsChangedEvent>()
            .add_systems(Startup, setup_table)
            .add_systems(Update, (on_window_resized, apply_theme_on_settings_change));
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

fn spawn_pile_markers(commands: &mut Commands, layout: &Layout) {
    let marker_colour = Color::srgba(1.0, 1.0, 1.0, 0.08);
    let marker_size = layout.card_size;

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
        commands.spawn((
            Sprite {
                color: marker_colour,
                custom_size: Some(marker_size),
                ..default()
            },
            Transform::from_xyz(pos.x, pos.y, Z_PILE_MARKER),
            PileMarker(pile),
        ));
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
}
