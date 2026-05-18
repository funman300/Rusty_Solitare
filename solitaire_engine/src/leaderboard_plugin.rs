//! In-game leaderboard panel.
//!
//! Press `L` to open the panel. On first open, an async fetch is kicked off
//! against the active [`SyncProvider`]. Fetched results are cached in
//! [`LeaderboardResource`] and re-displayed without another network trip until
//! the user explicitly presses `L` again while the panel is already open
//! (which closes it) and then `L` once more (which re-fetches).
//!
//! When the provider does not support leaderboards (e.g. `LocalOnlyProvider`)
//! the panel shows "Not available" immediately.

use bevy::input::{ButtonState, keyboard::KeyboardInput, mouse::{MouseScrollUnit, MouseWheel}};
use bevy::prelude::*;
use bevy::tasks::{futures_lite::future, AsyncComputeTaskPool, Task};
use solitaire_data::{save_settings_to, settings::SyncBackend};
use solitaire_sync::LeaderboardEntry;

use crate::events::{InfoToastEvent, ToggleLeaderboardRequestEvent};
use crate::font_plugin::FontResource;
use crate::settings_plugin::{SettingsResource, SettingsStoragePath};
use crate::sync_plugin::SyncProviderResource;
use crate::ui_modal::{
    spawn_modal, spawn_modal_actions, spawn_modal_button, spawn_modal_header, ButtonVariant,
    ScrimDismissible,
};
use crate::ui_theme::{
    ACCENT_PRIMARY, BG_ELEVATED, BORDER_SUBTLE, RADIUS_SM, STATE_INFO,
    TEXT_DISABLED, TEXT_PRIMARY, TEXT_SECONDARY, TYPE_BODY, TYPE_BODY_LG, TYPE_CAPTION,
    VAL_SPACE_2, VAL_SPACE_3, VAL_SPACE_4, Z_MODAL_PANEL, Z_PAUSE_DIALOG,
};

// ---------------------------------------------------------------------------
// Resources
// ---------------------------------------------------------------------------

/// State of the cached leaderboard fetch.
///
/// Distinguishes "fetch hasn't completed yet" from "fetch failed" from
/// "fetch succeeded but the leaderboard is empty" so the UI can show
/// targeted copy for each case rather than a single ambiguous "no
/// entries" line that hid network errors from the player.
#[derive(Resource, Default, Debug, Clone)]
pub enum LeaderboardResource {
    /// No fetch has completed yet — show "Fetching..." in the panel.
    #[default]
    Idle,
    /// Last fetch failed (network, auth, etc.) — show error copy.
    /// The wrapped string is the underlying error for logging only;
    /// the UI shows a fixed user-friendly message.
    Error(String),
    /// Fetch succeeded — wrapped Vec may be empty (legitimately empty
    /// leaderboard) or populated.
    Loaded(Vec<LeaderboardEntry>),
}

/// Set to `true` in the frame the user explicitly closes the panel so that a
/// fetch completing in the same frame doesn't immediately reopen it.
#[derive(Resource, Default)]
struct ClosedThisFrame(bool);

/// In-flight fetch task result carrier — transfers data from the task thread.
#[derive(Resource, Default)]
struct LeaderboardFetchResult(Option<Result<Vec<LeaderboardEntry>, String>>);

#[derive(Resource, Default)]
struct LeaderboardFetchTask(Option<Task<Result<Vec<LeaderboardEntry>, String>>>);

/// Marker on the leaderboard overlay root node.
#[derive(Component, Debug)]
pub struct LeaderboardScreen;

/// Marker on the scrollable body Node inside the Leaderboard modal.
///
/// The leaderboard caps at the top 10 entries today, but rendering the
/// caption + opt-in/opt-out row + 10 data rows on the 800x600 minimum
/// window is right at the edge of overflowing — long display names or
/// future row-count expansion would cut off entries below the fold.
/// Wrapping the data section in an `Overflow::scroll_y()` Node with a
/// constrained `max_height` keeps every row reachable. Mirrors the
/// `SettingsPanelScrollable` pattern.
#[derive(Component, Debug)]
pub struct LeaderboardScrollable;

/// Marker on the "Opt In" button inside the leaderboard panel.
#[derive(Component, Debug)]
struct LeaderboardOptInButton;

