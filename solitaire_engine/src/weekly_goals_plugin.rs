//! Tracks per-ISO-week goal progress: rolls the counter set when the week
//! changes, increments matching goals on `GameWonEvent`, awards
//! `WEEKLY_GOAL_XP` when a goal completes, and persists.

use bevy::prelude::*;
use chrono::Local;
use solitaire_data::{
    current_iso_week_key, save_progress_to, weekly_goal_by_id, WeeklyGoalContext, WEEKLY_GOALS,
    WEEKLY_GOAL_XP,
};

use crate::events::{GameWonEvent, XpAwardedEvent};
use crate::game_plugin::GameMutation;
use crate::progress_plugin::{LevelUpEvent, ProgressResource, ProgressStoragePath, ProgressUpdate};
use crate::resources::GameStateResource;

/// Fired when the player has just completed a weekly goal.
#[derive(Message, Debug, Clone)]
pub struct WeeklyGoalCompletedEvent {
    pub goal_id: String,
    pub description: String,
}

/// Tracks weekly goal progress (e.g. win N games, play without undo) and fires `WeeklyGoalCompletedEvent` when a goal is met.
/// Progress resets each Monday.
pub struct WeeklyGoalsPlugin;

impl Plugin for WeeklyGoalsPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<WeeklyGoalCompletedEvent>()
            .add_message::<GameWonEvent>()
            .add_message::<XpAwardedEvent>()
            .add_systems(Startup, roll_weekly_goals_on_startup)
            // Run after GameMutation (so GameWonEvent is available) and
            // ProgressUpdate (so we don't fight ProgressPlugin's add_xp).
            .add_systems(
                Update,
                evaluate_weekly_goals
                    .after(GameMutation)
                    .after(ProgressUpdate),
            );
    }
}

/// Rolls weekly-goal counters at startup so stale progress from a previous
/// week never shows in the UI when the player launches the game.
fn roll_weekly_goals_on_startup(
    mut progress: ResMut<ProgressResource>,
    path: Res<ProgressStoragePath>,
) {
    let week_key = current_iso_week_key(Local::now().date_naive());
    if progress.0.roll_weekly_goals_if_new_week(&week_key)
        && let Some(target) = &path.0
            && let Err(e) = save_progress_to(target, &progress.0) {
                warn!("failed to save progress after weekly reset on startup: {e}");
            }
}

fn evaluate_weekly_goals(
    mut wins: MessageReader<GameWonEvent>,
    game: Res<GameStateResource>,
    mut progress: ResMut<ProgressResource>,
    path: Res<ProgressStoragePath>,
    mut completions: MessageWriter<WeeklyGoalCompletedEvent>,
    mut levelups: MessageWriter<LevelUpEvent>,
    mut xp_awarded: MessageWriter<XpAwardedEvent>,
) {
    let mut events: Vec<&GameWonEvent> = wins.read().collect();
    if events.is_empty() {
        return;
    }
    // Roll the week first so progress for old weeks doesn't carry over.
    let week_key = current_iso_week_key(Local::now().date_naive());
    progress.0.roll_weekly_goals_if_new_week(&week_key);

    let mut any_change = false;
    let mut bonus_xp: u64 = 0;

    // Drain in order so earlier wins roll up before later ones are evaluated
    // (only matters for backlogged events; usually 1 per frame).
    for ev in events.drain(..) {
        let ctx = WeeklyGoalContext {
            time_seconds: ev.time_seconds,
            used_undo: game.0.undo_count > 0,
            draw_mode: game.0.draw_mode.clone(),
        };
        for def in WEEKLY_GOALS {
            if !def.matches(&ctx) {
                continue;
            }
            let just_completed = progress.0.record_weekly_progress(def.id, def.target);
            any_change = true;
            if just_completed {
                bonus_xp = bonus_xp.saturating_add(WEEKLY_GOAL_XP);
                completions.write(WeeklyGoalCompletedEvent {
                    goal_id: def.id.to_string(),
                    description: def.description.to_string(),
                });
            }
        }
    }

    if bonus_xp > 0 {
        xp_awarded.write(XpAwardedEvent { amount: bonus_xp });
        let prev_level = progress.0.add_xp(bonus_xp);
        if progress.0.leveled_up_from(prev_level) {
            levelups.write(LevelUpEvent {
                previous_level: prev_level,
                new_level: progress.0.level,
                total_xp: progress.0.total_xp,
            });
        }
    }

    if any_change
        && let Some(target) = &path.0
            && let Err(e) = save_progress_to(target, &progress.0) {
                warn!("failed to save progress after weekly goal update: {e}");
            }
}

