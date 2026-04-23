use async_trait::async_trait;
use solitaire_data::{SyncError, SyncProvider};
use solitaire_sync::{SyncPayload, SyncResponse};

/// Google Play Games Services sync client — desktop/iOS stub.
///
/// Always returns [`SyncError::UnsupportedPlatform`]. The real JNI implementation
/// lives in `android.rs` and is compiled only on Android (Phase: Android).
pub struct GpgsClient;

impl GpgsClient {
    /// Creates a new `GpgsClient` stub. No-op on non-Android platforms.
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