/// Marker on the "Opt Out" button inside the leaderboard panel.
#[derive(Component, Debug)]
struct LeaderboardOptOutButton;

/// In-flight opt-in task.
#[derive(Resource, Default)]
struct OptInTask(Option<Task<Result<(), String>>>);

/// In-flight opt-out task.
#[derive(Resource, Default)]
struct OptOutTask(Option<Task<Result<(), String>>>);

/// Marker on the "Set Name" button inside the leaderboard panel.
#[derive(Component, Debug)]
struct SetDisplayNameButton;

/// Marker on the display-name editor modal root.
#[derive(Component, Debug)]
struct DisplayNameModal;

/// Text currently typed in the display-name modal's input field.
#[derive(Resource, Default)]
struct DisplayNameBuffer(String);

/// Marker on the text node inside the display-name input field.
#[derive(Component, Debug)]
struct DisplayNameTextField;

/// Marker on the "Save" button in the display-name modal.
#[derive(Component, Debug)]
struct DisplayNameConfirmButton;

/// Marker on the "Cancel" button in the display-name modal.
#[derive(Component, Debug)]
struct DisplayNameCancelButton;

// ---------------------------------------------------------------------------
// Plugin
// ---------------------------------------------------------------------------

/// Manages the leaderboard overlay: fetches scores from the sync server, handles opt-in/opt-out, and displays the ranked list of player scores.
pub struct LeaderboardPlugin;

impl Plugin for LeaderboardPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LeaderboardResource>()
            .init_resource::<LeaderboardFetchResult>()
            .init_resource::<LeaderboardFetchTask>()
            .init_resource::<ClosedThisFrame>()
            .init_resource::<OptInTask>()
            .init_resource::<OptOutTask>()
            .init_resource::<DisplayNameBuffer>()
            .add_message::<ToggleLeaderboardRequestEvent>()
            // `MouseWheel` and `KeyboardInput` are emitted by Bevy's input
            // plugin under `DefaultPlugins`; register them explicitly so all
            // leaderboard systems run cleanly under `MinimalPlugins` in tests.
            .add_message::<MouseWheel>()
            .add_message::<KeyboardInput>()
            .add_systems(
                Update,
                (
                    reset_closed_flag,
                    toggle_leaderboard_screen,
                    handle_leaderboard_close_button,
                    poll_leaderboard_fetch,
                    update_leaderboard_panel,
                    handle_opt_in_button,
                    poll_opt_in_task,
                    handle_opt_out_button,
                    poll_opt_out_task,
                    handle_set_display_name_button,
                    handle_display_name_text_input,
                    handle_display_name_confirm,
                    handle_display_name_cancel,
                )
                    .chain(),
            )
            .add_systems(Update, scroll_leaderboard_panel);
    }
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// Clear the "closed this frame" flag at the start of each frame.
fn reset_closed_flag(mut flag: ResMut<ClosedThisFrame>) {
    flag.0 = false;
}

/// `L` keyboard accelerator or `ToggleLeaderboardRequestEvent` from the
/// HUD Menu popover — open or close the leaderboard panel. On open,
/// starts a new fetch if no data is cached or a fetch is not in flight.
#[allow(clippy::too_many_arguments)]
fn toggle_leaderboard_screen(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    mut requests: MessageReader<ToggleLeaderboardRequestEvent>,
    screens: Query<Entity, With<LeaderboardScreen>>,
    data: Res<LeaderboardResource>,
    provider: Option<Res<SyncProviderResource>>,
    settings: Option<Res<SettingsResource>>,
    font_res: Option<Res<FontResource>>,
    mut task_res: ResMut<LeaderboardFetchTask>,
    mut closed_flag: ResMut<ClosedThisFrame>,
) {
    let button_clicked = requests.read().count() > 0;
    if !keys.just_pressed(KeyCode::KeyL) && !button_clicked {
        return;
    }
    if let Ok(entity) = screens.single() {
        commands.entity(entity).despawn();
        closed_flag.0 = true;
        return;
    }

    // Spawn the panel immediately with whatever data we have so far.
    let remote_available = provider
        .as_ref()
        .is_some_and(|p| p.0.backend_name() != "local");
    let dn = settings.as_ref().and_then(|s| s.0.leaderboard_display_name.as_deref());
    spawn_leaderboard_screen(&mut commands, &data, remote_available, dn, font_res.as_deref());

    // Start a background fetch if not already in flight.
    if task_res.0.is_none()
        && let Some(p) = provider {
            let provider = p.0.clone();
            let task = AsyncComputeTaskPool::get().spawn(async move {
                provider.fetch_leaderboard().await.map_err(|e| e.to_string())
            });
            task_res.0 = Some(task);
        }
}

