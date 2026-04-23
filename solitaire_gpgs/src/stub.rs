use async_trait::async_trait;
use solitaire_data::{SyncError, SyncProvider};
use solitaire_sync::{SyncPayload, SyncResponse};

/// Desktop/iOS stub — always returns UnsupportedPlatform.
/// Real implementation lives in android.rs (Phase: Android).
pub struct GpgsClient;

impl GpgsClient {
    pub fn new() -> Self {
        Self
    }
}

impl Default for GpgsClient {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SyncProvider for GpgsClient {
    async fn pull(&self) -> Result<SyncPayload, SyncError> {
        Err(SyncError::UnsupportedPlatform)
    }

    async fn push(&self, _payload: &SyncPayload) -> Result<SyncResponse, SyncError> {
        Err(SyncError::UnsupportedPlatform)
    }

    fn backend_name(&self) -> &'static str {
        "Google Play Games (unavailable on this platform)"
    }

    fn is_authenticated(&self) -> bool {
        false
    }
}
