//! First-run onboarding banner.
//!
//! On startup, if `Settings.first_run_complete` is `false`, spawn a centered
//! welcome banner pointing at the **F1** cheat sheet. The first key or
//! mouse-button press dismisses it, sets the flag, and persists settings —
//! so returning players never see it again.
//!
//! **Key highlights** (#49): The key names **D** and **U** inside the
//! instructional text are rendered in a bright orange colour via `TextSpan`
//! children tagged with `KeyHighlightSpan`.

use std::path::PathBuf;

use bevy::prelude::*;
use solitaire_data::{save_settings_to, Settings};

use crate::settings_plugin::{SettingsResource, SettingsStoragePath};

/// Marker on the onboarding overlay root node.
#[derive(Component, Debug)]
pub struct OnboardingScreen;

/// Marker on `TextSpan` entities that display a key name (D, U …) in the
/// onboarding banner.  Colour distinct from body text; usable by tests and any
/// future flash-animation system.
#[derive(Component, Debug)]
pub struct KeyHighlightSpan;

/// Body text colour — golden yellow matching the rest of the UI.
const BODY_COLOR: Color = Color::srgb(1.0, 0.87, 0.0);

/// Bright orange used for key-name spans so they stand out from body text.
const KEY_COLOR: Color = Color::srgb(1.0, 0.55, 0.1);

pub struct OnboardingPlugin;

impl Plugin for OnboardingPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PostStartup, spawn_if_first_run)
            .add_systems(Update, dismiss_on_any_input);
    }
}

fn spawn_if_first_run(mut commands: Commands, settings: Option<Res<SettingsResource>>) {
    let Some(s) = settings else {
        return;
    };
    if s.0.first_run_complete {
        return;
    }
    spawn_onboarding_screen(&mut commands);
}

fn dismiss_on_any_input(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut settings: ResMut<SettingsResource>,
    path: Option<Res<SettingsStoragePath>>,
    screens: Query<Entity, With<OnboardingScreen>>,
) {
    let Ok(entity) = screens.get_single() else {
        return;
    };
    let pressed = keys.get_just_pressed().next().is_some()
        || mouse.get_just_pressed().next().is_some();
    if !pressed {
        return;
    }
    commands.entity(entity).despawn_recursive();
    settings.0.first_run_complete = true;
    persist(path.as_deref().map(|p| &p.0), &settings.0);
}

fn persist(path: Option<&Option<PathBuf>>, settings: &Settings) {
    let Some(Some(target)) = path else {
        return;
    };
    if let Err(e) = save_settings_to(target, settings) {
        warn!("failed to save settings (onboarding): {e}");
    }
}

fn spawn_onboarding_screen(commands: &mut Commands) {
    commands
        .spawn((
            OnboardingScreen,
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
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.92)),
            ZIndex(230),
        ))
        .with_children(|b| {
            // Title
            b.spawn((
                Text::new("Welcome to Solitaire Quest!"),
                TextFont { font_size: 40.0, ..default() },
                TextColor(BODY_COLOR),
            ));

            // Spacer
            b.spawn((Text::new(""), TextFont { font_size: 20.0, ..default() }));

            // Instruction line: "Drag cards between piles. Press D to draw, U to undo."
            // D is tagged KeyHighlightSpan; U uses KEY_COLOR but not the marker.
            b.spawn((
                Text::new("Drag cards between piles. Press "),
                TextFont { font_size: 22.0, ..default() },
                TextColor(BODY_COLOR),
            ))
            .with_children(|t| {
                t.spawn((
                    TextSpan::new("D"),
                    TextColor(KEY_COLOR),
                    KeyHighlightSpan,
                ));
                t.spawn((TextSpan::new(" to draw, "), TextColor(BODY_COLOR)));
                t.spawn((TextSpan::new("U"), TextColor(KEY_COLOR)));
                t.spawn((TextSpan::new(" to undo."), TextColor(BODY_COLOR)));
            });

            // Help line: "Press F1 at any time to see the full controls."
            b.spawn((
                Text::new("Press F1 at any time to see the full controls."),
                TextFont { font_size: 22.0, ..default() },
                TextColor(BODY_COLOR),
            ));

            // Spacer
            b.spawn((Text::new(""), TextFont { font_size: 20.0, ..default() }));

            // Dismiss hint
            b.spawn((
                Text::new("Press any key to begin"),
                TextFont { font_size: 20.0, ..default() },
                TextColor(Color::srgb(0.8, 0.8, 0.8)),
            ));
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings_plugin::SettingsPlugin;

    fn headless_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(SettingsPlugin::headless())
            .add_plugins(OnboardingPlugin);
        app.init_resource::<ButtonInput<KeyCode>>();
        app.init_resource::<ButtonInput<MouseButton>>();
        app
    }

    fn count_screens(app: &mut App) -> usize {
        app.world_mut()
            .query::<&OnboardingScreen>()
            .iter(app.world())
            .count()
    }

    #[test]
    fn first_run_spawns_banner() {
        let mut app = headless_app();
        app.update(); // PostStartup runs
        assert_eq!(count_screens(&mut app), 1);
    }

    #[test]
    fn returning_player_does_not_see_banner() {
        let mut app = headless_app();
        // Mark already-completed before PostStartup runs.
        app.world_mut()
            .resource_mut::<SettingsResource>()
            .0
            .first_run_complete = true;
        app.update();
        assert_eq!(count_screens(&mut app), 0);
    }

    #[test]
    fn keypress_dismisses_and_sets_flag() {
        let mut app = headless_app();
        app.update();
        assert_eq!(count_screens(&mut app), 1);

        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Space);
        app.update();

        assert_eq!(count_screens(&mut app), 0);
        assert!(
            app.world()
                .resource::<SettingsResource>()
                .0
                .first_run_complete,
            "first_run_complete should flip to true"
        );
    }

    #[test]
    fn mouseclick_dismisses_banner() {
        let mut app = headless_app();
        app.update();
        assert_eq!(count_screens(&mut app), 1);

        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        app.update();

        assert_eq!(count_screens(&mut app), 0);
    }

    #[test]
    fn banner_has_key_highlight_span_for_d() {
        // D must be tagged KeyHighlightSpan so its colour is distinct from body
        // text and future flash-animation systems can target it.
        let mut app = headless_app();
        app.update();
        let count = app
            .world_mut()
            .query::<&KeyHighlightSpan>()
            .iter(app.world())
            .count();
        assert_eq!(count, 1, "expected KeyHighlightSpan for D");
    }

    #[test]
    fn key_highlight_colour_differs_from_body_colour() {
        // Regression guard: KEY_COLOR must not accidentally match BODY_COLOR.
        assert_ne!(
            format!("{KEY_COLOR:?}"),
            format!("{BODY_COLOR:?}"),
            "key highlight colour should differ from body text colour"
        );
    }
}
