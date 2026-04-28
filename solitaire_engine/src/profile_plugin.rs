//! Toggleable full-window profile overlay (press **P**).
//!
//! Shows the player's sync account, progression, achievements, and a statistics
//! summary in a single scrollable panel. Spawned on the first `P` keypress and
//! despawned on the second.

use bevy::input::ButtonInput;
use bevy::prelude::*;
use solitaire_core::achievement::achievement_by_id;
use solitaire_data::SyncBackend;

use crate::achievement_plugin::AchievementsResource;
use crate::progress_plugin::ProgressResource;
use crate::resources::{SyncStatus, SyncStatusResource};
use crate::settings_plugin::SettingsResource;
use crate::stats_plugin::{format_fastest_win, format_win_rate, StatsResource};

/// Marker component on the profile overlay root node.
#[derive(Component, Debug)]
pub struct ProfileScreen;

/// Registers the `P` key toggle for the profile overlay.
pub struct ProfilePlugin;

impl Plugin for ProfilePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, toggle_profile_screen);
    }
}

#[allow(clippy::too_many_arguments)]
fn toggle_profile_screen(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    settings: Option<Res<SettingsResource>>,
    sync_status: Option<Res<SyncStatusResource>>,
    progress: Option<Res<ProgressResource>>,
    achievements: Option<Res<AchievementsResource>>,
    stats: Option<Res<StatsResource>>,
    screens: Query<Entity, With<ProfileScreen>>,
) {
    if !keys.just_pressed(KeyCode::KeyP) {
        return;
    }
    if let Ok(entity) = screens.get_single() {
        commands.entity(entity).despawn_recursive();
    } else {
        spawn_profile_screen(
            &mut commands,
            settings.as_deref(),
            sync_status.as_deref(),
            progress.as_deref(),
            achievements.as_deref(),
            stats.as_deref(),
        );
    }
}

