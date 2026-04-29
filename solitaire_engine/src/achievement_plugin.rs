//! Evaluates achievements on `GameWonEvent`, persists unlocks, and fires
//! `AchievementUnlockedEvent` for each newly unlocked achievement.
//!
//! The persistence path is configurable via `AchievementPlugin::storage_path`.
//! `AchievementPlugin::default()` uses the platform data dir;
//! `AchievementPlugin::headless()` disables I/O entirely (for tests).

use std::path::PathBuf;

use bevy::prelude::*;
use chrono::{Local, Timelike, Utc};
use solitaire_core::achievement::{
    achievement_by_id, check_achievements, AchievementContext, Reward, ALL_ACHIEVEMENTS,
};
use solitaire_data::{
    achievements_file_path, load_achievements_from, save_achievements_to, AchievementRecord,
    save_progress_to,
};

use crate::events::{AchievementUnlockedEvent, GameWonEvent, XpAwardedEvent};
use crate::game_plugin::GameMutation;
use crate::progress_plugin::{LevelUpEvent, ProgressResource, ProgressStoragePath, ProgressUpdate};
use crate::resources::GameStateResource;
use crate::stats_plugin::{StatsResource, StatsUpdate};

/// Marker on the achievements overlay root node.
#[derive(Component, Debug)]
pub struct AchievementsScreen;

/// All per-player achievement records (one per known achievement).
#[derive(Resource, Debug, Clone)]
pub struct AchievementsResource(pub Vec<AchievementRecord>);

/// Persistence path for `AchievementsResource`. `None` disables I/O.
#[derive(Resource, Debug, Clone)]
pub struct AchievementsStoragePath(pub Option<PathBuf>);

pub struct AchievementPlugin {
    pub storage_path: Option<PathBuf>,
}

impl Default for AchievementPlugin {
    fn default() -> Self {
        Self {
            storage_path: achievements_file_path(),
        }
    }
}

impl AchievementPlugin {
    /// Plugin configured with no persistence.
    pub fn headless() -> Self {
        Self { storage_path: None }
    }
}

impl Plugin for AchievementPlugin {
    fn build(&self, app: &mut App) {
        let mut records = match &self.storage_path {
            Some(path) => load_achievements_from(path),
            None => Vec::new(),
        };
        // Ensure every known achievement has a record. Keeps file forward-compatible
        // when new achievements are added in future releases.
        for def in ALL_ACHIEVEMENTS {
            if !records.iter().any(|r| r.id == def.id) {
                records.push(AchievementRecord::locked(def.id));
            }
        }

        app.insert_resource(AchievementsResource(records))
            .insert_resource(AchievementsStoragePath(self.storage_path.clone()))
            .add_message::<AchievementUnlockedEvent>()
            .add_message::<GameWonEvent>()
            .add_message::<XpAwardedEvent>()
            // Run after GameMutation (so GameWonEvent is available), after
            // StatsUpdate (so stats reflect this win), and after ProgressUpdate
            // (so daily_challenge_streak is up to date for daily_devotee).
            .add_systems(
                Update,
                evaluate_on_win
                    .after(GameMutation)
                    .after(StatsUpdate)
                    .after(ProgressUpdate),
            )
            .add_systems(Update, toggle_achievements_screen);
    }
}

