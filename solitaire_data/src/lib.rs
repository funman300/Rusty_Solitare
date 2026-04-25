use async_trait::async_trait;
use solitaire_sync::{SyncPayload, SyncResponse};
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
}

pub mod stats;
pub use stats::StatsSnapshot;

pub mod storage;
pub use storage::{load_stats, load_stats_from, save_stats, save_stats_to, stats_file_path};

pub mod achievements;
pub use achievements::{
    achievements_file_path, load_achievements_from, save_achievements_to, AchievementRecord,
};

pub mod progress;
pub use progress::{
    level_for_xp, load_progress_from, progress_file_path, save_progress_to, xp_for_win,
    PlayerProgress,
};
