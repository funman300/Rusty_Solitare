//! Toggleable on-screen help / cheat sheet showing keyboard bindings.
//!
//! Press **F1** to toggle. Listed shortcuts are grouped by intent —
//! gameplay, modes, and overlays.

use bevy::prelude::*;

/// Marker on the help overlay root node.
#[derive(Component, Debug)]
pub struct HelpScreen;

pub struct HelpPlugin;

impl Plugin for HelpPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, toggle_help_screen);
    }
}

fn toggle_help_screen(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    screens: Query<Entity, With<HelpScreen>>,
) {
    if !keys.just_pressed(KeyCode::F1) {
        return;
    }
    if let Ok(entity) = screens.single() {
        commands.entity(entity).despawn();
    } else {
        spawn_help_screen(&mut commands);
    }
}

fn spawn_help_screen(commands: &mut Commands) {
    let lines: Vec<String> = vec![
        "=== Controls ===".to_string(),
        String::new(),
        "-- Gameplay --".to_string(),
        "  D            Draw from stock".to_string(),
        "  U            Undo last move".to_string(),
        "  Drag         Move cards between piles".to_string(),
        "  Click stock  Draw".to_string(),
        String::new(),
        "-- New Game --".to_string(),
        "  N            New Classic game (N twice if in progress)".to_string(),
        "  C            Start today's daily challenge".to_string(),
        "  Z            Start a Zen game (level 5+)".to_string(),
        "  X            Start the next Challenge (level 5+)".to_string(),
        "  T            Start a Time Attack session (level 5+)".to_string(),
        String::new(),
        "-- Overlays --".to_string(),
        "  S            Stats & progression".to_string(),
        "  A            Achievements".to_string(),
        "  L            Leaderboard".to_string(),
        "  O            Settings".to_string(),
        "  F1            This help screen".to_string(),
        "  F11           Toggle fullscreen".to_string(),
        "  Esc          Pause / resume".to_string(),
        "  [ / ]        SFX volume down / up".to_string(),
        String::new(),
        "Press F1 to close".to_string(),
    ];

    commands
        .spawn((
            HelpScreen,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Percent(0.0),
                top: Val::Percent(0.0),
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                row_gap: Val::Px(4.0),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.88)),
            ZIndex(210),
        ))
        .with_children(|b| {
            for line in lines {
                b.spawn((
                    Text::new(line),
                    TextFont {
                        font_size: 22.0,
                        ..default()
                    },
                    TextColor(Color::srgb(0.95, 0.95, 0.90)),
                ));
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headless_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins).add_plugins(HelpPlugin);
        app.init_resource::<ButtonInput<KeyCode>>();
        app.update();
        app
    }

    #[test]
    fn pressing_f1_spawns_help_screen() {
        let mut app = headless_app();
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::F1);
        app.update();

        assert_eq!(
            app.world_mut()
                .query::<&HelpScreen>()
                .iter(app.world())
                .count(),
            1
        );
    }

    #[test]
    fn pressing_f1_twice_closes_help_screen() {
        let mut app = headless_app();
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::F1);
        app.update();

        {
            let mut input = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            input.release(KeyCode::F1);
            input.clear();
            input.press(KeyCode::F1);
        }
        app.update();

        assert_eq!(
            app.world_mut()
                .query::<&HelpScreen>()
                .iter(app.world())
                .count(),
            0
        );
    }
}
