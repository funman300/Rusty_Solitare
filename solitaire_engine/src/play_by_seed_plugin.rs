//! Play-by-Seed dialog: lets the player type a decimal seed number and start
//! a Classic game with that exact deal. A live solver-verification badge
//! updates asynchronously after a short typing debounce so the player knows
//! whether the deal is provably winnable before committing.
//!
//! # Flow
//!
//! 1. `HomePlugin` fires [`StartPlayBySeedRequestEvent`] when the "Play by
//!    Seed" card is clicked (or `6` is pressed in the Mode Launcher).
//! 2. `handle_open_dialog` reads the event and spawns the seed-input modal.
//! 3. `handle_text_input` appends decimal digits / handles Backspace while
//!    the modal is open, updating [`SeedInputBuffer`] each frame.
//! 4. `tick_debounce_and_spawn_solver_task` waits for 12 frames (~200 ms at
//!    60 Hz) of no input before spawning a [`try_solve`] task on
//!    [`AsyncComputeTaskPool`]. Any fresh keypress drops the in-flight task
//!    by resetting the resource.
//! 5. `poll_solver_task` polls the in-flight task each frame and updates the
//!    [`SolverVerdictBadge`] text node with the verdict.
//! 6. `handle_confirm` fires [`NewGameRequestEvent`] with the parsed seed and
//!    despawns the dialog on Play click or `Enter`.
//! 7. `handle_cancel` despawns the dialog on Cancel click or `Escape`.

use bevy::input::ButtonInput;
use bevy::prelude::*;
use bevy::tasks::{futures_lite::future, AsyncComputeTaskPool, Task};
use solitaire_core::game_state::DrawMode;
use solitaire_core::solver::{try_solve, SolverConfig, SolverResult};

use crate::events::{NewGameRequestEvent, StartPlayBySeedRequestEvent};
use crate::font_plugin::FontResource;
use crate::game_plugin::GameMutation;
use crate::settings_plugin::SettingsResource;
use crate::ui_modal::{
    spawn_modal, spawn_modal_actions, spawn_modal_body_text, spawn_modal_button, spawn_modal_header,
    ButtonVariant, ScrimDismissible,
};
use crate::ui_theme::{
    ACCENT_PRIMARY, BG_ELEVATED_PRESSED, BORDER_SUBTLE, HighContrastBorder, RADIUS_MD,
    TEXT_DISABLED, TEXT_PRIMARY, TEXT_SECONDARY, TYPE_BODY_LG, VAL_SPACE_2, VAL_SPACE_3,
    Z_MODAL_PANEL,
};

// ---------------------------------------------------------------------------
// Components and resources
// ---------------------------------------------------------------------------

/// Marker on the seed-input modal scrim (the despawn root).
#[derive(Component, Debug)]
pub struct PlayBySeedScreen;

/// Holds the decimal digit string the player is typing and a frame counter
/// used to debounce solver task spawning.
#[derive(Component, Debug, Default)]
struct SeedInputBuffer {
    /// Raw decimal digit string. Never longer than 20 chars (u64::MAX is 20
    /// decimal digits). Empty means "no seed entered".
    text: String,
    /// Frames elapsed since the last keystroke. The solver task is spawned
    /// once this crosses [`DEBOUNCE_FRAMES`] and the buffer is non-empty.
    frames_since_change: u32,
}

/// Marker on the text node that renders the solver verdict caption.
#[derive(Component, Debug)]
struct SolverVerdictBadge;

/// Marker on the Play (confirm) button so `handle_confirm` can find it.
#[derive(Component, Debug)]
struct PlayBySeedConfirmButton;

/// Marker on the Cancel button.
#[derive(Component, Debug)]
struct PlayBySeedCancelButton;

/// Marker on the input-field text node so `handle_text_input` can update
/// it without a separate query for the buffer entity.
#[derive(Component, Debug)]
struct SeedInputDisplay;

