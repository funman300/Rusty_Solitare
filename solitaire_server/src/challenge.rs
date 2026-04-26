//! Daily challenge endpoint.
//!
//! `GET /api/daily-challenge` — returns the challenge for today's date.
//!
//! The seed is deterministic (same for all players worldwide) and is
//! generated on first request for that date, then stored in the database
//! so subsequent calls return the same value.

use axum::{extract::State, Json};
use chrono::Utc;
use sqlx::SqlitePool;

use solitaire_sync::ChallengeGoal;

use crate::error::AppError;

// ---------------------------------------------------------------------------
// Seed generation
// ---------------------------------------------------------------------------

/// Compute a deterministic seed from a date string such as `"2026-04-26"`.
///
/// Uses a simple polynomial rolling hash over the UTF-8 bytes of the string.
/// The computation is identical across all server instances and all clients
/// that implement the same algorithm.
pub fn hash_date_to_u64(date: &str) -> u64 {
    date.bytes()
        .fold(0u64, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u64))
}

/// Generate a [`ChallengeGoal`] from a seed and date.
///
/// The goal type and parameters are derived deterministically from the seed
/// so all players face exactly the same challenge on the same day.
fn generate_goal(date: &str, seed: u64) -> ChallengeGoal {
    // Pick a goal variant based on seed modulo number-of-variants.
    // Three variants cycle through: timed, high-score, and open.
    match seed % 3 {
        0 => ChallengeGoal {
            date: date.to_string(),
            seed,
            description: "Win in under 5 minutes".to_string(),
            target_score: None,
            max_time_secs: Some(300),
        },
        1 => ChallengeGoal {
            date: date.to_string(),
            seed,
            description: "Reach a score of 4 000 or more".to_string(),
            target_score: Some(4_000),
            max_time_secs: None,
        },
        _ => ChallengeGoal {
            date: date.to_string(),
            seed,
            description: "Win today's deal".to_string(),
            target_score: None,
            max_time_secs: None,
        },
    }
}

// ---------------------------------------------------------------------------
// Database row helper
// ---------------------------------------------------------------------------

struct ChallengeRow {
    goal_json: Option<String>,
}

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

/// `GET /api/daily-challenge` — no auth required.
///
/// Looks up today's challenge in the database. If none exists yet, generates
/// one deterministically and stores it before returning.
pub async fn daily_challenge(
    State(pool): State<SqlitePool>,
) -> Result<Json<ChallengeGoal>, AppError> {
    let today = Utc::now().format("%Y-%m-%d").to_string();

    // Try to load an existing row.
    let row = sqlx::query_as!(
        ChallengeRow,
        "SELECT goal_json FROM daily_challenges WHERE date = ?",
        today
    )
    .fetch_optional(&pool)
    .await?;

    if let Some(r) = row {
        let json = r.goal_json.ok_or_else(|| AppError::Internal("missing goal_json".into()))?;
        let goal: ChallengeGoal = serde_json::from_str(&json)?;
        return Ok(Json(goal));
    }

    // No row yet — generate and store.
    let seed = hash_date_to_u64(&today);
    let goal = generate_goal(&today, seed);
    let goal_json = serde_json::to_string(&goal)?;
    let seed_i64 = seed as i64;

    sqlx::query!(
        "INSERT OR IGNORE INTO daily_challenges (date, seed, goal_json) VALUES (?, ?, ?)",
        today,
        seed_i64,
        goal_json
    )
    .execute(&pool)
    .await?;

    Ok(Json(goal))
}
