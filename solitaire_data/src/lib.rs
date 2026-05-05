use async_trait::async_trait;
use solitaire_sync::{ChallengeGoal, LeaderboardEntry, SyncPayload, SyncResponse};
use thiserror::Error;

/// All errors that can arise during sync operations.
#[derive(Debug, Error)]
pub enum SyncError {
    #[error("unsupported platform for this sync backend")]
    UnsupportedPlatform,
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
    /// Fetch today's daily challenge from the server. Returns `None` for
    /// backends that don't support it, or on any non-fatal network failure.
    async fn fetch_daily_challenge(&self) -> Result<Option<ChallengeGoal>, SyncError> {
        Ok(None)
    }
    /// Opt the authenticated player into the leaderboard with the given
    /// display name. No-op for backends that don't support leaderboards.
    async fn opt_in_leaderboard(&self, _display_name: &str) -> Result<(), SyncError> {
        Ok(())
    }
    /// Remove the authenticated player from the leaderboard.
    /// No-op for backends that don't support leaderboards.
    async fn opt_out_leaderboard(&self) -> Result<(), SyncError> {
        Ok(())
    }
    /// Permanently delete the authenticated player's account and all server
    /// data. No-op for backends that don't support account management.
    async fn delete_account(&self) -> Result<(), SyncError> {
        Ok(())
    }
    /// Upload a winning replay to the backend so it's available for web
    /// playback at `<server>/replays/<id>`. Default returns
    /// `UnsupportedPlatform` so backends without a server (e.g.
    /// `LocalOnlyProvider`) are silently no-op'd by the engine's
    /// push-on-win system, matching the same pattern `pull` / `push`
    /// follow.
    async fn push_replay(&self, _replay: &crate::replay::Replay) -> Result<(), SyncError> {
        Err(SyncError::UnsupportedPlatform)
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
    async fn fetch_daily_challenge(&self) -> Result<Option<ChallengeGoal>, SyncError> {
        (**self).fetch_daily_challenge().await
    }
    async fn opt_in_leaderboard(&self, display_name: &str) -> Result<(), SyncError> {
        (**self).opt_in_leaderboard(display_name).await
    }
    async fn opt_out_leaderboard(&self) -> Result<(), SyncError> {
        (**self).opt_out_leaderboard().await
    }
    async fn delete_account(&self) -> Result<(), SyncError> {
        (**self).delete_account().await
    }
    async fn push_replay(&self, replay: &crate::replay::Replay) -> Result<(), SyncError> {
        (**self).push_replay(replay).await
    }
}

pub mod stats;
pub use stats::{StatsExt, StatsSnapshot};

pub mod storage;
pub use storage::{
    cleanup_orphaned_tmp_files, delete_game_state_at, delete_time_attack_session_at,
    game_state_file_path, load_game_state_from, load_stats, load_stats_from,
    load_time_attack_session_from, load_time_attack_session_from_at, save_game_state_to,
    save_stats, save_stats_to, save_time_attack_session_to, stats_file_path,
    time_attack_session_path, time_attack_session_with_now, TimeAttackSession,
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
    Theme, WindowGeometry, TIME_BONUS_MULTIPLIER_MAX, TIME_BONUS_MULTIPLIER_MIN,
    TIME_BONUS_MULTIPLIER_STEP, TOOLTIP_DELAY_MAX_SECS, TOOLTIP_DELAY_MIN_SECS,
    TOOLTIP_DELAY_STEP_SECS,
};

pub mod auth_tokens;
pub use auth_tokens::{
    delete_tokens, load_access_token, load_refresh_token, store_tokens, TokenError,
};

pub mod sync_client;
pub use sync_client::{provider_for_backend, LocalOnlyProvider, SolitaireServerClient};

pub mod replay;
pub use replay::{
    latest_replay_path, load_latest_replay_from, save_latest_replay_to, Replay, ReplayMove,
    REPLAY_SCHEMA_VERSION,
};