#[allow(clippy::too_many_arguments)]
fn evaluate_on_win(
    mut wins: MessageReader<GameWonEvent>,
    mut unlocks: MessageWriter<AchievementUnlockedEvent>,
    mut levelups: MessageWriter<LevelUpEvent>,
    mut xp_awarded: MessageWriter<XpAwardedEvent>,
    game: Res<GameStateResource>,
    stats: Res<StatsResource>,
    path: Res<AchievementsStoragePath>,
    progress_path: Res<ProgressStoragePath>,
    mut achievements: ResMut<AchievementsResource>,
    mut progress: ResMut<ProgressResource>,
) {
    let Some(ev) = wins.read().last() else {
        return;
    };

    let ctx = AchievementContext {
        games_played: stats.0.games_played,
        games_won: stats.0.games_won,
        win_streak_current: stats.0.win_streak_current,
        best_single_score: stats.0.best_single_score,
        lifetime_score: stats.0.lifetime_score,
        draw_three_wins: stats.0.draw_three_wins,
        daily_challenge_streak: progress.0.daily_challenge_streak,
        last_win_score: ev.score,
        last_win_time_seconds: ev.time_seconds,
        last_win_used_undo: game.0.undo_count > 0,
        wall_clock_hour: Some(Local::now().hour()),
        last_win_recycle_count: game.0.recycle_count,
        last_win_is_zen: game.0.mode == solitaire_core::game_state::GameMode::Zen,
    };

    let hits = check_achievements(&ctx);
    if hits.is_empty() {
        return;
    }

    let now = Utc::now();
    let mut achievements_changed = false;
    let mut progress_changed = false;

    for def in hits {
        let Some(record) = achievements.0.iter_mut().find(|r| r.id == def.id) else {
            continue;
        };
        if record.unlocked {
            continue;
        }
        record.unlock(now);
        achievements_changed = true;

        // Grant the reward on first unlock.
        if !record.reward_granted {
            if let Some(reward) = def.reward {
                match reward {
                    Reward::CardBack(idx) => {
                        if !progress.0.unlocked_card_backs.contains(&idx) {
                            progress.0.unlocked_card_backs.push(idx);
                            progress_changed = true;
                        }
                    }
                    Reward::Background(idx) => {
                        if !progress.0.unlocked_backgrounds.contains(&idx) {
                            progress.0.unlocked_backgrounds.push(idx);
                            progress_changed = true;
                        }
                    }
                    Reward::BonusXp(amount) => {
                        xp_awarded.write(XpAwardedEvent { amount });
                        let prev_level = progress.0.add_xp(amount);
                        if progress.0.leveled_up_from(prev_level) {
                            levelups.write(LevelUpEvent {
                                previous_level: prev_level,
                                new_level: progress.0.level,
                                total_xp: progress.0.total_xp,
                            });
                        }
                        progress_changed = true;
                    }
                    Reward::Badge => {}
                }
            }
            record.reward_granted = true;
        }

        unlocks.write(AchievementUnlockedEvent(record.clone()));
    }

    if achievements_changed
        && let Some(target) = &path.0
            && let Err(e) = save_achievements_to(target, &achievements.0) {
                warn!("failed to save achievements: {e}");
            }

    if progress_changed
        && let Some(target) = &progress_path.0
            && let Err(e) = save_progress_to(target, &progress.0) {
                warn!("failed to save progress after reward: {e}");
            }
}

/// Convenience: resolve an achievement ID to its human-readable name.
/// Used by the toast renderer in `animation_plugin`.
pub fn display_name_for(id: &str) -> String {
    achievement_by_id(id)
        .map(|d| d.name.to_string())
        .unwrap_or_else(|| id.to_string())
}

/// Toggle the achievements overlay with the `A` key.
fn toggle_achievements_screen(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    achievements: Res<AchievementsResource>,
    screens: Query<Entity, With<AchievementsScreen>>,
) {
    if !keys.just_pressed(KeyCode::KeyA) {
        return;
    }
    if let Ok(entity) = screens.single() {
        commands.entity(entity).despawn();
    } else {
        spawn_achievements_screen(&mut commands, &achievements.0);
    }
}