fn spawn_profile_screen(
    commands: &mut Commands,
    settings: Option<&SettingsResource>,
    sync_status: Option<&SyncStatusResource>,
    progress: Option<&ProgressResource>,
    achievements: Option<&AchievementsResource>,
    stats: Option<&StatsResource>,
) {
    commands
        .spawn((
            ProfileScreen,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Percent(0.0),
                top: Val::Percent(0.0),
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::FlexStart,
                align_items: AlignItems::Center,
                row_gap: Val::Px(4.0),
                padding: UiRect::all(Val::Px(24.0)),
                overflow: Overflow::clip(),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.88)),
            ZIndex(200),
        ))
        .with_children(|root| {
            // ── Title ────────────────────────────────────────────────────────
            root.spawn((
                Text::new("Profile"),
                TextFont { font_size: 28.0, ..default() },
                TextColor(Color::srgb(1.0, 0.85, 0.3)),
            ));

            // ── Sync section ─────────────────────────────────────────────────
            if let Some(s) = settings {
                let (backend_name, username) = sync_info(&s.0.sync_backend);
                root.spawn((
                    Text::new(format!("Account: {username}  |  Backend: {backend_name}")),
                    TextFont { font_size: 17.0, ..default() },
                    TextColor(Color::srgb(0.7, 0.9, 1.0)),
                ));
            }
            if let Some(ss) = sync_status {
                let status_text = match &ss.0 {
                    SyncStatus::Idle => "Sync: idle".to_string(),
                    SyncStatus::Syncing => "Sync: syncing\u{2026}".to_string(),
                    SyncStatus::LastSynced(dt) => {
                        format!("Last synced: {}", dt.format("%Y-%m-%d %H:%M"))
                    }
                    SyncStatus::Error(e) => format!("Sync error: {e}"),
                };
                root.spawn((
                    Text::new(status_text),
                    TextFont { font_size: 15.0, ..default() },
                    TextColor(Color::srgb(0.7, 0.7, 0.7)),
                ));
            }

            // ── Progression section ───────────────────────────────────────────
            spawn_spacer(root, 4.0);
            root.spawn((
                Text::new("Progression"),
                TextFont { font_size: 22.0, ..default() },
                TextColor(Color::srgb(0.8, 0.9, 0.8)),
            ));
            if let Some(p) = progress {
                let prog = &p.0;
                let (xp_span, xp_done) = xp_progress(prog.total_xp, prog.level);
                let pct = if xp_span == 0 {
                    100u64
                } else {
                    xp_done.saturating_mul(100).checked_div(xp_span).unwrap_or(100)
                };
                root.spawn((
                    Text::new(format!(
                        "Level {}  \u{2014}  {} XP  ({}/{} to next, {}%)",
                        prog.level, prog.total_xp, xp_done, xp_span, pct
                    )),
                    TextFont { font_size: 17.0, ..default() },
                    TextColor(Color::srgb(0.85, 0.85, 0.85)),
                ));
                root.spawn((
                    Text::new(format!(
                        "Daily streak: {}  |  Card backs: {}  |  Backgrounds: {}",
                        prog.daily_challenge_streak,
                        prog.unlocked_card_backs.len(),
                        prog.unlocked_backgrounds.len(),
                    )),
                    TextFont { font_size: 17.0, ..default() },
                    TextColor(Color::srgb(0.85, 0.85, 0.85)),
                ));
            }

            // ── Achievements section ──────────────────────────────────────────
            spawn_spacer(root, 4.0);
            root.spawn((
                Text::new("Achievements"),
                TextFont { font_size: 22.0, ..default() },
                TextColor(Color::srgb(0.8, 0.9, 0.8)),
            ));
            if let Some(ar) = achievements {
                let records = &ar.0;
                let unlocked_count = records.iter().filter(|r| r.unlocked).count();
                root.spawn((
                    Text::new(format!("{} / 18 unlocked", unlocked_count)),
                    TextFont { font_size: 17.0, ..default() },
                    TextColor(Color::srgb(1.0, 0.85, 0.4)),
                ));

                let mut any_unlocked = false;
                for record in records {
                    let def = achievement_by_id(record.id.as_str());
                    // Skip secret achievements that are not unlocked.
                    let is_secret = def.map(|d| d.secret).unwrap_or(false);
                    if is_secret && !record.unlocked {
                        continue;
                    }
                    if !record.unlocked {
                        continue;
                    }
                    any_unlocked = true;
                    let name = def.map(|d| d.name).unwrap_or(record.id.as_str());
                    let date_str = match record.unlock_date {
                        Some(dt) => format!("  ({})", dt.format("%Y-%m-%d")),
                        None => String::new(),
                    };
                    root.spawn((
                        Text::new(format!("  [x] {name}{date_str}")),
                        TextFont { font_size: 14.0, ..default() },
                        TextColor(Color::srgb(0.7, 1.0, 0.7)),
                    ));
                }
                if !any_unlocked {
                    root.spawn((
                        Text::new("  No achievements unlocked yet."),
                        TextFont { font_size: 14.0, ..default() },
                        TextColor(Color::srgb(0.7, 0.7, 0.7)),
                    ));
                }
            }

            // ── Statistics summary section ────────────────────────────────────
            spawn_spacer(root, 4.0);
            root.spawn((
                Text::new("Statistics Summary"),
                TextFont { font_size: 22.0, ..default() },
                TextColor(Color::srgb(0.8, 0.9, 0.8)),
            ));
            if let Some(sr) = stats {
                let s = &sr.0;
                let best_score_str = if s.best_single_score == 0 {
                    "\u{2014}".to_string()
                } else {
                    s.best_single_score.to_string()
                };
                root.spawn((
                    Text::new(format!(
                        "Played: {}  |  Won: {}  |  Win rate: {}  |  Best time: {}",
                        s.games_played,
                        s.games_won,
                        format_win_rate(s),
                        format_fastest_win(s.fastest_win_seconds),
                    )),
                    TextFont { font_size: 16.0, ..default() },
                    TextColor(Color::srgb(0.85, 0.85, 0.85)),
                ));
                root.spawn((
                    Text::new(format!(
                        "Win streak: {} current, {} best  |  Best score: {}",
                        s.win_streak_current, s.win_streak_best, best_score_str,
                    )),
                    TextFont { font_size: 16.0, ..default() },
                    TextColor(Color::srgb(0.85, 0.85, 0.85)),
                ));
            }

            // ── Dismiss hint ──────────────────────────────────────────────────
            spawn_spacer(root, 8.0);
            root.spawn((
                Text::new("Press P to close"),
                TextFont { font_size: 16.0, ..default() },
                TextColor(Color::srgb(0.55, 0.55, 0.55)),
            ));
        });
}

