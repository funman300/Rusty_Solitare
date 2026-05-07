//! Backend-agnostic sync plugin for Solitaire Quest.
//!
//! On startup, the plugin spawns an async pull task on [`AsyncComputeTaskPool`]
//! that fetches the remote payload from the active [`SyncProvider`]. Once the
//! task resolves, the merged result is written to disk and the in-world
//! resources are updated. On app exit, a blocking push sends the current local
//! state to the backend.
//!
//! The plugin is completely backend-agnostic: the caller (usually
//! `solitaire_app`) constructs the right [`SyncProvider`] implementation and
//! passes it to [`SyncPlugin::new`]. No `match` on a backend enum variant ever
//! occurs inside this module.

use std::sync::Arc;

use bevy::prelude::*;
use bevy::tasks::{futures_lite::future, AsyncComputeTaskPool, Task};
use chrono::Utc;
use uuid::Uuid;

use solitaire_data::{
    save_achievements_to, save_progress_to, save_replay_history_to, save_stats_to,
    AchievementRecord, PlayerProgress, Replay, StatsSnapshot, SyncError, SyncProvider,
};
use solitaire_sync::{merge, SyncPayload, SyncResponse};

use crate::achievement_plugin::{AchievementsResource, AchievementsStoragePath};
use crate::events::{GameWonEvent, ManualSyncRequestEvent, SyncCompleteEvent};
use crate::game_plugin::RecordingReplay;
use crate::progress_plugin::{ProgressResource, ProgressStoragePath};
use crate::resources::{GameStateResource, SyncStatus, SyncStatusResource};
use crate::stats_plugin::{LatestReplayPath, ReplayHistoryResource, StatsResource, StatsStoragePath};

// ---------------------------------------------------------------------------
// Public resources
// ---------------------------------------------------------------------------

/// Wraps the active sync backend. Shared with async tasks via [`Arc`].
///
/// Registered by [`SyncPlugin`] during `build()`. Other plugins may read this
/// resource to check [`SyncProvider::is_authenticated`] or
/// [`SyncProvider::backend_name`].
#[derive(Resource, Clone)]
pub struct SyncProviderResource(pub Arc<dyn SyncProvider + Send + Sync>);

/// Holds a pending pull result transferred from the async compute task to the
/// main thread. Consumed and cleared by [`poll_pull_result`].
#[derive(Resource, Default)]
pub struct PullTaskResult(pub Option<Result<SyncPayload, SyncError>>);

// ---------------------------------------------------------------------------
// Internal resources
// ---------------------------------------------------------------------------

/// Holds the in-flight pull task so [`poll_pull_result`] can check its status
/// each frame without blocking the main thread.
#[derive(Resource, Default)]
struct PullTask(Option<Task<Result<SyncPayload, SyncError>>>);

/// Holds the in-flight winning-replay upload task so the polling
/// system can harvest the resulting share URL on the main thread
/// without blocking. `None` outside an active upload; `Some(task)`
/// from `GameWonEvent` until the response lands.
#[derive(Resource, Default)]
struct PendingReplayUpload(Option<Task<Result<String, SyncError>>>);

// ---------------------------------------------------------------------------
// Plugin struct
// ---------------------------------------------------------------------------

/// Bevy plugin that manages the full sync lifecycle:
///
/// - **Startup** — spawns an async pull task on [`AsyncComputeTaskPool`].
/// - **Update** — polls the task each frame; on completion merges the remote
///   payload with local data, persists the result, and updates in-world
///   resources.
/// - **Last** — on [`AppExit`], performs a blocking push of the current local
///   state to the active backend.
///
/// Construct via [`SyncPlugin::new`], passing any type that implements
/// [`SyncProvider`].
pub struct SyncPlugin {
    provider: Arc<dyn SyncProvider + Send + Sync>,
}

impl SyncPlugin {
    /// Create a new [`SyncPlugin`] backed by the given [`SyncProvider`].
    ///
    /// The provider is heap-allocated and reference-counted so it can be
    /// cloned cheaply into async tasks.
    pub fn new(provider: impl SyncProvider + 'static) -> Self {
        Self {
            provider: Arc::new(provider),
        }
    }
}

