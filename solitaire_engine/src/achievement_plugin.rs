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
    achievement_by_id, check_achievements, AchievementContext, AchievementDef, Reward,
    ALL_ACHIEVEMENTS,
};
use solitaire_data::{
    achievements_file_path, load_achievements_from, save_achievements_to, save_settings_to,
    AchievementRecord, save_progress_to,
};

use crate::events::{
    AchievementUnlockedEvent, GameWonEvent, InfoToastEvent, ToggleAchievementsRequestEvent,
    XpAwardedEvent,
};
use crate::font_plugin::FontResource;
use crate::game_plugin::GameMutation;
use crate::progress_plugin::{LevelUpEvent, ProgressResource, ProgressStoragePath, ProgressUpdate};
use crate::resources::GameStateResource;
use crate::settings_plugin::{SettingsResource, SettingsStoragePath};
use crate::stats_plugin::{StatsResource, StatsUpdate};
use crate::ui_modal::{
    spawn_modal, spawn_modal_actions, spawn_modal_button, spawn_modal_header, ButtonVariant,
};
use crate::ui_theme::{
    ACCENT_PRIMARY, BORDER_SUBTLE, STATE_SUCCESS, TEXT_DISABLED, TEXT_PRIMARY, TEXT_SECONDARY,
    TYPE_BODY, TYPE_BODY_LG, TYPE_CAPTION, VAL_SPACE_1, Z_MODAL_PANEL,
};
use crate::ui_tooltip::Tooltip;

/// Marker on the achievements overlay root node.
#[derive(Component, Debug)]
pub struct AchievementsScreen;

/// Marker on each per-achievement row inside the Achievements modal. Used by
/// hover-tooltip plumbing and tests so a row can be identified independently
/// of its visible text.
#[derive(Component, Debug)]
pub struct AchievementRow;

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
            .add_message::<InfoToastEvent>()
            .add_message::<ToggleAchievementsRequestEvent>()
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
            // Achievement-onboarding cue: fires once after the player's very
            // first win to teach the Achievements panel exists. Must run
            // `.after(StatsUpdate)` so `stats.games_won` reflects the win
            // that just landed (StatsUpdate increments it on `GameWonEvent`).
            .add_systems(
                Update,
                fire_achievement_onboarding_toast
                    .after(GameMutation)
                    .after(StatsUpdate),
            )
            .add_systems(Update, toggle_achievements_screen)
            .add_systems(Update, handle_achievements_close_button);
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

/// Achievement-onboarding cue.
///
/// On the player's very first win — and only their first — fires a single
/// `InfoToastEvent` nudging them toward the Achievements panel (`A` hotkey)
/// so they discover the progression layer.
///
/// Three guards prevent spurious or repeat firings:
///
/// * `stats.games_won == 1` — the post-condition is checked **after**
///   `StatsUpdate` increments `games_won`, so the cue only fires for the
///   true first win, not (for example) a player who imported existing
///   sync data and won a later game.
/// * `!settings.shown_achievement_onboarding` — flips to `true` after
///   the toast fires, persists to `settings.json`, and serves as the
///   one-shot guard across launches and merged sync.
/// * The system bails immediately when no `GameWonEvent` arrived this
///   frame so it is a no-op outside the post-win frame.
///
/// The `A` hotkey is mentioned verbatim in the toast text so players who
/// dismiss the cue still know where to find the panel.
fn fire_achievement_onboarding_toast(
    mut wins: MessageReader<GameWonEvent>,
    stats: Res<StatsResource>,
    mut settings: Option<ResMut<SettingsResource>>,
    settings_path: Option<Res<SettingsStoragePath>>,
    mut toast: MessageWriter<InfoToastEvent>,
) {
    // Drain the event queue regardless — multiple wins on a single frame
    // only need a single onboarding toast at most.
    let any_win = wins.read().last().is_some();
    if !any_win {
        return;
    }

    // Without a `SettingsResource` (headless tests that omit `SettingsPlugin`)
    // we have no flag to consult; bail out cleanly.
    let Some(settings) = settings.as_mut() else {
        return;
    };
    if settings.0.shown_achievement_onboarding {
        return;
    }
    if stats.0.games_won != 1 {
        return;
    }

    toast.write(InfoToastEvent(
        "First win! Press A to see your achievements.".to_string(),
    ));
    settings.0.shown_achievement_onboarding = true;

    // Persist so the cue stays one-shot across launches. `None` storage
    // (headless / test) is a documented no-op.
    if let Some(path) = settings_path.as_ref()
        && let Some(target) = path.0.as_deref()
        && let Err(e) = save_settings_to(target, &settings.0)
    {
        warn!("failed to save settings (achievement onboarding): {e}");
    }
}

