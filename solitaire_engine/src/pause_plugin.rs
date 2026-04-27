//! Pause overlay (Esc).
//!
//! While paused:
//! - The `PausedResource` flag is true.
//! - Elapsed-time and Time Attack tickers stop counting (they read this
//!   resource and bail out early).
//!
//! Pressing Esc again dismisses the overlay and resumes ticking. Other
//! input (drag, keyboard hotkeys) is **not** blocked — pause is purely a
//! "stop the clock" screen for now. A future polish slice can layer
//! input-blocking on top if desired.

use bevy::prelude::*;
use solitaire_data::save_game_state_to;

use crate::game_plugin::GameStatePath;
use crate::resources::GameStateResource;

/// Toggleable flag read by `tick_elapsed_time` and `advance_time_attack`.
#[derive(Resource, Debug, Default)]
pub struct PausedResource(pub bool);

/// Marker on the pause overlay root node.
#[derive(Component, Debug)]
pub struct PauseScreen;

pub struct PausePlugin;

impl Plugin for PausePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PausedResource>()
            .add_systems(Update, toggle_pause);
    }
}

fn toggle_pause(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    mut paused: ResMut<PausedResource>,
    screens: Query<Entity, With<PauseScreen>>,
    game: Option<Res<GameStateResource>>,
    path: Option<Res<GameStatePath>>,
) {
    if !keys.just_pressed(KeyCode::Escape) {
        return;
    }
    if let Ok(entity) = screens.get_single() {
        commands.entity(entity).despawn_recursive();
        paused.0 = false;
    } else {
        spawn_pause_screen(&mut commands);
        paused.0 = true;
        // Persist the current game state whenever the player opens the pause
        // overlay so an OS-level kill still leaves a resumable save.
        if let (Some(g), Some(p)) = (game, path) {
            if let Some(disk_path) = p.0.as_deref() {
                if let Err(e) = save_game_state_to(disk_path, &g.0) {
                    warn!("game_state: failed to save on pause: {e}");
                }
            }
        }
    }
}

fn spawn_pause_screen(commands: &mut Commands) {
    commands
        .spawn((
            PauseScreen,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Percent(0.0),
                top: Val::Percent(0.0),
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                row_gap: Val::Px(8.0),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.82)),
            ZIndex(220),
        ))
        .with_children(|b| {
            b.spawn((
                Text::new("Paused"),
                TextFont {
                    font_size: 48.0,
                    ..default()
                },
                TextColor(Color::srgb(1.0, 0.87, 0.0)),
            ));
            b.spawn((
                Text::new("Press Esc to resume"),
                TextFont {
                    font_size: 22.0,
                    ..default()
                },
                TextColor(Color::srgb(0.85, 0.85, 0.80)),
            ));
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headless_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins).add_plugins(PausePlugin);
        app.init_resource::<ButtonInput<KeyCode>>();
        app.update();
        app
    }

    fn press_esc(app: &mut App) {
        let mut input = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        input.release(KeyCode::Escape);
        input.clear();
        input.press(KeyCode::Escape);
    }

    #[test]
    fn pressing_esc_pauses() {
        let mut app = headless_app();
        press_esc(&mut app);
        app.update();
        assert!(app.world().resource::<PausedResource>().0);
        assert_eq!(
            app.world_mut()
                .query::<&PauseScreen>()
                .iter(app.world())
                .count(),
            1
        );
    }

    #[test]
    fn pressing_esc_twice_resumes() {
        let mut app = headless_app();
        press_esc(&mut app);
        app.update();
        press_esc(&mut app);
        app.update();
        assert!(!app.world().resource::<PausedResource>().0);
        assert_eq!(
            app.world_mut()
                .query::<&PauseScreen>()
                .iter(app.world())
                .count(),
            0
        );
    }
}