/// In-flight async solver verification task. At most one is live at a time —
/// a fresh keypress resets this resource (dropping the previous `Task<_>`)
/// before spawning the next one.
#[derive(Resource, Default)]
struct PendingVerification {
    seed: Option<u64>,
    handle: Option<Task<SolverResult>>,
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Frames of no-keypress activity before the solver task is spawned.
/// 12 frames ≈ 200 ms at 60 Hz — long enough to avoid thrashing on fast
/// typists but short enough to feel responsive.
const DEBOUNCE_FRAMES: u32 = 12;

/// Maximum decimal digits accepted. 20 covers all of u64::MAX (18,446,744,073,709,551,615).
const MAX_SEED_DIGITS: usize = 20;

// ---------------------------------------------------------------------------
// Plugin
// ---------------------------------------------------------------------------

/// Registers all play-by-seed systems and resources.
pub struct PlayBySeedPlugin;

impl Plugin for PlayBySeedPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PendingVerification>()
            .add_message::<StartPlayBySeedRequestEvent>()
            .add_message::<NewGameRequestEvent>()
            .add_systems(
                Update,
                (
                    handle_open_dialog,
                    handle_text_input,
                    tick_debounce_and_spawn_solver_task,
                    poll_solver_task,
                    handle_confirm,
                    handle_cancel,
                )
                    .chain()
                    // Fire before GameMutation so `handle_confirm`'s
                    // NewGameRequestEvent is processed on the same frame.
                    .before(GameMutation),
            );
    }
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// Spawns the seed-input dialog when `StartPlayBySeedRequestEvent` fires.
fn handle_open_dialog(
    mut commands: Commands,
    mut requests: MessageReader<StartPlayBySeedRequestEvent>,
    font_res: Option<Res<FontResource>>,
    existing: Query<(), With<PlayBySeedScreen>>,
) {
    if requests.read().count() == 0 {
        return;
    }
    // Guard against double-spawn (e.g. two events in one frame).
    if !existing.is_empty() {
        return;
    }
    let font = font_res.as_deref();
    let font_handle = font.map(|f| f.0.clone()).unwrap_or_default();

    let scrim = spawn_modal(&mut commands, PlayBySeedScreen, Z_MODAL_PANEL, |card| {
        spawn_modal_header(card, "Play by Seed", font);
        spawn_modal_body_text(
            card,
            "Enter a number to play that specific deal.",
            TEXT_SECONDARY,
            font,
        );

        // Input field — a bordered box that shows the typed digits.
        card.spawn((
            Node {
                width: Val::Percent(100.0),
                padding: UiRect::axes(VAL_SPACE_3, VAL_SPACE_2),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(RADIUS_MD)),
                ..default()
            },
            BackgroundColor(BG_ELEVATED_PRESSED),
            BorderColor::all(BORDER_SUBTLE),
            HighContrastBorder::with_default(BORDER_SUBTLE),
            SeedInputBuffer::default(),
        ))
        .with_children(|field| {
            field.spawn((
                SeedInputDisplay,
                Text::new(""),
                TextFont {
                    font: font_handle.clone(),
                    font_size: TYPE_BODY_LG,
                    ..default()
                },
                TextColor(TEXT_DISABLED),
            ));
        });

        // Solver verdict badge — updates as solver runs.
        card.spawn((
            SolverVerdictBadge,
            Text::new("Type a number"),
            TextFont {
                font: font_handle,
                font_size: TYPE_BODY_LG,
                ..default()
            },
            TextColor(TEXT_SECONDARY),
        ));

        spawn_modal_actions(card, |row| {
            spawn_modal_button(
                row,
                PlayBySeedCancelButton,
                "Cancel",
                Some("Esc"),
                ButtonVariant::Secondary,
                font,
            );
            spawn_modal_button(
                row,
                PlayBySeedConfirmButton,
                "Play",
                Some("Enter"),
                ButtonVariant::Primary,
                font,
            );
        });
    });

    // Play-by-Seed is read-only input — opt into click-outside-to-dismiss.
    commands.entity(scrim).insert(ScrimDismissible);
}

