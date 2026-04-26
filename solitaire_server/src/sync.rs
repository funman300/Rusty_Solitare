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

use crate::{error::AppError, middleware::AuthenticatedUser};

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
    State(pool): State<SqlitePool>,
    user: AuthenticatedUser,
) -> Result<Json<SyncResponse>, AppError> {
    let stored_payload = match load_sync_row(&pool, &user.user_id).await? {
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
pub async fn push(
    State(pool): State<SqlitePool>,
    user: AuthenticatedUser,
    Json(client_payload): Json<SyncPayload>,
) -> Result<Json<SyncResponse>, AppError> {
    // Reject payloads that claim to belong to a different user.
    if client_payload.user_id.to_string() != user.user_id {
        return Err(AppError::BadRequest("user_id mismatch".into()));
    }

    let server_payload = match load_sync_row(&pool, &user.user_id).await? {
        Some(row) => row_to_payload(&row, &user.user_id)?,
        None => {
            // First push — nothing to merge against; store directly.
            store_payload(&pool, &user.user_id, &client_payload).await?;
            return Ok(Json(SyncResponse {
                merged: client_payload,
                server_time: Utc::now(),
                conflicts: vec![],
            }));
        }
    };

    let (merged, conflicts) = merge(&client_payload, &server_payload);

    store_payload(&pool, &user.user_id, &merged).await?;

    Ok(Json(SyncResponse {
        merged,
        server_time: Utc::now(),
        conflicts,
    }))
}
