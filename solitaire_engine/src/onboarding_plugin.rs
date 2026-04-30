//! First-run onboarding multi-slide flow.
//!
//! On startup, if `Settings.first_run_complete` is `false`, a three-slide
//! modal flow is shown. The player navigates with a primary `Next` button
//! (`→` / `Enter` accelerators) and a secondary `Back` button (`←`).
//! The final slide's primary button is `Start playing`, which sets
//! `first_run_complete = true` and persists settings — exactly as the
//! previous single-screen implementation did.
//!
//! Slides:
//!
//! 1. **Welcome** — brief introduction to Solitaire Quest.
//! 2. **How to play** — drag-and-drop, double-click, and right-click hints.
//! 3. **Keyboard shortcuts** — a summary pulled from the same canonical list
//!    used in `HelpScreen`. Accelerators: `Esc` anywhere in the flow skips
//!    the whole thing (equivalent to `first_run_complete = true`).
//!
//! Slide state is tracked by the [`OnboardingSlideIndex`] resource (0-based,
//! max `SLIDE_COUNT - 1`). Button clicks and keyboard accelerators update the
//! resource, then `rebuild_slide` despawns the current modal and respawns the
//! next one.

use std::path::PathBuf;

use bevy::prelude::*;
use solitaire_data::{save_settings_to, Settings};

use crate::font_plugin::FontResource;
use crate::settings_plugin::{SettingsResource, SettingsStoragePath};
use crate::ui_modal::{
    spawn_modal, spawn_modal_actions, spawn_modal_body_text, spawn_modal_button,
    spawn_modal_header, ButtonVariant,
};
use crate::ui_theme::{
    BORDER_SUBTLE, RADIUS_SM, TEXT_PRIMARY, TEXT_SECONDARY, TYPE_CAPTION, TYPE_BODY, VAL_SPACE_1,
    VAL_SPACE_2, VAL_SPACE_3, Z_ONBOARDING,
};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Total number of onboarding slides (0-based index goes 0..SLIDE_COUNT-1).
const SLIDE_COUNT: u8 = 3;

// ---------------------------------------------------------------------------
// Components (private — never re-exported)
// ---------------------------------------------------------------------------

/// Marker on the onboarding overlay scrim (root entity for this modal).
#[derive(Component, Debug)]
pub struct OnboardingScreen;

/// Marker on the `Next` / `Start playing` primary button.
#[derive(Component, Debug)]
struct OnboardingNextButton;

/// Marker on the `Back` secondary button.
#[derive(Component, Debug)]
struct OnboardingBackButton;

/// Marker on the `Skip` tertiary button (slide 0 only).
#[derive(Component, Debug)]
struct OnboardingSkipButton;

// ---------------------------------------------------------------------------
// Resource
// ---------------------------------------------------------------------------

/// Which slide (0-indexed) the player is currently viewing.
///
/// Persists across the despawn/respawn cycle so the rebuild system knows
/// which slide to spawn next.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct OnboardingSlideIndex(pub u8);

// ---------------------------------------------------------------------------
// Slide data — hotkey rows are taken verbatim from `help_plugin.rs` so the
// two screens stay in sync without a shared abstraction.
// ---------------------------------------------------------------------------

/// A single `key — description` pair shown on slide 3.
struct HotkeyRow {
    keys: &'static str,
    description: &'static str,
}

/// Most-used shortcuts from the `help_plugin` canonical list.
///
/// Updating the list in `help_plugin.rs` should be mirrored here. The
/// ARCHITECTURE.md decision log calls out that we copy values rather than
/// refactor the help plugin.
const HOTKEYS: &[HotkeyRow] = &[
    HotkeyRow { keys: "D / Space", description: "Draw from stock" },
    HotkeyRow { keys: "U", description: "Undo last move" },
    HotkeyRow { keys: "N", description: "New Classic game" },
    HotkeyRow { keys: "S", description: "Stats & progression" },
    HotkeyRow { keys: "A", description: "Achievements" },
    HotkeyRow { keys: "O", description: "Settings" },
    HotkeyRow { keys: "Esc", description: "Pause / resume" },
    HotkeyRow { keys: "F1", description: "Help / controls" },
];

// ---------------------------------------------------------------------------
// Plugin
// ---------------------------------------------------------------------------