/// Convenience: resolve an achievement ID to its human-readable name.
/// Used by the toast renderer in `animation_plugin`.
pub fn display_name_for(id: &str) -> String {
    achievement_by_id(id).map_or_else(|| id.to_string(), |d| d.name.to_string())
}

/// Marker on the "Done" button inside the Achievements modal.
#[derive(Component, Debug)]
pub struct AchievementsCloseButton;

/// Toggle the achievements overlay — `A` keyboard accelerator or
/// `ToggleAchievementsRequestEvent` from the HUD Menu popover.
fn toggle_achievements_screen(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    mut requests: MessageReader<ToggleAchievementsRequestEvent>,
    achievements: Res<AchievementsResource>,
    font_res: Option<Res<FontResource>>,
    screens: Query<Entity, With<AchievementsScreen>>,
) {
    let button_clicked = requests.read().count() > 0;
    if !keys.just_pressed(KeyCode::KeyA) && !button_clicked {
        return;
    }
    if let Ok(entity) = screens.single() {
        commands.entity(entity).despawn();
    } else {
        spawn_achievements_screen(&mut commands, &achievements.0, font_res.as_deref());
    }
}

/// Click handler for the modal's "Done" button — despawns the overlay
/// the same way the `A` accelerator does.
fn handle_achievements_close_button(
    mut commands: Commands,
    close_buttons: Query<&Interaction, (With<AchievementsCloseButton>, Changed<Interaction>)>,
    screens: Query<Entity, With<AchievementsScreen>>,
) {
    if !close_buttons.iter().any(|i| *i == Interaction::Pressed) {
        return;
    }
    for entity in &screens {
        commands.entity(entity).despawn();
    }
}