/// Resolve a goal id to its description (used for toasts).
pub fn weekly_goal_description(id: &str) -> String {
    weekly_goal_by_id(id)
        .map(|g| g.description.to_string())
        .unwrap_or_else(|| id.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game_plugin::GamePlugin;
    use crate::progress_plugin::ProgressPlugin;
    use crate::table_plugin::TablePlugin;

    fn headless_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(GamePlugin)
            .add_plugins(TablePlugin)
            .add_plugins(ProgressPlugin::headless())
            .add_plugins(WeeklyGoalsPlugin);
        app.update();
        app
    }

    #[test]
    fn first_win_increments_win_game_goal() {
        let mut app = headless_app();
        app.world_mut().write_message(GameWonEvent {
            score: 500,
            time_seconds: 200,
        });
        app.update();
        let p = &app.world().resource::<ProgressResource>().0;
        assert_eq!(p.weekly_goal_progress.get("weekly_5_wins"), Some(&1));
        // No-undo + slow win → no_undo goal also ticked, fast goal NOT ticked.
        assert_eq!(p.weekly_goal_progress.get("weekly_3_no_undo"), Some(&1));
        assert!(!p.weekly_goal_progress.contains_key("weekly_3_fast"));
    }

    #[test]
    fn fast_win_ticks_fast_goal_too() {
        let mut app = headless_app();
        app.world_mut().write_message(GameWonEvent {
            score: 500,
            time_seconds: 60,
        });
        app.update();
        let p = &app.world().resource::<ProgressResource>().0;
        assert_eq!(p.weekly_goal_progress.get("weekly_3_fast"), Some(&1));
    }

    #[test]
    fn win_after_undo_does_not_tick_no_undo_goal() {
        let mut app = headless_app();
        app.world_mut()
            .resource_mut::<GameStateResource>()
            .0
            .undo_count = 1;

        app.world_mut().write_message(GameWonEvent {
            score: 500,
            time_seconds: 200,
        });
        app.update();
        let p = &app.world().resource::<ProgressResource>().0;
        assert_eq!(p.weekly_goal_progress.get("weekly_5_wins"), Some(&1));
        assert!(!p.weekly_goal_progress.contains_key("weekly_3_no_undo"));
    }

    #[test]
    fn completing_a_goal_fires_event_and_awards_bonus() {
        let mut app = headless_app();
        // Pre-set the weekly_3_fast goal to 2/3 so the next fast win completes it.
        // Also pre-complete weekly_1_under_five (target=1) and weekly_5_wins /
        // weekly_3_no_undo at target so a 60-second win only completes weekly_3_fast,
        // keeping the XP delta predictable.
        {
            let mut p = app.world_mut().resource_mut::<ProgressResource>();
            p.0.weekly_goal_progress.insert("weekly_3_fast".to_string(), 2);
            p.0.weekly_goal_progress.insert("weekly_1_under_five".to_string(), 1);
            p.0.weekly_goal_progress.insert("weekly_5_wins".to_string(), 5);
            p.0.weekly_goal_progress.insert("weekly_3_no_undo".to_string(), 3);
        }
        // Match the current ISO week key so roll_weekly_goals doesn't clear it.
        let key = current_iso_week_key(Local::now().date_naive());
        app.world_mut()
            .resource_mut::<ProgressResource>()
            .0
            .weekly_goal_week_iso = Some(key);

        let xp_before = app.world().resource::<ProgressResource>().0.total_xp;

        app.world_mut().write_message(GameWonEvent {
            score: 500,
            time_seconds: 60,
        });
        app.update();

        let p = &app.world().resource::<ProgressResource>().0;
        assert_eq!(p.weekly_goal_progress.get("weekly_3_fast"), Some(&3));
        // Delta = base win XP (from ProgressPlugin in the headless app) +
        // WEEKLY_GOAL_XP for completing the goal. Verify the goal bonus is
        // included by checking `delta - base_win_xp == WEEKLY_GOAL_XP`.
        let base_win_xp = solitaire_data::xp_for_win(60, false);
        assert_eq!(p.total_xp - xp_before, base_win_xp + WEEKLY_GOAL_XP);

        let events = app.world().resource::<Messages<WeeklyGoalCompletedEvent>>();
        let mut cursor = events.get_cursor();
        let fired: Vec<_> = cursor.read(events).cloned().collect();
        assert!(fired.iter().any(|e| e.goal_id == "weekly_3_fast"));
    }

    #[test]
    fn stale_weekly_progress_is_cleared_on_startup() {
        let mut app = headless_app();
        // Inject progress from a past week.
        {
            let mut p = app.world_mut().resource_mut::<ProgressResource>();
            p.0.weekly_goal_week_iso = Some("1970-W01".to_string());
            p.0.weekly_goal_progress
                .insert("weekly_5_wins".to_string(), 3);
        }
        // A second Startup run (re-init) is hard to trigger directly; instead
        // call the helper through a fresh app that starts with stale data.
        // Here we simulate the effect: roll_weekly_goals_if_new_week clears.
        let current_week = current_iso_week_key(Local::now().date_naive());
        let rolled = app
            .world_mut()
            .resource_mut::<ProgressResource>()
            .0
            .roll_weekly_goals_if_new_week(&current_week);
        assert!(rolled, "expected stale week to trigger a roll");
        assert!(
            app.world()
                .resource::<ProgressResource>()
                .0
                .weekly_goal_progress
                .is_empty()
        );
    }

    #[test]
    fn weekly_bonus_xp_fires_levelup_when_threshold_crossed() {
        let mut app = headless_app();
        // Set XP just below the first level boundary (500) so the 75-XP bonus crosses it.
        app.world_mut().resource_mut::<ProgressResource>().0.total_xp = 430;
        // Pre-set goal to 2/3 so the next fast win completes it.
        app.world_mut()
            .resource_mut::<ProgressResource>()
            .0
            .weekly_goal_progress
            .insert("weekly_3_fast".to_string(), 2);
        let key = current_iso_week_key(Local::now().date_naive());
        app.world_mut()
            .resource_mut::<ProgressResource>()
            .0
            .weekly_goal_week_iso = Some(key);

        app.world_mut().write_message(GameWonEvent {
            score: 500,
            time_seconds: 60,
        });
        app.update();

        let events = app.world().resource::<Messages<LevelUpEvent>>();
        let mut cursor = events.get_cursor();
        let fired: Vec<_> = cursor.read(events).copied().collect();
        assert!(!fired.is_empty(), "LevelUpEvent must fire when weekly bonus pushes past a level threshold");
    }

    #[test]
    fn weekly_goal_description_resolves_known_and_unknown() {
        assert_eq!(
            weekly_goal_description("weekly_5_wins"),
            "Win 5 games this week"
        );
        assert_eq!(weekly_goal_description("nope"), "nope");
    }
}
