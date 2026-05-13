//! Toggleable full-window profile overlay (press **P**).
//!
//! Shows the player's sync account, progression, achievements, and a statistics
//! summary in a single scrollable panel. Spawned on the first `P` keypress and
//! despawned on the second.

use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::input::ButtonInput;
use bevy::prelude::*;
use chrono::{Duration, Local, NaiveDate};
use solitaire_core::achievement::{achievement_by_id, ALL_ACHIEVEMENTS};
use solitaire_data::SyncBackend;

use crate::achievement_plugin::AchievementsResource;
use crate::events::ToggleProfileRequestEvent;
use crate::font_plugin::FontResource;
use crate::progress_plugin::ProgressResource;
use crate::resources::{SyncStatus, SyncStatusResource};
use crate::settings_plugin::SettingsResource;
use crate::stats_plugin::{format_fastest_win, format_win_rate, StatsResource};
use crate::ui_modal::{
    spawn_modal, spawn_modal_actions, spawn_modal_button, spawn_modal_header, ButtonVariant,
    ScrimDismissible,
};
use crate::ui_theme::{
    ACCENT_PRIMARY, BG_ELEVATED, BORDER_STRONG, SPACE_1, STATE_INFO, STATE_SUCCESS, TEXT_PRIMARY,
    TEXT_SECONDARY, TYPE_BODY, TYPE_BODY_LG, TYPE_CAPTION, VAL_SPACE_1, VAL_SPACE_2, Z_MODAL_PANEL,
};

/// Number of days surfaced in the daily-challenge calendar row.
///
/// 14 = trailing two weeks ending today. At ~12 px per dot with a 6 px gap
/// the row is ~246 px wide — well inside the 360 px minimum modal width on
/// the smallest supported window (800 px).
const CALENDAR_DAYS: usize = 14;

/// Diameter of each calendar dot, in pixels.
const CALENDAR_DOT_SIZE_PX: f32 = 12.0;

/// Marker component on the profile overlay root node.
#[derive(Component, Debug)]
pub struct ProfileScreen;

/// Marker on each daily-challenge calendar dot inside the Profile modal.
///
/// One entity per day in the trailing 14-day window — tests can query
/// for this component to assert the row was rendered.
#[derive(Component, Debug, Clone, Copy)]
pub struct DailyCalendarDot {
    /// The calendar date this dot represents.
    pub date: NaiveDate,
    /// Whether the player completed the daily challenge on `date`.
    pub completed: bool,
    /// `true` if `date == today` (the rightmost dot).
    pub is_today: bool,
}

/// Registers the `P` key toggle for the profile overlay.
pub struct ProfilePlugin;

/// Marker on the "Done" button inside the Profile modal.
#[derive(Component, Debug)]
pub struct ProfileCloseButton;

/// Marker on the scrollable body Node inside the Profile modal.
///
/// The Profile panel renders sync info, progression (incl. 14-day
/// calendar), every unlocked achievement (up to ~18), and a stats
/// summary, which can overflow the modal on the 800x600 minimum window
/// once a player has unlocked several achievements. This marker tags
/// the inner container that carries `Overflow::scroll_y()` plus a
/// `max_height` constraint. Mirrors the `SettingsPanelScrollable`
/// pattern.
#[derive(Component, Debug)]
pub struct ProfileScrollable;

impl Plugin for ProfilePlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<ToggleProfileRequestEvent>()
            // `MouseWheel` is emitted by Bevy's input plugin under
            // `DefaultPlugins`; register it explicitly so the
            // profile-scroll system also runs cleanly under
            // `MinimalPlugins` in tests.
            .add_message::<MouseWheel>()
            .add_systems(
                Update,
                (
                    toggle_profile_screen,
                    handle_profile_close_button,
                    scroll_profile_panel,
                ),
            );
    }
}

/// Routes mouse-wheel events into the Profile modal's scrollable body
/// while the panel is open. No-op when no `ProfileScrollable` exists in
/// the world (modal closed). Mirrors `scroll_settings_panel`.
fn scroll_profile_panel(
    mut scroll_evr: MessageReader<MouseWheel>,
    mut scrollables: Query<&mut ScrollPosition, With<ProfileScrollable>>,
) {
    if scrollables.is_empty() {
        scroll_evr.clear();
        return;
    }
    let delta_y: f32 = scroll_evr
        .read()
        .map(|ev| match ev.unit {
            MouseScrollUnit::Line => ev.y * 50.0,
            MouseScrollUnit::Pixel => ev.y,
        })
        .sum();
    if delta_y == 0.0 {
        return;
    }
    for mut sp in scrollables.iter_mut() {
        sp.0.y = (sp.0.y - delta_y).max(0.0);
    }
}