/// Spawn a fixed-height vertical spacer node.
fn spawn_spacer(parent: &mut ChildBuilder, height_px: f32) {
    parent.spawn(Node {
        height: Val::Px(height_px),
        ..default()
    });
}

/// Return `(backend_name, username_display)` for the given sync backend.
fn sync_info(backend: &SyncBackend) -> (&'static str, String) {
    match backend {
        SyncBackend::Local => ("Local", "—".to_string()),
        SyncBackend::SolitaireServer { username, .. } => {
            ("Solitaire Server", username.clone())
        }
        SyncBackend::GooglePlayGames => ("Google Play Games", "—".to_string()),
    }
}

/// Return `(xp_span_for_level, xp_done_in_level)` for the given `total_xp` and `level`.
///
/// Levels 1–10 each require 500 XP; levels 11+ each require 1 000 XP.
fn xp_progress(total_xp: u64, level: u32) -> (u64, u64) {
    let level_start = if level < 10 {
        level as u64 * 500
    } else {
        5_000 + (level as u64 - 10) * 1_000
    };
    let xp_span: u64 = if level < 10 { 500 } else { 1_000 };
    let xp_done = total_xp.saturating_sub(level_start).min(xp_span);
    (xp_span, xp_done)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::achievement_plugin::AchievementPlugin;
    use crate::game_plugin::GamePlugin;
    use crate::progress_plugin::ProgressPlugin;
    use crate::settings_plugin::SettingsPlugin;
    use crate::stats_plugin::StatsPlugin;
    use crate::table_plugin::TablePlugin;

    fn headless_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(GamePlugin)
            .add_plugins(TablePlugin)
            .add_plugins(StatsPlugin::headless())
            .add_plugins(ProgressPlugin::headless())
            .add_plugins(AchievementPlugin::headless())
            .add_plugins(SettingsPlugin::headless())
            .add_plugins(ProfilePlugin);
        app.init_resource::<ButtonInput<KeyCode>>();
        app.update();
        app
    }

    #[test]
    fn pressing_p_spawns_profile_screen() {
        let mut app = headless_app();
        assert_eq!(
            app.world_mut()
                .query::<&ProfileScreen>()
                .iter(app.world())
                .count(),
            0
        );
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyP);
        app.update();
        assert_eq!(
            app.world_mut()
                .query::<&ProfileScreen>()
                .iter(app.world())
                .count(),
            1
        );
    }

    #[test]
    fn pressing_p_twice_closes_profile_screen() {
        let mut app = headless_app();
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyP);
        app.update();

        {
            let mut input = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            input.release(KeyCode::KeyP);
            input.clear();
            input.press(KeyCode::KeyP);
        }
        app.update();

        assert_eq!(
            app.world_mut()
                .query::<&ProfileScreen>()
                .iter(app.world())
                .count(),
            0
        );
    }

    #[test]
    fn xp_progress_at_zero() {
        assert_eq!(xp_progress(0, 0), (500, 0));
    }

    #[test]
    fn xp_progress_halfway_through_level_1() {
        // Level 1 starts at 500 XP; span is 500.  At 750 XP: done = 250.
        assert_eq!(xp_progress(750, 1), (500, 250));
    }

    #[test]
    fn xp_progress_at_level_10() {
        // Level 10 is the first post-table level (span = 1000, starts at 5000).
        assert_eq!(xp_progress(5_000, 10), (1_000, 0));
    }
}
