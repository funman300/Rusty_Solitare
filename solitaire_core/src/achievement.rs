//! Static achievement definitions + evaluation.
//!
//! `solitaire_core` cannot import from `solitaire_data`, so conditions are
//! not given `StatsSnapshot` directly — the engine packages the relevant
//! stats fields into an [`AchievementContext`] at evaluation time.
//!
//! Evaluation is called once per [`GameWonEvent`] in the engine: the engine
//! walks `ALL_ACHIEVEMENTS`, evaluates each `condition`, and emits an
//! unlock event for any `AchievementDef` whose record is not yet unlocked.

/// Fields needed by achievement conditions. Constructed by the engine from
/// `StatsSnapshot`, the final `GameState`, and wall-clock time.
#[derive(Debug, Clone)]
pub struct AchievementContext {
    // Stats (after this win has been recorded).
    pub games_played: u32,
    pub games_won: u32,
    pub win_streak_current: u32,
    pub best_single_score: u32,
    pub lifetime_score: u64,
    pub draw_three_wins: u32,

    // Progression.
    /// Current daily-challenge completion streak (consecutive days).
    pub daily_challenge_streak: u32,

    // Last-win facts (GameWonEvent + GameState at win time).
    pub last_win_score: i32,
    pub last_win_time_seconds: u64,
    /// `true` if `undo()` was called at least once during the won game.
    pub last_win_used_undo: bool,

    /// Local hour (0–23) at the time of win. `None` if unknown.
    pub wall_clock_hour: Option<u32>,

    /// Number of times waste was recycled back to stock during the won game.
    pub last_win_recycle_count: u32,
    /// `true` if the game was played in Zen mode.
    pub last_win_is_zen: bool,
}

/// Reward granted when an achievement is first unlocked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reward {
    /// Unlocks a card-back design at the given index (0 is always unlocked).
    CardBack(usize),
    /// Unlocks a background design at the given index (0 is always unlocked).
    Background(usize),
    /// Awards bonus XP on top of the standard win XP.
    BonusXp(u64),
    /// A visual badge — no gameplay effect.
    Badge,
}

/// A single achievement's static metadata + unlock condition.
#[derive(Debug, Clone, Copy)]
pub struct AchievementDef {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    /// Hidden from the achievements screen until unlocked.
    pub secret: bool,
    /// Reward granted on first unlock. `None` for cosmetic-only recognition.
    pub reward: Option<Reward>,
    pub condition: fn(&AchievementContext) -> bool,
}

impl AchievementDef {
    pub fn is_unlocked_by(&self, ctx: &AchievementContext) -> bool {
        (self.condition)(ctx)
    }
}

// ---------------------------------------------------------------------------
// Condition predicates
// ---------------------------------------------------------------------------

fn first_win(c: &AchievementContext) -> bool {
    c.games_won >= 1
}
fn on_a_roll(c: &AchievementContext) -> bool {
    c.win_streak_current >= 3
}
fn unstoppable(c: &AchievementContext) -> bool {
    c.win_streak_current >= 10
}
fn century(c: &AchievementContext) -> bool {
    c.games_played >= 100
}
fn veteran(c: &AchievementContext) -> bool {
    c.games_played >= 500
}
fn speed_demon(c: &AchievementContext) -> bool {
    c.last_win_time_seconds < 180
}
fn lightning(c: &AchievementContext) -> bool {
    c.last_win_time_seconds < 90
}
fn high_scorer(c: &AchievementContext) -> bool {
    c.best_single_score >= 5_000
}
fn point_machine(c: &AchievementContext) -> bool {
    c.lifetime_score >= 50_000
}
fn no_undo(c: &AchievementContext) -> bool {
    !c.last_win_used_undo
}
fn draw_three_master(c: &AchievementContext) -> bool {
    c.draw_three_wins >= 10
}
fn night_owl(c: &AchievementContext) -> bool {
    // Late-night session: 22:00–02:59 local time.
    matches!(c.wall_clock_hour, Some(h) if !(3..22).contains(&h))
}
fn early_bird(c: &AchievementContext) -> bool {
    // Early-morning session: 05:00–06:59 local time.
    matches!(c.wall_clock_hour, Some(h) if (5..7).contains(&h))
}
fn speed_and_skill(c: &AchievementContext) -> bool {
    c.last_win_time_seconds < 90 && !c.last_win_used_undo
}
fn daily_devotee(c: &AchievementContext) -> bool {
    c.daily_challenge_streak >= 7
}
fn perfectionist(c: &AchievementContext) -> bool {
    !c.last_win_used_undo && c.last_win_score >= 5_000
}
fn comeback(c: &AchievementContext) -> bool {
    c.last_win_recycle_count >= 3
}
fn zen_winner(c: &AchievementContext) -> bool {
    c.last_win_is_zen
}

