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
    // "Play after midnight" — 00:00 through 05:59 local time.
    matches!(c.wall_clock_hour, Some(h) if h < 6)
}
fn early_bird(c: &AchievementContext) -> bool {
    // "Play before 6am" — same window as night_owl; both unlock together
    // when someone wins in the small hours. Retained for progression variety.
    matches!(c.wall_clock_hour, Some(h) if h < 6)
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
        description: "Win a game after midnight",
        secret: false,
        reward: None,
        condition: night_owl,
    },
    AchievementDef {
        id: "early_bird",
        name: "Early Bird",
        description: "Win a game before 6am",
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
    fn night_owl_requires_early_hours() {
        let mut c = ctx();
        c.games_won = 1;
        c.wall_clock_hour = Some(2);
        let ids: Vec<&str> = check_achievements(&c).iter().map(|d| d.id).collect();
        assert!(ids.contains(&"night_owl"));

        c.wall_clock_hour = Some(12);
        let ids: Vec<&str> = check_achievements(&c).iter().map(|d| d.id).collect();
        assert!(!ids.contains(&"night_owl"));
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
}
