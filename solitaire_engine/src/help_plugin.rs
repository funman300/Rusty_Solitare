//! Toggleable on-screen help / cheat sheet showing keyboard bindings.
//!
//! Reachable from the HUD "Help" button (per the UI-first principle); `F1`
//! is an optional accelerator. Listed shortcuts are grouped by intent —
//! gameplay, modes, and overlays.

use bevy::prelude::*;

use crate::events::HelpRequestEvent;
use crate::font_plugin::FontResource;
use crate::ui_modal::{
    spawn_modal, spawn_modal_actions, spawn_modal_button, spawn_modal_header, ButtonVariant,
};
use crate::ui_theme::{
    Z_MODAL_PANEL, BORDER_SUBTLE, RADIUS_SM, SPACE_2, TEXT_PRIMARY, TEXT_SECONDARY, TYPE_BODY,
    TYPE_CAPTION, VAL_SPACE_1, VAL_SPACE_2, VAL_SPACE_3,
};

/// Marker on the help overlay root node.
#[derive(Component, Debug)]
pub struct HelpScreen;

/// Marker on the "Done" button inside the Help modal.
#[derive(Component, Debug)]
pub struct HelpCloseButton;

/// Spawns and despawns the help / controls overlay shown when the player
/// clicks the "Help" HUD button or presses `F1`. All hotkeys and gesture
/// guides live here.
pub struct HelpPlugin;

impl Plugin for HelpPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<HelpRequestEvent>()
            .add_systems(Update, (toggle_help_screen, handle_help_close_button));
    }
}

fn toggle_help_screen(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    mut requests: MessageReader<HelpRequestEvent>,
    screens: Query<Entity, With<HelpScreen>>,
    font_res: Option<Res<FontResource>>,
) {
    // Either F1 or a click on the HUD "Help" button (which fires
    // HelpRequestEvent) toggles the overlay.
    let button_clicked = requests.read().count() > 0;
    if !keys.just_pressed(KeyCode::F1) && !button_clicked {
        return;
    }
    if let Ok(entity) = screens.single() {
        commands.entity(entity).despawn();
    } else {
        spawn_help_screen(&mut commands, font_res.as_deref());
    }
}

/// Click handler for the modal's "Done" button. F1 toggles the overlay
/// the same way; this just exposes the close action to mouse / touch.
fn handle_help_close_button(
    mut commands: Commands,
    close_buttons: Query<&Interaction, (With<HelpCloseButton>, Changed<Interaction>)>,
    screens: Query<Entity, With<HelpScreen>>,
) {
    if !close_buttons.iter().any(|i| *i == Interaction::Pressed) {
        return;
    }
    for entity in &screens {
        commands.entity(entity).despawn();
    }
}

/// Each entry in the controls reference table.
struct ControlRow {
    keys: &'static str,
    description: &'static str,
}

/// Each section of the controls reference. Sections render with a
/// section title and a vertically stacked list of `ControlRow`s.
struct ControlSection {
    title: &'static str,
    rows: &'static [ControlRow],
}

const CONTROL_SECTIONS: &[ControlSection] = &[
    ControlSection {
        title: "Gameplay",
        rows: &[
            ControlRow { keys: "Drag", description: "Move cards between piles" },
            ControlRow { keys: "D / Space", description: "Draw from stock" },
            ControlRow { keys: "U", description: "Undo last move" },
            ControlRow { keys: "Click stock", description: "Draw" },
        ],
    },
    ControlSection {
        title: "New Game",
        rows: &[
            ControlRow { keys: "N", description: "New Classic game (N twice if in progress)" },
            ControlRow { keys: "C", description: "Start today's daily challenge" },
            ControlRow { keys: "Z", description: "Start a Zen game (level 5+)" },
            ControlRow { keys: "X", description: "Start the next Challenge (level 5+)" },
            ControlRow { keys: "T", description: "Start a Time Attack session (level 5+)" },
        ],
    },
    ControlSection {
        title: "Overlays",
        rows: &[
            ControlRow { keys: "S", description: "Stats & progression" },
            ControlRow { keys: "A", description: "Achievements" },
            ControlRow { keys: "L", description: "Leaderboard" },
            ControlRow { keys: "O", description: "Settings" },
            ControlRow { keys: "F1", description: "This help screen" },
            ControlRow { keys: "F11", description: "Toggle fullscreen" },
            ControlRow { keys: "Esc", description: "Pause / resume" },
            ControlRow { keys: "[ / ]", description: "SFX volume down / up" },
        ],
    },
];

fn spawn_help_screen(commands: &mut Commands, font_res: Option<&FontResource>) {
    let font_handle = font_res.map(|f| f.0.clone()).unwrap_or_default();
    let font_section = TextFont {
        font: font_handle.clone(),
        font_size: TYPE_BODY,
        ..default()
    };
    let font_row = font_section.clone();
    let font_kbd = TextFont {
        font: font_handle,
        font_size: TYPE_CAPTION,
        ..default()
    };

    spawn_modal(commands, HelpScreen, Z_MODAL_PANEL, |card| {
        spawn_modal_header(card, "Controls", font_res);

        for section in CONTROL_SECTIONS {
            // Section title in muted text — distinguishes from row content.
            card.spawn((
                Text::new(section.title),
                font_section.clone(),
                TextColor(TEXT_SECONDARY),
            ));

            // Each row is a flex-row: kbd-style chip + description.
            for row in section.rows {
                card.spawn(Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: VAL_SPACE_3,
                    ..default()
                })
                .with_children(|line| {
                    // The hotkey rendered as a small chip with a border —
                    // visual cue that it's a key reference, not part of
                    // the description text.
                    line.spawn((
                        Node {
                            padding: UiRect::axes(VAL_SPACE_2, VAL_SPACE_1),
                            min_width: Val::Px(64.0),
                            justify_content: JustifyContent::Center,
                            border: UiRect::all(Val::Px(1.0)),
                            border_radius: BorderRadius::all(Val::Px(RADIUS_SM)),
                            ..default()
                        },
                        BorderColor::all(BORDER_SUBTLE),
                    ))
                    .with_children(|chip| {
                        chip.spawn((
                            Text::new(row.keys),
                            font_kbd.clone(),
                            TextColor(TEXT_PRIMARY),
                        ));
                    });
                    line.spawn((
                        Text::new(row.description),
                        font_row.clone(),
                        TextColor(TEXT_PRIMARY),
                    ));
                });
            }

            // Section spacer — small empty box. Keeps each section
            // visually grouped.
            card.spawn(Node {
                height: Val::Px(SPACE_2),
                ..default()
            });
        }

        spawn_modal_actions(card, |actions| {
            spawn_modal_button(
                actions,
                HelpCloseButton,
                "Done",
                Some("F1"),
                ButtonVariant::Primary,
                font_res,
            );
        });
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