/// Poll the background fetch task; store results when complete.
fn poll_leaderboard_fetch(
    mut task_res: ResMut<LeaderboardFetchTask>,
    mut result_res: ResMut<LeaderboardFetchResult>,
) {
    let Some(task) = task_res.0.as_mut() else { return };
    let Some(result) = future::block_on(future::poll_once(task)) else { return };
    task_res.0 = None;
    result_res.0 = Some(result);
}

/// When a fetch completes, cache the data and update any open panel.
/// Skips the panel rebuild if the user closed the panel in this same frame
/// (commands are deferred, so the query would still see the despawned entity).
#[allow(clippy::too_many_arguments)]
fn update_leaderboard_panel(
    mut commands: Commands,
    mut result_res: ResMut<LeaderboardFetchResult>,
    mut data: ResMut<LeaderboardResource>,
    screens: Query<Entity, With<LeaderboardScreen>>,
    provider: Option<Res<SyncProviderResource>>,
    settings: Option<Res<SettingsResource>>,
    font_res: Option<Res<FontResource>>,
    closed_flag: Res<ClosedThisFrame>,
) {
    let Some(result) = result_res.0.take() else { return };

    match result {
        Ok(entries) => {
            *data = LeaderboardResource::Loaded(entries);
        }
        Err(e) => {
            warn!("leaderboard fetch failed: {e}");
            // Preserve previously-loaded data on a transient failure so a
            // momentary network blip doesn't wipe a populated list. Only
            // surface an Error state when we have nothing better to show.
            if !matches!(*data, LeaderboardResource::Loaded(_)) {
                *data = LeaderboardResource::Error(e);
            }
        }
    }

    // Rebuild the panel if it's open — but not if the user just closed it in
    // this frame (their despawn command is still deferred).
    if closed_flag.0 {
        return;
    }
    let remote_available = provider
        .as_ref()
        .is_some_and(|p| p.0.backend_name() != "local");
    let dn = settings.as_ref().and_then(|s| s.0.leaderboard_display_name.as_deref());
    for entity in &screens {
        commands.entity(entity).despawn();
        spawn_leaderboard_screen(&mut commands, &data, remote_available, dn, font_res.as_deref());
    }
}

/// Click handler for the modal's "Done" button — despawns the overlay.
/// Routes mouse-wheel events into the Leaderboard modal's scrollable
/// data body while the panel is open. No-op when no
/// `LeaderboardScrollable` exists in the world (modal closed). Mirrors
/// `scroll_settings_panel`.
fn scroll_leaderboard_panel(
    mut scroll_evr: MessageReader<MouseWheel>,
    mut scrollables: Query<&mut ScrollPosition, With<LeaderboardScrollable>>,
) {
    if scrollables.is_empty() {
        scroll_evr.clear();
        return;
    }
    let delta_y: f32 = scroll_evr
        .read()
        .map(|ev| match ev.unit {
            MouseScrollUnit::Line => ev.y * 50.0,
            MouseScrollUnit::Pixel => ev.y,
        })
        .sum();
    if delta_y == 0.0 {
        return;
    }
    for mut sp in scrollables.iter_mut() {
        sp.0.y = (sp.0.y - delta_y).max(0.0);
    }
}

fn handle_leaderboard_close_button(
    mut commands: Commands,
    close_buttons: Query<&Interaction, (With<LeaderboardCloseButton>, Changed<Interaction>)>,
    screens: Query<Entity, With<LeaderboardScreen>>,
    mut closed_flag: ResMut<ClosedThisFrame>,
) {
    if !close_buttons.iter().any(|i| *i == Interaction::Pressed) {
        return;
    }
    for entity in &screens {
        commands.entity(entity).despawn();
        closed_flag.0 = true;
    }
}

