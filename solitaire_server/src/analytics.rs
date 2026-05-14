//! Analytics ingest endpoint.
//!
//! `POST /api/analytics` — accept a batch of game-play events from an
//! opted-in client. No authentication required; the endpoint is public
//! so events can be captured before the player logs in.
//!
//! Each event is validated individually — a bad event is skipped rather
//! than rejecting the whole batch. Duplicate event IDs are silently
//! ignored via `INSERT OR IGNORE` so clients may safely retry a failed
//! batch without creating duplicate rows.

use axum::{extract::State, Json};
use chrono::Utc;
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::{error::AppError, AppState};

// ---------------------------------------------------------------------------
// Wire types
// ---------------------------------------------------------------------------

/// Batch of events from a single client session.
#[derive(Debug, Deserialize)]
pub struct AnalyticsBatch {
    /// UUID v4 generated once per app launch by the client.
    pub session_id: String,
    /// Optional username — populated when the player is logged in.
    pub user_id: Option<String>,
    /// Events to ingest. Batches with more than 50 events are rejected.
    pub events: Vec<AnalyticsEvent>,
}

/// One game-play event within a batch.
#[derive(Debug, Deserialize)]
pub struct AnalyticsEvent {
    /// UUID v4 minted client-side. Used as idempotency key.
    pub id: String,
    /// Lowercase snake-case type, e.g. `"game_won"`. Max 64 chars.
    pub event_type: String,
    /// Event-specific JSON payload.
    pub payload: Value,
    /// ISO-8601 timestamp from the client clock.
    pub client_time: String,
}

// Validated, ready-to-insert form of an event.
struct ValidEvent {
    id: String,
    event_type: String,
    payload_json: String,
    client_time: String,
}

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

/// `POST /api/analytics` — ingest a batch of analytics events.
pub async fn ingest(
    State(state): State<AppState>,
    Json(batch): Json<AnalyticsBatch>,
) -> Result<Json<serde_json::Value>, AppError> {
    if batch.events.len() > 50 {
        return Err(AppError::BadRequest("batch may contain at most 50 events".into()));
    }

    let now = Utc::now();
    let received_at = now.to_rfc3339();
    // Reject events whose client_time claims to be more than 24 h in the future
    // (clock skew protection; stale events from the past are fine).
    let future_cutoff = now + chrono::Duration::hours(24);

    let valid: Vec<ValidEvent> = batch
        .events
        .iter()
        .filter_map(|e| {
            // Idempotency key must be a valid UUID.
            if Uuid::parse_str(&e.id).is_err() {
                return None;
            }
            // event_type: lowercase letters and underscores only, 1–64 chars.
            if e.event_type.is_empty()
                || e.event_type.len() > 64
                || !e.event_type.chars().all(|c| c.is_ascii_lowercase() || c == '_')
            {
                return None;
            }
            // client_time must parse and not be too far in the future.
            let parsed = e.client_time.parse::<chrono::DateTime<Utc>>().ok()?;
            if parsed > future_cutoff {
                return None;
            }
            let payload_json = serde_json::to_string(&e.payload).ok()?;
            Some(ValidEvent {
                id: e.id.clone(),
                event_type: e.event_type.clone(),
                payload_json,
                client_time: parsed.to_rfc3339(),
            })
        })
        .collect();

    if valid.is_empty() {
        return Ok(Json(serde_json::json!({ "ok": true, "accepted": 0 })));
    }

    let mut tx = state.pool.begin().await?;
    for ev in &valid {
        sqlx::query!(
            r#"INSERT OR IGNORE INTO analytics_events
               (id, user_id, session_id, event_type, payload, client_time, received_at)
               VALUES (?, ?, ?, ?, ?, ?, ?)"#,
            ev.id,
            batch.user_id,
            batch.session_id,
            ev.event_type,
            ev.payload_json,
            ev.client_time,
            received_at,
        )
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;

    let accepted = valid.len() as i64;
    Ok(Json(serde_json::json!({ "ok": true, "accepted": accepted })))
}
