//! Winning-replay storage and retrieval.
//!
//! `POST /api/replays`        — upload a winning replay (auth required).
//! `GET  /api/replays/recent` — list the N most-recent replays across users.
//! `GET  /api/replays/:id`    — fetch a single replay's full JSON.
//!
//! The replay payload itself is opaque to the server — the desktop client
//! generates a `solitaire_data::Replay` and the web playback re-executes
//! the same atomic input list against a fresh `GameState`. The server
//! just persists, indexes, and serves the JSON; it does not validate the
//! semantics of the move list.
//!
//! Three columns are projected out of the replay JSON at insert time
//! (`final_score`, `time_seconds`, `recorded_at`) so list endpoints can
//! be served without scanning every blob.

use axum::{
    extract::{Path, Query, State},
    Json,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{error::AppError, middleware::AuthenticatedUser, AppState};

// ---------------------------------------------------------------------------
// Wire types
// ---------------------------------------------------------------------------

/// Subset of `Replay` fields the server needs to project out of the
/// uploaded JSON to populate the denormalised columns. Mirrors the
/// fields on `solitaire_data::Replay`; we don't depend on
/// `solitaire_data` here because the server crate must not pull in
/// the desktop client's transitive dependencies.
#[derive(Debug, Deserialize)]
struct ReplayHeader {
    seed: i64,
    draw_mode: String,
    mode: String,
    time_seconds: i64,
    final_score: i64,
    recorded_at: String,
}

/// Successful upload acknowledgement. The server-minted `id` is what
/// the client / web UI uses to link to `/replays/<id>`.
#[derive(Debug, Serialize)]
pub struct ReplayUploadResponse {
    /// UUID v4 minted server-side at insert time.
    pub id: String,
}

/// One row in the recent-replays list. Just the projection columns —
/// the full move list lives behind `GET /api/replays/:id`.
#[derive(Debug, Serialize)]
pub struct ReplaySummary {
    pub id: String,
    pub username: String,
    pub seed: i64,
    pub draw_mode: String,
    pub mode: String,
    pub time_seconds: i64,
    pub final_score: i64,
    pub recorded_at: String,
    pub received_at: String,
}

/// `GET /api/replays/recent?limit=N` — bound the result set so a
/// long-tail history doesn't ship megabytes per request.
#[derive(Debug, Deserialize)]
pub struct RecentQuery {
    pub limit: Option<u32>,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `POST /api/replays` — accept a winning replay JSON, persist it,
/// return the server-minted `id`. Auth required (the upload is
/// attributed to the authenticated user).
pub async fn upload(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Json(payload): Json<serde_json::Value>,
) -> Result<Json<ReplayUploadResponse>, AppError> {
    // Project the header fields the SQL columns need. The full payload
    // is stored verbatim — schema_version sits inside it and the
    // playback path is what enforces compatibility.
    let header: ReplayHeader = serde_json::from_value(payload.clone())
        .map_err(|e| AppError::BadRequest(format!("replay JSON missing fields: {e}")))?;

    let id = Uuid::new_v4().to_string();
    let received_at = Utc::now().to_rfc3339();
    let replay_json = serde_json::to_string(&payload)?;

    sqlx::query!(
        r#"INSERT INTO replays (
              id, user_id, seed, draw_mode, mode, time_seconds, final_score,
              recorded_at, received_at, replay_json
           ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"#,
        id,
        user.user_id,
        header.seed,
        header.draw_mode,
        header.mode,
        header.time_seconds,
        header.final_score,
        header.recorded_at,
        received_at,
        replay_json,
    )
    .execute(&state.pool)
    .await?;

    Ok(Json(ReplayUploadResponse { id }))
}

/// `GET /api/replays/recent` — list the N most-recent replays across
/// every user, newest first. Auth not required so the web UI can show
/// a public "latest wins" feed without a logged-in client.
pub async fn recent(
    State(state): State<AppState>,
    Query(q): Query<RecentQuery>,
) -> Result<Json<Vec<ReplaySummary>>, AppError> {
    // 50 is a sane upper bound so a `?limit=999999` request can't make
    // the server allocate megabytes. 20 is the default for a quick feed.
    let limit = q.limit.unwrap_or(20).min(50) as i64;

    let rows = sqlx::query!(
        r#"SELECT
              r.id              AS "id!: String",
              u.username        AS "username!: String",
              r.seed            AS "seed!: i64",
              r.draw_mode       AS "draw_mode!: String",
              r.mode            AS "mode!: String",
              r.time_seconds    AS "time_seconds!: i64",
              r.final_score     AS "final_score!: i64",
              r.recorded_at     AS "recorded_at!: String",
              r.received_at     AS "received_at!: String"
           FROM replays r
           JOIN users   u ON u.id = r.user_id
           ORDER BY r.received_at DESC
           LIMIT ?"#,
        limit,
    )
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(
        rows.into_iter()
            .map(|r| ReplaySummary {
                id: r.id,
                username: r.username,
                seed: r.seed,
                draw_mode: r.draw_mode,
                mode: r.mode,
                time_seconds: r.time_seconds,
                final_score: r.final_score,
                recorded_at: r.recorded_at,
                received_at: r.received_at,
            })
            .collect(),
    ))
}

/// `GET /api/replays/:id` — return the full replay JSON the desktop
/// client uploaded. Public; the web UI fetches this directly.
///
/// The server does not validate or transform the payload — what was
/// stored is what's returned. Schema-version compatibility is the
/// responsibility of the playback side (web UI), matching the
/// `schema_version` gate the desktop loader uses.
pub async fn get_by_id(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let row = sqlx::query!(
        "SELECT replay_json FROM replays WHERE id = ?",
        id,
    )
    .fetch_optional(&state.pool)
    .await?;

    let replay_json = row
        .ok_or_else(|| AppError::NotFound("replay not found".into()))?
        .replay_json;
    let value: serde_json::Value = serde_json::from_str(&replay_json)?;
    Ok(Json(value))
}
