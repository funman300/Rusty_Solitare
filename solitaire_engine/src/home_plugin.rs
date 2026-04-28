//! Toggleable main menu overlay showing the current game mode and a full
//! keyboard shortcut reference.
//!
//! Press **M** to open or close the overlay.

use bevy::input::ButtonInput;
use bevy::prelude::*;
use solitaire_core::game_state::GameMode;

use crate::resources::GameStateResource;

/// Marker component on the home-menu overlay root node.
#[derive(Component, Debug)]
pub struct HomeScreen;

/// Registers the M-key toggle and the overlay spawn/despawn logic.
pub struct HomePlugin;

impl Plugin for HomePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, toggle_home_screen);
    }
}

fn toggle_home_screen(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    game: Res<GameStateResource>,
    screens: Query<Entity, With<HomeScreen>>,
) {
    if !keys.just_pressed(KeyCode::KeyM) {
        return;
    }
    if let Ok(entity) = screens.single() {
        commands.entity(entity).despawn();
    } else {
        spawn_home_screen(&mut commands, &game);
    }
}

/// Spawns the full-window home-menu overlay derived from the current `game` state.
fn spawn_home_screen(commands: &mut Commands, game: &GameStateResource) {
    let mode_label = match game.0.mode {
        GameMode::Classic => "Classic",
        GameMode::Zen => "Zen",
        GameMode::Challenge => "Challenge",
        GameMode::TimeAttack => "Time Attack",
    };

    commands
        .spawn((
            HomeScreen,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Percent(0.0),
                top: Val::Percent(0.0),
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::FlexStart,
                align_items: AlignItems::Center,
                row_gap: Val::Px(6.0),
                padding: UiRect::all(Val::Px(24.0)),
                overflow: Overflow::clip(),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.88)),
            ZIndex(200),
        ))
        .with_children(|root| {
            // Title
            root.spawn((
                Text::new("Solitaire Quest"),
                TextFont { font_size: 48.0, ..default() },
                TextColor(Color::srgb(1.0, 0.85, 0.3)),
            ));

            // Mode subtitle
            root.spawn((
                Text::new(format!("Current mode: {mode_label}")),
                TextFont { font_size: 28.0, ..default() },
                TextColor(Color::srgb(0.8, 0.8, 0.8)),
            ));

            // Spacer
            root.spawn(Node {
                height: Val::Px(8.0),
                ..default()
            });

            // "Game Controls" section header
            root.spawn((
                Text::new("Game Controls"),
                TextFont { font_size: 22.0, ..default() },
                TextColor(Color::srgb(0.9, 0.9, 0.9)),
            ));

            spawn_shortcut_row(root, "N", "New game  (N again confirms)");
            spawn_shortcut_row(root, "U", "Undo last move");
            spawn_shortcut_row(root, "Space / D", "Draw from stock");
            spawn_shortcut_row(root, "G", "Forfeit current game");
            spawn_shortcut_row(root, "Tab", "Cycle hint highlight");
            spawn_shortcut_row(root, "Enter", "Auto-complete if available");

            // Spacer
            root.spawn(Node {
                height: Val::Px(8.0),
                ..default()
            });

            // "Screens" section header
            root.spawn((
                Text::new("Screens"),
                TextFont { font_size: 22.0, ..default() },
                TextColor(Color::srgb(0.9, 0.9, 0.9)),
            ));

            spawn_shortcut_row(root, "M", "Main menu (this screen)");
            spawn_shortcut_row(root, "S", "Statistics");
            spawn_shortcut_row(root, "A", "Achievements");
            spawn_shortcut_row(root, "O", "Settings");
            spawn_shortcut_row(root, "P", "Profile");
            spawn_shortcut_row(root, "F1", "Help");
            spawn_shortcut_row(root, "F11", "Toggle fullscreen");
            spawn_shortcut_row(root, "Esc", "Pause / Resume");

            // Spacer
            root.spawn(Node {
                height: Val::Px(16.0),
                ..default()
            });

            // Dismiss hint
            root.spawn((
                Text::new("Press M to close"),
                TextFont { font_size: 16.0, ..default() },
                TextColor(Color::srgb(0.55, 0.55, 0.55)),
            ));
        });
}

fn spawn_shortcut_row(parent: &mut ChildSpawnerCommands, key: &str, action: &str) {
    parent
        .spawn(Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            min_width: Val::Px(380.0),
            column_gap: Val::Px(16.0),
            ..default()
        })
        .with_children(|row| {
            row.spawn((
                Text::new(key.to_string()),
                TextFont { font_size: 16.0, ..default() },
                TextColor(Color::srgb(1.0, 0.85, 0.4)),
                Node {
                    min_width: Val::Px(120.0),
                    ..default()
                },
            ));
            row.spawn((
                Text::new(action.to_string()),
                TextFont { font_size: 16.0, ..default() },
                TextColor(Color::srgb(0.85, 0.85, 0.85)),
            ));
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game_plugin::GamePlugin;
    use crate::table_plugin::TablePlugin;

    fn headless_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(GamePlugin)
            .add_plugins(TablePlugin)
            .add_plugins(HomePlugin);
        app.init_resource::<ButtonInput<KeyCode>>();
        app.update();
        app
    }

    #[test]
    fn pressing_m_spawns_home_screen() {
        let mut app = headless_app();
        assert_eq!(
            app.world_mut()
                .query::<&HomeScreen>()
                .iter(app.world())
                .count(),
            0
        );

        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyM);
        app.update();

        assert_eq!(
            app.world_mut()
                .query::<&HomeScreen>()
                .iter(app.world())
                .count(),
            1
        );
    }

    #[test]
    fn pressing_m_twice_closes_home_screen() {
        let mut app = headless_app();

        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyM);
        app.update();

        {
            let mut input = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            input.release(KeyCode::KeyM);
            input.clear();
            input.press(KeyCode::KeyM);
        }
        app.update();

        assert_eq!(
            app.world_mut()
                .query::<&HomeScreen>()
                .iter(app.world())
                .count(),
            0
        );
    }
}