/// Drives the first-run multi-slide onboarding flow.
pub struct OnboardingPlugin;

impl Plugin for OnboardingPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<OnboardingSlideIndex>()
            .add_systems(PostStartup, spawn_if_first_run)
            .add_systems(
                Update,
                (
                    handle_onboarding_buttons,
                    handle_onboarding_keyboard,
                )
                    .chain(),
            );
    }
}

// ---------------------------------------------------------------------------
// Startup
// ---------------------------------------------------------------------------

fn spawn_if_first_run(
    mut commands: Commands,
    settings: Option<Res<SettingsResource>>,
    font_res: Option<Res<FontResource>>,
    mut slide_index: ResMut<OnboardingSlideIndex>,
) {
    let Some(s) = settings else { return };
    if s.0.first_run_complete {
        return;
    }
    slide_index.0 = 0;
    spawn_slide(&mut commands, 0, font_res.as_deref());
}

// ---------------------------------------------------------------------------
// Button click handler
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn handle_onboarding_buttons(
    mut commands: Commands,
    next_buttons: Query<&Interaction, (With<OnboardingNextButton>, Changed<Interaction>)>,
    back_buttons: Query<&Interaction, (With<OnboardingBackButton>, Changed<Interaction>)>,
    skip_buttons: Query<&Interaction, (With<OnboardingSkipButton>, Changed<Interaction>)>,
    screens: Query<Entity, With<OnboardingScreen>>,
    mut slide_index: ResMut<OnboardingSlideIndex>,
    mut settings: Option<ResMut<SettingsResource>>,
    path: Option<Res<SettingsStoragePath>>,
    font_res: Option<Res<FontResource>>,
) {
    let next_pressed = next_buttons.iter().any(|i| *i == Interaction::Pressed);
    let back_pressed = back_buttons.iter().any(|i| *i == Interaction::Pressed);
    let skip_pressed = skip_buttons.iter().any(|i| *i == Interaction::Pressed);

    if !next_pressed && !back_pressed && !skip_pressed {
        return;
    }

    if skip_pressed || (next_pressed && slide_index.0 == SLIDE_COUNT - 1) {
        // Skip or final-slide "Start playing" — complete onboarding.
        complete_onboarding(
            &mut commands,
            &screens,
            settings.as_deref_mut(),
            path.as_deref(),
        );
        return;
    }

    // Navigate between slides.
    let new_index = if next_pressed {
        (slide_index.0 + 1).min(SLIDE_COUNT - 1)
    } else {
        slide_index.0.saturating_sub(1)
    };

    if new_index != slide_index.0 {
        despawn_screen(&mut commands, &screens);
        slide_index.0 = new_index;
        spawn_slide(&mut commands, new_index, font_res.as_deref());
    }
}

// ---------------------------------------------------------------------------
// Keyboard accelerator handler
// ---------------------------------------------------------------------------