/// Fires an async opt-in request when the player presses the "Opt In" button.
///
/// The display name is taken from the configured server username in
/// `SettingsResource`. If no server backend is active, the button is a no-op.
fn handle_opt_in_button(
    interaction_query: Query<&Interaction, (Changed<Interaction>, With<LeaderboardOptInButton>)>,
    settings: Option<Res<SettingsResource>>,
    provider: Option<Res<SyncProviderResource>>,
    mut task_res: ResMut<OptInTask>,
) {
    if task_res.0.is_some() {
        return; // already in flight
    }
    let Some(provider) = provider else { return };
    for interaction in &interaction_query {
        if *interaction != Interaction::Pressed {
            continue;
        }
        let display_name = settings
            .as_ref()
            .and_then(|s| {
                // Prefer an explicit display name; fall back to server username.
                s.0.leaderboard_display_name
                    .as_deref()
                    .or_else(|| {
                        if let SyncBackend::SolitaireServer { username, .. } = &s.0.sync_backend {
                            Some(username.as_str())
                        } else {
                            None
                        }
                    })
                    .map(|n| n.chars().take(32).collect::<String>())
            })
            .unwrap_or_else(|| "Player".to_string());

        let provider = provider.0.clone();
        let task = AsyncComputeTaskPool::get()
            .spawn(async move { provider.opt_in_leaderboard(&display_name).await.map_err(|e| e.to_string()) });
        task_res.0 = Some(task);
    }
}

/// Polls the opt-in task; fires an `InfoToastEvent` on completion or failure.
fn poll_opt_in_task(
    mut task_res: ResMut<OptInTask>,
    mut toast: MessageWriter<InfoToastEvent>,
) {
    let Some(task) = task_res.0.as_mut() else { return };
    let Some(result) = future::block_on(future::poll_once(task)) else { return };
    task_res.0 = None;
    match result {
        Ok(()) => {
            toast.write(InfoToastEvent("Opted in to leaderboard".to_string()));
        }
        Err(e) => {
            warn!("leaderboard opt-in failed: {e}");
            toast.write(InfoToastEvent("Failed to join leaderboard".to_string()));
        }
    }
}

/// Fires an async opt-out request when the player presses the "Opt Out" button.
fn handle_opt_out_button(
    interaction_query: Query<&Interaction, (Changed<Interaction>, With<LeaderboardOptOutButton>)>,
    provider: Option<Res<SyncProviderResource>>,
    mut task_res: ResMut<OptOutTask>,
) {
    if task_res.0.is_some() {
        return;
    }
    let Some(provider) = provider else { return };
    for interaction in &interaction_query {
        if *interaction != Interaction::Pressed {
            continue;
        }
        let provider = provider.0.clone();
        let task = AsyncComputeTaskPool::get()
            .spawn(async move { provider.opt_out_leaderboard().await.map_err(|e| e.to_string()) });
        task_res.0 = Some(task);
    }
}

/// Polls the opt-out task; fires an `InfoToastEvent` on completion or failure.
fn poll_opt_out_task(
    mut task_res: ResMut<OptOutTask>,
    mut toast: MessageWriter<InfoToastEvent>,
) {
    let Some(task) = task_res.0.as_mut() else { return };
    let Some(result) = future::block_on(future::poll_once(task)) else { return };
    task_res.0 = None;
    match result {
        Ok(()) => {
            toast.write(InfoToastEvent("Opted out of leaderboard".to_string()));
        }
        Err(e) => {
            warn!("leaderboard opt-out failed: {e}");
            toast.write(InfoToastEvent("Failed to leave leaderboard".to_string()));
        }
    }
}

// ---------------------------------------------------------------------------
// UI construction
// ---------------------------------------------------------------------------

/// Marker on the "Done" button inside the Leaderboard modal.
#[derive(Component, Debug)]
pub struct LeaderboardCloseButton;

