use async_trait::async_trait;
use solitaire_sync::{LeaderboardEntry, SyncPayload, SyncResponse};
use thiserror::Error;

/// All errors that can arise during sync operations.
#[derive(Debug, Error)]
pub enum SyncError {
    #[error("unsupported platform for this sync backend")]
    UnsupportedPlatform,
    // TODO: Replace String with concrete source error types (e.g. reqwest::Error,
    // serde_json::Error) when real implementations are added in Phase 8.
    #[error("network error: {0}")]
    Network(String),
    #[error("authentication error: {0}")]
    Auth(String),
    #[error("serialization error: {0}")]
    Serialization(String),
}

/// Every sync backend implements this trait. The SyncPlugin only calls these
/// methods — it never matches on a backend enum variant.
#[async_trait]
pub trait SyncProvider: Send + Sync {
    /// Fetch the remote sync payload. Returns the latest server state for merging.
    async fn pull(&self) -> Result<SyncPayload, SyncError>;
    /// Push the local payload to the backend. Returns the merged server response.
    async fn push(&self, payload: &SyncPayload) -> Result<SyncResponse, SyncError>;
    /// Human-readable name of this backend, used in settings UI and logs.
    fn backend_name(&self) -> &'static str;
    /// Returns true if the user is currently authenticated with this backend.
    fn is_authenticated(&self) -> bool;
    /// Mirror an achievement unlock to this backend (no-op for most backends).
    async fn mirror_achievement(&self, _id: &str) -> Result<(), SyncError> {
        Ok(())
    }
    /// Fetch the global leaderboard from this backend. Returns an empty list
    /// for backends that do not support leaderboards (e.g. `LocalOnlyProvider`).
    async fn fetch_leaderboard(&self) -> Result<Vec<LeaderboardEntry>, SyncError> {
        Ok(vec![])
    }
}

/// Blanket impl so `Box<dyn SyncProvider + Send + Sync>` (returned by
/// `provider_for_backend`) can be passed directly to `SyncPlugin::new`.
#[async_trait]
impl SyncProvider for Box<dyn SyncProvider + Send + Sync> {
    async fn pull(&self) -> Result<SyncPayload, SyncError> {
        (**self).pull().await
    }
    async fn push(&self, payload: &SyncPayload) -> Result<SyncResponse, SyncError> {
        (**self).push(payload).await
    }
    fn backend_name(&self) -> &'static str {
        (**self).backend_name()
    }
    fn is_authenticated(&self) -> bool {
        (**self).is_authenticated()
    }
    async fn mirror_achievement(&self, id: &str) -> Result<(), SyncError> {
        (**self).mirror_achievement(id).await
    }
    async fn fetch_leaderboard(&self) -> Result<Vec<LeaderboardEntry>, SyncError> {
        (**self).fetch_leaderboard().await
    }
}

pub mod stats;
pub use stats::{StatsExt, StatsSnapshot};

pub mod storage;
pub use storage::{
    cleanup_orphaned_tmp_files, delete_game_state_at, game_state_file_path, load_game_state_from,
    load_stats, load_stats_from, save_game_state_to, save_stats, save_stats_to, stats_file_path,
};

pub mod achievements;
pub use achievements::{
    achievements_file_path, load_achievements_from, save_achievements_to, AchievementRecord,
};

pub mod progress;
pub use progress::{
    daily_seed_for, level_for_xp, load_progress_from, progress_file_path, save_progress_to,
    xp_for_win, PlayerProgress,
};

pub mod weekly;
pub use weekly::{
    current_iso_week_key, weekly_goal_by_id, WeeklyGoalContext, WeeklyGoalDef, WeeklyGoalKind,
    WEEKLY_GOALS, WEEKLY_GOAL_XP,
};

pub mod challenge;
pub use challenge::{challenge_count, challenge_seed_for, CHALLENGE_SEEDS};

pub mod settings;
pub use settings::{
    load_settings_from, save_settings_to, settings_file_path, AnimSpeed, Settings, SyncBackend,
    Theme,
};

pub mod auth_tokens;
pub use auth_tokens::{
    delete_tokens, load_access_token, load_refresh_token, store_tokens, TokenError,
};

pub mod sync_client;
pub use sync_client::{provider_for_backend, LocalOnlyProvider, SolitaireServerClient};
