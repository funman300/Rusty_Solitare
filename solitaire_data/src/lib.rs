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
