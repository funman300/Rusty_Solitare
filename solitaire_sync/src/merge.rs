//! Pure merge logic for sync payloads.
//!
//! All functions are free of I/O and side effects — safe to call from any
//! context including unit tests and the Bevy main thread.

use chrono::Utc;

use crate::{AchievementRecord, ConflictReport, PlayerProgress, StatsSnapshot, SyncPayload};
use crate::progress::level_for_xp;

/// Merge two [`SyncPayload`]s into a single authoritative result.
///
/// The merge strategy is additive and conflict-free for most fields:
/// - Counters: take the maximum (games_played, games_won, etc.)
/// - Best records: take the minimum for times, maximum for scores/xp
/// - Achievements: union by id, preserving the earliest `unlock_date`
/// - Cosmetic unlocks: union of both vectors
/// - Level: recomputed from merged `total_xp`
///
/// Fields that cannot be merged deterministically (e.g. diverged streak
/// counts) are recorded in [`ConflictReport`] entries returned alongside
/// the merged payload. Data is never silently discarded.
///
/// # Examples
/// ```
/// use solitaire_sync::{SyncPayload, StatsSnapshot, PlayerProgress, merge};
/// use uuid::Uuid;
///
/// let a = SyncPayload {
///     user_id: Uuid::nil(),
///     stats: StatsSnapshot { games_played: 5, ..Default::default() },
///     achievements: vec![],
///     progress: PlayerProgress::default(),
///     last_modified: chrono::Utc::now(),
/// };
/// let b = SyncPayload {
///     user_id: Uuid::nil(),
///     stats: StatsSnapshot { games_played: 3, ..Default::default() },
///     achievements: vec![],
///     progress: PlayerProgress::default(),
///     last_modified: chrono::Utc::now(),
/// };
/// let (merged, conflicts) = merge(&a, &b);
/// assert_eq!(merged.stats.games_played, 5);
/// assert!(conflicts.is_empty());
/// ```
pub fn merge(local: &SyncPayload, remote: &SyncPayload) -> (SyncPayload, Vec<ConflictReport>) {
    let mut conflicts = Vec::new();

    let stats = merge_stats(&local.stats, &remote.stats, &mut conflicts);
    let achievements = merge_achievements(&local.achievements, &remote.achievements);
    let progress = merge_progress(&local.progress, &remote.progress, &mut conflicts);

    let merged = SyncPayload {
        user_id: local.user_id,
        stats,
        achievements,
        progress,
        last_modified: Utc::now(),
    };

    (merged, conflicts)
}

// ---------------------------------------------------------------------------
// Stats
// ---------------------------------------------------------------------------

fn merge_stats(
    local: &StatsSnapshot,
    remote: &StatsSnapshot,
    conflicts: &mut Vec<ConflictReport>,
) -> StatsSnapshot {
    // win_streak_current cannot be merged deterministically — record conflict
    // but take the higher value as a best-effort resolution.
    if local.win_streak_current != remote.win_streak_current {
        conflicts.push(ConflictReport {
            field: "win_streak_current".to_string(),
            local_value: local.win_streak_current.to_string(),
            remote_value: remote.win_streak_current.to_string(),
        });
    }

    let merged_games_won = local.games_won.max(remote.games_won);
    let merged_games_played = local.games_played.max(remote.games_played);

    // Recompute average time from the merged totals. If no wins yet, keep 0.
    let avg_time_seconds = if merged_games_won == 0 {
        0
    } else {
        // Use whichever side has more wins to approximate total time, then blend.
        // We don't have total_time stored, so we reconstruct it from avg * count.
        let local_total = local.avg_time_seconds as u128 * local.games_won as u128;
        let remote_total = remote.avg_time_seconds as u128 * remote.games_won as u128;
        // Take max total time (conservative — avoids underestimating total play time).
        let best_total = local_total.max(remote_total);
        (best_total / merged_games_won as u128) as u64
    };

    StatsSnapshot {
        games_played: merged_games_played,
        games_won: merged_games_won,
        games_lost: local.games_lost.max(remote.games_lost),
        win_streak_current: local.win_streak_current.max(remote.win_streak_current),
        win_streak_best: local.win_streak_best.max(remote.win_streak_best),
        avg_time_seconds,
        fastest_win_seconds: local.fastest_win_seconds.min(remote.fastest_win_seconds),
        lifetime_score: local.lifetime_score.max(remote.lifetime_score),
        best_single_score: local.best_single_score.max(remote.best_single_score),
        draw_one_wins: local.draw_one_wins.max(remote.draw_one_wins),
        draw_three_wins: local.draw_three_wins.max(remote.draw_three_wins),
        last_modified: Utc::now(),
    }
}

// ---------------------------------------------------------------------------
// Achievements
// ---------------------------------------------------------------------------