fn spawn_leaderboard_screen(
    commands: &mut Commands,
    data: &LeaderboardResource,
    remote_available: bool,
    effective_display_name: Option<&str>,
    font_res: Option<&FontResource>,
) {
    let scrim = spawn_modal(commands, LeaderboardScreen, Z_MODAL_PANEL, |card| {
        spawn_modal_header(card, "Leaderboard", font_res);

        // Subhead — what the screen does + what the buttons control.
        let font_handle = font_res.map(|f| f.0.clone()).unwrap_or_default();
        let font_caption = TextFont {
            font: font_handle.clone(),
            font_size: TYPE_CAPTION,
            ..default()
        };
        let font_status = TextFont {
            font: font_handle.clone(),
            font_size: TYPE_BODY_LG,
            ..default()
        };
        let font_row = TextFont {
            font: font_handle.clone(),
            font_size: TYPE_BODY,
            ..default()
        };
        let font_header = TextFont {
            font: font_handle,
            font_size: TYPE_CAPTION,
            ..default()
        };

        if remote_available {
            card.spawn((
                Text::new("Use Opt In / Opt Out to control your visibility on the server."),
                font_caption.clone(),
                TextColor(TEXT_SECONDARY),
            ));

            // Public name row: shows the effective display name + "Set Name" button.
            card.spawn(Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: VAL_SPACE_3,
                ..default()
            })
            .with_children(|row| {
                let label = match effective_display_name {
                    Some(n) => format!("Public name: {n}"),
                    None => "Public name: (same as username)".to_string(),
                };
                row.spawn((
                    Text::new(label),
                    font_caption.clone(),
                    TextColor(TEXT_SECONDARY),
                ));
                spawn_modal_button(
                    row,
                    SetDisplayNameButton,
                    "Set Name",
                    None,
                    ButtonVariant::Tertiary,
                    font_res,
                );
            });

            // Opt In / Opt Out row uses the same modal-button helpers as
            // the rest of the UI for consistent hover / press feedback.
            spawn_modal_actions(card, |row| {
                spawn_modal_button(
                    row,
                    LeaderboardOptInButton,
                    "Opt In",
                    None,
                    ButtonVariant::Secondary,
                    font_res,
                );
                spawn_modal_button(
                    row,
                    LeaderboardOptOutButton,
                    "Opt Out",
                    None,
                    ButtonVariant::Tertiary,
                    font_res,
                );
            });
        } else {
            // No remote sync provider configured — opt-in/out would be a
            // silent no-op, so show a single explanatory line instead.
            card.spawn((
                Text::new(
                    "Leaderboards require cloud sync. Configure a server in Settings to participate.",
                ),
                font_caption.clone(),
                TextColor(TEXT_SECONDARY),
            ));
        }

        // Subtle separator between the controls and the data area.
        card.spawn((
            Node {
                height: Val::Px(1.0),
                ..default()
            },
            BackgroundColor(BORDER_SUBTLE),
        ));

        // Scrollable data section — caps at top 10 rows today, but on the
        // 800x600 minimum window the header + caption + opt-in row + 10
        // entries crowds the modal. Wrapping in `Overflow::scroll_y()`
        // with a `max_height` keeps every entry reachable and survives
        // any future expansion of the row cap.
        card.spawn((
            LeaderboardScrollable,
            ScrollPosition::default(),
            Node {
                flex_direction: FlexDirection::Column,
                row_gap: VAL_SPACE_2,
                max_height: Val::Vh(50.0),
                overflow: Overflow::scroll_y(),
                ..default()
            },
        ))
        .with_children(|body| {
            match data {
                LeaderboardResource::Idle => {
                    body.spawn((
                        Text::new("Fetching\u{2026}"),
                        font_status.clone(),
                        TextColor(STATE_INFO),
                    ));
                }
                LeaderboardResource::Error(_) => {
                    body.spawn((
                        Text::new("Couldn't reach the leaderboard. Try again later."),
                        font_status.clone(),
                        TextColor(TEXT_SECONDARY),
                    ));
                }
                LeaderboardResource::Loaded(rows) if rows.is_empty() => {
                    body.spawn((
                        Text::new("Be the first on the leaderboard."),
                        font_status.clone(),
                        TextColor(TEXT_PRIMARY),
                    ));
                    body.spawn((
                        Text::new("Win a game and opt in to appear here."),
                        font_row.clone(),
                        TextColor(TEXT_SECONDARY),
                    ));
                }
                LeaderboardResource::Loaded(rows) => {
                    // Column headers
                    body.spawn(Node {
                        flex_direction: FlexDirection::Row,
                        column_gap: VAL_SPACE_4,
                        ..default()
                    })
                    .with_children(|row| {
                        header_cell(row, "#", 30.0, &font_header);
                        header_cell(row, "Player", 160.0, &font_header);
                        header_cell(row, "Best Score", 100.0, &font_header);
                        header_cell(row, "Fastest Win", 110.0, &font_header);
                    });

                    let mut sorted = rows.to_vec();
                    sorted.sort_by_key(|e| std::cmp::Reverse(e.best_score.unwrap_or(0)));

                    for (i, entry) in sorted.iter().take(10).enumerate() {
                        // Top three get accent treatments to highlight the
                        // podium without leaning on hand-picked metallic
                        // colours that sit outside the token system.
                        let rank_color = match i {
                            0 => ACCENT_PRIMARY, // Balatro yellow for #1
                            1 | 2 => TEXT_PRIMARY,
                            _ => TEXT_SECONDARY,
                        };

                        let time_str = entry
                            .best_time_secs
                            .map_or_else(|| "-".to_string(), format_secs);
                        let score_str = entry
                            .best_score
                            .map_or_else(|| "-".to_string(), |s| s.to_string());

                        body.spawn(Node {
                            flex_direction: FlexDirection::Row,
                            column_gap: VAL_SPACE_4,
                            ..default()
                        })
                        .with_children(|row| {
                            data_cell(row, &format!("{}", i + 1), 30.0, rank_color, &font_row);
                            data_cell(row, &entry.display_name, 160.0, TEXT_PRIMARY, &font_row);
                            data_cell(row, &score_str, 100.0, TEXT_PRIMARY, &font_row);
                            data_cell(row, &time_str, 110.0, TEXT_PRIMARY, &font_row);
                        });
                    }
                }
            }
        });

        spawn_modal_actions(card, |actions| {
            spawn_modal_button(
                actions,
                LeaderboardCloseButton,
                "Done",
                Some("L"),
                ButtonVariant::Primary,
                font_res,
            );
        });
    });
    // Leaderboard is read-only — opt into click-outside-to-dismiss.
    commands.entity(scrim).insert(ScrimDismissible);
}

