//! Leaderboard endpoints.
//!
//! `GET  /api/leaderboard`       — list all opted-in entries (requires auth).
//! `POST /api/leaderboard/opt-in` — opt in and set / update display name.

use axum::{extract::State, Json};
use chrono::Utc;
use serde::Deserialize;
use sqlx::SqlitePool;

use solitaire_sync::LeaderboardEntry;

use crate::{error::AppError, middleware::AuthenticatedUser};

// ---------------------------------------------------------------------------
// Request shapes
// ---------------------------------------------------------------------------

/// Body for `POST /api/leaderboard/opt-in`.
#[derive(Debug, Deserialize)]
pub struct OptInRequest {
    /// The display name the player wants shown on the leaderboard.
    pub display_name: String,
}

// ---------------------------------------------------------------------------
// Database row helper
// ---------------------------------------------------------------------------

struct LeaderboardRow {
    display_name: Option<String>,
    best_score: Option<i64>,
    best_time_secs: Option<i64>,
    recorded_at: Option<String>,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `GET /api/leaderboard` — return all opted-in leaderboard entries.
///
/// Returns entries sorted by `best_score` descending (nulls last).
pub async fn get_leaderboard(
    State(pool): State<SqlitePool>,
    _user: AuthenticatedUser,
) -> Result<Json<Vec<LeaderboardEntry>>, AppError> {
    let rows = sqlx::query_as!(
        LeaderboardRow,
        r#"SELECT l.display_name, l.best_score, l.best_time_secs, l.recorded_at
           FROM leaderboard l
           JOIN users u ON u.id = l.user_id
           WHERE u.leaderboard_opt_in = 1
           ORDER BY
               CASE WHEN l.best_score IS NULL THEN 1 ELSE 0 END ASC,
               l.best_score DESC,
               CASE WHEN l.best_time_secs IS NULL THEN 1 ELSE 0 END ASC,
               l.best_time_secs ASC"#
    )
    .fetch_all(&pool)
    .await?;

    let entries: Result<Vec<LeaderboardEntry>, AppError> = rows
        .into_iter()
        .map(|r| -> Result<LeaderboardEntry, AppError> {
            let display_name = r
                .display_name
                .ok_or_else(|| AppError::Internal("missing display_name".into()))?;
            let recorded_at_str = r
                .recorded_at
                .ok_or_else(|| AppError::Internal("missing recorded_at".into()))?;
            let recorded_at = recorded_at_str
                .parse::<chrono::DateTime<Utc>>()
                .map_err(|e| AppError::Internal(format!("invalid recorded_at: {e}")))?;
            Ok(LeaderboardEntry {
                display_name,
                best_score: r.best_score.map(|v| v as i32),
                best_time_secs: r.best_time_secs.map(|v| v as u64),
                recorded_at,
            })
        })
        .collect();

    Ok(Json(entries?))
}

/// `DELETE /api/leaderboard/opt-in` — opt out, hiding the player's entry.
///
/// Sets `leaderboard_opt_in = 0` on the user row so the entry no longer
/// appears in `GET /api/leaderboard`. The leaderboard row itself is kept
/// so scores are preserved if the player opts back in later.
pub async fn opt_out(
    State(pool): State<SqlitePool>,
    user: AuthenticatedUser,
) -> Result<Json<serde_json::Value>, AppError> {
    sqlx::query!(
        "UPDATE users SET leaderboard_opt_in = 0 WHERE id = ?",
        user.user_id
    )
    .execute(&pool)
    .await?;

    Ok(Json(serde_json::json!({ "ok": true })))
}

/// `POST /api/leaderboard/opt-in` — opt in and upsert the player's entry.
///
/// Sets `leaderboard_opt_in = 1` on the user row and creates/updates the
/// leaderboard entry with the supplied display name.
pub async fn opt_in(
    State(pool): State<SqlitePool>,
    user: AuthenticatedUser,
    Json(body): Json<OptInRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    if body.display_name.trim().is_empty() {
        return Err(AppError::BadRequest("display_name must not be empty".into()));
    }

    // Mark the user as opted in.
    sqlx::query!(
        "UPDATE users SET leaderboard_opt_in = 1 WHERE id = ?",
        user.user_id
    )
    .execute(&pool)
    .await?;

    let now = Utc::now().to_rfc3339();

    // Upsert leaderboard row (preserve best_score / best_time if already present).
    sqlx::query!(
        r#"INSERT INTO leaderboard (user_id, display_name, recorded_at)
           VALUES (?, ?, ?)
           ON CONFLICT(user_id) DO UPDATE SET
               display_name = excluded.display_name,
               recorded_at  = excluded.recorded_at"#,
        user.user_id,
        body.display_name,
        now
    )
    .execute(&pool)
    .await?;

    Ok(Json(serde_json::json!({ "ok": true })))
}
