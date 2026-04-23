use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Payload sent from client to server (and returned after server merge).
/// Full fields are added in Phase 8 (Sync System).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncPayload {
    pub user_id: Uuid,
    pub last_modified: DateTime<Utc>,
}

/// Response returned by the sync server after merging.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncResponse {
    pub server_time: DateTime<Utc>,
}
