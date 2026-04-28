use async_trait::async_trait;
use solitaire_data::{SyncError, SyncProvider};
use solitaire_sync::{ChallengeGoal, LeaderboardEntry, SyncPayload, SyncResponse};

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

    /// No-op stub — returns UnsupportedPlatform on non-Android targets.
    async fn mirror_achievement(&self, _id: &str) -> Result<(), SyncError> {
        Err(SyncError::UnsupportedPlatform)
    }

    /// No-op stub — returns UnsupportedPlatform on non-Android targets.
    async fn fetch_leaderboard(&self) -> Result<Vec<LeaderboardEntry>, SyncError> {
        Err(SyncError::UnsupportedPlatform)
    }

    /// No-op stub — returns UnsupportedPlatform on non-Android targets.
    async fn fetch_daily_challenge(&self) -> Result<Option<ChallengeGoal>, SyncError> {
        Err(SyncError::UnsupportedPlatform)
    }

    /// No-op stub — returns UnsupportedPlatform on non-Android targets.
    async fn opt_in_leaderboard(&self, _display_name: &str) -> Result<(), SyncError> {
        Err(SyncError::UnsupportedPlatform)
    }

    /// No-op stub — returns UnsupportedPlatform on non-Android targets.
    async fn opt_out_leaderboard(&self) -> Result<(), SyncError> {
        Err(SyncError::UnsupportedPlatform)
    }

    /// No-op stub — returns UnsupportedPlatform on non-Android targets.
    async fn delete_account(&self) -> Result<(), SyncError> {
        Err(SyncError::UnsupportedPlatform)
    }
}
