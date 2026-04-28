//! Leaderboard endpoints.
//!
//! `GET  /api/leaderboard`       — list all opted-in entries (requires auth).
//! `POST /api/leaderboard/opt-in` — opt in and set / update display name.

use axum::{extract::State, Json};
use chrono::Utc;
use serde::Deserialize;

use solitaire_sync::LeaderboardEntry;

use crate::{error::AppError, middleware::AuthenticatedUser, AppState};

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
    State(state): State<AppState>,
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
    .fetch_all(&state.pool)
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
    State(state): State<AppState>,
    user: AuthenticatedUser,
) -> Result<Json<serde_json::Value>, AppError> {
    sqlx::query!(
        "UPDATE users SET leaderboard_opt_in = 0 WHERE id = ?",
        user.user_id
    )
    .execute(&state.pool)
    .await?;

    Ok(Json(serde_json::json!({ "ok": true })))
}

/// Maximum allowed character count for a leaderboard display name (32 chars).
/// Keeps names readable in the leaderboard UI while allowing reasonable creativity.
const DISPLAY_NAME_MAX: usize = 32;

/// `POST /api/leaderboard/opt-in` — opt in and upsert the player's entry.
///
/// Sets `leaderboard_opt_in = 1` on the user row and creates/updates the
/// leaderboard entry with the supplied display name.
pub async fn opt_in(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Json(body): Json<OptInRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let display_name = body.display_name.trim();
    if display_name.is_empty() {
        return Err(AppError::BadRequest("display_name must not be empty".into()));
    }
    if display_name.chars().count() > DISPLAY_NAME_MAX {
        return Err(AppError::BadRequest(format!(
            "display_name must be at most {DISPLAY_NAME_MAX} characters"
        )));
    }
    let display_name = display_name.to_string();

    // Mark the user as opted in.
    sqlx::query!(
        "UPDATE users SET leaderboard_opt_in = 1 WHERE id = ?",
        user.user_id
    )
    .execute(&state.pool)
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
        display_name,
        now
    )
    .execute(&state.pool)
    .await?;

    Ok(Json(serde_json::json!({ "ok": true })))
}

// ---------------------------------------------------------------------------
// Tests — data shape and display-name logic; no database required
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use solitaire_sync::LeaderboardEntry;

    /// Helper that constructs a `LeaderboardEntry` with the given display name
    /// and best score. `best_time_secs` is left as `None`.
    fn entry(display_name: &str, best_score: Option<i32>) -> LeaderboardEntry {
        LeaderboardEntry {
            display_name: display_name.to_string(),
            best_score,
            best_time_secs: None,
            recorded_at: Utc::now(),
        }
    }

    // -----------------------------------------------------------------------
    // 1. A LeaderboardEntry always carries a non-empty display_name.
    // -----------------------------------------------------------------------

    #[test]
    fn leaderboard_entry_has_display_name() {
        let e = entry("Alice", Some(4_500));
        assert!(
            !e.display_name.is_empty(),
            "display_name must not be empty for a valid leaderboard entry"
        );
        assert_eq!(e.display_name, "Alice");
    }

    // -----------------------------------------------------------------------
    // 2. A Vec of entries sorts by best_score descending (matching the SQL
    //    ORDER BY used in get_leaderboard).
    // -----------------------------------------------------------------------

    #[test]
    fn leaderboard_entries_sorted_by_score_descending() {
        let mut entries = vec![
            entry("Charlie", Some(1_200)),
            entry("Alice",   Some(8_000)),
            entry("Bob",     Some(3_500)),
            entry("Dave",    None),        // no score — should rank last
        ];

        // Mirrors the SQL sort:
        //   CASE WHEN best_score IS NULL THEN 1 ELSE 0 END ASC,
        //   best_score DESC
        entries.sort_by(|a, b| {
            let a_null = a.best_score.is_none() as u8;
            let b_null = b.best_score.is_none() as u8;
            a_null
                .cmp(&b_null)
                .then_with(|| b.best_score.cmp(&a.best_score))
        });

        // Scored entries first, in descending order.
        assert_eq!(entries[0].display_name, "Alice",   "highest scorer must be first");
        assert_eq!(entries[1].display_name, "Bob",     "second-highest scorer must be second");
        assert_eq!(entries[2].display_name, "Charlie", "lowest scorer must be third");
        // Null-score entry sinks to the bottom.
        assert_eq!(entries[3].display_name, "Dave",    "entry with no score must rank last");
    }
}