impl Plugin for SyncPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(SyncProviderResource(self.provider.clone()))
            .init_resource::<SyncStatusResource>()
            .init_resource::<PullTaskResult>()
            .init_resource::<PullTask>()
            .init_resource::<PendingReplayUpload>()
            .add_message::<ManualSyncRequestEvent>()
            .add_message::<SyncCompleteEvent>()
            .add_systems(Startup, start_pull)
            .add_systems(
                Update,
                (
                    poll_pull_result,
                    handle_manual_sync_request,
                    push_replay_on_win,
                    poll_replay_upload_result,
                ),
            )
            .add_systems(Last, push_on_exit);
    }
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// Startup system: spawns the async pull task and sets status to `Syncing`.
fn start_pull(
    provider: Res<SyncProviderResource>,
    mut task_res: ResMut<PullTask>,
    mut status: ResMut<SyncStatusResource>,
) {
    let provider = provider.0.clone();
    let task = AsyncComputeTaskPool::get().spawn(async move {
        provider.pull().await
    });
    task_res.0 = Some(task);
    status.0 = SyncStatus::Syncing;
}

/// Update system: starts a new pull task when `ManualSyncRequestEvent` is
/// received, but only if no pull is already in flight.
fn handle_manual_sync_request(
    mut events: MessageReader<ManualSyncRequestEvent>,
    provider: Res<SyncProviderResource>,
    mut task_res: ResMut<PullTask>,
    mut status: ResMut<SyncStatusResource>,
) {
    if events.is_empty() {
        return;
    }
    events.clear();
    if task_res.0.is_some() {
        return; // Already pulling — ignore.
    }
    let provider = provider.0.clone();
    let task = AsyncComputeTaskPool::get().spawn(async move {
        provider.pull().await
    });
    task_res.0 = Some(task);
    status.0 = SyncStatus::Syncing;
}

/// Update system: polls the pull task without blocking.
///
/// When the task resolves successfully:
/// 1. Merges the remote payload with the current local state.
/// 2. Persists the merged result atomically.
/// 3. Updates the in-world [`StatsResource`], [`AchievementsResource`], and
///    [`ProgressResource`].
/// 4. Sets [`SyncStatusResource`] to [`SyncStatus::LastSynced`].
///
/// On failure, sets [`SyncStatusResource`] to [`SyncStatus::Error`].
#[allow(clippy::too_many_arguments)]
fn poll_pull_result(
    mut task_res: ResMut<PullTask>,
    mut status: ResMut<SyncStatusResource>,
    mut stats: ResMut<StatsResource>,
    stats_path: Res<StatsStoragePath>,
    mut achievements: ResMut<AchievementsResource>,
    achievements_path: Res<AchievementsStoragePath>,
    mut progress: ResMut<ProgressResource>,
    progress_path: Res<ProgressStoragePath>,
    mut complete_writer: MessageWriter<SyncCompleteEvent>,
) {
    let Some(task) = task_res.0.as_mut() else {
        return;
    };
    let Some(result) = future::block_on(future::poll_once(task)) else {
        return;
    };
    task_res.0 = None;

    match result {
        Ok(remote) => {
            let local = build_payload(&stats.0, &achievements.0, &progress.0);
            let (merged, conflicts) = merge(&local, &remote);

            // Persist merged state atomically.
            if let Some(p) = &stats_path.0
                && let Err(e) = save_stats_to(p, &merged.stats) {
                    warn!("sync: failed to persist stats: {e}");
                }
            if let Some(p) = &achievements_path.0
                && let Err(e) = save_achievements_to(p, &merged.achievements) {
                    warn!("sync: failed to persist achievements: {e}");
                }
            if let Some(p) = &progress_path.0
                && let Err(e) = save_progress_to(p, &merged.progress) {
                    warn!("sync: failed to persist progress: {e}");
                }

            // Update in-world resources.
            let now = Utc::now();
            stats.0 = merged.stats.clone();
            achievements.0 = merged.achievements.clone();
            progress.0 = merged.progress.clone();
            status.0 = SyncStatus::LastSynced(now);

            complete_writer.write(SyncCompleteEvent(Ok(SyncResponse {
                merged,
                server_time: now,
                conflicts,
            })));
        }
        Err(SyncError::UnsupportedPlatform) => {
            // No backend configured — not an error, just leave status as Idle.
            status.0 = SyncStatus::Idle;
        }
        Err(e) => {
            warn!("sync pull failed: {e}");
            let msg = match &e {
                SyncError::Network(_) => "Can't reach server — check your connection".to_string(),
                SyncError::Auth(_) => "Login expired — tap Sync Now after re-logging in".to_string(),
                SyncError::Serialization(_) => format!("Unexpected server response: {e}"),
                SyncError::UnsupportedPlatform => unreachable!("handled above"),
            };
            status.0 = SyncStatus::Error(msg.clone());
            complete_writer.write(SyncCompleteEvent(Err(msg)));
        }
    }
}