/// Union of local and remote achievement records.
///
/// - Achievements never disappear from the merged set.
/// - If both sides have an achievement unlocked, the *earliest* `unlock_date`
///   is preserved.
/// - If only one side has an achievement unlocked, it is carried forward.
fn merge_achievements(
    local: &[AchievementRecord],
    remote: &[AchievementRecord],
) -> Vec<AchievementRecord> {
    use std::collections::HashMap;

    let mut map: HashMap<&str, AchievementRecord> = HashMap::new();

    // Insert all local records first.
    for rec in local {
        map.insert(rec.id.as_str(), rec.clone());
    }

    // Merge in remote records.
    for remote_rec in remote {
        match map.get_mut(remote_rec.id.as_str()) {
            Some(existing) => {
                // Merge: once unlocked, never lock again.
                if remote_rec.unlocked && !existing.unlocked {
                    // Remote is unlocked but local isn't — adopt remote unlock.
                    existing.unlocked = true;
                    existing.unlock_date = remote_rec.unlock_date;
                    existing.reward_granted = remote_rec.reward_granted;
                } else if remote_rec.unlocked && existing.unlocked {
                    // Both unlocked — keep the earlier date.
                    match (existing.unlock_date, remote_rec.unlock_date) {
                        (Some(local_dt), Some(remote_dt)) if remote_dt < local_dt => {
                            existing.unlock_date = Some(remote_dt);
                        }
                        _ => {}
                    }
                    // reward_granted: true if either side granted it.
                    existing.reward_granted = existing.reward_granted || remote_rec.reward_granted;
                }
                // If only local is unlocked — nothing changes.
            }
            None => {
                // Remote has an achievement that local doesn't know about.
                map.insert(remote_rec.id.as_str(), remote_rec.clone());
            }
        }
    }

    let mut result: Vec<AchievementRecord> = map.into_values().collect();
    result.sort_by(|a, b| a.id.cmp(&b.id));
    result
}

// ---------------------------------------------------------------------------
// Progress
// ---------------------------------------------------------------------------

fn merge_progress(
    local: &PlayerProgress,
    remote: &PlayerProgress,
    conflicts: &mut Vec<ConflictReport>,
) -> PlayerProgress {
    // daily_challenge_streak cannot be merged deterministically.
    if local.daily_challenge_streak != remote.daily_challenge_streak {
        conflicts.push(ConflictReport {
            field: "daily_challenge_streak".to_string(),
            local_value: local.daily_challenge_streak.to_string(),
            remote_value: remote.daily_challenge_streak.to_string(),
        });
    }

    let total_xp = local.total_xp.max(remote.total_xp);

    // Union cosmetic unlocks.
    let unlocked_card_backs = union_usize_vecs(&local.unlocked_card_backs, &remote.unlocked_card_backs);
    let unlocked_backgrounds =
        union_usize_vecs(&local.unlocked_backgrounds, &remote.unlocked_backgrounds);

    // Keep the most recently completed daily challenge date (latest).
    let daily_challenge_last_completed =
        match (local.daily_challenge_last_completed, remote.daily_challenge_last_completed) {
            (Some(l), Some(r)) => Some(l.max(r)),
            (Some(l), None) => Some(l),
            (None, Some(r)) => Some(r),
            (None, None) => None,
        };

    // Take the higher streak as a best-effort resolution.
    let daily_challenge_streak =
        local.daily_challenge_streak.max(remote.daily_challenge_streak);

    // weekly_goal_progress: use whichever side has the more recent ISO week key.
    let (weekly_goal_week_iso, weekly_goal_progress) =
        match (&local.weekly_goal_week_iso, &remote.weekly_goal_week_iso) {
            (Some(l), Some(r)) if r > l => {
                (remote.weekly_goal_week_iso.clone(), remote.weekly_goal_progress.clone())
            }
            (Some(_), Some(_)) => {
                (local.weekly_goal_week_iso.clone(), local.weekly_goal_progress.clone())
            }
            (Some(_), None) => {
                (local.weekly_goal_week_iso.clone(), local.weekly_goal_progress.clone())
            }
            (None, Some(_)) => {
                (remote.weekly_goal_week_iso.clone(), remote.weekly_goal_progress.clone())
            }
            (None, None) => (None, Default::default()),
        };

    // Challenge index: take the higher (further ahead in challenge progression).
    let challenge_index = local.challenge_index.max(remote.challenge_index);

    PlayerProgress {
        total_xp,
        level: level_for_xp(total_xp),
        daily_challenge_last_completed,
        daily_challenge_streak,
        weekly_goal_progress,
        weekly_goal_week_iso,
        unlocked_card_backs,
        unlocked_backgrounds,
        challenge_index,
        last_modified: Utc::now(),
    }
}