fn header_cell(parent: &mut ChildSpawnerCommands, text: &str, width: f32, font: &TextFont) {
    parent.spawn((
        Text::new(text.to_string()),
        font.clone(),
        TextColor(TEXT_SECONDARY),
        Node {
            width: Val::Px(width),
            ..default()
        },
    ));
}

fn data_cell(
    parent: &mut ChildSpawnerCommands,
    text: &str,
    width: f32,
    color: Color,
    font: &TextFont,
) {
    parent.spawn((
        Text::new(text.to_string()),
        font.clone(),
        TextColor(color),
        Node {
            width: Val::Px(width),
            ..default()
        },
    ));
}

// ---------------------------------------------------------------------------
// Display-name editor
// ---------------------------------------------------------------------------

/// Opens the display-name editor modal when the "Set Name" button is pressed.
fn handle_set_display_name_button(
    button_q: Query<&Interaction, (Changed<Interaction>, With<SetDisplayNameButton>)>,
    existing: Query<(), With<DisplayNameModal>>,
    mut commands: Commands,
    settings: Option<Res<SettingsResource>>,
    font_res: Option<Res<FontResource>>,
    mut buf: ResMut<DisplayNameBuffer>,
) {
    if !button_q.iter().any(|i| *i == Interaction::Pressed) {
        return;
    }
    if !existing.is_empty() {
        return; // already open
    }
    buf.0 = settings
        .as_ref()
        .and_then(|s| s.0.leaderboard_display_name.clone())
        .unwrap_or_default();
    spawn_display_name_modal(&mut commands, &buf.0, font_res.as_deref());
}

/// Routes keyboard input into the display-name buffer while the editor is open.
fn handle_display_name_text_input(
    screen: Query<(), With<DisplayNameModal>>,
    mut key_events: MessageReader<KeyboardInput>,
    mut buf: ResMut<DisplayNameBuffer>,
    mut text_q: Query<&mut Text, With<DisplayNameTextField>>,
) {
    if screen.is_empty() {
        key_events.clear();
        return;
    }
    for ev in key_events.read() {
        if ev.state != ButtonState::Pressed {
            continue;
        }
        if ev.key_code == KeyCode::Backspace {
            buf.0.pop();
        } else if let Some(ch) = ev.text.as_deref().and_then(printable_char_dn)
            && buf.0.len() < 32
        {
            buf.0.push(ch);
        }
    }
    for mut text in &mut text_q {
        text.0 = if buf.0.is_empty() {
            " ".to_string()
        } else {
            buf.0.clone()
        };
    }
}