/// Appends decimal digits and handles Backspace while the dialog is open.
fn handle_text_input(
    keys: Res<ButtonInput<KeyCode>>,
    screen: Query<(), With<PlayBySeedScreen>>,
    mut buffers: Query<&mut SeedInputBuffer>,
    mut displays: Query<(&mut Text, &mut TextColor), With<SeedInputDisplay>>,
    mut pending: ResMut<PendingVerification>,
) {
    if screen.is_empty() {
        return;
    }
    let Ok(mut buf) = buffers.single_mut() else {
        return;
    };

    let digit_keys = [
        (KeyCode::Digit0, '0'),
        (KeyCode::Digit1, '1'),
        (KeyCode::Digit2, '2'),
        (KeyCode::Digit3, '3'),
        (KeyCode::Digit4, '4'),
        (KeyCode::Digit5, '5'),
        (KeyCode::Digit6, '6'),
        (KeyCode::Digit7, '7'),
        (KeyCode::Digit8, '8'),
        (KeyCode::Digit9, '9'),
        (KeyCode::Numpad0, '0'),
        (KeyCode::Numpad1, '1'),
        (KeyCode::Numpad2, '2'),
        (KeyCode::Numpad3, '3'),
        (KeyCode::Numpad4, '4'),
        (KeyCode::Numpad5, '5'),
        (KeyCode::Numpad6, '6'),
        (KeyCode::Numpad7, '7'),
        (KeyCode::Numpad8, '8'),
        (KeyCode::Numpad9, '9'),
    ];

    let mut changed = false;

    for (key, ch) in digit_keys {
        if keys.just_pressed(key) && buf.text.len() < MAX_SEED_DIGITS {
            // Drop a leading zero unless the buffer is empty (prevents "007").
            if ch == '0' && buf.text.is_empty() {
                continue;
            }
            buf.text.push(ch);
            changed = true;
        }
    }

    if keys.just_pressed(KeyCode::Backspace) && !buf.text.is_empty() {
        buf.text.pop();
        changed = true;
    }

    if changed {
        buf.frames_since_change = 0;
        // Cancel any in-flight solver task — its seed is now stale.
        *pending = PendingVerification::default();

        // Update the display node.
        if let Ok((mut text, mut color)) = displays.single_mut() {
            if buf.text.is_empty() {
                text.0 = String::new();
                color.0 = TEXT_DISABLED;
            } else {
                text.0 = buf.text.clone();
                color.0 = TEXT_PRIMARY;
            }
        }
    }
}

/// Increments the debounce counter each frame and spawns the solver task
/// once the counter passes [`DEBOUNCE_FRAMES`] and the buffer holds a
/// valid u64.
fn tick_debounce_and_spawn_solver_task(
    screen: Query<(), With<PlayBySeedScreen>>,
    mut buffers: Query<&mut SeedInputBuffer>,
    mut pending: ResMut<PendingVerification>,
    mut badges: Query<(&mut Text, &mut TextColor), With<SolverVerdictBadge>>,
    settings: Option<Res<SettingsResource>>,
) {
    if screen.is_empty() {
        return;
    }
    let Ok(mut buf) = buffers.single_mut() else {
        return;
    };

    // Always update the badge when the buffer is empty.
    if buf.text.is_empty() {
        if let Ok((mut text, mut color)) = badges.single_mut() {
            text.0 = "Type a number".to_string();
            color.0 = TEXT_SECONDARY;
        }
        return;
    }

    // Don't spawn if a task is already running for this seed.
    let parsed = buf.text.parse::<u64>().ok();
    if pending.handle.is_some() && pending.seed == parsed {
        return;
    }

    buf.frames_since_change = buf.frames_since_change.saturating_add(1);
    if buf.frames_since_change < DEBOUNCE_FRAMES {
        return;
    }

    let Some(seed) = parsed else {
        return;
    };

    let draw_mode = settings
        .as_ref()
        .map_or(DrawMode::DrawOne, |s| s.0.draw_mode);
    let cfg = SolverConfig::default();
    let task = AsyncComputeTaskPool::get()
        .spawn(async move { try_solve(seed, draw_mode, &cfg) });

    pending.seed = Some(seed);
    pending.handle = Some(task);

    if let Ok((mut text, mut color)) = badges.single_mut() {
        text.0 = "Verifying\u{2026}".to_string();
        color.0 = TEXT_SECONDARY;
    }
}

/// Polls the in-flight solver task and updates the verdict badge on completion.
fn poll_solver_task(
    mut pending: ResMut<PendingVerification>,
    mut badges: Query<(&mut Text, &mut TextColor), With<SolverVerdictBadge>>,
) {
    let Some(handle) = pending.handle.as_mut() else {
        return;
    };
    let Some(result) = future::block_on(future::poll_once(handle)) else {
        return;
    };
    pending.handle = None;

    let Ok((mut text, mut color)) = badges.single_mut() else {
        return;
    };
    match result {
        SolverResult::Winnable => {
            text.0 = "\u{2713} Provably winnable".to_string();
            color.0 = ACCENT_PRIMARY;
        }
        SolverResult::Inconclusive => {
            text.0 = "? Likely winnable (search timed out)".to_string();
            color.0 = TEXT_SECONDARY;
        }
        SolverResult::Unwinnable => {
            text.0 = "\u{2717} Provably unwinnable".to_string();
            color.0 = TEXT_DISABLED;
        }
    }
}

