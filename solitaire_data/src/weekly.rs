//! Weekly goal definitions and helpers.
//!
//! Goals reset every ISO week. Engine evaluates them on `GameWonEvent` and
//! increments matching counters in `PlayerProgress::weekly_goal_progress`.

use chrono::{Datelike, NaiveDate};

/// XP awarded each time a weekly goal is just completed.
pub const WEEKLY_GOAL_XP: u64 = 75;

/// What kind of game outcome counts as progress toward this goal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WeeklyGoalKind {
    /// Any win counts.
    WinGame,
    /// A win without using `undo` counts.
    WinWithoutUndo,
    /// A win in strictly fewer than `seconds` seconds counts.
    WinUnder { seconds: u64 },
}

/// Static metadata for a single weekly goal.
#[derive(Debug, Clone, Copy)]
pub struct WeeklyGoalDef {
    pub id: &'static str,
    pub description: &'static str,
    pub target: u32,
    pub kind: WeeklyGoalKind,
}

/// Per-event facts a goal needs to decide whether it matched.
#[derive(Debug, Clone, Copy)]
pub struct WeeklyGoalContext {
    pub time_seconds: u64,
    pub used_undo: bool,
}

impl WeeklyGoalDef {
    /// Returns `true` if this win event counts as one tick of progress
    /// toward this goal.
    pub fn matches(&self, ctx: &WeeklyGoalContext) -> bool {
        match self.kind {
            WeeklyGoalKind::WinGame => true,
            WeeklyGoalKind::WinWithoutUndo => !ctx.used_undo,
            WeeklyGoalKind::WinUnder { seconds } => ctx.time_seconds < seconds,
        }
    }
}

/// All currently-active weekly goals.
pub const WEEKLY_GOALS: &[WeeklyGoalDef] = &[
    WeeklyGoalDef {
        id: "weekly_5_wins",
        description: "Win 5 games this week",
        target: 5,
        kind: WeeklyGoalKind::WinGame,
    },
    WeeklyGoalDef {
        id: "weekly_3_no_undo",
        description: "Win 3 games without undo this week",
        target: 3,
        kind: WeeklyGoalKind::WinWithoutUndo,
    },
    WeeklyGoalDef {
        id: "weekly_3_fast",
        description: "Win 3 games in under 3 minutes this week",
        target: 3,
        kind: WeeklyGoalKind::WinUnder { seconds: 180 },
    },
];

/// Stable identifier for the ISO week containing `date`, e.g. `"2026-W17"`.
/// Same string for every player worldwide on the same calendar week.
pub fn current_iso_week_key(date: NaiveDate) -> String {
    let iso = date.iso_week();
    format!("{}-W{:02}", iso.year(), iso.week())
}

/// Look up a weekly-goal definition by id.
pub fn weekly_goal_by_id(id: &str) -> Option<&'static WeeklyGoalDef> {
    WEEKLY_GOALS.iter().find(|g| g.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(time: u64, undo: bool) -> WeeklyGoalContext {
        WeeklyGoalContext {
            time_seconds: time,
            used_undo: undo,
        }
    }

    #[test]
    fn all_goal_ids_are_unique() {
        let mut ids: Vec<&str> = WEEKLY_GOALS.iter().map(|g| g.id).collect();
        ids.sort();
        let len = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), len);
    }

    #[test]
    fn win_game_always_matches() {
        let g = weekly_goal_by_id("weekly_5_wins").unwrap();
        assert!(g.matches(&ctx(60, false)));
        assert!(g.matches(&ctx(99999, true)));
    }

    #[test]
    fn no_undo_only_matches_clean_wins() {
        let g = weekly_goal_by_id("weekly_3_no_undo").unwrap();
        assert!(g.matches(&ctx(120, false)));
        assert!(!g.matches(&ctx(120, true)));
    }

    #[test]
    fn fast_only_matches_under_3_minutes() {
        let g = weekly_goal_by_id("weekly_3_fast").unwrap();
        assert!(g.matches(&ctx(60, true)));
        assert!(g.matches(&ctx(179, true)));
        assert!(!g.matches(&ctx(180, true)));
        assert!(!g.matches(&ctx(300, false)));
    }

    #[test]
    fn iso_week_key_is_stable_within_a_week() {
        let monday = NaiveDate::from_ymd_opt(2026, 4, 20).unwrap(); // 2026-W17 Mon
        let sunday = NaiveDate::from_ymd_opt(2026, 4, 26).unwrap(); // 2026-W17 Sun
        assert_eq!(current_iso_week_key(monday), current_iso_week_key(sunday));
    }

    #[test]
    fn iso_week_key_differs_across_weeks() {
        let w17 = NaiveDate::from_ymd_opt(2026, 4, 20).unwrap();
        let w18 = NaiveDate::from_ymd_opt(2026, 4, 27).unwrap();
        assert_ne!(current_iso_week_key(w17), current_iso_week_key(w18));
    }

    #[test]
    fn iso_week_key_format_includes_year_and_week() {
        let d = NaiveDate::from_ymd_opt(2026, 4, 20).unwrap();
        let key = current_iso_week_key(d);
        assert!(key.starts_with("2026-W"));
        assert_eq!(key.len(), 8);
    }
}
