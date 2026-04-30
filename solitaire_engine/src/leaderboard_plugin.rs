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

use bevy::prelude::*;
use bevy::tasks::{futures_lite::future, AsyncComputeTaskPool, Task};
use solitaire_data::settings::SyncBackend;
use solitaire_sync::LeaderboardEntry;

use crate::events::{InfoToastEvent, ToggleLeaderboardRequestEvent};
use crate::font_plugin::FontResource;
use crate::settings_plugin::SettingsResource;
use crate::sync_plugin::SyncProviderResource;
use crate::ui_modal::{
    spawn_modal, spawn_modal_actions, spawn_modal_button, spawn_modal_header, ButtonVariant,
};
use crate::ui_theme::{
    ACCENT_PRIMARY, BORDER_SUBTLE, STATE_INFO, TEXT_PRIMARY, TEXT_SECONDARY, TYPE_BODY,
    TYPE_BODY_LG, TYPE_CAPTION, VAL_SPACE_4, Z_MODAL_PANEL,
};

// ---------------------------------------------------------------------------
// Resources
// ---------------------------------------------------------------------------

/// Cached leaderboard data. `None` means no fetch has completed yet.
#[derive(Resource, Default, Debug, Clone)]
pub struct LeaderboardResource(pub Option<Vec<LeaderboardEntry>>);

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
            .add_message::<ToggleLeaderboardRequestEvent>()
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
                )
                    .chain(),
            );
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

    // Spawn the panel immediately with whatever data we have (may be None).
    spawn_leaderboard_screen(&mut commands, data.0.as_deref(), font_res.as_deref());

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
fn update_leaderboard_panel(
    mut commands: Commands,
    mut result_res: ResMut<LeaderboardFetchResult>,
    mut data: ResMut<LeaderboardResource>,
    screens: Query<Entity, With<LeaderboardScreen>>,
    font_res: Option<Res<FontResource>>,
    closed_flag: Res<ClosedThisFrame>,
) {
    let Some(result) = result_res.0.take() else { return };

    match result {
        Ok(entries) => {
            data.0 = Some(entries);
        }
        Err(e) => {
            warn!("leaderboard fetch failed: {e}");
            if data.0.is_none() {
                data.0 = Some(vec![]); // show empty rather than spinner forever
            }
        }
    }

    // Rebuild the panel if it's open — but not if the user just closed it in
    // this frame (their despawn command is still deferred).
    if closed_flag.0 {
        return;
    }
    for entity in &screens {
        commands.entity(entity).despawn();
        spawn_leaderboard_screen(&mut commands, data.0.as_deref(), font_res.as_deref());
    }
}

/// Click handler for the modal's "Done" button — despawns the overlay.
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
                if let SyncBackend::SolitaireServer { username, .. } = &s.0.sync_backend {
                    Some(username.clone())
                } else {
                    None
                }
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
            toast.write(InfoToastEvent("Leaderboard update failed".to_string()));
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
            toast.write(InfoToastEvent("Leaderboard update failed".to_string()));
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
    entries: Option<&[LeaderboardEntry]>,
    font_res: Option<&FontResource>,
) {
    spawn_modal(commands, LeaderboardScreen, Z_MODAL_PANEL, |card| {
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

        card.spawn((
            Text::new("Use Opt In / Opt Out to control your visibility on the server."),
            font_caption.clone(),
            TextColor(TEXT_SECONDARY),
        ));

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

        // Subtle separator between the controls and the data area.
        card.spawn((
            Node {
                height: Val::Px(1.0),
                ..default()
            },
            BackgroundColor(BORDER_SUBTLE),
        ));

        match entries {
            None => {
                card.spawn((
                    Text::new("Fetching\u{2026}"),
                    font_status.clone(),
                    TextColor(STATE_INFO),
                ));
            }
            Some([]) => {
                card.spawn((
                    Text::new("No entries yet \u{2014} sync and opt in to appear here."),
                    font_row.clone(),
                    TextColor(TEXT_SECONDARY),
                ));
            }
            Some(rows) => {
                // Column headers
                card.spawn(Node {
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
                        .map(format_secs)
                        .unwrap_or_else(|| "-".to_string());
                    let score_str = entry
                        .best_score
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| "-".to_string());

                    card.spawn(Node {
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
        assert!(app.world().resource::<LeaderboardResource>().0.is_none());
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