/// All currently-evaluable achievements. Order is stable so persistence files
/// remain readable across versions (new achievements append).
pub const ALL_ACHIEVEMENTS: &[AchievementDef] = &[
    AchievementDef {
        id: "first_win",
        name: "First Win",
        description: "Win your first game",
        secret: false,
        reward: None,
        condition: first_win,
    },
    AchievementDef {
        id: "on_a_roll",
        name: "On a Roll",
        description: "Win 3 games in a row",
        secret: false,
        reward: Some(Reward::CardBack(1)),
        condition: on_a_roll,
    },
    AchievementDef {
        id: "unstoppable",
        name: "Unstoppable",
        description: "Win 10 games in a row",
        secret: false,
        reward: Some(Reward::Background(1)),
        condition: unstoppable,
    },
    AchievementDef {
        id: "century",
        name: "Century",
        description: "Play 100 games",
        secret: false,
        reward: None,
        condition: century,
    },
    AchievementDef {
        id: "veteran",
        name: "Veteran",
        description: "Play 500 games",
        secret: false,
        reward: Some(Reward::Badge),
        condition: veteran,
    },
    AchievementDef {
        id: "speed_demon",
        name: "Speed Demon",
        description: "Win in under 3 minutes",
        secret: false,
        reward: None,
        condition: speed_demon,
    },
    AchievementDef {
        id: "lightning",
        name: "Lightning",
        description: "Win in under 90 seconds",
        secret: false,
        reward: Some(Reward::CardBack(2)),
        condition: lightning,
    },
    AchievementDef {
        id: "high_scorer",
        name: "High Scorer",
        description: "Score at least 5,000 in one game",
        secret: false,
        reward: None,
        condition: high_scorer,
    },
    AchievementDef {
        id: "point_machine",
        name: "Point Machine",
        description: "Accumulate 50,000 lifetime points",
        secret: false,
        reward: Some(Reward::Background(2)),
        condition: point_machine,
    },
    AchievementDef {
        id: "no_undo",
        name: "No Undo",
        description: "Win a game without using undo",
        secret: false,
        reward: Some(Reward::BonusXp(25)),
        condition: no_undo,
    },
    AchievementDef {
        id: "draw_three_master",
        name: "Draw 3 Master",
        description: "Win 10 games in Draw 3 mode",
        secret: false,
        reward: Some(Reward::CardBack(3)),
        condition: draw_three_master,
    },
    AchievementDef {
        id: "night_owl",
        name: "Night Owl",
        description: "Win a game between 10pm and 3am",
        secret: false,
        reward: None,
        condition: night_owl,
    },
    AchievementDef {
        id: "early_bird",
        name: "Early Bird",
        description: "Win a game between 5am and 7am",
        secret: false,
        reward: None,
        condition: early_bird,
    },
    AchievementDef {
        id: "speed_and_skill",
        name: "???",
        description: "A secret achievement",
        secret: true,
        reward: Some(Reward::CardBack(4)),
        condition: speed_and_skill,
    },
    AchievementDef {
        id: "daily_devotee",
        name: "Daily Devotee",
        description: "Complete the daily challenge 7 days in a row",
        secret: false,
        reward: Some(Reward::Background(3)),
        condition: daily_devotee,
    },
    AchievementDef {
        id: "perfectionist",
        name: "Perfectionist",
        description: "Win without undo and score at least 5,000",
        secret: false,
        reward: Some(Reward::Badge),
        condition: perfectionist,
    },
    AchievementDef {
        id: "comeback",
        name: "???",
        description: "A secret achievement",
        secret: true,
        reward: Some(Reward::Background(4)),
        condition: comeback,
    },
    AchievementDef {
        id: "zen_winner",
        name: "???",
        description: "A secret achievement",
        secret: true,
        reward: Some(Reward::Badge),
        condition: zen_winner,
    },
];

/// Return every `AchievementDef` whose condition is satisfied by `ctx`.
pub fn check_achievements(ctx: &AchievementContext) -> Vec<&'static AchievementDef> {
    ALL_ACHIEVEMENTS
        .iter()
        .filter(|d| d.is_unlocked_by(ctx))
        .collect()
}

