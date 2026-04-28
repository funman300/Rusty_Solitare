//! Sync pull and push handlers.
//!
//! `GET /api/sync/pull`  — return the server's stored payload for this user.
//! `POST /api/sync/push` — receive the client's payload, merge, store, return.

use axum::{extract::State, Json};
use chrono::Utc;
use sqlx::SqlitePool;

use solitaire_sync::{
    merge, AchievementRecord, PlayerProgress, StatsSnapshot, SyncPayload, SyncResponse,
};

use crate::{error::AppError, middleware::AuthenticatedUser, AppState};

// ---------------------------------------------------------------------------
// Database row helpers
// ---------------------------------------------------------------------------

struct SyncRow {
    stats_json: Option<String>,
    achievements_json: Option<String>,
    progress_json: Option<String>,
}

/// Load the stored `SyncPayload` for `user_id` from the database.
/// Returns `None` if this user has not pushed any data yet.
async fn load_sync_row(pool: &SqlitePool, user_id: &str) -> Result<Option<SyncRow>, AppError> {
    let row = sqlx::query_as!(
        SyncRow,
        "SELECT stats_json, achievements_json, progress_json FROM sync_state WHERE user_id = ?",
        user_id
    )
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// Deserialize a stored `SyncRow` into a `SyncPayload`.
fn row_to_payload(row: &SyncRow, user_id: &str) -> Result<SyncPayload, AppError> {
    let stats_json = row.stats_json.as_deref()
        .ok_or_else(|| AppError::Internal("missing stats_json".into()))?;
    let achievements_json = row.achievements_json.as_deref()
        .ok_or_else(|| AppError::Internal("missing achievements_json".into()))?;
    let progress_json = row.progress_json.as_deref()
        .ok_or_else(|| AppError::Internal("missing progress_json".into()))?;

    let stats: StatsSnapshot = serde_json::from_str(stats_json)?;
    let achievements: Vec<AchievementRecord> = serde_json::from_str(achievements_json)?;
    let progress: PlayerProgress = serde_json::from_str(progress_json)?;

    Ok(SyncPayload {
        user_id: user_id
            .parse()
            .map_err(|_| AppError::Internal("stored user_id is not a valid UUID".into()))?,
        stats,
        achievements,
        progress,
        last_modified: Utc::now(),
    })
}

/// Persist a `SyncPayload` for `user_id` using an upsert.
async fn store_payload(
    pool: &SqlitePool,
    user_id: &str,
    payload: &SyncPayload,
) -> Result<(), AppError> {
    let stats_json = serde_json::to_string(&payload.stats)?;
    let achievements_json = serde_json::to_string(&payload.achievements)?;
    let progress_json = serde_json::to_string(&payload.progress)?;
    let now = Utc::now().to_rfc3339();

    sqlx::query!(
        r#"INSERT INTO sync_state (user_id, stats_json, achievements_json, progress_json, last_modified)
           VALUES (?, ?, ?, ?, ?)
           ON CONFLICT(user_id) DO UPDATE SET
               stats_json        = excluded.stats_json,
               achievements_json = excluded.achievements_json,
               progress_json     = excluded.progress_json,
               last_modified     = excluded.last_modified"#,
        user_id,
        stats_json,
        achievements_json,
        progress_json,
        now
    )
    .execute(pool)
    .await?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `GET /api/sync/pull` — return the server's stored payload for this user.
///
/// If the user has never pushed any data, returns a default payload.
pub async fn pull(
    State(state): State<AppState>,
    user: AuthenticatedUser,
) -> Result<Json<SyncResponse>, AppError> {
    let stored_payload = match load_sync_row(&state.pool, &user.user_id).await? {
        Some(row) => row_to_payload(&row, &user.user_id)?,
        None => {
            // First pull — no server data yet; return an empty default payload.
            let uid = user
                .user_id
                .parse()
                .map_err(|_| AppError::Internal("invalid user_id UUID".into()))?;
            SyncPayload {
                user_id: uid,
                stats: StatsSnapshot::default(),
                achievements: vec![],
                progress: PlayerProgress::default(),
                last_modified: Utc::now(),
            }
        }
    };

    Ok(Json(SyncResponse {
        merged: stored_payload,
        server_time: Utc::now(),
        conflicts: vec![],
    }))
}

/// `POST /api/sync/push` — merge the client's payload with the server's
/// stored payload, persist the result, and return it.
///
/// If the user has opted in to the leaderboard, the leaderboard row is also
/// updated with the merged `best_single_score` and `fastest_win_seconds` so
/// scores stay in sync without a separate submission step.
pub async fn push(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Json(client_payload): Json<SyncPayload>,
) -> Result<Json<SyncResponse>, AppError> {
    // Reject payloads that claim to belong to a different user.
    if client_payload.user_id.to_string() != user.user_id {
        return Err(AppError::BadRequest("user_id mismatch".into()));
    }

    let server_payload = match load_sync_row(&state.pool, &user.user_id).await? {
        Some(row) => row_to_payload(&row, &user.user_id)?,
        None => {
            // First push — nothing to merge against; store directly.
            store_payload(&state.pool, &user.user_id, &client_payload).await?;
            update_leaderboard_if_opted_in(&state.pool, &user.user_id, &client_payload).await?;
            return Ok(Json(SyncResponse {
                merged: client_payload,
                server_time: Utc::now(),
                conflicts: vec![],
            }));
        }
    };

    let (merged, conflicts) = merge(&client_payload, &server_payload);

    store_payload(&state.pool, &user.user_id, &merged).await?;
    update_leaderboard_if_opted_in(&state.pool, &user.user_id, &merged).await?;

    Ok(Json(SyncResponse {
        merged,
        server_time: Utc::now(),
        conflicts,
    }))
}

/// If the user is opted in to the leaderboard, update their row with the
/// better of the stored and incoming `best_single_score` / `fastest_win_seconds`.
///
/// Uses SQLite `MIN`/`MAX` in the UPDATE so the database never regresses
/// a score even if the client sends stale data.
async fn update_leaderboard_if_opted_in(
    pool: &SqlitePool,
    user_id: &str,
    payload: &SyncPayload,
) -> Result<(), AppError> {
    // Only update if the user has opted in (leaderboard row exists).
    let opted_in: Option<i64> = sqlx::query_scalar!(
        "SELECT leaderboard_opt_in FROM users WHERE id = ?",
        user_id
    )
    .fetch_optional(pool)
    .await?;

    if opted_in != Some(1) {
        return Ok(());
    }

    let best_score = payload.stats.best_single_score as i64;
    let fastest = if payload.stats.fastest_win_seconds == u64::MAX {
        // Sentinel "never won" value — don't store.
        None::<i64>
    } else {
        Some(payload.stats.fastest_win_seconds as i64)
    };
    let now = Utc::now().to_rfc3339();

    sqlx::query!(
        r#"UPDATE leaderboard
           SET best_score     = MAX(COALESCE(best_score, 0), ?),
               best_time_secs = CASE
                   WHEN ? IS NULL THEN best_time_secs
                   WHEN best_time_secs IS NULL THEN ?
                   ELSE MIN(best_time_secs, ?)
               END,
               recorded_at = ?
           WHERE user_id = ?"#,
        best_score,
        fastest, fastest, fastest,
        now,
        user_id
    )
    .execute(pool)
    .await?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests — pure merge logic; no database required
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use solitaire_sync::{AchievementRecord, PlayerProgress, StatsSnapshot, SyncPayload, merge};
    use uuid::Uuid;

    /// Build a minimal `SyncPayload` with default fields, overridden by the
    /// caller as needed. Using `Uuid::nil()` keeps every test self-contained.
    fn make_payload(stats: StatsSnapshot, achievements: Vec<AchievementRecord>) -> SyncPayload {
        SyncPayload {
            user_id: Uuid::nil(),
            stats,
            achievements,
            progress: PlayerProgress::default(),
            last_modified: Utc::now(),
        }
    }

    fn default_payload() -> SyncPayload {
        make_payload(StatsSnapshot::default(), vec![])
    }

    // -----------------------------------------------------------------------
    // 1. Merge keeps the higher games_played from the remote side.
    // -----------------------------------------------------------------------

    #[test]
    fn sync_merge_keeps_higher_games_played() {
        let mut local = default_payload();
        local.stats.games_played = 10;

        let mut remote = default_payload();
        remote.stats.games_played = 25; // remote is ahead

        let (merged, _) = merge(&local, &remote);
        assert_eq!(
            merged.stats.games_played, 25,
            "merge must keep the higher games_played value from remote"
        );
    }

    // -----------------------------------------------------------------------
    // 2. Merge keeps the higher best_single_score from the local side.
    // -----------------------------------------------------------------------

    #[test]
    fn sync_merge_keeps_best_single_score() {
        let mut local = default_payload();
        local.stats.best_single_score = 8_000; // local is better

        let mut remote = default_payload();
        remote.stats.best_single_score = 3_500;

        let (merged, _) = merge(&local, &remote);
        assert_eq!(
            merged.stats.best_single_score, 8_000,
            "merge must keep the higher best_single_score (local in this case)"
        );
    }

    // -----------------------------------------------------------------------
    // 3. Merge never removes an achievement that is unlocked on one side.
    // -----------------------------------------------------------------------

    #[test]
    fn sync_merge_never_removes_unlocked_achievement() {
        let mut unlocked = AchievementRecord::locked("first_win");
        unlocked.unlock(Utc::now());

        // local has the achievement unlocked; remote has no achievements at all.
        let local = make_payload(StatsSnapshot::default(), vec![unlocked]);
        let remote = make_payload(StatsSnapshot::default(), vec![]);

        let (merged, _) = merge(&local, &remote);

        let found = merged
            .achievements
            .iter()
            .find(|a| a.id == "first_win")
            .expect("achievement must survive the merge");
        assert!(
            found.unlocked,
            "achievement unlocked on local must remain unlocked after merge with remote that lacks it"
        );
    }

    // -----------------------------------------------------------------------
    // 4. merge(payload, payload) is idempotent for key numeric fields.
    // -----------------------------------------------------------------------

    #[test]
    fn sync_merge_is_idempotent() {
        let mut payload = default_payload();
        payload.stats.games_played = 42;
        payload.stats.games_won = 20;
        payload.stats.best_single_score = 5_500;
        payload.stats.fastest_win_seconds = 90;
        payload.stats.lifetime_score = 110_000;
        payload.progress.total_xp = 3_000;

        let (merged, _) = merge(&payload, &payload);

        assert_eq!(merged.stats.games_played, 42, "idempotent: games_played");
        assert_eq!(merged.stats.games_won, 20, "idempotent: games_won");
        assert_eq!(merged.stats.best_single_score, 5_500, "idempotent: best_single_score");
        assert_eq!(merged.stats.fastest_win_seconds, 90, "idempotent: fastest_win_seconds");
        assert_eq!(merged.stats.lifetime_score, 110_000, "idempotent: lifetime_score");
        assert_eq!(merged.progress.total_xp, 3_000, "idempotent: total_xp");
    }
}