/// Last-schedule system: pushes the current local state on [`AppExit`].
///
/// A blocking push is acceptable here — ARCHITECTURE.md §4 explicitly notes
/// that blocking on exit is permitted because the game loop is already
/// shutting down.
fn push_on_exit(
    mut exit_events: MessageReader<AppExit>,
    provider: Res<SyncProviderResource>,
    stats: Res<StatsResource>,
    achievements: Res<AchievementsResource>,
    progress: Res<ProgressResource>,
) {
    if exit_events.is_empty() {
        return;
    }
    exit_events.clear();

    let payload = build_payload(&stats.0, &achievements.0, &progress.0);
    let provider = provider.0.clone();

    // Prefer an existing tokio runtime; fall back to futures_lite block_on
    // for environments (e.g. tests) that don't have one.
    let result = match tokio::runtime::Handle::try_current() {
        Ok(handle) => handle.block_on(provider.push(&payload)),
        Err(_) => future::block_on(provider.push(&payload)),
    };
    match result {
        Ok(_) => {}
        // `UnsupportedPlatform` is the expected response of
        // `LocalOnlyProvider`; treat it the same as the pull path does —
        // no backend configured is not a failure.
        Err(SyncError::UnsupportedPlatform) => {}
        Err(e) => {
            // Log real push failures on exit so they appear in crash/log
            // reports. We cannot surface them to the UI at this point (game
            // loop is done).
            warn!("sync push on exit failed: {e}");
        }
    }
}

/// Update-schedule system: on each `GameWonEvent` push the just-completed
/// replay to the active sync backend so it's available for web playback.
///
/// Spawned as a fire-and-forget task on `AsyncComputeTaskPool` — the game
/// loop never blocks on the network round-trip. Errors are logged but
/// never surfaced to the UI; failure to upload is non-fatal because the
/// replay is also persisted locally by `game_plugin::record_replay_on_win`,
/// so the player can still review it on the next login. `LocalOnlyProvider`'s
/// `UnsupportedPlatform` is silently absorbed in the same way the
/// `push_on_exit` path handles it.
fn push_replay_on_win(
    mut wins: MessageReader<GameWonEvent>,
    provider: Res<SyncProviderResource>,
    game: Res<GameStateResource>,
    recording: Res<RecordingReplay>,
    mut pending: ResMut<PendingReplayUpload>,
) {
    for ev in wins.read() {
        // Empty-recording guard mirrors `record_replay_on_win` —
        // synthesised win events from XP / streak tests must not trigger
        // a server upload.
        if recording.moves.is_empty() {
            continue;
        }
        let replay = Replay::new(
            game.0.seed,
            game.0.draw_mode.clone(),
            game.0.mode,
            ev.time_seconds,
            ev.score,
            Utc::now().date_naive(),
            recording.moves.clone(),
        );
        let provider = provider.0.clone();
        let task = AsyncComputeTaskPool::get()
            .spawn(async move { provider.push_replay(&replay).await });
        // If a previous upload is still in flight, drop it — the most
        // recent win is the one whose share link the player will care
        // about. Bevy's `Task` Drop cancels cooperatively.
        pending.0 = Some(task);
    }
}