fn spawn_achievements_screen(commands: &mut Commands, records: &[AchievementRecord]) {
    let unlocked: Vec<_> = records.iter().filter(|r| r.unlocked).collect();
    let total = ALL_ACHIEVEMENTS.len();

    commands
        .spawn((
            AchievementsScreen,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Percent(0.0),
                top: Val::Percent(0.0),
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.82)),
            ZIndex(210),
        ))
        .with_children(|root| {
            root.spawn((
                Node {
                    flex_direction: FlexDirection::Column,
                    padding: UiRect::all(Val::Px(28.0)),
                    row_gap: Val::Px(8.0),
                    min_width: Val::Px(380.0),
                    max_height: Val::Percent(80.0),
                    overflow: Overflow::clip_y(),
                    border_radius: BorderRadius::all(Val::Px(8.0)),
                    ..default()
                },
                BackgroundColor(Color::srgb(0.09, 0.09, 0.12)),
            ))
            .with_children(|card| {
                // Header
                card.spawn((
                    Text::new(format!(
                        "Achievements  ({}/{})",
                        unlocked.len(),
                        total
                    )),
                    TextFont { font_size: 26.0, ..default() },
                    TextColor(Color::WHITE),
                ));
                card.spawn((
                    Text::new("Press A to close"),
                    TextFont { font_size: 14.0, ..default() },
                    TextColor(Color::srgb(0.55, 0.55, 0.60)),
                ));

                // Separator
                card.spawn((
                    Node {
                        height: Val::Px(1.0),
                        margin: UiRect::vertical(Val::Px(6.0)),
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.25, 0.25, 0.30)),
                ));

                // Achievement rows — unlocked first, then locked
                let mut sorted: Vec<_> = records.iter().collect();
                sorted.sort_by_key(|r| (!r.unlocked, r.id.clone()));

                for record in &sorted {
                    let def = achievement_by_id(&record.id);
                    let (name, description) = def
                        .map(|d| (d.name, d.description))
                        .unwrap_or((&record.id, ""));

                    // Hide secret locked achievements
                    let is_secret = def.map(|d| d.secret).unwrap_or(false);
                    if is_secret && !record.unlocked {
                        continue;
                    }

                    let (name_color, desc_color, prefix) = if record.unlocked {
                        (
                            Color::srgb(1.0, 0.87, 0.0),
                            Color::srgb(0.75, 0.75, 0.70),
                            "✓ ",
                        )
                    } else {
                        (
                            Color::srgb(0.45, 0.45, 0.50),
                            Color::srgb(0.35, 0.35, 0.40),
                            "◯ ",
                        )
                    };

                    card.spawn(Node {
                        flex_direction: FlexDirection::Column,
                        row_gap: Val::Px(1.0),
                        margin: UiRect::bottom(Val::Px(4.0)),
                        ..default()
                    })
                    .with_children(|row| {
                        row.spawn((
                            Text::new(format!("{prefix}{name}")),
                            TextFont { font_size: 16.0, ..default() },
                            TextColor(name_color),
                        ));
                        if !description.is_empty() {
                            row.spawn((
                                Text::new(format!("   {description}")),
                                TextFont { font_size: 13.0, ..default() },
                                TextColor(desc_color),
                            ));
                        }
                        // Reward line
                        if let Some(reward_str) = def.and_then(|d| d.reward).map(format_reward) {
                            row.spawn((
                                Text::new(format!("   Reward: {reward_str}")),
                                TextFont { font_size: 12.0, ..default() },
                                TextColor(Color::srgb(0.45, 0.75, 0.45)),
                            ));
                        }
                        // Unlock date for unlocked achievements
                        if let Some(date) = record.unlock_date {
                            row.spawn((
                                Text::new(format!("   Unlocked {}", date.format("%Y-%m-%d"))),
                                TextFont { font_size: 11.0, ..default() },
                                TextColor(Color::srgb(0.40, 0.40, 0.45)),
                            ));
                        }
                    });
                }
            });
        });
}