fn handle_onboarding_keyboard(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    screens: Query<Entity, With<OnboardingScreen>>,
    mut slide_index: ResMut<OnboardingSlideIndex>,
    mut settings: Option<ResMut<SettingsResource>>,
    path: Option<Res<SettingsStoragePath>>,
    font_res: Option<Res<FontResource>>,
) {
    if screens.is_empty() {
        return;
    }

    let advance = keys.just_pressed(KeyCode::ArrowRight) || keys.just_pressed(KeyCode::Enter);
    let retreat = keys.just_pressed(KeyCode::ArrowLeft);
    let skip = keys.just_pressed(KeyCode::Escape);

    if skip || (advance && slide_index.0 == SLIDE_COUNT - 1) {
        complete_onboarding(
            &mut commands,
            &screens,
            settings.as_deref_mut(),
            path.as_deref(),
        );
        return;
    }

    if advance {
        let new_index = (slide_index.0 + 1).min(SLIDE_COUNT - 1);
        if new_index != slide_index.0 {
            despawn_screen(&mut commands, &screens);
            slide_index.0 = new_index;
            spawn_slide(&mut commands, new_index, font_res.as_deref());
        }
    } else if retreat && slide_index.0 > 0 {
        let new_index = slide_index.0 - 1;
        despawn_screen(&mut commands, &screens);
        slide_index.0 = new_index;
        spawn_slide(&mut commands, new_index, font_res.as_deref());
    }
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn despawn_screen(commands: &mut Commands, screens: &Query<Entity, With<OnboardingScreen>>) {
    for entity in screens {
        commands.entity(entity).despawn();
    }
}

fn complete_onboarding(
    commands: &mut Commands,
    screens: &Query<Entity, With<OnboardingScreen>>,
    settings: Option<&mut SettingsResource>,
    path: Option<&SettingsStoragePath>,
) {
    despawn_screen(commands, screens);
    if let Some(s) = settings {
        s.0.first_run_complete = true;
        persist(path.map(|p| &p.0), &s.0);
    }
}

fn persist(path: Option<&Option<PathBuf>>, settings: &Settings) {
    let Some(Some(target)) = path else { return };
    if let Err(e) = save_settings_to(target, settings) {
        warn!("failed to save settings (onboarding): {e}");
    }
}

// ---------------------------------------------------------------------------
// Slide spawning
// ---------------------------------------------------------------------------

fn spawn_slide(commands: &mut Commands, index: u8, font_res: Option<&FontResource>) {
    match index {
        0 => spawn_slide_welcome(commands, font_res),
        1 => spawn_slide_how_to_play(commands, font_res),
        2 => spawn_slide_hotkeys(commands, font_res),
        _ => spawn_slide_welcome(commands, font_res),
    }
}

/// Slide 1 — Welcome.
fn spawn_slide_welcome(commands: &mut Commands, font_res: Option<&FontResource>) {
    spawn_modal(commands, OnboardingScreen, Z_ONBOARDING, |card| {
        spawn_modal_header(card, "Welcome to Solitaire Quest", font_res);
        spawn_modal_body_text(
            card,
            "Solitaire Quest is a free, offline-first Klondike Solitaire game. \
             Play classic draw-1 or draw-3 Klondike, earn XP, unlock achievements, \
             and compete on the leaderboard. Your progress is saved locally — \
             optional sync to your own server keeps it in step across all your devices.",
            TEXT_SECONDARY,
            font_res,
        );
        spawn_modal_actions(card, |actions| {
            spawn_modal_button(
                actions,
                OnboardingSkipButton,
                "Skip",
                Some("Esc"),
                ButtonVariant::Tertiary,
                font_res,
            );
            spawn_modal_button(
                actions,
                OnboardingNextButton,
                "Next",
                Some("→"),
                ButtonVariant::Primary,
                font_res,
            );
        });
    });
}

/// Slide 2 — How to play.
fn spawn_slide_how_to_play(commands: &mut Commands, font_res: Option<&FontResource>) {
    spawn_modal(commands, OnboardingScreen, Z_ONBOARDING, |card| {
        spawn_modal_header(card, "Drag cards to play", font_res);
        spawn_modal_body_text(
            card,
            "Left-click and drag any face-up card to move it between piles. \
             You can drag a whole column at once by grabbing the topmost card \
             you want to move. Double-click a face-up card to send it to a \
             foundation pile automatically (when the move is legal). \
             Right-click a card for a hint — valid destinations will highlight.",
            TEXT_SECONDARY,
            font_res,
        );
        spawn_modal_actions(card, |actions| {
            spawn_modal_button(
                actions,
                OnboardingBackButton,
                "Back",
                Some("←"),
                ButtonVariant::Secondary,
                font_res,
            );
            spawn_modal_button(
                actions,
                OnboardingNextButton,
                "Next",
                Some("→"),
                ButtonVariant::Primary,
                font_res,
            );
        });
    });
}

/// Slide 3 — Keyboard shortcuts.
fn spawn_slide_hotkeys(commands: &mut Commands, font_res: Option<&FontResource>) {
    let font_handle = font_res.map(|f| f.0.clone()).unwrap_or_default();
    let font_row = TextFont {
        font: font_handle.clone(),
        font_size: TYPE_BODY,
        ..default()
    };
    let font_kbd = TextFont {
        font: font_handle,
        font_size: TYPE_CAPTION,
        ..default()
    };

    spawn_modal(commands, OnboardingScreen, Z_ONBOARDING, |card| {
        spawn_modal_header(card, "Keyboard shortcuts", font_res);

        // Vertical list of `key — description` rows, same chip style as HelpScreen.
        for row in HOTKEYS {
            card.spawn(Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: VAL_SPACE_3,
                ..default()
            })
            .with_children(|line| {
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

        spawn_modal_actions(card, |actions| {
            spawn_modal_button(
                actions,
                OnboardingBackButton,
                "Back",
                Some("←"),
                ButtonVariant::Secondary,
                font_res,
            );
            spawn_modal_button(
                actions,
                OnboardingNextButton,
                "Start playing",
                Some("→"),
                ButtonVariant::Primary,
                font_res,
            );
        });
    });
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

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

    fn current_slide(app: &App) -> u8 {
        app.world().resource::<OnboardingSlideIndex>().0
    }

    fn press_key(app: &mut App, key: KeyCode) {
        let mut input = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        input.release(key);
        input.clear();
        input.press(key);
    }

    // -----------------------------------------------------------------------
    // Basic visibility
    // -----------------------------------------------------------------------

    #[test]
    fn first_run_spawns_onboarding() {
        let mut app = headless_app();
        app.update(); // PostStartup runs
        assert_eq!(count_screens(&mut app), 1);
    }

    #[test]
    fn returning_player_does_not_see_onboarding() {
        let mut app = headless_app();
        app.world_mut()
            .resource_mut::<SettingsResource>()
            .0
            .first_run_complete = true;
        app.update();
        assert_eq!(count_screens(&mut app), 0);
    }

    #[test]
    fn starts_on_slide_zero() {
        let mut app = headless_app();
        app.update();
        assert_eq!(current_slide(&app), 0);
    }

    // -----------------------------------------------------------------------
    // Next / Back navigation
    // -----------------------------------------------------------------------

    #[test]
    fn next_button_advances_slide() {
        let mut app = headless_app();
        app.update();
        assert_eq!(current_slide(&app), 0);

        // Spawn a Next button with Pressed interaction.
        app.world_mut().spawn((OnboardingNextButton, Button, Interaction::Pressed));
        app.update();

        assert_eq!(current_slide(&app), 1, "Next must advance to slide 1");
        assert_eq!(count_screens(&mut app), 1, "exactly one modal must be visible");
    }

    #[test]
    fn back_button_retreats_slide() {
        let mut app = headless_app();
        app.update();
        // Manually move to slide 2.
        app.world_mut().resource_mut::<OnboardingSlideIndex>().0 = 2;
        // Despawn the old screen and respawn slide 2.
        {
            let entities: Vec<Entity> = app
                .world_mut()
                .query_filtered::<Entity, With<OnboardingScreen>>()
                .iter(app.world())
                .collect();
            for e in entities {
                app.world_mut().despawn(e);
            }
        }

        app.world_mut().spawn((OnboardingBackButton, Button, Interaction::Pressed));
        app.update();

        assert_eq!(current_slide(&app), 1, "Back must retreat from slide 2 to slide 1");
    }

    #[test]
    fn back_on_first_slide_does_not_underflow() {
        let mut app = headless_app();
        app.update();
        assert_eq!(current_slide(&app), 0);

        // Pressing Back on slide 0 must be a no-op (no underflow to u8::MAX).
        app.world_mut().spawn((OnboardingBackButton, Button, Interaction::Pressed));
        app.update();

        assert_eq!(current_slide(&app), 0, "Back on slide 0 must not underflow");
        // The screen must still be present (we didn't skip or complete).
        assert_eq!(count_screens(&mut app), 1);
    }

    #[test]
    fn next_cannot_advance_past_last_slide() {
        let mut app = headless_app();
        app.update();
        app.world_mut().resource_mut::<OnboardingSlideIndex>().0 = SLIDE_COUNT - 1;

        // Next on the last slide should complete onboarding, not advance further.
        app.world_mut().spawn((OnboardingNextButton, Button, Interaction::Pressed));
        app.update();

        // first_run_complete must be set.
        assert!(
            app.world().resource::<SettingsResource>().0.first_run_complete,
            "Next on last slide must set first_run_complete"
        );
        assert_eq!(count_screens(&mut app), 0, "modal must be gone after completion");
    }

    // -----------------------------------------------------------------------
    // Skip
    // -----------------------------------------------------------------------

    #[test]
    fn skip_button_completes_onboarding_from_slide_zero() {
        let mut app = headless_app();
        app.update();

        app.world_mut().spawn((OnboardingSkipButton, Button, Interaction::Pressed));
        app.update();

        assert!(
            app.world().resource::<SettingsResource>().0.first_run_complete,
            "Skip must set first_run_complete"
        );
        assert_eq!(count_screens(&mut app), 0);
    }

    // -----------------------------------------------------------------------
    // Keyboard accelerators
    // -----------------------------------------------------------------------

    #[test]
    fn arrow_right_advances_slide() {
        let mut app = headless_app();
        app.update();
        assert_eq!(current_slide(&app), 0);

        press_key(&mut app, KeyCode::ArrowRight);
        app.update();

        assert_eq!(current_slide(&app), 1);
    }

    #[test]
    fn enter_advances_slide() {
        let mut app = headless_app();
        app.update();

        press_key(&mut app, KeyCode::Enter);
        app.update();

        assert_eq!(current_slide(&app), 1);
    }

    #[test]
    fn arrow_left_retreats_slide() {
        let mut app = headless_app();
        app.update();
        app.world_mut().resource_mut::<OnboardingSlideIndex>().0 = 1;
        // Re-spawn a screen so the keyboard handler finds one.
        app.world_mut().spawn(OnboardingScreen);

        press_key(&mut app, KeyCode::ArrowLeft);
        app.update();

        assert_eq!(current_slide(&app), 0);
    }

    #[test]
    fn esc_skips_onboarding() {
        let mut app = headless_app();
        app.update();
        assert_eq!(count_screens(&mut app), 1);

        press_key(&mut app, KeyCode::Escape);
        app.update();

        assert_eq!(count_screens(&mut app), 0, "Esc must dismiss onboarding");
        assert!(
            app.world().resource::<SettingsResource>().0.first_run_complete,
            "Esc must set first_run_complete"
        );
    }

    #[test]
    fn enter_on_last_slide_completes_onboarding() {
        let mut app = headless_app();
        app.update();
        app.world_mut().resource_mut::<OnboardingSlideIndex>().0 = SLIDE_COUNT - 1;
        // Ensure a screen exists for the keyboard handler.
        app.world_mut().spawn(OnboardingScreen);

        press_key(&mut app, KeyCode::Enter);
        app.update();

        assert!(
            app.world().resource::<SettingsResource>().0.first_run_complete,
            "Enter on last slide must complete onboarding"
        );
        assert_eq!(count_screens(&mut app), 0);
    }

    // -----------------------------------------------------------------------
    // Slide-index bounds
    // -----------------------------------------------------------------------

    #[test]
    fn slide_count_constant_is_three() {
        assert_eq!(SLIDE_COUNT, 3, "SLIDE_COUNT must be 3");
    }

    #[test]
    fn slide_index_default_is_zero() {
        let idx = OnboardingSlideIndex::default();
        assert_eq!(idx.0, 0);
    }

    // -----------------------------------------------------------------------
    // Completion semantics
    // -----------------------------------------------------------------------

    #[test]
    fn keypress_on_last_slide_sets_first_run_complete() {
        let mut app = headless_app();
        app.update();

        // Navigate to the last slide via arrow keys.
        for _ in 0..(SLIDE_COUNT - 1) {
            press_key(&mut app, KeyCode::ArrowRight);
            app.update();
            {
                let mut input = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
                input.clear();
            }
        }
        assert_eq!(current_slide(&app), SLIDE_COUNT - 1);

        press_key(&mut app, KeyCode::Enter);
        app.update();

        assert!(
            app.world().resource::<SettingsResource>().0.first_run_complete,
            "completing the last slide must set first_run_complete"
        );
        assert_eq!(count_screens(&mut app), 0);
    }

    // -----------------------------------------------------------------------
    // Hotkey list is non-empty (guards against accidental truncation)
    // -----------------------------------------------------------------------

    #[test]
    fn hotkey_list_is_non_empty() {
        assert!(!HOTKEYS.is_empty(), "HOTKEYS must not be empty");
    }

    #[test]
    fn all_hotkey_rows_have_non_empty_fields() {
        for row in HOTKEYS {
            assert!(!row.keys.is_empty(), "hotkey key field must not be empty");
            assert!(!row.description.is_empty(), "hotkey description must not be empty");
        }
    }
}