/// Returns the sorted union of two `Vec<usize>` slices with duplicates removed.
fn union_usize_vecs(a: &[usize], b: &[usize]) -> Vec<usize> {
    use std::collections::BTreeSet;
    let set: BTreeSet<usize> = a.iter().chain(b.iter()).copied().collect();
    set.into_iter().collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};
    use uuid::Uuid;

    use crate::{AchievementRecord, PlayerProgress, StatsSnapshot, SyncPayload};

    fn make_payload(stats: StatsSnapshot, achievements: Vec<AchievementRecord>, progress: PlayerProgress) -> SyncPayload {
        SyncPayload {
            user_id: Uuid::nil(),
            stats,
            achievements,
            progress,
            last_modified: Utc::now(),
        }
    }

    fn default_payload() -> SyncPayload {
        make_payload(StatsSnapshot::default(), vec![], PlayerProgress::default())
    }

    // -----------------------------------------------------------------------
    // Idempotency
    // -----------------------------------------------------------------------

    #[test]
    fn merge_is_idempotent_for_equal_payloads() {
        let mut a = default_payload();
        a.stats.games_played = 10;
        a.stats.games_won = 5;
        a.stats.fastest_win_seconds = 120;
        a.stats.lifetime_score = 5000;
        a.progress.total_xp = 2000;
        a.progress.unlocked_card_backs = vec![0, 1];

        let (merged, conflicts) = merge(&a, &a);

        assert_eq!(merged.stats.games_played, 10);
        assert_eq!(merged.stats.games_won, 5);
        assert_eq!(merged.stats.fastest_win_seconds, 120);
        assert_eq!(merged.stats.lifetime_score, 5000);
        assert_eq!(merged.progress.total_xp, 2000);
        assert_eq!(merged.progress.unlocked_card_backs, vec![0, 1]);
        // Identical payloads produce no conflicts.
        assert!(conflicts.is_empty());
    }

    // -----------------------------------------------------------------------
    // Stats merge
    // -----------------------------------------------------------------------

    #[test]
    fn stats_games_played_takes_max() {
        let mut local = default_payload();
        local.stats.games_played = 20;
        let mut remote = default_payload();
        remote.stats.games_played = 15;

        let (merged, _) = merge(&local, &remote);
        assert_eq!(merged.stats.games_played, 20);
    }

    #[test]
    fn stats_games_won_takes_max() {
        let mut local = default_payload();
        local.stats.games_won = 7;
        let mut remote = default_payload();
        remote.stats.games_won = 12;

        let (merged, _) = merge(&local, &remote);
        assert_eq!(merged.stats.games_won, 12);
    }

    #[test]
    fn stats_fastest_win_takes_min() {
        let mut local = default_payload();
        local.stats.fastest_win_seconds = 300;
        let mut remote = default_payload();
        remote.stats.fastest_win_seconds = 120;

        let (merged, _) = merge(&local, &remote);
        assert_eq!(merged.stats.fastest_win_seconds, 120);
    }

    #[test]
    fn stats_best_score_takes_max() {
        let mut local = default_payload();
        local.stats.best_single_score = 4000;
        let mut remote = default_payload();
        remote.stats.best_single_score = 6000;

        let (merged, _) = merge(&local, &remote);
        assert_eq!(merged.stats.best_single_score, 6000);
    }

    #[test]
    fn differing_win_streak_current_generates_conflict() {
        let mut local = default_payload();
        local.stats.win_streak_current = 3;
        let mut remote = default_payload();
        remote.stats.win_streak_current = 5;

        let (merged, conflicts) = merge(&local, &remote);
        assert_eq!(merged.stats.win_streak_current, 5);
        assert!(
            conflicts.iter().any(|c| c.field == "win_streak_current"),
            "expected conflict report for win_streak_current"
        );
    }

    #[test]
    fn identical_win_streak_current_produces_no_conflict() {
        let mut local = default_payload();
        local.stats.win_streak_current = 4;
        let mut remote = default_payload();
        remote.stats.win_streak_current = 4;

        let (_, conflicts) = merge(&local, &remote);
        assert!(
            !conflicts.iter().any(|c| c.field == "win_streak_current"),
            "no conflict expected for matching streaks"
        );
    }

    // -----------------------------------------------------------------------
    // Achievement merge
    // -----------------------------------------------------------------------

    #[test]
    fn achievements_are_never_removed() {
        let unlocked = {
            let mut r = AchievementRecord::locked("first_win");
            r.unlock(Utc::now());
            r
        };
        let local = make_payload(StatsSnapshot::default(), vec![unlocked.clone()], PlayerProgress::default());
        let remote = make_payload(StatsSnapshot::default(), vec![], PlayerProgress::default());

        let (merged, _) = merge(&local, &remote);
        assert!(
            merged.achievements.iter().any(|a| a.id == "first_win" && a.unlocked),
            "unlocked achievement must survive merge even if absent from remote"
        );
    }

    #[test]
    fn achievements_remote_unlock_propagates_to_local() {
        let locked = AchievementRecord::locked("century");
        let mut unlocked = AchievementRecord::locked("century");
        unlocked.unlock(Utc::now());

        let local = make_payload(StatsSnapshot::default(), vec![locked], PlayerProgress::default());
        let remote = make_payload(StatsSnapshot::default(), vec![unlocked.clone()], PlayerProgress::default());

        let (merged, _) = merge(&local, &remote);
        let ach = merged.achievements.iter().find(|a| a.id == "century").expect("must exist");
        assert!(ach.unlocked);
        assert_eq!(ach.unlock_date, unlocked.unlock_date);
    }

    #[test]
    fn achievements_earliest_unlock_date_wins_on_conflict() {
        let earlier = Utc::now() - Duration::hours(2);
        let later = Utc::now();

        let mut local_rec = AchievementRecord::locked("speed_demon");
        local_rec.unlock(later);
        let mut remote_rec = AchievementRecord::locked("speed_demon");
        remote_rec.unlock(earlier);

        let local = make_payload(StatsSnapshot::default(), vec![local_rec], PlayerProgress::default());
        let remote = make_payload(StatsSnapshot::default(), vec![remote_rec], PlayerProgress::default());

        let (merged, _) = merge(&local, &remote);
        let ach = merged.achievements.iter().find(|a| a.id == "speed_demon").expect("must exist");
        assert_eq!(ach.unlock_date, Some(earlier), "earlier date must win");
    }

    #[test]
    fn achievements_union_includes_both_sides() {
        let mut a1 = AchievementRecord::locked("first_win");
        a1.unlock(Utc::now());
        let mut a2 = AchievementRecord::locked("century");
        a2.unlock(Utc::now());

        let local = make_payload(StatsSnapshot::default(), vec![a1], PlayerProgress::default());
        let remote = make_payload(StatsSnapshot::default(), vec![a2], PlayerProgress::default());

        let (merged, _) = merge(&local, &remote);
        assert_eq!(merged.achievements.len(), 2);
    }

    // -----------------------------------------------------------------------
    // Progress merge
    // -----------------------------------------------------------------------

    #[test]
    fn progress_total_xp_takes_max() {
        let mut local = default_payload();
        local.progress.total_xp = 1500;
        local.progress.level = crate::progress::level_for_xp(1500);
        let mut remote = default_payload();
        remote.progress.total_xp = 2500;
        remote.progress.level = crate::progress::level_for_xp(2500);

        let (merged, _) = merge(&local, &remote);
        assert_eq!(merged.progress.total_xp, 2500);
        assert_eq!(merged.progress.level, crate::progress::level_for_xp(2500));
    }

    #[test]
    fn progress_unlocked_card_backs_are_union() {
        let mut local = default_payload();
        local.progress.unlocked_card_backs = vec![0, 1];
        let mut remote = default_payload();
        remote.progress.unlocked_card_backs = vec![0, 2];

        let (merged, _) = merge(&local, &remote);
        assert!(merged.progress.unlocked_card_backs.contains(&0));
        assert!(merged.progress.unlocked_card_backs.contains(&1));
        assert!(merged.progress.unlocked_card_backs.contains(&2));
    }

    #[test]
    fn progress_unlocked_backgrounds_are_union() {
        let mut local = default_payload();
        local.progress.unlocked_backgrounds = vec![0, 3];
        let mut remote = default_payload();
        remote.progress.unlocked_backgrounds = vec![0, 4];

        let (merged, _) = merge(&local, &remote);
        assert!(merged.progress.unlocked_backgrounds.contains(&3));
        assert!(merged.progress.unlocked_backgrounds.contains(&4));
    }

    #[test]
    fn differing_daily_challenge_streak_generates_conflict() {
        let mut local = default_payload();
        local.progress.daily_challenge_streak = 5;
        let mut remote = default_payload();
        remote.progress.daily_challenge_streak = 3;

        let (_, conflicts) = merge(&local, &remote);
        assert!(
            conflicts.iter().any(|c| c.field == "daily_challenge_streak"),
            "expected conflict for daily_challenge_streak"
        );
    }

    #[test]
    fn level_is_recomputed_from_merged_xp() {
        let mut local = default_payload();
        local.progress.total_xp = 4500; // level 9
        let mut remote = default_payload();
        remote.progress.total_xp = 5500; // level 10

        let (merged, _) = merge(&local, &remote);
        assert_eq!(merged.progress.total_xp, 5500);
        assert_eq!(merged.progress.level, crate::progress::level_for_xp(5500));
    }
}