fn spawn_achievements_screen(
    commands: &mut Commands,
    records: &[AchievementRecord],
    font_res: Option<&FontResource>,
) {
    let unlocked: Vec<_> = records.iter().filter(|r| r.unlocked).collect();
    let total = ALL_ACHIEVEMENTS.len();
    let header = format!("Achievements  ({}/{})", unlocked.len(), total);

    let font_handle = font_res.map(|f| f.0.clone()).unwrap_or_default();
    let font_name = TextFont {
        font: font_handle.clone(),
        font_size: TYPE_BODY_LG,
        ..default()
    };
    let font_desc = TextFont {
        font: font_handle.clone(),
        font_size: TYPE_BODY,
        ..default()
    };
    let font_meta = TextFont {
        font: font_handle,
        font_size: TYPE_CAPTION,
        ..default()
    };

    spawn_modal(commands, AchievementsScreen, Z_MODAL_PANEL, |card| {
        spawn_modal_header(card, header, font_res);

        // Achievement rows — unlocked first, then locked alphabetical.
        let mut sorted: Vec<_> = records.iter().collect();
        sorted.sort_by_key(|r| (!r.unlocked, r.id.clone()));

        for record in &sorted {
            let def = achievement_by_id(&record.id);
            let (name, description) = def.map_or((record.id.as_str(), ""), |d| (d.name, d.description));

            // Hide secret locked achievements so they remain a surprise.
            let is_secret = def.is_some_and(|d| d.secret);
            if is_secret && !record.unlocked {
                continue;
            }

            let (name_color, desc_color, prefix) = if record.unlocked {
                (ACCENT_PRIMARY, TEXT_PRIMARY, "\u{2713} ")
            } else {
                (TEXT_DISABLED, TEXT_DISABLED, "\u{25CB} ")
            };

            let tooltip_text = tooltip_for_row(record.unlocked, def);

            card.spawn((
                Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: VAL_SPACE_1,
                    ..default()
                },
                AchievementRow,
                Tooltip::new(tooltip_text),
            ))
            .with_children(|row| {
                row.spawn((
                    Text::new(format!("{prefix}{name}")),
                    font_name.clone(),
                    TextColor(name_color),
                ));
                if !description.is_empty() {
                    row.spawn((
                        Text::new(format!("   {description}")),
                        font_desc.clone(),
                        TextColor(desc_color),
                    ));
                }
                if let Some(reward_str) = def.and_then(|d| d.reward).map(format_reward) {
                    row.spawn((
                        Text::new(format!("   Reward: {reward_str}")),
                        font_meta.clone(),
                        TextColor(STATE_SUCCESS),
                    ));
                }
                if let Some(date) = record.unlock_date {
                    row.spawn((
                        Text::new(format!("   Unlocked {}", date.format("%Y-%m-%d"))),
                        font_meta.clone(),
                        TextColor(TEXT_SECONDARY),
                    ));
                }
            });

            // Subtle row separator — keeps the long list scannable.
            card.spawn((
                Node {
                    height: Val::Px(1.0),
                    ..default()
                },
                BackgroundColor(BORDER_SUBTLE),
            ));
        }

        spawn_modal_actions(card, |actions| {
            spawn_modal_button(
                actions,
                AchievementsCloseButton,
                "Done",
                Some("A"),
                ButtonVariant::Primary,
                font_res,
            );
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

/// Compose the per-row hover-tooltip string. Surfaces information that the
/// row itself does not always make obvious:
///
/// * Unlocked + reward → "Reward: <reward>." — celebrates the prize.
/// * Unlocked, no reward → "Earned!".
/// * Locked, non-secret → "How to unlock: <description>." plus the reward
///   when one is defined; the visible row already shows the same lines, but
///   gathering them in one tooltip keeps the long list scannable on hover.
/// * Locked, secret rows are filtered out before they reach this helper —
///   they get no tooltip so the unlock condition stays a surprise.
///
/// Defs are looked up at the call site; `None` means the record refers to an
/// achievement no longer present in `ALL_ACHIEVEMENTS` (forward-compat) and
/// gets a generic fallback.
fn tooltip_for_row(unlocked: bool, def: Option<&AchievementDef>) -> String {
    if unlocked {
        match def.and_then(|d| d.reward).map(format_reward) {
            Some(reward) => format!("Reward: {reward}."),
            None => "Earned!".to_string(),
        }
    } else {
        let description = def.map_or("", |d| d.description);
        let how = if description.is_empty() {
            "How to unlock: keep playing.".to_string()
        } else {
            format!("How to unlock: {description}.")
        };
        match def.and_then(|d| d.reward).map(format_reward) {
            Some(reward) => format!("{how} Reward: {reward}."),
            None => how,
        }
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

    // -----------------------------------------------------------------------
    // draw_three_master integration
    // -----------------------------------------------------------------------

    #[test]
    fn draw_three_master_fires_on_tenth_draw_three_win() {
        let mut app = headless_app();

        // Pre-seed nine prior Draw-Three wins. The pending GameWonEvent will
        // trigger update_stats_on_win first (StatsUpdate runs before
        // evaluate_on_win), bumping draw_three_wins to 10 — the unlock
        // threshold for the draw_three_master achievement.
        app.world_mut().resource_mut::<StatsResource>().0.draw_three_wins = 9;

        // The current game must be in DrawThree mode so update_on_win
        // increments draw_three_wins (and not draw_one_wins).
        app.world_mut()
            .resource_mut::<GameStateResource>()
            .0
            .draw_mode = solitaire_core::game_state::DrawMode::DrawThree;

        app.world_mut().write_message(GameWonEvent {
            score: 500,
            time_seconds: 240,
        });
        app.update();

        // Sanity-check that the win was actually attributed to Draw-Three so
        // the achievement reads the correct counter.
        let stats = &app.world().resource::<StatsResource>().0;
        assert_eq!(stats.draw_three_wins, 10);

        let unlocked = app
            .world()
            .resource::<AchievementsResource>()
            .0
            .iter()
            .find(|r| r.id == "draw_three_master")
            .map(|r| r.unlocked)
            .unwrap_or(false);
        assert!(unlocked, "draw_three_master must unlock at the 10th Draw-Three win");

        // Verify the AchievementUnlockedEvent fired for this id.
        let events = app.world().resource::<Messages<AchievementUnlockedEvent>>();
        let mut cursor = events.get_cursor();
        let fired: Vec<String> = cursor.read(events).map(|e| e.0.id.clone()).collect();
        assert!(
            fired.contains(&"draw_three_master".to_string()),
            "AchievementUnlockedEvent for draw_three_master must fire; got {fired:?}"
        );
    }

    #[test]
    fn draw_three_master_does_not_fire_at_nine_wins() {
        let mut app = headless_app();

        // Pre-seed eight prior Draw-Three wins. The pending GameWonEvent
        // brings draw_three_wins to 9 — one short of the threshold.
        app.world_mut().resource_mut::<StatsResource>().0.draw_three_wins = 8;
        app.world_mut()
            .resource_mut::<GameStateResource>()
            .0
            .draw_mode = solitaire_core::game_state::DrawMode::DrawThree;

        app.world_mut().write_message(GameWonEvent {
            score: 500,
            time_seconds: 240,
        });
        app.update();

        let stats = &app.world().resource::<StatsResource>().0;
        assert_eq!(stats.draw_three_wins, 9);

        let unlocked = app
            .world()
            .resource::<AchievementsResource>()
            .0
            .iter()
            .find(|r| r.id == "draw_three_master")
            .map(|r| r.unlocked)
            .unwrap_or(false);
        assert!(!unlocked, "draw_three_master must remain locked at 9 Draw-Three wins");

        let events = app.world().resource::<Messages<AchievementUnlockedEvent>>();
        let mut cursor = events.get_cursor();
        let fired: Vec<String> = cursor.read(events).map(|e| e.0.id.clone()).collect();
        assert!(
            !fired.contains(&"draw_three_master".to_string()),
            "draw_three_master must not fire below threshold; got {fired:?}"
        );
    }

    // -----------------------------------------------------------------------
    // zen_winner integration
    // -----------------------------------------------------------------------

    #[test]
    fn zen_winner_fires_on_zen_mode_win() {
        let mut app = headless_app();

        // Put the active game in Zen mode. evaluate_on_win reads
        // GameStateResource.mode directly to populate last_win_is_zen.
        app.world_mut()
            .resource_mut::<GameStateResource>()
            .0
            .mode = solitaire_core::game_state::GameMode::Zen;

        app.world_mut().write_message(GameWonEvent {
            score: 0,
            time_seconds: 600,
        });
        app.update();

        let unlocked = app
            .world()
            .resource::<AchievementsResource>()
            .0
            .iter()
            .find(|r| r.id == "zen_winner")
            .map(|r| r.unlocked)
            .unwrap_or(false);
        assert!(unlocked, "zen_winner must unlock when the game mode is Zen");

        let events = app.world().resource::<Messages<AchievementUnlockedEvent>>();
        let mut cursor = events.get_cursor();
        let fired: Vec<String> = cursor.read(events).map(|e| e.0.id.clone()).collect();
        assert!(
            fired.contains(&"zen_winner".to_string()),
            "AchievementUnlockedEvent for zen_winner must fire; got {fired:?}"
        );
    }

    #[test]
    fn zen_winner_does_not_fire_for_classic_win() {
        let mut app = headless_app();

        // Default GameMode is Classic; assert and rely on it.
        assert_eq!(
            app.world().resource::<GameStateResource>().0.mode,
            solitaire_core::game_state::GameMode::Classic
        );

        app.world_mut().write_message(GameWonEvent {
            score: 1000,
            time_seconds: 300,
        });
        app.update();

        let unlocked = app
            .world()
            .resource::<AchievementsResource>()
            .0
            .iter()
            .find(|r| r.id == "zen_winner")
            .map(|r| r.unlocked)
            .unwrap_or(false);
        assert!(!unlocked, "zen_winner must remain locked outside Zen mode");

        let events = app.world().resource::<Messages<AchievementUnlockedEvent>>();
        let mut cursor = events.get_cursor();
        let fired: Vec<String> = cursor.read(events).map(|e| e.0.id.clone()).collect();
        assert!(
            !fired.contains(&"zen_winner".to_string()),
            "zen_winner must not fire on a Classic-mode win; got {fired:?}"
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

    // -----------------------------------------------------------------------
    // Per-row tooltips
    // -----------------------------------------------------------------------

    /// Collects every `Tooltip` string attached to an `AchievementRow` in the
    /// current world. Order is unspecified — callers should search for a
    /// substring rather than rely on positions.
    fn collect_row_tooltips(app: &mut App) -> Vec<String> {
        let mut q = app
            .world_mut()
            .query_filtered::<&Tooltip, With<AchievementRow>>();
        q.iter(app.world())
            .map(|t| t.0.clone().into_owned())
            .collect()
    }

    /// `on_a_roll` is unlocked and has `Reward::CardBack(1)`. Its row's
    /// tooltip must surface that reward — the row UI already lists it, but
    /// the tooltip exists so the value is never just below the fold on
    /// long lists.
    #[test]
    fn unlocked_achievement_row_carries_tooltip_with_reward() {
        let mut app = headless_app();

        // Pre-unlock on_a_roll directly on the resource so the row renders
        // in the "unlocked" branch when the screen spawns.
        {
            let mut achievements = app.world_mut().resource_mut::<AchievementsResource>();
            let record = achievements
                .0
                .iter_mut()
                .find(|r| r.id == "on_a_roll")
                .expect("on_a_roll record must be seeded by AchievementPlugin");
            record.unlock(Utc::now());
            record.reward_granted = true;
        }

        press(&mut app, KeyCode::KeyA);
        app.update();

        let tips = collect_row_tooltips(&mut app);
        assert!(
            !tips.is_empty(),
            "spawning the achievements screen must attach Tooltips to rows"
        );

        // The reward for on_a_roll is `Card Back #1`. Find a tooltip
        // mentioning "Card back" (case-insensitive on "Back" → match the
        // exact format_reward output).
        let has_card_back_reward = tips.iter().any(|t| t.contains("Card Back"));
        assert!(
            has_card_back_reward,
            "expected an unlocked-row tooltip to mention the Card Back reward; got: {tips:?}"
        );
    }

    /// Locked secret achievements are filtered out of the row list, so the
    /// screen must not contain a row tooltip carrying the secret
    /// achievement's reward (`Card Back #4` for `speed_and_skill`) — the
    /// only fingerprint that would betray the row's identity even though
    /// the canonical description is already cryptic.
    #[test]
    fn locked_secret_achievement_does_not_reveal_condition() {
        let mut app = headless_app();

        // `speed_and_skill` starts locked under headless_app(); confirm.
        let locked = app
            .world()
            .resource::<AchievementsResource>()
            .0
            .iter()
            .find(|r| r.id == "speed_and_skill")
            .map(|r| !r.unlocked)
            .unwrap_or(false);
        assert!(
            locked,
            "precondition: speed_and_skill must be locked in a fresh headless app"
        );

        press(&mut app, KeyCode::KeyA);
        app.update();

        let tips = collect_row_tooltips(&mut app);
        // No row may carry the secret reward — that's the only way the
        // secret row's identity could leak through the tooltip surface.
        for t in &tips {
            assert!(
                !t.contains("Card Back #4"),
                "tooltip leaks the secret reward: {t:?}"
            );
        }

        // No row may quote the verbatim secret-condition vocabulary. The
        // canonical secret description in `solitaire_core` is already
        // generic ("A secret achievement"); these checks guard against a
        // future leak where someone replaces it with the literal predicate.
        let leaked_predicate = tips.iter().any(|t| {
            t.contains("90") && t.to_lowercase().contains("without undo")
        });
        assert!(
            !leaked_predicate,
            "no tooltip may state the speed_and_skill predicate: {tips:?}"
        );

        // Sanity: the screen actually rendered some rows. If the spawn
        // path were broken there'd be nothing to leak in the first place.
        assert!(!tips.is_empty(), "screen must have rendered rows");
    }

    // -----------------------------------------------------------------------
    // tooltip_for_row policy
    // -----------------------------------------------------------------------

    #[test]
    fn tooltip_for_row_unlocked_with_reward_mentions_reward() {
        let def = achievement_by_id("on_a_roll").expect("on_a_roll exists");
        let s = tooltip_for_row(true, Some(def));
        assert!(s.contains("Card Back"), "got {s:?}");
    }

    #[test]
    fn tooltip_for_row_unlocked_without_reward_says_earned() {
        let def = achievement_by_id("first_win").expect("first_win exists");
        assert_eq!(tooltip_for_row(true, Some(def)), "Earned!");
    }

    #[test]
    fn tooltip_for_row_locked_includes_description_and_reward() {
        let def = achievement_by_id("lightning").expect("lightning exists");
        let s = tooltip_for_row(false, Some(def));
        assert!(s.contains("How to unlock"));
        assert!(s.contains("under 90 seconds"));
        assert!(s.contains("Card Back #2"));
    }

    #[test]
    fn tooltip_for_row_locked_no_reward_omits_reward() {
        let def = achievement_by_id("first_win").expect("first_win exists");
        let s = tooltip_for_row(false, Some(def));
        assert!(s.contains("How to unlock"));
        assert!(!s.contains("Reward"), "got {s:?}");
    }

    // -----------------------------------------------------------------------
    // Achievement-onboarding cue (`fire_achievement_onboarding_toast`)
    // -----------------------------------------------------------------------

    /// Builds a headless app that **also** includes `SettingsPlugin::headless()`
    /// so the achievement-onboarding system (which reads `SettingsResource`)
    /// has a flag to consult and persist into.
    fn onboarding_test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(GamePlugin)
            .add_plugins(TablePlugin)
            .add_plugins(StatsPlugin::headless())
            .add_plugins(crate::progress_plugin::ProgressPlugin::headless())
            .add_plugins(crate::settings_plugin::SettingsPlugin::headless())
            .add_plugins(AchievementPlugin::headless());
        app.init_resource::<bevy::input::ButtonInput<KeyCode>>();
        app.update();
        app
    }

    /// Collects every `InfoToastEvent` written so tests can assert on
    /// count and message contents.
    fn drain_info_toasts(app: &App) -> Vec<String> {
        let events = app.world().resource::<Messages<InfoToastEvent>>();
        let mut cursor = events.get_cursor();
        cursor.read(events).map(|e| e.0.clone()).collect()
    }

    /// First-win path: with the flag false and `games_won` about to be
    /// 1, exactly one `InfoToastEvent` mentioning the `A` hotkey must
    /// fire and the flag must flip to `true`.
    #[test]
    fn first_win_fires_achievement_onboarding_toast() {
        let mut app = onboarding_test_app();

        // Sanity: fresh app starts with games_won = 0 and the flag unset.
        assert_eq!(app.world().resource::<StatsResource>().0.games_won, 0);
        assert!(
            !app.world()
                .resource::<SettingsResource>()
                .0
                .shown_achievement_onboarding
        );

        // StatsPlugin (StatsUpdate) increments games_won to 1 *before* the
        // achievement-onboarding system reads stats — our system runs
        // `.after(StatsUpdate)`. The system then sees games_won == 1 and
        // the cue fires.
        app.world_mut().write_message(GameWonEvent {
            score: 1000,
            time_seconds: 300,
        });
        app.update();

        let toasts = drain_info_toasts(&app);
        let onboarding_toasts: Vec<&String> = toasts
            .iter()
            .filter(|t| t.contains("Press A") && t.contains("achievements"))
            .collect();
        assert_eq!(
            onboarding_toasts.len(),
            1,
            "exactly one achievement-onboarding toast must fire on the first win; \
             saw all toasts: {toasts:?}"
        );
        assert!(
            app.world()
                .resource::<SettingsResource>()
                .0
                .shown_achievement_onboarding,
            "shown_achievement_onboarding must flip to true after the toast fires"
        );
    }

    /// Second-win path: with the flag already `true` (player already
    /// saw the cue on a previous run), no onboarding toast may fire.
    #[test]
    fn subsequent_wins_do_not_fire_achievement_onboarding_toast() {
        let mut app = onboarding_test_app();

        // Pre-set the flag to simulate a player who already dismissed
        // the cue on a previous run.
        app.world_mut()
            .resource_mut::<SettingsResource>()
            .0
            .shown_achievement_onboarding = true;

        app.world_mut().write_message(GameWonEvent {
            score: 1000,
            time_seconds: 300,
        });
        app.update();

        let onboarding_toasts: Vec<String> = drain_info_toasts(&app)
            .into_iter()
            .filter(|t| t.contains("Press A") && t.contains("achievements"))
            .collect();
        assert!(
            onboarding_toasts.is_empty(),
            "no onboarding toast must fire when shown_achievement_onboarding is already true; \
             got: {onboarding_toasts:?}"
        );
    }

    /// Sync-import path: a player imports stats with `games_won = 5`
    /// already on the books. The flag is still `false` (they were on a
    /// pre-cue release on this device), but the cue must NOT fire because
    /// this isn't actually their first win — the post-condition
    /// `games_won == 1` guards against retroactive nagging.
    #[test]
    fn non_first_win_does_not_fire_achievement_onboarding_toast() {
        let mut app = onboarding_test_app();

        // Pre-seed games_won = 5 BEFORE the win lands. StatsUpdate will
        // bump it to 6 on the GameWonEvent, taking the system well past
        // the `games_won == 1` post-condition.
        app.world_mut().resource_mut::<StatsResource>().0.games_won = 5;

        // Confirm the flag is still false so we know the guard that
        // prevents firing is the games-won post-condition, not the flag.
        assert!(
            !app.world()
                .resource::<SettingsResource>()
                .0
                .shown_achievement_onboarding
        );

        app.world_mut().write_message(GameWonEvent {
            score: 1000,
            time_seconds: 300,
        });
        app.update();

        let onboarding_toasts: Vec<String> = drain_info_toasts(&app)
            .into_iter()
            .filter(|t| t.contains("Press A") && t.contains("achievements"))
            .collect();
        assert!(
            onboarding_toasts.is_empty(),
            "no onboarding toast must fire on a non-first win; got: {onboarding_toasts:?}"
        );
        // And the flag must remain false so the cue can still teach a
        // genuinely-fresh second device or a wiped install.
        assert!(
            !app.world()
                .resource::<SettingsResource>()
                .0
                .shown_achievement_onboarding,
            "shown_achievement_onboarding must remain false when the cue did not fire"
        );
    }

    /// Without any `GameWonEvent` arriving the system must be a no-op:
    /// no toast, no flag flip — even on update ticks where stats happen
    /// to read `games_won == 1`.
    #[test]
    fn no_win_event_means_no_achievement_onboarding_toast() {
        let mut app = onboarding_test_app();

        // Pre-seed games_won = 1 to simulate the misleading mid-frame
        // state without actually firing a GameWonEvent.
        app.world_mut().resource_mut::<StatsResource>().0.games_won = 1;

        app.update();

        let onboarding_toasts: Vec<String> = drain_info_toasts(&app)
            .into_iter()
            .filter(|t| t.contains("Press A") && t.contains("achievements"))
            .collect();
        assert!(
            onboarding_toasts.is_empty(),
            "no onboarding toast must fire without a GameWonEvent; got: {onboarding_toasts:?}"
        );
        assert!(
            !app.world()
                .resource::<SettingsResource>()
                .0
                .shown_achievement_onboarding,
            "flag must not flip without a win event"
        );
    }
}