/// Saves the typed display name to `SettingsResource` and closes the modal.
fn handle_display_name_confirm(
    button_q: Query<&Interaction, (Changed<Interaction>, With<DisplayNameConfirmButton>)>,
    screens: Query<Entity, With<DisplayNameModal>>,
    mut commands: Commands,
    buf: Res<DisplayNameBuffer>,
    settings: Option<ResMut<SettingsResource>>,
    settings_path: Option<Res<SettingsStoragePath>>,
) {
    if !button_q.iter().any(|i| *i == Interaction::Pressed) {
        return;
    }
    if let Some(mut settings) = settings {
        let trimmed: String = buf.0.trim().chars().take(32).collect();
        settings.0.leaderboard_display_name = if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        };
        if let Some(path) = settings_path.as_ref().and_then(|p| p.0.as_ref())
            && let Err(e) = save_settings_to(path, &settings.0)
        {
            warn!("failed to save settings: {e}");
        }
    }
    for entity in &screens {
        commands.entity(entity).despawn();
    }
}

/// Discards any typed text and closes the display-name editor modal.
fn handle_display_name_cancel(
    button_q: Query<&Interaction, (Changed<Interaction>, With<DisplayNameCancelButton>)>,
    screens: Query<Entity, With<DisplayNameModal>>,
    mut commands: Commands,
) {
    if !button_q.iter().any(|i| *i == Interaction::Pressed) {
        return;
    }
    for entity in &screens {
        commands.entity(entity).despawn();
    }
}

fn spawn_display_name_modal(
    commands: &mut Commands,
    current_name: &str,
    font_res: Option<&FontResource>,
) {
    let make_font = |size: f32| TextFont {
        font: font_res.map(|f| f.0.clone()).unwrap_or_default(),
        font_size: size,
        ..default()
    };

    spawn_modal(commands, DisplayNameModal, Z_PAUSE_DIALOG, |card| {
        spawn_modal_header(card, "Public Display Name", font_res);

        card.spawn((
            Text::new(
                "Shown on the leaderboard when you opt in. Leave blank to use your username.",
            ),
            make_font(TYPE_CAPTION),
            TextColor(TEXT_SECONDARY),
        ));

        // Input field container.
        card.spawn((
            Node {
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(RADIUS_SM)),
                padding: UiRect::axes(VAL_SPACE_3, Val::Px(6.0)),
                min_height: Val::Px(32.0),
                min_width: Val::Px(260.0),
                ..default()
            },
            BackgroundColor(BG_ELEVATED),
            BorderColor::all(ACCENT_PRIMARY),
        ))
        .with_children(|border| {
            let initial = if current_name.is_empty() {
                " ".to_string()
            } else {
                current_name.to_string()
            };
            border.spawn((
                DisplayNameTextField,
                Text::new(initial),
                make_font(TYPE_BODY),
                TextColor(if current_name.is_empty() {
                    TEXT_DISABLED
                } else {
                    TEXT_PRIMARY
                }),
            ));
        });

        card.spawn((
            Text::new("Max 32 characters."),
            make_font(TYPE_CAPTION),
            TextColor(TEXT_SECONDARY),
        ));

        spawn_modal_actions(card, |actions| {
            spawn_modal_button(
                actions,
                DisplayNameCancelButton,
                "Cancel",
                None,
                ButtonVariant::Tertiary,
                font_res,
            );
            spawn_modal_button(
                actions,
                DisplayNameConfirmButton,
                "Save",
                None,
                ButtonVariant::Primary,
                font_res,
            );
        });
    });
}

/// Accepts printable ASCII characters (0x20–0x7e) for the display-name field.
fn printable_char_dn(text: &str) -> Option<char> {
    let ch = text.chars().next()?;
    (' '..='~').contains(&ch).then_some(ch)
}