/// Update-schedule system: harvests the upload task's result on the
/// main thread once it resolves. On success writes the share URL into
/// the most-recent entry of [`ReplayHistoryResource`] (`replays[0]`,
/// guaranteed by `record_replay_on_win` to be the win this upload
/// covers, since `cancel-on-replace` in `push_replay_on_win` drops any
/// older in-flight task) and persists the updated history to disk so
/// the URL survives a restart. `UnsupportedPlatform` (the
/// `LocalOnlyProvider` no-op path) is silently absorbed; real network
/// / auth errors log a warn but never clobber an existing URL.
fn poll_replay_upload_result(
    mut pending: ResMut<PendingReplayUpload>,
    mut history: ResMut<ReplayHistoryResource>,
    replay_path: Res<LatestReplayPath>,
) {
    let Some(task) = pending.0.as_mut() else {
        return;
    };
    let Some(result) = future::block_on(future::poll_once(task)) else {
        return;
    };
    pending.0 = None;
    let url = match result {
        Ok(url) => url,
        Err(SyncError::UnsupportedPlatform) => return,
        Err(e) => {
            warn!("replay upload failed: {e}");
            return;
        }
    };
    let Some(entry) = history.0.replays.first_mut() else {
        // Defensive: `push_replay_on_win` only fires after a win, so a
        // missing replays[0] means another system cleared the history
        // mid-upload. Drop the URL silently rather than panicking.
        return;
    };
    entry.share_url = Some(url);
    if let Some(path) = replay_path.0.as_deref()
        && let Err(e) = save_replay_history_to(path, &history.0)
    {
        warn!("failed to persist share URL into replay history: {e}");
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Constructs a [`SyncPayload`] from the current in-world state.
///
/// `user_id` is set to [`Uuid::nil()`] — the server replaces it with the
/// authenticated user's real ID when it processes the push request.
fn build_payload(
    stats: &StatsSnapshot,
    achievements: &[AchievementRecord],
    progress: &PlayerProgress,
) -> SyncPayload {
    SyncPayload {
        user_id: Uuid::nil(),
        stats: stats.clone(),
        achievements: achievements.to_vec(),
        progress: progress.clone(),
        last_modified: Utc::now(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use solitaire_data::SyncError;
    use solitaire_sync::SyncResponse;

    /// A no-op sync provider that always returns a default payload on pull
    /// and succeeds silently on push. Used to exercise the plugin in headless
    /// tests without any network I/O.
    struct NoOpProvider;

    #[async_trait::async_trait]
    impl SyncProvider for NoOpProvider {
        async fn pull(&self) -> Result<SyncPayload, SyncError> {
            Ok(SyncPayload {
                user_id: Uuid::nil(),
                stats: StatsSnapshot::default(),
                achievements: vec![],
                progress: PlayerProgress::default(),
                last_modified: Utc::now(),
            })
        }

        async fn push(&self, _payload: &SyncPayload) -> Result<SyncResponse, SyncError> {
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

        fn backend_name(&self) -> &'static str {
            "no-op"
        }

        fn is_authenticated(&self) -> bool {
            false
        }
    }

    /// A provider that always fails on pull, used to test the error path.
    struct FailingProvider;

    #[async_trait::async_trait]
    impl SyncProvider for FailingProvider {
        async fn pull(&self) -> Result<SyncPayload, SyncError> {
            Err(SyncError::Network("simulated failure".to_string()))
        }

        async fn push(&self, _payload: &SyncPayload) -> Result<SyncResponse, SyncError> {
            Err(SyncError::Network("simulated failure".to_string()))
        }

        fn backend_name(&self) -> &'static str {
            "failing"
        }

        fn is_authenticated(&self) -> bool {
            false
        }
    }

    fn headless_app_with(provider: impl SyncProvider + 'static) -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(crate::game_plugin::GamePlugin)
            .add_plugins(crate::table_plugin::TablePlugin)
            .add_plugins(crate::stats_plugin::StatsPlugin::headless())
            .add_plugins(crate::progress_plugin::ProgressPlugin::headless())
            .add_plugins(crate::achievement_plugin::AchievementPlugin::headless())
            .add_plugins(SyncPlugin::new(provider));
        // MinimalPlugins does not register keyboard input.
        app.init_resource::<bevy::input::ButtonInput<KeyCode>>();
        app.update();
        app
    }

    #[test]
    fn sync_provider_resource_is_registered() {
        let app = headless_app_with(NoOpProvider);
        assert!(app.world().get_resource::<SyncProviderResource>().is_some());
    }

    #[test]
    fn sync_status_becomes_syncing_on_startup() {
        // After the first update() the startup system has run and set Syncing,
        // but the async task may not have resolved yet.
        let mut app = headless_app_with(NoOpProvider);
        // Run a second update to give the task pool a chance to complete.
        app.update();
        // Status is either Syncing (task still running) or LastSynced (resolved).
        let status = &app.world().resource::<SyncStatusResource>().0;
        assert!(
            matches!(
                status,
                SyncStatus::Syncing | SyncStatus::LastSynced(_)
            ),
            "status should be Syncing or LastSynced, got {status:?}"
        );
    }

    #[test]
    fn pull_failure_sets_error_status() {
        let mut app = headless_app_with(FailingProvider);
        // Wall-clock-bounded loop instead of a fixed 5-update budget.
        // Under heavy parallel cargo-test load the AsyncComputeTaskPool
        // can be starved long enough that 5 updates aren't sufficient
        // for the failing pull to surface. Pumping until either the
        // status flips to `Error` or a 5-second deadline elapses
        // mirrors the auto-save flake fix and turns this test from
        // "pass on a fast machine" into "pass on any machine that
        // makes meaningful progress".
        let deadline =
            std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            app.update();
            if matches!(
                app.world().resource::<SyncStatusResource>().0,
                SyncStatus::Error(_)
            ) {
                break;
            }
            if std::time::Instant::now() >= deadline {
                break;
            }
            std::thread::yield_now();
        }
        let status = &app.world().resource::<SyncStatusResource>().0;
        assert!(
            matches!(status, SyncStatus::Error(_)),
            "expected Error status after failing pull, got {status:?}"
        );
    }

    #[test]
    fn build_payload_sets_nil_user_id() {
        let payload = build_payload(
            &StatsSnapshot::default(),
            &[],
            &PlayerProgress::default(),
        );
        assert_eq!(payload.user_id, Uuid::nil());
    }

    #[test]
    fn build_payload_clones_stats() {
        let stats = StatsSnapshot { games_played: 42, ..Default::default() };
        let payload = build_payload(&stats, &[], &PlayerProgress::default());
        assert_eq!(payload.stats.games_played, 42);
    }

    /// `poll_replay_upload_result` must write the resolved share URL
    /// into `replays[0].share_url` AND persist the updated history to
    /// disk so the URL survives a restart. Pins v0.19.0's persistent
    /// share-link contract — the v0.18.0 ephemeral
    /// `LastSharedReplayUrl` resource is gone, so a regression here
    /// would silently drop the link.
    #[test]
    fn upload_result_writes_share_url_into_replay_and_persists() {
        use solitaire_core::game_state::{DrawMode, GameMode};
        use solitaire_data::{
            load_replay_history_from, save_replay_history_to, Replay, ReplayHistory,
        };

        let mut app = headless_app_with(NoOpProvider);
        let path = std::env::temp_dir()
            .join("solitaire_test_replay_share_url_persist.json");
        let _ = std::fs::remove_file(&path);

        // Seed the in-memory history with a single replay carrying no
        // share_url — the upload-poll path must populate it.
        let initial = Replay::new(
            42,
            DrawMode::DrawOne,
            GameMode::Classic,
            60,
            500,
            chrono::NaiveDate::from_ymd_opt(2026, 5, 6).expect("valid date"),
            vec![],
        );
        let history = ReplayHistory {
            schema_version: solitaire_data::REPLAY_HISTORY_SCHEMA_VERSION,
            replays: vec![initial],
        };
        save_replay_history_to(&path, &history).expect("seed history on disk");
        app.insert_resource(crate::stats_plugin::ReplayHistoryResource(history));
        app.insert_resource(crate::stats_plugin::LatestReplayPath(Some(path.clone())));

        // Pre-resolved task carrying the URL the production path would
        // get back from the server.
        let url = "https://example.test/replays/abc123".to_string();
        let task = AsyncComputeTaskPool::get().spawn({
            let url = url.clone();
            async move { Ok::<String, SyncError>(url) }
        });
        app.world_mut()
            .resource_mut::<PendingReplayUpload>()
            .0 = Some(task);

        // Pump frames until the polling system observes the task as
        // ready and clears `PendingReplayUpload`.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while app.world().resource::<PendingReplayUpload>().0.is_some() {
            app.update();
            std::thread::yield_now();
            if std::time::Instant::now() >= deadline {
                break;
            }
        }
        assert!(
            app.world().resource::<PendingReplayUpload>().0.is_none(),
            "upload task should have been consumed within 15 s wall-clock",
        );

        // In-memory contract: replays[0].share_url is now Some(url).
        let live = app
            .world()
            .resource::<crate::stats_plugin::ReplayHistoryResource>();
        assert_eq!(
            live.0.replays.first().and_then(|r| r.share_url.clone()),
            Some(url.clone()),
            "share URL must be written into replays[0].share_url",
        );
        // Persistence contract: a fresh load picks up the same URL.
        let on_disk = load_replay_history_from(&path).expect("history must reload");
        assert_eq!(
            on_disk.replays.first().and_then(|r| r.share_url.clone()),
            Some(url),
            "share URL must survive a save/load round-trip",
        );

        let _ = std::fs::remove_file(&path);
    }
}
