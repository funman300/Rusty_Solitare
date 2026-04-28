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
    save_achievements_to, save_progress_to, save_stats_to, AchievementRecord, PlayerProgress,
    StatsSnapshot, SyncError, SyncProvider,
};
use solitaire_sync::{merge, SyncPayload};

use crate::achievement_plugin::{AchievementsResource, AchievementsStoragePath};
use crate::events::ManualSyncRequestEvent;
use crate::progress_plugin::{ProgressResource, ProgressStoragePath};
use crate::resources::{SyncStatus, SyncStatusResource};
use crate::stats_plugin::{StatsResource, StatsStoragePath};

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
            .add_message::<ManualSyncRequestEvent>()
            .add_systems(Startup, start_pull)
            .add_systems(Update, (poll_pull_result, handle_manual_sync_request))
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
            let (merged, _conflicts) = merge(&local, &remote);

            // Persist merged state atomically.
            if let Some(p) = &stats_path.0 {
                if let Err(e) = save_stats_to(p, &merged.stats) {
                    warn!("sync: failed to persist stats: {e}");
                }
            }
            if let Some(p) = &achievements_path.0 {
                if let Err(e) = save_achievements_to(p, &merged.achievements) {
                    warn!("sync: failed to persist achievements: {e}");
                }
            }
            if let Some(p) = &progress_path.0 {
                if let Err(e) = save_progress_to(p, &merged.progress) {
                    warn!("sync: failed to persist progress: {e}");
                }
            }

            // Update in-world resources.
            stats.0 = merged.stats;
            achievements.0 = merged.achievements;
            progress.0 = merged.progress;
            status.0 = SyncStatus::LastSynced(Utc::now());
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
            status.0 = SyncStatus::Error(msg);
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
    if let Err(e) = result {
        // Log push failures on exit so they appear in crash/log reports.
        // We cannot surface them to the UI at this point (game loop is done).
        warn!("sync push on exit failed: {e}");
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
        // Pump frames until the task resolves (it's synchronous under
        // AsyncComputeTaskPool in test mode, so a few updates suffice).
        for _ in 0..5 {
            app.update();
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
}
