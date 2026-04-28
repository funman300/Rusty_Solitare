//! Daily challenge endpoint.
//!
//! `GET /api/daily-challenge` — returns the challenge for today's date.
//!
//! The seed is deterministic (same for all players worldwide) and is
//! generated on first request for that date, then stored in the database
//! so subsequent calls return the same value.

use axum::{extract::State, Json};
use chrono::Utc;

use solitaire_sync::ChallengeGoal;

use crate::{error::AppError, AppState};

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
    // Six variants give a fortnight of variety before any repeat.
    match seed % 6 {
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
        2 => ChallengeGoal {
            date: date.to_string(),
            seed,
            description: "Win in under 3 minutes".to_string(),
            target_score: None,
            max_time_secs: Some(180),
        },
        3 => ChallengeGoal {
            date: date.to_string(),
            seed,
            description: "Reach a score of 5 000 or more".to_string(),
            target_score: Some(5_000),
            max_time_secs: None,
        },
        4 => ChallengeGoal {
            date: date.to_string(),
            seed,
            description: "Win in under 8 minutes".to_string(),
            target_score: None,
            max_time_secs: Some(480),
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
///
/// The `INSERT OR IGNORE` followed by a re-SELECT ensures that concurrent
/// requests racing to create today's row all return the same persisted value
/// rather than each returning their own locally-generated copy.
pub async fn daily_challenge(
    State(state): State<AppState>,
) -> Result<Json<ChallengeGoal>, AppError> {
    let today = Utc::now().format("%Y-%m-%d").to_string();

    // Try to load an existing row first (fast path — no generation needed).
    let row = sqlx::query_as!(
        ChallengeRow,
        "SELECT goal_json FROM daily_challenges WHERE date = ?",
        today
    )
    .fetch_optional(&state.pool)
    .await?;

    if let Some(r) = row {
        let json = r.goal_json.ok_or_else(|| AppError::Internal("missing goal_json".into()))?;
        let goal: ChallengeGoal = serde_json::from_str(&json)?;
        return Ok(Json(goal));
    }

    // No row yet — generate the goal locally and attempt to store it.
    // `INSERT OR IGNORE` means a concurrent request that wins the race will
    // silently ignore our insert.  We then re-SELECT to ensure both requests
    // return the same persisted row regardless of which one won.
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
    .execute(&state.pool)
    .await?;

    // Re-SELECT to return exactly what is stored — handles the race where
    // another request inserted a row between our initial SELECT and INSERT.
    let stored = sqlx::query_as!(
        ChallengeRow,
        "SELECT goal_json FROM daily_challenges WHERE date = ?",
        today
    )
    .fetch_one(&state.pool)
    .await?;

    let stored_json = stored.goal_json.ok_or_else(|| AppError::Internal("missing goal_json after insert".into()))?;
    let stored_goal: ChallengeGoal = serde_json::from_str(&stored_json)?;
    Ok(Json(stored_goal))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_date_is_deterministic() {
        let date = "2026-04-26";
        assert_eq!(hash_date_to_u64(date), hash_date_to_u64(date));
    }

    #[test]
    fn hash_date_differs_across_adjacent_days() {
        assert_ne!(hash_date_to_u64("2026-04-26"), hash_date_to_u64("2026-04-27"));
        assert_ne!(hash_date_to_u64("2026-04-26"), hash_date_to_u64("2026-04-25"));
    }

    #[test]
    fn hash_date_differs_across_years() {
        assert_ne!(hash_date_to_u64("2026-01-01"), hash_date_to_u64("2027-01-01"));
    }

    #[test]
    fn hash_date_is_nonzero_for_real_dates() {
        // Zero would be pathological — every date must produce a non-zero seed
        // so the RNG initialises properly.
        assert_ne!(hash_date_to_u64("2026-04-26"), 0);
        assert_ne!(hash_date_to_u64("2026-01-01"), 0);
    }

    #[test]
    fn generate_goal_covers_all_six_variants() {
        // The six variants are selected by seed % 6. Verify each branch
        // produces a non-empty description and a non-empty date string.
        for variant_idx in 0u64..6 {
            let goal = generate_goal("2026-04-26", variant_idx);
            assert_eq!(goal.date, "2026-04-26");
            assert!(!goal.description.is_empty());
            // seed field must match the passed-in seed.
            assert_eq!(goal.seed, variant_idx);
        }
    }

    #[test]
    fn generate_goal_time_and_score_variants_are_set_correctly() {
        // Variant 0: max_time_secs = 300, no score.
        let g = generate_goal("2026-04-26", 0);
        assert_eq!(g.max_time_secs, Some(300));
        assert!(g.target_score.is_none());

        // Variant 1: target_score = 4000, no time.
        let g = generate_goal("2026-04-26", 1);
        assert_eq!(g.target_score, Some(4_000));
        assert!(g.max_time_secs.is_none());

        // Variant 5: fallback — no time, no score (just win).
        let g = generate_goal("2026-04-26", 5);
        assert!(g.target_score.is_none());
        assert!(g.max_time_secs.is_none());
    }

    #[test]
    fn generate_goal_all_variants_have_sane_ranges() {
        for variant_idx in 0u64..6 {
            let g = generate_goal("2026-04-26", variant_idx);
            assert!(!g.description.is_empty(), "variant {variant_idx}: description must not be empty");
            if let Some(t) = g.max_time_secs {
                assert!(
                    (60..=3600).contains(&t),
                    "variant {variant_idx}: max_time_secs {t} outside [60, 3600]"
                );
            }
            if let Some(s) = g.target_score {
                assert!(
                    (1_000..=10_000).contains(&s),
                    "variant {variant_idx}: target_score {s} outside [1000, 10000]"
                );
            }
        }
    }
}
