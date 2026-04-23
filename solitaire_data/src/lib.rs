use async_trait::async_trait;
use solitaire_sync::{SyncPayload, SyncResponse};
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
    async fn pull(&self) -> Result<SyncPayload, SyncError>;
    async fn push(&self, payload: &SyncPayload) -> Result<SyncResponse, SyncError>;
    fn backend_name(&self) -> &'static str;
    fn is_authenticated(&self) -> bool;
    /// Mirror an achievement unlock to this backend (no-op for most backends).
    async fn mirror_achievement(&self, _id: &str) -> Result<(), SyncError> {
        Ok(())
    }
}
