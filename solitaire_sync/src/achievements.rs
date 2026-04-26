//! Shared `AchievementRecord` definition — used by both the game client and
//! the sync server.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// One player's unlock state for a single achievement.
///
/// The achievement *definition* (name, description, condition fn) lives in
/// `solitaire_core`. This record only tracks runtime unlock state and is
/// what gets persisted and synced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AchievementRecord {
    /// Matches the `id` field of the corresponding `AchievementDef` in
    /// `solitaire_core`.
    pub id: String,
    /// Whether the achievement has been unlocked.
    pub unlocked: bool,
    /// The UTC timestamp at which the achievement was first unlocked.
    /// `None` when not yet unlocked.
    pub unlock_date: Option<DateTime<Utc>>,
    /// Whether the unlock reward (XP, cosmetic, etc.) has been granted.
    pub reward_granted: bool,
}

impl AchievementRecord {
    /// Construct an initial record for an achievement that is not yet unlocked.
    pub fn locked(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            unlocked: false,
            unlock_date: None,
            reward_granted: false,
        }
    }

    /// Mark this record unlocked at the given timestamp.
    ///
    /// No-op if already unlocked — preserves the earliest `unlock_date` so
    /// that merging two unlock records always keeps the older timestamp.
    pub fn unlock(&mut self, at: DateTime<Utc>) {
        if self.unlocked {
            return;
        }
        self.unlocked = true;
        self.unlock_date = Some(at);
    }
}