fn handle_profile_close_button(
    mut commands: Commands,
    close_buttons: Query<&Interaction, (With<ProfileCloseButton>, Changed<Interaction>)>,
    screens: Query<Entity, With<ProfileScreen>>,
) {
    if !close_buttons.iter().any(|i| *i == Interaction::Pressed) {
        return;
    }
    for entity in &screens {
        commands.entity(entity).despawn();
    }
}

#[allow(clippy::too_many_arguments)]
fn toggle_profile_screen(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    mut requests: MessageReader<ToggleProfileRequestEvent>,
    settings: Option<Res<SettingsResource>>,
    sync_status: Option<Res<SyncStatusResource>>,
    progress: Option<Res<ProgressResource>>,
    achievements: Option<Res<AchievementsResource>>,
    stats: Option<Res<StatsResource>>,
    font_res: Option<Res<FontResource>>,
    screens: Query<Entity, With<ProfileScreen>>,
) {
    let button_clicked = requests.read().count() > 0;
    let p_pressed = keys.just_pressed(KeyCode::KeyP);
    let esc_pressed = keys.just_pressed(KeyCode::Escape);
    let already_open = !screens.is_empty();
    // P / button toggles open-or-close. Esc only ever closes — when
    // Profile is layered over Home (clicking the new Home stats chip
    // opens this on top), Esc must dismiss the *topmost* modal.
    // Without this branch, Esc fell through to Home's cancel handler
    // and closed the wrong modal.
    let want_open = !already_open && (p_pressed || button_clicked);
    let want_close = already_open && (p_pressed || button_clicked || esc_pressed);
    if !want_open && !want_close {
        return;
    }
    if let Ok(entity) = screens.single() {
        commands.entity(entity).despawn();
    } else {
        spawn_profile_screen(
            &mut commands,
            settings.as_deref(),
            sync_status.as_deref(),
            progress.as_deref(),
            achievements.as_deref(),
            stats.as_deref(),
            font_res.as_deref(),
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
    font_res: Option<&FontResource>,
) {
    let font_handle = font_res.map(|f| f.0.clone()).unwrap_or_default();
    let font_section = TextFont {
        font: font_handle.clone(),
        font_size: TYPE_BODY_LG,
        ..default()
    };
    let font_row = TextFont {
        font: font_handle,
        font_size: TYPE_BODY,
        ..default()
    };

    let scrim = spawn_modal(commands, ProfileScreen, Z_MODAL_PANEL, |card| {
        spawn_modal_header(card, "Profile", font_res);

        // Scrollable body — the Profile panel renders sync info,
        // progression (incl. a 14-day calendar), every unlocked
        // achievement (up to ~18), and a stats summary, which can
        // overflow the modal on the 800x600 minimum window once the
        // player has unlocked several achievements. The Done action
        // stays fixed outside the scroll.
        card.spawn((
            ProfileScrollable,
            ScrollPosition::default(),
            Node {
                flex_direction: FlexDirection::Column,
                row_gap: VAL_SPACE_1,
                max_height: Val::Vh(70.0),
                overflow: Overflow::scroll_y(),
                ..default()
            },
        ))
        .with_children(|body| {
            // First-launch welcome — only when the player has zero XP and
            // zero daily streak, so the profile doesn't read as a wall of
            // zeros to a brand-new player.
            if let Some(p) = progress
                && p.0.total_xp == 0
                && p.0.daily_challenge_streak == 0
            {
                body.spawn((
                    Text::new("Welcome! Play games to earn XP and unlock achievements."),
                    font_section.clone(),
                    TextColor(ACCENT_PRIMARY),
                    Node {
                        margin: UiRect {
                            bottom: VAL_SPACE_2,
                            ..default()
                        },
                        ..default()
                    },
                ));
            }

            // ── Sync section ────────────────────────────────────────────
            body.spawn((
                Text::new("Sync"),
                font_section.clone(),
                TextColor(STATE_INFO),
            ));
            if let Some(s) = settings {
                let (backend_name, username) = sync_info(&s.0.sync_backend);
                body.spawn((
                    Text::new(format!("Account: {username}  |  Backend: {backend_name}")),
                    font_row.clone(),
                    TextColor(TEXT_PRIMARY),
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
                body.spawn((
                    Text::new(status_text),
                    font_row.clone(),
                    TextColor(TEXT_SECONDARY),
                ));
            }

            // ── Progression section ─────────────────────────────────────
            spawn_spacer(body, VAL_SPACE_2);
            body.spawn((
                Text::new("Progression"),
                font_section.clone(),
                TextColor(STATE_INFO),
            ));
            if let Some(p) = progress {
                let prog = &p.0;
                let (xp_span, xp_done) = xp_progress(prog.total_xp, prog.level);
                let pct = if xp_span == 0 {
                    100u64
                } else {
                    xp_done.saturating_mul(100).checked_div(xp_span).unwrap_or(100)
                };
                body.spawn((
                    Text::new(format!(
                        "Level {}  \u{2014}  {} XP  ({}/{} to next, {}%)",
                        prog.level, prog.total_xp, xp_done, xp_span, pct
                    )),
                    font_row.clone(),
                    TextColor(TEXT_PRIMARY),
                ));
                body.spawn((
                    Text::new(format!(
                        "Daily streak: {}  |  Card backs: {}  |  Backgrounds: {}",
                        prog.daily_challenge_streak,
                        prog.unlocked_card_backs.len(),
                        prog.unlocked_backgrounds.len(),
                    )),
                    font_row.clone(),
                    TextColor(TEXT_PRIMARY),
                ));

                // 14-day daily-challenge calendar row.
                spawn_daily_calendar(
                    body,
                    &prog.daily_challenge_history,
                    prog.daily_challenge_streak,
                    prog.daily_challenge_longest_streak,
                    Local::now().date_naive(),
                    font_res,
                );
            }

            // ── Achievements section ────────────────────────────────────
            spawn_spacer(body, VAL_SPACE_2);
            body.spawn((
                Text::new("Achievements"),
                font_section.clone(),
                TextColor(STATE_INFO),
            ));
            if let Some(ar) = achievements {
                let records = &ar.0;
                let unlocked_count = records.iter().filter(|r| r.unlocked).count();
                body.spawn((
                    Text::new(format!("{unlocked_count} / {} unlocked", ALL_ACHIEVEMENTS.len())),
                    font_row.clone(),
                    TextColor(ACCENT_PRIMARY),
                ));

                let mut any_unlocked = false;
                for record in records {
                    let def = achievement_by_id(record.id.as_str());
                    let is_secret = def.is_some_and(|d| d.secret);
                    if is_secret && !record.unlocked {
                        continue;
                    }
                    if !record.unlocked {
                        continue;
                    }
                    any_unlocked = true;
                    let name = def.map_or(record.id.as_str(), |d| d.name);
                    let date_str = match record.unlock_date {
                        Some(dt) => format!("  ({})", dt.format("%Y-%m-%d")),
                        None => String::new(),
                    };
                    body.spawn((
                        Text::new(format!("  [x] {name}{date_str}")),
                        font_row.clone(),
                        TextColor(STATE_SUCCESS),
                    ));
                }
                if !any_unlocked {
                    body.spawn((
                        Text::new("  No achievements unlocked yet."),
                        font_row.clone(),
                        TextColor(TEXT_SECONDARY),
                    ));
                }
            }

            // ── Statistics summary section ──────────────────────────────
            spawn_spacer(body, VAL_SPACE_2);
            body.spawn((
                Text::new("Statistics Summary"),
                font_section.clone(),
                TextColor(STATE_INFO),
            ));
            if let Some(sr) = stats {
                let s = &sr.0;
                let best_score_str = if s.best_single_score == 0 {
                    "\u{2014}".to_string()
                } else {
                    s.best_single_score.to_string()
                };
                body.spawn((
                    Text::new(format!(
                        "Played: {}  |  Won: {}  |  Win rate: {}  |  Best time: {}",
                        s.games_played,
                        s.games_won,
                        format_win_rate(s),
                        format_fastest_win(s.fastest_win_seconds),
                    )),
                    font_row.clone(),
                    TextColor(TEXT_PRIMARY),
                ));
                body.spawn((
                    Text::new(format!(
                        "Win streak: {} current, {} best  |  Best score: {}",
                        s.win_streak_current, s.win_streak_best, best_score_str,
                    )),
                    font_row.clone(),
                    TextColor(TEXT_PRIMARY),
                ));
            }
        });

        spawn_modal_actions(card, |actions| {
            spawn_modal_button(
                actions,
                ProfileCloseButton,
                "Done",
                Some("P"),
                ButtonVariant::Primary,
                font_res,
            );
        });
    });
    // Profile is read-only — opt into click-outside-to-dismiss.
    commands.entity(scrim).insert(ScrimDismissible);
}

/// Spawn a fixed-height vertical spacer node.
fn spawn_spacer(parent: &mut ChildSpawnerCommands, height: Val) {
    parent.spawn(Node {
        height,
        ..default()
    });
}

/// Spawn the daily-challenge calendar row: a caption + 14 dots.
///
/// `history` is the player's full chronological completion history.
/// `current_streak` and `longest_streak` are surfaced in the caption.
/// `today` is passed in (rather than read directly) so the function is
/// trivially testable with a fixed reference date.
///
/// Layout: caption row → row of 14 dots (~12 px each, 6 px gap). The
/// rightmost dot represents today; past dots fill from oldest (left) to
/// most recent (right). Each dot carries a [`DailyCalendarDot`] marker.
fn spawn_daily_calendar(
    parent: &mut ChildSpawnerCommands,
    history: &[NaiveDate],
    current_streak: u32,
    longest_streak: u32,
    today: NaiveDate,
    font_res: Option<&FontResource>,
) {
    use std::collections::HashSet;
    let history_set: HashSet<NaiveDate> = history.iter().copied().collect();

    let font_caption = TextFont {
        font: font_res.map(|f| f.0.clone()).unwrap_or_default(),
        font_size: TYPE_CAPTION,
        ..default()
    };

    parent.spawn((
        Text::new(format!(
            "Current streak: {current_streak}  \u{00B7}  Longest: {longest_streak}"
        )),
        font_caption,
        TextColor(TEXT_SECONDARY),
        Node {
            margin: UiRect {
                top: VAL_SPACE_1,
                bottom: VAL_SPACE_1,
                ..default()
            },
            ..default()
        },
    ));

    parent
        .spawn(Node {
            flex_direction: FlexDirection::Row,
            column_gap: Val::Px(SPACE_1 + 2.0), // 6 px between dots
            align_items: AlignItems::Center,
            ..default()
        })
        .with_children(|row| {
            // Iterate from oldest (today − 13) to today (rightmost).
            for offset in (0..CALENDAR_DAYS as i64).rev() {
                let date = today - Duration::days(offset);
                let is_today = offset == 0;
                let completed = history_set.contains(&date);
                // Today's dot keeps the outlined-ring look (Balatro-yellow
                // accent border) regardless of completion; past days use a
                // subtle border so the row reads as a row of pills, not a
                // strip of bare squares.
                let border_color = if is_today { ACCENT_PRIMARY } else { BORDER_STRONG };
                let border_width = if is_today { 2.0 } else { 0.0 };
                row.spawn((
                    DailyCalendarDot {
                        date,
                        completed,
                        is_today,
                    },
                    Node {
                        width: Val::Px(CALENDAR_DOT_SIZE_PX),
                        height: Val::Px(CALENDAR_DOT_SIZE_PX),
                        border: UiRect::all(Val::Px(border_width)),
                        border_radius: BorderRadius::all(Val::Px(CALENDAR_DOT_SIZE_PX / 2.0)),
                        ..default()
                    },
                    BackgroundColor(calendar_dot_color(completed)),
                    BorderColor::all(border_color),
                ));
            }
        });
}

/// Background colour for a calendar dot. `STATE_SUCCESS` for completed
/// days, `BG_ELEVATED` for missed/pending days.
fn calendar_dot_color(completed: bool) -> Color {
    if completed {
        STATE_SUCCESS
    } else {
        BG_ELEVATED
    }
}

/// Return `(backend_name, username_display)` for the given sync backend.
fn sync_info(backend: &SyncBackend) -> (&'static str, String) {
    match backend {
        SyncBackend::Local => ("Local", "—".to_string()),
        SyncBackend::SolitaireServer { username, .. } => {
            ("Solitaire Server", username.clone())
        }
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
    fn profile_modal_body_is_scrollable() {
        let mut app = headless_app();
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyP);
        app.update();

        let count = app
            .world_mut()
            .query::<&ProfileScrollable>()
            .iter(app.world())
            .count();
        assert_eq!(
            count, 1,
            "Profile modal must spawn exactly one ProfileScrollable body"
        );

        let mut q = app
            .world_mut()
            .query_filtered::<&Node, With<ProfileScrollable>>();
        let nodes: Vec<&Node> = q.iter(app.world()).collect();
        assert_ne!(
            nodes[0].max_height,
            Val::Auto,
            "scrollable body must set a non-default max_height"
        );
        assert_eq!(nodes[0].overflow, Overflow::scroll_y());
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

    #[test]
    fn profile_modal_renders_14_calendar_dots() {
        // Open the Profile modal and assert the 14-day calendar row was
        // populated with one DailyCalendarDot entity per day.
        let mut app = headless_app();
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyP);
        app.update();

        let dot_count = app
            .world_mut()
            .query::<&DailyCalendarDot>()
            .iter(app.world())
            .count();
        assert_eq!(
            dot_count, CALENDAR_DAYS,
            "Profile modal must render exactly {CALENDAR_DAYS} calendar dots"
        );
    }

    #[test]
    fn calendar_dot_today_marker_is_set_on_rightmost_dot_only() {
        // Exactly one of the 14 dots is the "today" dot (the rightmost).
        let mut app = headless_app();
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyP);
        app.update();

        let today_count = app
            .world_mut()
            .query::<&DailyCalendarDot>()
            .iter(app.world())
            .filter(|d| d.is_today)
            .count();
        assert_eq!(today_count, 1, "exactly one dot must be marked is_today");
    }
}