fn format_reward(reward: Reward) -> String {
    match reward {
        Reward::CardBack(idx) => format!("Card Back #{idx}"),
        Reward::Background(idx) => format!("Background #{idx}"),
        Reward::BonusXp(xp) => format!("+{xp} XP"),
        Reward::Badge => "Badge".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game_plugin::GamePlugin;
    use crate::stats_plugin::StatsPlugin;
    use crate::table_plugin::TablePlugin;

    fn headless_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(GamePlugin)
            .add_plugins(TablePlugin)
            .add_plugins(StatsPlugin::headless())
            .add_plugins(crate::progress_plugin::ProgressPlugin::headless())
            .add_plugins(AchievementPlugin::headless());
        // StatsPlugin's UI toggle system reads ButtonInput<KeyCode>; under
        // MinimalPlugins it isn't auto-registered.
        app.init_resource::<bevy::input::ButtonInput<KeyCode>>();
        app.update();
        app
    }

    #[test]
    fn resource_is_populated_with_all_known_ids() {
        let app = headless_app();
        let records = &app.world().resource::<AchievementsResource>().0;
        assert_eq!(records.len(), ALL_ACHIEVEMENTS.len());
        for def in ALL_ACHIEVEMENTS {
            assert!(records.iter().any(|r| r.id == def.id && !r.unlocked));
        }
    }

    #[test]
    fn win_unlocks_first_win_and_fires_event() {
        let mut app = headless_app();

        // StatsPlugin runs update_stats_on_win first (after GameMutation); that
        // bumps games_won to 1 before evaluate_on_win reads StatsResource.
        app.world_mut().write_message(GameWonEvent {
            score: 1000,
            time_seconds: 300,
        });
        app.update();

        let unlocked_first_win = app
            .world()
            .resource::<AchievementsResource>()
            .0
            .iter()
            .find(|r| r.id == "first_win")
            .map(|r| r.unlocked)
            .unwrap_or(false);
        assert!(unlocked_first_win);

        // Verify the event was emitted.
        let events = app.world().resource::<Messages<AchievementUnlockedEvent>>();
        let mut cursor = events.get_cursor();
        let fired: Vec<String> = cursor.read(events).map(|e| e.0.id.clone()).collect();
        assert!(fired.contains(&"first_win".to_string()));
    }

    #[test]
    fn repeated_win_does_not_refire_already_unlocked_achievement() {
        let mut app = headless_app();

        app.world_mut().write_message(GameWonEvent {
            score: 1000,
            time_seconds: 300,
        });
        app.update();

        // Clear events from first win.
        app.world_mut()
            .resource_mut::<Messages<AchievementUnlockedEvent>>()
            .clear();

        app.world_mut().write_message(GameWonEvent {
            score: 1000,
            time_seconds: 300,
        });
        app.update();

        let events = app.world().resource::<Messages<AchievementUnlockedEvent>>();
        let mut cursor = events.get_cursor();
        let fired: Vec<String> = cursor.read(events).map(|e| e.0.id.clone()).collect();
        assert!(
            !fired.contains(&"first_win".to_string()),
            "first_win must not re-fire on subsequent wins"
        );
    }

    #[test]
    fn display_name_resolves_known_and_unknown_ids() {
        assert_eq!(display_name_for("first_win"), "First Win");
        assert_eq!(display_name_for("bogus"), "bogus");
    }

    #[test]
    fn bonus_xp_reward_fires_xp_awarded_event() {
        let mut app = headless_app();
        // "no_undo" achievement awards BonusXp(25). Trigger it by sending a
        // GameWonEvent with undo_count == 0 (default) and enough stats to match.
        app.world_mut().write_message(GameWonEvent {
            score: 1000,
            time_seconds: 300,
        });
        app.update();

        let events = app.world().resource::<Messages<XpAwardedEvent>>();
        let mut cursor = events.get_cursor();
        let xp_events: Vec<u64> = cursor.read(events).map(|e| e.amount).collect();
        // The no_undo achievement (BonusXp 25) must have fired an XpAwardedEvent.
        assert!(
            xp_events.contains(&25),
            "BonusXp(25) must fire XpAwardedEvent; got {xp_events:?}"
        );
    }

    #[test]
    fn no_undo_achievement_does_not_fire_when_undo_was_used() {
        let mut app = headless_app();
        // Simulate a win where the player used undo at least once.
        app.world_mut()
            .resource_mut::<GameStateResource>()
            .0
            .undo_count = 1;

        app.world_mut().write_message(GameWonEvent {
            score: 1000,
            time_seconds: 300,
        });
        app.update();

        // "no_undo" awards BonusXp(25). If undo was used it must NOT fire.
        let events = app.world().resource::<Messages<XpAwardedEvent>>();
        let mut cursor = events.get_cursor();
        let xp_events: Vec<u64> = cursor.read(events).map(|e| e.amount).collect();
        assert!(
            !xp_events.contains(&25),
            "BonusXp(25) must not fire when undo_count > 0; got {xp_events:?}"
        );
    }

    fn press(app: &mut App, key: KeyCode) {
        let mut input = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        input.release(key);
        input.clear();
        input.press(key);
    }

    #[test]
    fn pressing_a_spawns_achievements_screen() {
        let mut app = headless_app();
        press(&mut app, KeyCode::KeyA);
        app.update();
        let count = app
            .world_mut()
            .query::<&AchievementsScreen>()
            .iter(app.world())
            .count();
        assert_eq!(count, 1);
    }

    #[test]
    fn pressing_a_twice_dismisses_screen() {
        let mut app = headless_app();
        press(&mut app, KeyCode::KeyA);
        app.update();
        press(&mut app, KeyCode::KeyA);
        app.update();
        let count = app
            .world_mut()
            .query::<&AchievementsScreen>()
            .iter(app.world())
            .count();
        assert_eq!(count, 0);
    }

    // -----------------------------------------------------------------------
    // format_reward
    // -----------------------------------------------------------------------

    #[test]
    fn format_reward_card_back() {
        assert_eq!(format_reward(Reward::CardBack(2)), "Card Back #2");
    }

    #[test]
    fn format_reward_background() {
        assert_eq!(format_reward(Reward::Background(3)), "Background #3");
    }

    #[test]
    fn format_reward_bonus_xp() {
        assert_eq!(format_reward(Reward::BonusXp(25)), "+25 XP");
    }

    #[test]
    fn format_reward_badge() {
        assert_eq!(format_reward(Reward::Badge), "Badge");
    }
}