fn format_secs(secs: u64) -> String {
    let m = secs / 60;
    let s = secs % 60;
    if m > 0 {
        format!("{m}:{s:02}")
    } else {
        format!("{s}s")
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
    use crate::sync_plugin::SyncPlugin;
    use solitaire_data::SyncError;
    use solitaire_sync::{SyncPayload, SyncResponse};
    use chrono::Utc;
    use uuid::Uuid;
    use solitaire_sync::PlayerProgress;
    use solitaire_data::StatsSnapshot;

    struct NoOpProvider;

    #[async_trait::async_trait]
    impl solitaire_data::SyncProvider for NoOpProvider {
        async fn pull(&self) -> Result<SyncPayload, SyncError> {
            Ok(SyncPayload {
                user_id: Uuid::nil(),
                stats: StatsSnapshot::default(),
                achievements: vec![],
                progress: PlayerProgress::default(),
                last_modified: Utc::now(),
            })
        }
        async fn push(&self, _p: &SyncPayload) -> Result<SyncResponse, SyncError> {
            Ok(SyncResponse {
                merged: SyncPayload {
                    user_id: Uuid::nil(),
                    stats: StatsSnapshot::default(),
                    achievements: vec![],
                    progress: PlayerProgress::default(),
                    last_modified: Utc::now(),
                },
                server_time: Utc::now(),
                conflicts: vec![],
            })
        }
        fn backend_name(&self) -> &'static str { "no-op" }
        fn is_authenticated(&self) -> bool { false }

        async fn fetch_leaderboard(&self) -> Result<Vec<LeaderboardEntry>, SyncError> {
            Ok(vec![
                LeaderboardEntry {
                    display_name: "Alice".to_string(),
                    best_score: Some(5000),
                    best_time_secs: Some(180),
                    recorded_at: Utc::now(),
                },
            ])
        }
    }

    fn headless_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(GamePlugin)
            .add_plugins(TablePlugin)
            .add_plugins(crate::stats_plugin::StatsPlugin::headless())
            .add_plugins(crate::progress_plugin::ProgressPlugin::headless())
            .add_plugins(crate::achievement_plugin::AchievementPlugin::headless())
            .add_plugins(SyncPlugin::new(NoOpProvider))
            .add_plugins(LeaderboardPlugin);
        app.init_resource::<bevy::input::ButtonInput<KeyCode>>();
        app.update();
        app
    }

    fn press(app: &mut App, key: KeyCode) {
        let mut input = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        input.release(key);
        input.clear();
        input.press(key);
    }

    #[test]
    fn resource_starts_empty() {
        let app = headless_app();
        assert!(matches!(
            app.world().resource::<LeaderboardResource>(),
            LeaderboardResource::Idle
        ));
    }

    #[test]
    fn pressing_l_spawns_screen() {
        let mut app = headless_app();
        press(&mut app, KeyCode::KeyL);
        app.update();
        let count = app
            .world_mut()
            .query::<&LeaderboardScreen>()
            .iter(app.world())
            .count();
        assert_eq!(count, 1);
    }

    #[test]
    fn leaderboard_modal_body_is_scrollable() {
        let mut app = headless_app();
        press(&mut app, KeyCode::KeyL);
        app.update();

        let count = app
            .world_mut()
            .query::<&LeaderboardScrollable>()
            .iter(app.world())
            .count();
        assert_eq!(
            count, 1,
            "Leaderboard modal must spawn exactly one LeaderboardScrollable body"
        );

        let mut q = app
            .world_mut()
            .query_filtered::<&Node, With<LeaderboardScrollable>>();
        let nodes: Vec<&Node> = q.iter(app.world()).collect();
        assert_ne!(
            nodes[0].max_height,
            Val::Auto,
            "scrollable body must set a non-default max_height"
        );
        assert_eq!(nodes[0].overflow, Overflow::scroll_y());
    }

    #[test]
    fn pressing_l_twice_dismisses_screen() {
        let mut app = headless_app();
        press(&mut app, KeyCode::KeyL);
        app.update();
        press(&mut app, KeyCode::KeyL);
        app.update();
        let count = app
            .world_mut()
            .query::<&LeaderboardScreen>()
            .iter(app.world())
            .count();
        assert_eq!(count, 0);
    }

    #[test]
    fn format_secs_below_minute() {
        assert_eq!(format_secs(45), "45s");
    }

    #[test]
    fn format_secs_above_minute() {
        assert_eq!(format_secs(183), "3:03");
    }

    #[test]
    fn format_secs_zero() {
        assert_eq!(format_secs(0), "0s");
    }

    #[test]
    fn format_secs_59_stays_below_minute() {
        assert_eq!(format_secs(59), "59s");
    }

    #[test]
    fn format_secs_60_crosses_into_minutes() {
        assert_eq!(format_secs(60), "1:00");
    }

    #[test]
    fn format_secs_pads_seconds_with_leading_zero() {
        // 65 seconds = 1:05, not 1:5
        assert_eq!(format_secs(65), "1:05");
    }
}