/// Look up an achievement definition by ID.
pub fn achievement_by_id(id: &str) -> Option<&'static AchievementDef> {
    ALL_ACHIEVEMENTS.iter().find(|d| d.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> AchievementContext {
        AchievementContext {
            games_played: 0,
            games_won: 0,
            win_streak_current: 0,
            best_single_score: 0,
            lifetime_score: 0,
            draw_three_wins: 0,
            daily_challenge_streak: 0,
            last_win_score: 0,
            last_win_time_seconds: u64::MAX,
            last_win_used_undo: true,
            wall_clock_hour: None,
            last_win_recycle_count: 0,
            last_win_is_zen: false,
        }
    }

    #[test]
    fn all_ids_are_unique() {
        let mut ids: Vec<&str> = ALL_ACHIEVEMENTS.iter().map(|d| d.id).collect();
        ids.sort();
        let len = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), len, "duplicate achievement ID in ALL_ACHIEVEMENTS");
    }

    #[test]
    fn no_achievements_unlocked_at_default() {
        let c = ctx();
        assert!(check_achievements(&c).is_empty());
    }

    #[test]
    fn first_win_unlocks_on_first_won_game() {
        let mut c = ctx();
        c.games_won = 1;
        let ids: Vec<&str> = check_achievements(&c).iter().map(|d| d.id).collect();
        assert!(ids.contains(&"first_win"));
    }

    #[test]
    fn lightning_requires_under_90_seconds() {
        let mut c = ctx();
        c.games_won = 1;
        c.last_win_time_seconds = 89;
        let ids: Vec<&str> = check_achievements(&c).iter().map(|d| d.id).collect();
        assert!(ids.contains(&"lightning"));
        assert!(ids.contains(&"speed_demon"));

        c.last_win_time_seconds = 90;
        let ids: Vec<&str> = check_achievements(&c).iter().map(|d| d.id).collect();
        assert!(!ids.contains(&"lightning"));
        assert!(ids.contains(&"speed_demon"));
    }

    #[test]
    fn no_undo_requires_clean_win() {
        let mut c = ctx();
        c.games_won = 1;
        c.last_win_used_undo = false;
        let ids: Vec<&str> = check_achievements(&c).iter().map(|d| d.id).collect();
        assert!(ids.contains(&"no_undo"));

        c.last_win_used_undo = true;
        let ids: Vec<&str> = check_achievements(&c).iter().map(|d| d.id).collect();
        assert!(!ids.contains(&"no_undo"));
    }

    #[test]
    fn secret_speed_and_skill_requires_both_clean_and_fast() {
        let mut c = ctx();
        c.games_won = 1;
        c.last_win_time_seconds = 60;
        c.last_win_used_undo = false;
        let ids: Vec<&str> = check_achievements(&c).iter().map(|d| d.id).collect();
        assert!(ids.contains(&"speed_and_skill"));

        c.last_win_used_undo = true;
        let ids: Vec<&str> = check_achievements(&c).iter().map(|d| d.id).collect();
        assert!(!ids.contains(&"speed_and_skill"));
    }

    #[test]
    fn night_owl_triggers_in_late_night_window() {
        let mut c = ctx();
        c.games_won = 1;
        // Late night: 22:00–02:59
        for hour in [22u32, 23, 0, 1, 2] {
            c.wall_clock_hour = Some(hour);
            let ids: Vec<&str> = check_achievements(&c).iter().map(|d| d.id).collect();
            assert!(ids.contains(&"night_owl"), "expected night_owl at hour {hour}");
        }
        // Daytime hours must not trigger.
        for hour in [3u32, 7, 12, 20, 21] {
            c.wall_clock_hour = Some(hour);
            let ids: Vec<&str> = check_achievements(&c).iter().map(|d| d.id).collect();
            assert!(!ids.contains(&"night_owl"), "unexpected night_owl at hour {hour}");
        }
    }

    #[test]
    fn early_bird_triggers_in_morning_window() {
        let mut c = ctx();
        c.games_won = 1;
        // Early morning: 05:00–06:59
        for hour in [5u32, 6] {
            c.wall_clock_hour = Some(hour);
            let ids: Vec<&str> = check_achievements(&c).iter().map(|d| d.id).collect();
            assert!(ids.contains(&"early_bird"), "expected early_bird at hour {hour}");
        }
        // Outside the window must not trigger.
        for hour in [0u32, 3, 4, 7, 12, 23] {
            c.wall_clock_hour = Some(hour);
            let ids: Vec<&str> = check_achievements(&c).iter().map(|d| d.id).collect();
            assert!(!ids.contains(&"early_bird"), "unexpected early_bird at hour {hour}");
        }
    }

    #[test]
    fn daily_devotee_requires_7_day_streak() {
        let mut c = ctx();
        c.daily_challenge_streak = 6;
        let ids: Vec<&str> = check_achievements(&c).iter().map(|d| d.id).collect();
        assert!(!ids.contains(&"daily_devotee"));

        c.daily_challenge_streak = 7;
        let ids: Vec<&str> = check_achievements(&c).iter().map(|d| d.id).collect();
        assert!(ids.contains(&"daily_devotee"));
    }

    #[test]
    fn perfectionist_requires_no_undo_and_high_score() {
        let mut c = ctx();
        c.last_win_used_undo = false;
        c.last_win_score = 5_000;
        let ids: Vec<&str> = check_achievements(&c).iter().map(|d| d.id).collect();
        assert!(ids.contains(&"perfectionist"));

        c.last_win_used_undo = true;
        let ids: Vec<&str> = check_achievements(&c).iter().map(|d| d.id).collect();
        assert!(!ids.contains(&"perfectionist"));

        c.last_win_used_undo = false;
        c.last_win_score = 4_999;
        let ids: Vec<&str> = check_achievements(&c).iter().map(|d| d.id).collect();
        assert!(!ids.contains(&"perfectionist"));
    }

    #[test]
    fn comeback_requires_at_least_three_recycles() {
        let mut c = ctx();
        c.last_win_recycle_count = 2;
        let ids: Vec<&str> = check_achievements(&c).iter().map(|d| d.id).collect();
        assert!(!ids.contains(&"comeback"));

        c.last_win_recycle_count = 3;
        let ids: Vec<&str> = check_achievements(&c).iter().map(|d| d.id).collect();
        assert!(ids.contains(&"comeback"));
    }

    #[test]
    fn zen_winner_requires_zen_mode() {
        let mut c = ctx();
        c.last_win_is_zen = false;
        let ids: Vec<&str> = check_achievements(&c).iter().map(|d| d.id).collect();
        assert!(!ids.contains(&"zen_winner"));

        c.last_win_is_zen = true;
        let ids: Vec<&str> = check_achievements(&c).iter().map(|d| d.id).collect();
        assert!(ids.contains(&"zen_winner"));
    }

    #[test]
    fn achievement_by_id_finds_known_and_returns_none_for_unknown() {
        assert_eq!(achievement_by_id("first_win").map(|d| d.name), Some("First Win"));
        assert!(achievement_by_id("nonexistent").is_none());
    }

    #[test]
    fn on_a_roll_requires_streak_of_3() {
        let mut c = ctx();
        c.win_streak_current = 2;
        let ids: Vec<&str> = check_achievements(&c).iter().map(|d| d.id).collect();
        assert!(!ids.contains(&"on_a_roll"));

        c.win_streak_current = 3;
        let ids: Vec<&str> = check_achievements(&c).iter().map(|d| d.id).collect();
        assert!(ids.contains(&"on_a_roll"));
    }

    #[test]
    fn unstoppable_requires_streak_of_10() {
        let mut c = ctx();
        c.win_streak_current = 9;
        let ids: Vec<&str> = check_achievements(&c).iter().map(|d| d.id).collect();
        assert!(!ids.contains(&"unstoppable"));
        assert!(ids.contains(&"on_a_roll"), "streak 9 must still satisfy on_a_roll");

        c.win_streak_current = 10;
        let ids: Vec<&str> = check_achievements(&c).iter().map(|d| d.id).collect();
        assert!(ids.contains(&"unstoppable"));
        assert!(ids.contains(&"on_a_roll"), "streak 10 must also satisfy on_a_roll");
    }

    #[test]
    fn century_requires_100_games_played() {
        let mut c = ctx();
        c.games_played = 99;
        let ids: Vec<&str> = check_achievements(&c).iter().map(|d| d.id).collect();
        assert!(!ids.contains(&"century"));

        c.games_played = 100;
        let ids: Vec<&str> = check_achievements(&c).iter().map(|d| d.id).collect();
        assert!(ids.contains(&"century"));
    }

    #[test]
    fn veteran_requires_500_games_played() {
        let mut c = ctx();
        c.games_played = 499;
        let ids: Vec<&str> = check_achievements(&c).iter().map(|d| d.id).collect();
        assert!(!ids.contains(&"veteran"));
        assert!(ids.contains(&"century"), "499 games must also satisfy century");

        c.games_played = 500;
        let ids: Vec<&str> = check_achievements(&c).iter().map(|d| d.id).collect();
        assert!(ids.contains(&"veteran"));
        assert!(ids.contains(&"century"), "500 games must also satisfy century");
    }

    #[test]
    fn high_scorer_requires_best_single_score_of_5000() {
        let mut c = ctx();
        c.best_single_score = 4_999;
        let ids: Vec<&str> = check_achievements(&c).iter().map(|d| d.id).collect();
        assert!(!ids.contains(&"high_scorer"));

        c.best_single_score = 5_000;
        let ids: Vec<&str> = check_achievements(&c).iter().map(|d| d.id).collect();
        assert!(ids.contains(&"high_scorer"));
    }

    #[test]
    fn point_machine_requires_50000_lifetime_score() {
        let mut c = ctx();
        c.lifetime_score = 49_999;
        let ids: Vec<&str> = check_achievements(&c).iter().map(|d| d.id).collect();
        assert!(!ids.contains(&"point_machine"));

        c.lifetime_score = 50_000;
        let ids: Vec<&str> = check_achievements(&c).iter().map(|d| d.id).collect();
        assert!(ids.contains(&"point_machine"));
    }

    #[test]
    fn draw_three_master_requires_10_draw_three_wins() {
        let mut c = ctx();
        c.draw_three_wins = 9;
        let ids: Vec<&str> = check_achievements(&c).iter().map(|d| d.id).collect();
        assert!(!ids.contains(&"draw_three_master"));

        c.draw_three_wins = 10;
        let ids: Vec<&str> = check_achievements(&c).iter().map(|d| d.id).collect();
        assert!(ids.contains(&"draw_three_master"));
    }

    #[test]
    fn speed_demon_boundary_at_180_seconds() {
        let mut c = ctx();
        c.games_won = 1;
        c.last_win_time_seconds = 179;
        let ids: Vec<&str> = check_achievements(&c).iter().map(|d| d.id).collect();
        assert!(ids.contains(&"speed_demon"));

        c.last_win_time_seconds = 180;
        let ids: Vec<&str> = check_achievements(&c).iter().map(|d| d.id).collect();
        assert!(!ids.contains(&"speed_demon"));
    }

    #[test]
    fn check_achievements_returns_multiple_when_conditions_met() {
        // A context where first_win, on_a_roll, and no_undo all trigger at once.
        let mut c = ctx();
        c.games_won = 1;
        c.win_streak_current = 3;
        c.last_win_used_undo = false;
        c.last_win_time_seconds = 999;

        let ids: Vec<&str> = check_achievements(&c).iter().map(|d| d.id).collect();
        assert!(ids.contains(&"first_win"), "first_win should unlock");
        assert!(ids.contains(&"on_a_roll"), "on_a_roll should unlock");
        assert!(ids.contains(&"no_undo"), "no_undo should unlock");
        assert!(ids.len() >= 3, "at least 3 achievements must fire simultaneously");
    }

    #[test]
    fn perfectionist_implies_no_undo_both_fire_together() {
        // perfectionist requires !used_undo && score >= 5000, which is a strict
        // superset of no_undo's condition. Both must appear in the result.
        let mut c = ctx();
        c.games_won = 1;
        c.last_win_used_undo = false;
        c.last_win_score = 5_000;
        c.last_win_time_seconds = 999;

        let ids: Vec<&str> = check_achievements(&c).iter().map(|d| d.id).collect();
        assert!(ids.contains(&"perfectionist"), "perfectionist must unlock");
        assert!(ids.contains(&"no_undo"), "no_undo must also unlock when perfectionist does");
    }

    #[test]
    fn perfectionist_score_well_above_threshold_still_passes() {
        let mut c = ctx();
        c.games_won = 1;
        c.last_win_used_undo = false;
        c.last_win_score = 50_000;

        let ids: Vec<&str> = check_achievements(&c).iter().map(|d| d.id).collect();
        assert!(ids.contains(&"perfectionist"), "score far above threshold must pass");
    }
}