/// Fires [`NewGameRequestEvent`] with the parsed seed when Play is clicked
/// or `Enter` is pressed, then despawns the dialog. Does nothing when the
/// buffer is empty.
fn handle_confirm(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    buttons: Query<&Interaction, (With<PlayBySeedConfirmButton>, Changed<Interaction>)>,
    buffers: Query<&SeedInputBuffer>,
    screen: Query<Entity, With<PlayBySeedScreen>>,
    mut new_game: MessageWriter<NewGameRequestEvent>,
) {
    if screen.is_empty() {
        return;
    }

    let click = buttons.iter().any(|i| *i == Interaction::Pressed);
    let enter = keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::NumpadEnter);
    if !click && !enter {
        return;
    }

    let Ok(buf) = buffers.single() else { return };
    let Ok(seed) = buf.text.parse::<u64>() else { return };

    new_game.write(NewGameRequestEvent {
        seed: Some(seed),
        mode: None,
        // The player explicitly clicked Play (or pressed Enter) after typing
        // a seed — treat this as an affirmative confirmation so the
        // abandon-current-game dialog is not shown on top of the already-
        // dismissed seed dialog.
        confirmed: true,
    });

    for entity in &screen {
        commands.entity(entity).despawn();
    }
}

/// Despawns the dialog on Cancel click or `Escape`.
fn handle_cancel(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    buttons: Query<&Interaction, (With<PlayBySeedCancelButton>, Changed<Interaction>)>,
    screen: Query<Entity, With<PlayBySeedScreen>>,
    other_scrims: Query<(), (With<crate::ui_modal::ModalScrim>, Without<PlayBySeedScreen>)>,
) {
    if screen.is_empty() {
        return;
    }

    let click = buttons.iter().any(|i| *i == Interaction::Pressed);
    // Esc only closes this dialog when it is the topmost modal.
    let esc = keys.just_pressed(KeyCode::Escape) && other_scrims.is_empty();
    if !click && !esc {
        return;
    }

    for entity in &screen {
        commands.entity(entity).despawn();
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

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
            .add_plugins(PlayBySeedPlugin);
        app.init_resource::<ButtonInput<KeyCode>>();
        app.update();
        app
    }

    fn open_dialog(app: &mut App) {
        app.world_mut()
            .write_message(StartPlayBySeedRequestEvent);
        app.update();
    }

    fn press_key(app: &mut App, key: KeyCode) {
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(key);
        app.update();
        // Simulate what Bevy's PreUpdate input system does: flush just_pressed /
        // just_released so stale key state doesn't bleed into the next frame.
        let mut input = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        input.release(key);
        input.clear();
    }

    fn dialog_present(app: &mut App) -> bool {
        app.world_mut()
            .query::<&PlayBySeedScreen>()
            .iter(app.world())
            .next()
            .is_some()
    }

    fn read_buffer_text(app: &mut App) -> String {
        let mut q = app.world_mut().query::<&SeedInputBuffer>();
        q.iter(app.world())
            .next()
            .map(|b| b.text.clone())
            .unwrap_or_default()
    }

    #[test]
    fn dialog_spawns_on_request() {
        let mut app = headless_app();
        assert!(!dialog_present(&mut app));
        open_dialog(&mut app);
        assert!(dialog_present(&mut app));
    }

    #[test]
    fn digit_keys_append_to_buffer() {
        let mut app = headless_app();
        open_dialog(&mut app);

        press_key(&mut app, KeyCode::Digit4);
        press_key(&mut app, KeyCode::Digit2);

        assert_eq!(read_buffer_text(&mut app), "42");
    }

    #[test]
    fn backspace_removes_last_char() {
        let mut app = headless_app();
        open_dialog(&mut app);

        press_key(&mut app, KeyCode::Digit4);
        press_key(&mut app, KeyCode::Digit2);
        press_key(&mut app, KeyCode::Backspace);

        assert_eq!(read_buffer_text(&mut app), "4");
    }

    #[test]
    fn confirm_does_nothing_when_buffer_is_empty() {
        let mut app = headless_app();
        open_dialog(&mut app);

        // Simulate Enter with empty buffer.
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Enter);
        app.update();

        let msgs = app.world().resource::<Messages<NewGameRequestEvent>>();
        let mut cursor = msgs.get_cursor();
        assert!(cursor.read(msgs).next().is_none(), "no NewGameRequestEvent when buffer empty");
        // Dialog should still be open.
        assert!(dialog_present(&mut app));
    }

    #[test]
    fn confirm_writes_new_game_request_with_parsed_seed() {
        let mut app = headless_app();
        open_dialog(&mut app);

        press_key(&mut app, KeyCode::Digit4);
        press_key(&mut app, KeyCode::Digit2);

        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Enter);
        app.update();

        let msgs = app.world().resource::<Messages<NewGameRequestEvent>>();
        let mut cursor = msgs.get_cursor();
        let fired: Vec<_> = cursor.read(msgs).copied().collect();
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].seed, Some(42));
        assert_eq!(fired[0].mode, None);
        // confirmed: true — the player explicitly clicked Play, so no
        // abandon-current-game dialog should appear.
        assert!(fired[0].confirmed);

        // Dialog should be gone.
        assert!(!dialog_present(&mut app));
    }

    #[test]
    fn cancel_despawns_dialog_without_new_game_request() {
        let mut app = headless_app();
        open_dialog(&mut app);

        press_key(&mut app, KeyCode::Escape);

        assert!(!dialog_present(&mut app));

        let msgs = app.world().resource::<Messages<NewGameRequestEvent>>();
        let mut cursor = msgs.get_cursor();
        assert!(cursor.read(msgs).next().is_none());
    }

    #[test]
    fn solver_task_spawns_after_debounce_window() {
        let mut app = headless_app();
        open_dialog(&mut app);

        press_key(&mut app, KeyCode::Digit4);
        press_key(&mut app, KeyCode::Digit2);

        // Debounce window — no task yet.
        for _ in 0..DEBOUNCE_FRAMES {
            app.update();
        }

        let pending = app.world().resource::<PendingVerification>();
        assert!(pending.handle.is_some(), "solver task should have been spawned after debounce");
        assert_eq!(pending.seed, Some(42));
    }

    #[test]
    fn keypress_mid_flight_cancels_previous_solver_task() {
        let mut app = headless_app();
        open_dialog(&mut app);

        press_key(&mut app, KeyCode::Digit4);
        press_key(&mut app, KeyCode::Digit2);

        // Let the debounce fire.
        for _ in 0..DEBOUNCE_FRAMES {
            app.update();
        }
        assert!(app.world().resource::<PendingVerification>().handle.is_some());

        // New keypress should cancel the in-flight task.
        press_key(&mut app, KeyCode::Digit3);
        assert!(app.world().resource::<PendingVerification>().handle.is_none());
        assert_eq!(app.world().resource::<PendingVerification>().seed, None);
    }

    #[test]
    fn solver_task_completes_and_updates_badge() {
        use std::time::Instant;

        let mut app = headless_app();
        open_dialog(&mut app);

        // Seed 42 — solver will return some verdict.
        press_key(&mut app, KeyCode::Digit4);
        press_key(&mut app, KeyCode::Digit2);

        // Wait for the debounce to spawn the task.
        for _ in 0..DEBOUNCE_FRAMES {
            app.update();
        }

        // Poll until the solver task resolves (cap at 15 s wall-clock).
        let deadline = Instant::now() + std::time::Duration::from_secs(15);
        while app.world().resource::<PendingVerification>().handle.is_some()
            && Instant::now() < deadline
        {
            app.update();
            std::thread::yield_now();
        }

        // Badge text should no longer read "Verifying…".
        let badge_text = app
            .world_mut()
            .query::<(&Text, &SolverVerdictBadge)>()
            .iter(app.world())
            .next()
            .map(|(t, _)| t.0.clone())
            .unwrap_or_default();
        assert_ne!(badge_text, "Verifying\u{2026}", "badge should have resolved to a verdict");
        assert_ne!(badge_text, "Type a number", "badge should show verdict, not idle state");
    }
}
