//! Loads, updates, and persists `StatsSnapshot` in response to game events,
//! and provides a toggleable full-window stats overlay (press `S`).
//!
//! The persistence path is configurable via `StatsPlugin::storage_path`.
//! In production, `StatsPlugin::default()` loads/saves from the platform
//! data dir. In tests, use `StatsPlugin::headless()` to disable all file
//! I/O so the user's real stats file is neither read nor overwritten.

use std::path::PathBuf;

use bevy::input::ButtonInput;
use bevy::prelude::*;
use solitaire_data::{
    latest_replay_path, load_latest_replay_from, load_stats_from, save_stats_to, stats_file_path,
    PlayerProgress, Replay, StatsExt, StatsSnapshot, WEEKLY_GOALS,
};

use crate::auto_complete_plugin::AutoCompleteState;
use crate::challenge_plugin::challenge_progress_label;
use crate::events::{
    ForfeitEvent, GameWonEvent, InfoToastEvent, NewGameRequestEvent, ToggleStatsRequestEvent,
    WinStreakMilestoneEvent,
};
use crate::game_plugin::GameMutation;
use crate::progress_plugin::ProgressResource;
use crate::font_plugin::FontResource;
use crate::resources::GameStateResource;
use crate::time_attack_plugin::TimeAttackResource;
use crate::ui_modal::{
    spawn_modal, spawn_modal_actions, spawn_modal_button, spawn_modal_header, ButtonVariant,
};
use crate::ui_theme::{
    ACCENT_PRIMARY, BORDER_SUBTLE, RADIUS_SM, STATE_INFO, STATE_WARNING, STREAK_MILESTONES,
    TEXT_PRIMARY, TEXT_SECONDARY, TYPE_BODY, TYPE_BODY_LG, TYPE_CAPTION, TYPE_HEADLINE, VAL_SPACE_2,
    VAL_SPACE_3, VAL_SPACE_4, Z_MODAL_PANEL,
};

/// Bevy resource wrapping the current stats.
#[derive(Resource, Debug, Clone)]
pub struct StatsResource(pub StatsSnapshot);

/// Persistence path for `StatsResource`. `None` disables I/O.
#[derive(Resource, Debug, Clone)]
pub struct StatsStoragePath(pub Option<PathBuf>);

/// System set for the stats-mutating systems. Downstream plugins that read
/// `StatsResource` after a win/abandon should run `.after(StatsUpdate)`.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct StatsUpdate;

/// Marker component on the stats overlay root node.
#[derive(Component, Debug)]
pub struct StatsScreen;

/// Marker component on an individual stat cell inside the stats overlay.
///
/// Each cell contains a large value label and a small descriptor label below it.
#[derive(Component, Debug)]
pub struct StatsCell;

/// Resource holding the most recently loaded winning [`Replay`], if any.
///
/// Populated from `<data_dir>/solitaire_quest/latest_replay.json` at
/// startup and refreshed in-place whenever the engine writes a new
/// winning replay (the path the Stats UI calls into is unchanged so a
/// re-open of the modal sees the latest record).
///
/// The Stats overlay reads this to decide whether to render the
/// "Watch replay" call-to-action or the "No replay recorded yet"
/// caption.
#[derive(Resource, Debug, Default, Clone)]
pub struct LatestReplayResource(pub Option<Replay>);

/// Persistence path for the latest winning replay file. `None` disables
/// I/O — used by tests and by `StatsPlugin::headless`.
#[derive(Resource, Debug, Clone)]
pub struct LatestReplayPath(pub Option<PathBuf>);

/// Marker on the "Watch replay" button inside the Stats modal. Clicking
/// it currently fires an [`InfoToastEvent`] indicating playback ships
/// in a future build — see [`handle_watch_replay_button`].
#[derive(Component, Debug)]
pub struct WatchReplayButton;

/// Marker component on each per-mode bests row in the stats overlay.
///
/// One row per supported [`solitaire_core::game_state::GameMode`] (Classic,
/// Zen, Challenge — Time Attack and Daily are intentionally excluded; see
/// `StatsSnapshot` doc comments). Tests query by this marker to assert the
/// per-mode section rendered.
#[derive(Component, Debug)]
pub struct PerModeBestsRow;

/// Registers stats resources, update systems, and the UI toggle.
pub struct StatsPlugin {
    /// Where to persist stats. `None` disables all file I/O (for tests).
    pub storage_path: Option<PathBuf>,
}

impl Default for StatsPlugin {
    fn default() -> Self {
        Self {
            storage_path: stats_file_path(),
        }
    }
}

impl StatsPlugin {
    /// Plugin configured with no persistence. Use in tests and headless apps
    /// where touching `~/.local/share/solitaire_quest/stats.json` would be
    /// incorrect.
    pub fn headless() -> Self {
        Self { storage_path: None }
    }
}

impl Plugin for StatsPlugin {
    fn build(&self, app: &mut App) {
        let loaded = match &self.storage_path {
            Some(path) => load_stats_from(path),
            None => StatsSnapshot::default(),
        };
        // Replay file lives next to stats.json — when the StatsPlugin
        // is in headless mode (storage_path = None), we mirror that
        // policy and disable replay I/O too. Otherwise resolve the
        // platform-default path via `latest_replay_path()`.
        let replay_path = self.storage_path.as_ref().and(latest_replay_path());
        let initial_replay = replay_path
            .as_deref()
            .and_then(load_latest_replay_from);
        app.insert_resource(StatsResource(loaded))
            .insert_resource(StatsStoragePath(self.storage_path.clone()))
            .insert_resource(LatestReplayResource(initial_replay))
            .insert_resource(LatestReplayPath(replay_path))
            .add_message::<GameWonEvent>()
            .add_message::<NewGameRequestEvent>()
            .add_message::<ForfeitEvent>()
            .add_message::<InfoToastEvent>()
            .add_message::<ToggleStatsRequestEvent>()
            .add_message::<WinStreakMilestoneEvent>()
            // record_abandoned must read `move_count` BEFORE handle_new_game
            // clobbers it with a fresh game. These are NOT in StatsUpdate because
            // StatsUpdate (as a set) is ordered after GameMutation by external
            // constraints (win_summary_plugin: cache_win_data.before(StatsUpdate)),
            // and a system cannot be both inside a set and individually before a
            // set-level ordering constraint.
            .add_systems(
                Update,
                update_stats_on_new_game.before(GameMutation),
            )
            .add_systems(
                Update,
                update_stats_on_win.after(GameMutation).in_set(StatsUpdate),
            )
            .add_systems(
                Update,
                handle_forfeit.before(GameMutation),
            )
            .add_systems(Update, toggle_stats_screen.after(GameMutation))
            .add_systems(Update, handle_stats_close_button)
            .add_systems(
                Update,
                refresh_latest_replay_on_win.after(GameMutation),
            )
            .add_systems(Update, handle_watch_replay_button);
    }
}

/// After a win, the engine has just persisted a fresh winning replay.
/// Re-load it so the next time the player opens the Stats overlay, the
/// "Watch replay" call-to-action reflects the most recent victory
/// rather than an older session.
fn refresh_latest_replay_on_win(
    mut wins: MessageReader<GameWonEvent>,
    mut latest: ResMut<LatestReplayResource>,
    path: Res<LatestReplayPath>,
) {
    // Only re-load when at least one win actually fired.
    if wins.read().next().is_none() {
        return;
    }
    let Some(p) = path.0.as_deref() else {
        return;
    };
    latest.0 = load_latest_replay_from(p);
}

/// Click handler for the "Watch replay" button.
///
/// Starts in-engine replay playback when the Watch Replay button is
/// pressed. If no replay has been recorded yet, surfaces an
/// [`InfoToastEvent`] instead. The playback path resets the live
/// game to the recorded deal and ticks through the move list via
/// [`crate::replay_playback`]; the [`crate::replay_overlay`] banner
/// surfaces while playback runs.
fn handle_watch_replay_button(
    mut commands: Commands,
    buttons: Query<&Interaction, (With<WatchReplayButton>, Changed<Interaction>)>,
    latest: Res<LatestReplayResource>,
    playback: Option<ResMut<crate::replay_playback::ReplayPlaybackState>>,
    mut toast: MessageWriter<InfoToastEvent>,
) {
    if !buttons.iter().any(|i| *i == Interaction::Pressed) {
        return;
    }
    match (&latest.0, playback) {
        (Some(replay), Some(mut playback)) => {
            crate::replay_playback::start_replay_playback(
                &mut commands,
                &mut playback,
                replay.clone(),
            );
        }
        (Some(replay), None) => {
            // ReplayPlaybackPlugin not registered (headless test
            // fixtures); fall back to a descriptive toast.
            toast.write(InfoToastEvent(format!(
                "Replay ready ({})",
                format_replay_caption(replay)
            )));
        }
        (None, _) => {
            toast.write(InfoToastEvent(
                "No replay recorded yet \u{2014} win a game first.".to_string(),
            ));
        }
    }
}

/// Pure helper: render a one-line caption for a [`Replay`] suitable
/// for the Stats overlay button label and the "Replay loaded" toast.
///
/// Format: `"M:SS win on YYYY-MM-DD"`. For a 134-second win recorded
/// on 2026-05-02, returns `"2:14 win on 2026-05-02"`.
pub fn format_replay_caption(replay: &Replay) -> String {
    format!(
        "{} win on {}",
        format_duration(replay.time_seconds),
        replay.recorded_at,
    )
}

fn persist(path: &StatsStoragePath, stats: &StatsSnapshot, context: &str) {
    let Some(target) = &path.0 else {
        return;
    };
    if let Err(e) = save_stats_to(target, stats) {
        warn!("failed to save stats after {context}: {e}");
    }
}

fn update_stats_on_win(
    mut events: MessageReader<GameWonEvent>,
    game: Res<GameStateResource>,
    mut stats: ResMut<StatsResource>,
    path: Res<StatsStoragePath>,
    mut milestone: MessageWriter<WinStreakMilestoneEvent>,
    mut toast: MessageWriter<InfoToastEvent>,
) {
    for ev in events.read() {
        let prev_streak = stats.0.win_streak_current;
        stats
            .0
            .update_on_win(ev.score, ev.time_seconds, &game.0.draw_mode);
        // Per-mode best score / fastest win — additive on top of the
        // lifetime totals tracked by `update_on_win`. TimeAttack is a
        // no-op inside the helper because it has its own session-level
        // scoring model.
        stats
            .0
            .update_per_mode_bests(ev.score, ev.time_seconds, game.0.mode);
        let new_streak = stats.0.win_streak_current;
        // Fire the streak-milestone event only on the threshold
        // crossing — `prev < threshold && new >= threshold`. This
        // guarantees the flourish never retriggers at every win past
        // the highest milestone.
        if let Some(crossed) = streak_milestone_crossed(prev_streak, new_streak) {
            milestone.write(WinStreakMilestoneEvent { streak: crossed });
            toast.write(InfoToastEvent(format!(
                "Win streak: {crossed}! \u{1F525}"
            )));
        }
        persist(&path, &stats.0, "win");
    }
}

/// Returns the milestone value that the player just crossed, if any.
///
/// A milestone is "crossed" when `prev < threshold && new >= threshold`
/// for some `threshold` in [`STREAK_MILESTONES`]. Returns the largest
/// such threshold (so a single win that vaults the player from a
/// streak of 0 directly to 5 — implausible, but defensive — fires the
/// most-celebrated milestone, not the smallest).
///
/// Returns `None` when no threshold was crossed, i.e. either:
/// - the streak did not change,
/// - the streak rose but stayed below every threshold, or
/// - the streak rose past a threshold that `prev` was already at or
///   above.
///
/// Pure function exposed for unit testing without Bevy.
pub fn streak_milestone_crossed(prev: u32, new: u32) -> Option<u32> {
    if new <= prev {
        return None;
    }
    STREAK_MILESTONES
        .iter()
        .copied()
        .filter(|&t| prev < t && new >= t)
        .max()
}

fn update_stats_on_new_game(
    mut events: MessageReader<NewGameRequestEvent>,
    game: Res<GameStateResource>,
    mut stats: ResMut<StatsResource>,
    path: Res<StatsStoragePath>,
    mut toast: MessageWriter<InfoToastEvent>,
) {
    for _ in events.read() {
        if game.0.move_count > 0 && !game.0.is_won {
            let streak = stats.0.win_streak_current;
            stats.0.record_abandoned();
            persist(&path, &stats.0, "abandoned game");
            if streak > 1 {
                toast.write(InfoToastEvent(format!("Streak of {streak} broken!")));
            }
        }
    }
}

/// When the player presses G to forfeit, record the game as abandoned, save
/// stats, fire an informational toast, and start a new game.
///
/// `AutoCompleteState` is reset here so the "AUTO" badge and chime do not bleed
/// into the new deal (task #41).
fn handle_forfeit(
    mut events: MessageReader<ForfeitEvent>,
    game: Res<GameStateResource>,
    mut stats: ResMut<StatsResource>,
    path: Res<StatsStoragePath>,
    mut new_game: MessageWriter<NewGameRequestEvent>,
    mut toast: MessageWriter<InfoToastEvent>,
    mut auto_complete: Option<ResMut<AutoCompleteState>>,
) {
    for _ in events.read() {
        if game.0.move_count > 0 && !game.0.is_won {
            let streak = stats.0.win_streak_current;
            stats.0.record_abandoned();
            persist(&path, &stats.0, "forfeit");
            if streak > 1 {
                toast.write(InfoToastEvent(format!("Streak of {streak} broken!")));
            }
        }
        // Reset auto-complete so the badge and chime don't carry over to the
        // new game that is about to start.
        if let Some(ref mut ac) = auto_complete {
            **ac = AutoCompleteState::default();
        }
        toast.write(InfoToastEvent("Game forfeited".to_string()));
        new_game.write(NewGameRequestEvent::default());
    }
}

/// Marker on the "Done" button inside the Stats modal. Click despawns
/// the overlay; `S` keyboard shortcut toggles it the same way.
#[derive(Component, Debug)]
pub struct StatsCloseButton;

#[allow(clippy::too_many_arguments)]
fn toggle_stats_screen(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    mut requests: MessageReader<ToggleStatsRequestEvent>,
    stats: Res<StatsResource>,
    progress: Option<Res<ProgressResource>>,
    time_attack: Option<Res<TimeAttackResource>>,
    font_res: Option<Res<FontResource>>,
    latest_replay: Res<LatestReplayResource>,
    screens: Query<Entity, With<StatsScreen>>,
) {
    let button_clicked = requests.read().count() > 0;
    if !keys.just_pressed(KeyCode::KeyS) && !button_clicked {
        return;
    }
    if let Ok(entity) = screens.single() {
        commands.entity(entity).despawn();
    } else {
        spawn_stats_screen(
            &mut commands,
            &stats.0,
            progress.as_deref().map(|p| &p.0),
            time_attack.as_deref(),
            font_res.as_deref(),
            latest_replay.0.as_ref(),
        );
    }
}

/// Click handler for the modal's "Done" button — despawns the overlay
/// the same way the `S` accelerator does.
fn handle_stats_close_button(
    mut commands: Commands,
    close_buttons: Query<&Interaction, (With<StatsCloseButton>, Changed<Interaction>)>,
    screens: Query<Entity, With<StatsScreen>>,
) {
    if !close_buttons.iter().any(|i| *i == Interaction::Pressed) {
        return;
    }
    for entity in &screens {
        commands.entity(entity).despawn();
    }
}

fn spawn_stats_screen(
    commands: &mut Commands,
    stats: &StatsSnapshot,
    progress: Option<&PlayerProgress>,
    time_attack: Option<&TimeAttackResource>,
    font_res: Option<&FontResource>,
    latest_replay: Option<&Replay>,
) {
    // --- primary stat cells ---
    // First-launch zero-state: when no games have been played yet, render
    // every top-level cell as an em-dash so the panel doesn't read as a
    // mix of "0" counters and "—" sentinels (which feels buggy).
    let is_first_launch = stats.games_played == 0;
    let dash = "\u{2014}".to_string();
    let win_rate_str    = if is_first_launch { dash.clone() } else { format_win_rate(stats) };
    let played_str      = if is_first_launch { dash.clone() } else { format_stat_value(stats.games_played) };
    let won_str         = if is_first_launch { dash.clone() } else { format_stat_value(stats.games_won) };
    let lost_str        = if is_first_launch { dash.clone() } else { format_stat_value(stats.games_lost) };
    let fastest_str     = if is_first_launch { dash.clone() } else { format_fastest_win(stats.fastest_win_seconds) };
    let avg_time_str    = if is_first_launch { dash.clone() } else { format_avg_time(stats) };
    let best_score_str  = if is_first_launch { dash.clone() } else { format_optional_u32(stats.best_single_score) };
    let best_streak_str = if is_first_launch { dash.clone() } else { format_stat_value(stats.win_streak_best) };

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

    spawn_modal(commands, StatsScreen, Z_MODAL_PANEL, |card| {
        spawn_modal_header(card, "Statistics", font_res);

        // First-launch caption — sits above the grid as gentle nudge so
        // the wall of em-dashes reads as "nothing to track yet" rather
        // than as broken state.
        if is_first_launch {
            card.spawn((
                Text::new("Play a game to start tracking stats."),
                TextFont {
                    font_size: TYPE_CAPTION,
                    ..default()
                },
                TextColor(TEXT_SECONDARY),
                Node {
                    margin: UiRect {
                        bottom: VAL_SPACE_2,
                        ..default()
                    },
                    ..default()
                },
            ));
        }

        // --- primary stat cells grid ---
        card.spawn(Node {
            flex_direction: FlexDirection::Row,
            flex_wrap: FlexWrap::Wrap,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::FlexStart,
            column_gap: VAL_SPACE_4,
            row_gap: VAL_SPACE_3,
            width: Val::Percent(100.0),
            ..default()
        })
        .with_children(|grid| {
            spawn_stat_cell(grid, &win_rate_str,    "Win Rate");
            spawn_stat_cell(grid, &played_str,      "Games Played");
            spawn_stat_cell(grid, &won_str,         "Games Won");
            spawn_stat_cell(grid, &lost_str,        "Games Lost");
            spawn_stat_cell(grid, &fastest_str,     "Fastest Win");
            spawn_stat_cell(grid, &avg_time_str,    "Avg Time");
            spawn_stat_cell(grid, &best_score_str,  "Best Score");
            spawn_stat_cell(grid, &best_streak_str, "Best Streak");
        });

        // --- per-mode bests section ---
        // Three rows, one per supported mode. Time Attack uses session-level
        // scoring (count of wins inside a 10-minute window) so a per-game
        // best wouldn't compose; Daily uses Classic scoring and so already
        // contributes to the Classic row.
        card.spawn((
            Text::new("Per-mode bests"),
            font_section.clone(),
            TextColor(STATE_INFO),
        ));
        card.spawn(Node {
            flex_direction: FlexDirection::Column,
            width: Val::Percent(100.0),
            row_gap: VAL_SPACE_2,
            ..default()
        })
        .with_children(|column| {
            spawn_per_mode_bests_row(
                column,
                "Classic",
                stats.classic_best_score,
                stats.classic_fastest_win_seconds,
                &font_row,
            );
            spawn_per_mode_bests_row(
                column,
                "Zen",
                stats.zen_best_score,
                stats.zen_fastest_win_seconds,
                &font_row,
            );
            spawn_per_mode_bests_row(
                column,
                "Challenge",
                stats.challenge_best_score,
                stats.challenge_fastest_win_seconds,
                &font_row,
            );
        });

        // --- progression section ---
        if let Some(p) = progress {
            card.spawn((
                Text::new("Progression"),
                font_section.clone(),
                TextColor(STATE_INFO),
            ));

            let level_str     = format_stat_value(p.level);
            let xp_str        = format_stat_value(p.total_xp as u32);
            let next_label    = xp_to_next_level_label(p.total_xp, p.level);
            let daily_str     = format_stat_value(p.daily_challenge_streak);
            let challenge_str = challenge_progress_label(p.challenge_index);

            card.spawn(Node {
                flex_direction: FlexDirection::Row,
                flex_wrap: FlexWrap::Wrap,
                justify_content: JustifyContent::Center,
                align_items: AlignItems::FlexStart,
                column_gap: VAL_SPACE_4,
                row_gap: VAL_SPACE_3,
                width: Val::Percent(100.0),
                ..default()
            })
            .with_children(|grid| {
                spawn_stat_cell(grid, &level_str,     "Level");
                spawn_stat_cell(grid, &xp_str,        "Total XP");
                spawn_stat_cell(grid, &next_label,    "Next Level");
                spawn_stat_cell(grid, &daily_str,     "Daily Streak");
                spawn_stat_cell(grid, &challenge_str, "Challenge");
            });

            // Weekly goals
            card.spawn((
                Text::new("Weekly Goals"),
                font_section.clone(),
                TextColor(TEXT_SECONDARY),
            ));
            for goal in WEEKLY_GOALS {
                let pv = p.weekly_goal_progress.get(goal.id).copied().unwrap_or(0);
                card.spawn((
                    Text::new(format!("  {}: {}/{}", goal.description, pv, goal.target)),
                    font_row.clone(),
                    TextColor(TEXT_PRIMARY),
                ));
            }

            // Unlocks line
            card.spawn((
                Text::new(format!(
                    "Card Backs: {}  |  Backgrounds: {}",
                    format_id_list(&p.unlocked_card_backs),
                    format_id_list(&p.unlocked_backgrounds),
                )),
                font_row.clone(),
                TextColor(TEXT_SECONDARY),
            ));
        }

        // --- Time Attack section ---
        if let Some(ta) = time_attack
            && ta.active {
                let mins = (ta.remaining_secs / 60.0).floor() as u64;
                let secs = (ta.remaining_secs % 60.0).floor() as u64;
                card.spawn((
                    Text::new(format!(
                        "Time Attack \u{2014} {mins}m {secs:02}s left  |  Wins: {}",
                        ta.wins
                    )),
                    font_section.clone(),
                    TextColor(STATE_WARNING),
                ));
            }

        // --- Latest replay caption ---
        // Surfaces the most recent winning game so the player can spot
        // whether their last victory has been recorded. The Watch
        // Replay action below is what the player clicks to revisit it.
        let replay_caption = match latest_replay {
            Some(r) => format!("Latest win: {}", format_replay_caption(r)),
            None => "No replay recorded yet \u{2014} win a game first.".to_string(),
        };
        card.spawn((
            Text::new(replay_caption),
            font_row.clone(),
            TextColor(TEXT_SECONDARY),
        ));

        spawn_modal_actions(card, |actions| {
            // The Watch Replay button is always rendered so the
            // affordance is discoverable from a fresh install. When no
            // replay exists, the click handler surfaces a clear
            // "No replay recorded yet" toast rather than silently
            // doing nothing.
            spawn_modal_button(
                actions,
                WatchReplayButton,
                "Watch replay",
                None,
                ButtonVariant::Secondary,
                font_res,
            );
            spawn_modal_button(
                actions,
                StatsCloseButton,
                "Done",
                Some("S"),
                ButtonVariant::Primary,
                font_res,
            );
        });
    });
}

/// Spawn one row of the "Per-mode bests" section: the mode label on the
/// left, then the best-score and best-time readouts right-aligned. Each
/// row is tagged with [`PerModeBestsRow`] so tests can count them.
///
/// `best_score == 0` and `fastest_win_seconds == 0` both render as an
/// em-dash, consistent with the first-launch zero-state treatment used
/// by the primary cells above.
fn spawn_per_mode_bests_row(
    parent: &mut ChildSpawnerCommands,
    mode_label: &str,
    best_score: u32,
    fastest_win_seconds: u64,
    font_row: &TextFont,
) {
    let dash = "\u{2014}".to_string();
    let score_str = if best_score == 0 {
        format!("Best {dash}")
    } else {
        format!("Best {best_score}")
    };
    let time_str = if fastest_win_seconds == 0 {
        format!("Best time {dash}")
    } else {
        format!("Best time {}", format_duration(fastest_win_seconds))
    };

    parent
        .spawn((
            PerModeBestsRow,
            Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::SpaceBetween,
                width: Val::Percent(100.0),
                column_gap: VAL_SPACE_3,
                ..default()
            },
        ))
        .with_children(|row| {
            // Mode label on the left.
            row.spawn((
                Text::new(mode_label.to_string()),
                font_row.clone(),
                TextColor(TEXT_PRIMARY),
            ));
            // Right-aligned readouts grouped together.
            row.spawn(Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::FlexEnd,
                column_gap: VAL_SPACE_3,
                ..default()
            })
            .with_children(|readouts| {
                readouts.spawn((
                    Text::new(score_str),
                    font_row.clone(),
                    TextColor(ACCENT_PRIMARY),
                ));
                readouts.spawn((
                    Text::new(time_str),
                    font_row.clone(),
                    TextColor(TEXT_SECONDARY),
                ));
            });
        });
}

/// Spawn a single stat cell: a large value label on top and a small
/// descriptor below, inside a fixed-min-width column with a subtle
/// border. Recoloured to use ui_theme tokens — the prior 6%-alpha-white
/// fill clashed against the new midnight-purple modal surface.
fn spawn_stat_cell(parent: &mut ChildSpawnerCommands, value: &str, label: &str) {
    parent
        .spawn((
            StatsCell,
            Node {
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                min_width: Val::Px(110.0),
                padding: UiRect::all(VAL_SPACE_2),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(RADIUS_SM)),
                ..default()
            },
            BorderColor::all(BORDER_SUBTLE),
        ))
        .with_children(|cell| {
            // Large value label — accent yellow makes the number sing
            // against the dark card surface.
            cell.spawn((
                Text::new(value.to_string()),
                TextFont {
                    font_size: TYPE_HEADLINE,
                    ..default()
                },
                TextColor(ACCENT_PRIMARY),
            ));
            // Small descriptor below the value.
            cell.spawn((
                Text::new(label.to_string()),
                TextFont {
                    font_size: TYPE_BODY,
                    ..default()
                },
                TextColor(TEXT_SECONDARY),
            ));
        });
}

/// Format a win-rate value for display.
///
/// Returns `"—"` when no games have been played, otherwise `"N%"`.
pub fn format_win_rate(stats: &StatsSnapshot) -> String {
    match stats.win_rate() {
        None => "\u{2014}".to_string(),
        Some(r) => format!("{}%", (r) as u32),
    }
}

/// Format `fastest_win_seconds` for display.
///
/// Returns `"—"` when the value is `u64::MAX` (sentinel for "no wins yet") or
/// zero. Otherwise delegates to [`format_duration`].
pub fn format_fastest_win(fastest_win_seconds: u64) -> String {
    if fastest_win_seconds == u64::MAX || fastest_win_seconds == 0 {
        "\u{2014}".to_string()
    } else {
        format_duration(fastest_win_seconds)
    }
}

/// Format `avg_time_seconds` for display.
///
/// Returns `"—"` when no games have been won yet (`games_won == 0`), otherwise
/// delegates to [`format_duration`].
pub fn format_avg_time(stats: &StatsSnapshot) -> String {
    if stats.games_won == 0 {
        "\u{2014}".to_string()
    } else {
        format_duration(stats.avg_time_seconds)
    }
}

/// Format an optional `u32` statistic.
///
/// Returns `"—"` when `value` is `0`, otherwise the decimal representation.
pub fn format_optional_u32(value: u32) -> String {
    if value == 0 {
        "\u{2014}".to_string()
    } else {
        value.to_string()
    }
}

/// Format any `u32`-like stat value as a decimal string.
///
/// Unlike [`format_optional_u32`], this always shows the number (even if zero).
pub fn format_stat_value<T: std::fmt::Display>(value: T) -> String {
    format!("{value}")
}

/// Returns XP remaining until next level, formatted as "N XP (P%)".
fn xp_to_next_level_label(total_xp: u64, level: u32) -> String {
    let xp_current = if level < 10 {
        level as u64 * 500
    } else {
        5_000 + (level as u64 - 10) * 1_000
    };
    let xp_next = if level < 10 {
        (level as u64 + 1) * 500
    } else {
        5_000 + (level as u64 - 9) * 1_000
    };
    let span = xp_next - xp_current;
    let done = total_xp.saturating_sub(xp_current).min(span);
    let pct = if span == 0 { 100 } else { done.saturating_mul(100).checked_div(span).unwrap_or(100) };
    let remaining = span - done;
    format!("{remaining} XP ({pct}%)")
}

/// Format a duration given in whole seconds as `"M:SS"`.
///
/// Example: `90` → `"1:30"`.
pub fn format_duration(secs: u64) -> String {
    let m = secs / 60;
    let s = secs % 60;
    format!("{m}:{s:02}")
}

/// Renders a sorted, comma-separated list of unlock indexes for the overlay.
/// Empty list shows as "None".
fn format_id_list(ids: &[usize]) -> String {
    if ids.is_empty() {
        return "None".to_string();
    }
    let mut sorted: Vec<usize> = ids.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    sorted
        .iter()
        .map(|i| format!("#{i}"))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game_plugin::GamePlugin;
    use crate::table_plugin::TablePlugin;

    fn headless_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(GamePlugin)
            .add_plugins(TablePlugin)
            .add_plugins(StatsPlugin::headless());
        // MinimalPlugins doesn't register keyboard input — add it so the
        // toggle system can read ButtonInput<KeyCode> in tests.
        app.init_resource::<ButtonInput<KeyCode>>();
        // ProgressResource is an optional dependency for the stats screen;
        // include it so toggle tests exercise the progression panel.
        app.add_plugins(crate::progress_plugin::ProgressPlugin::headless());
        app.update();
        app
    }

    #[test]
    fn stats_resource_exists_after_startup() {
        let app = headless_app();
        assert!(app.world().get_resource::<StatsResource>().is_some());
    }

    #[test]
    fn headless_plugin_starts_with_default_stats() {
        let app = headless_app();
        let stats = &app.world().resource::<StatsResource>().0;
        assert_eq!(stats, &StatsSnapshot::default());
    }

    #[test]
    fn win_event_increments_games_won() {
        let mut app = headless_app();
        app.world_mut().write_message(GameWonEvent {
            score: 1000,
            time_seconds: 120,
        });
        app.update();

        let stats = &app.world().resource::<StatsResource>().0;
        assert_eq!(stats.games_won, 1);
        assert_eq!(stats.games_played, 1);
    }

    #[test]
    fn draw_three_win_increments_draw_three_wins_only() {
        let mut app = headless_app();
        app.world_mut()
            .resource_mut::<crate::resources::GameStateResource>()
            .0
            .draw_mode = solitaire_core::game_state::DrawMode::DrawThree;

        app.world_mut().write_message(GameWonEvent {
            score: 500,
            time_seconds: 200,
        });
        app.update();

        let stats = &app.world().resource::<StatsResource>().0;
        assert_eq!(stats.draw_three_wins, 1, "draw_three_wins must increment for DrawThree mode");
        assert_eq!(stats.draw_one_wins, 0, "draw_one_wins must not increment for DrawThree mode");
    }

    #[test]
    fn new_game_after_moves_records_abandoned() {
        let mut app = headless_app();

        app.world_mut()
            .resource_mut::<crate::resources::GameStateResource>()
            .0
            .move_count = 3;

        app.world_mut()
            .write_message(NewGameRequestEvent { seed: Some(999), mode: None, confirmed: false });
        app.update();

        let stats = &app.world().resource::<StatsResource>().0;
        assert_eq!(stats.games_played, 1);
        assert_eq!(stats.games_lost, 1);
        assert_eq!(stats.win_streak_current, 0);
    }

    #[test]
    fn new_game_without_moves_does_not_record_abandoned() {
        let mut app = headless_app();
        app.world_mut()
            .write_message(NewGameRequestEvent { seed: Some(42), mode: None, confirmed: false });
        app.update();

        let stats = &app.world().resource::<StatsResource>().0;
        assert_eq!(stats.games_played, 0);
    }

    #[test]
    fn pressing_s_spawns_stats_screen() {
        let mut app = headless_app();
        assert_eq!(
            app.world_mut()
                .query::<&StatsScreen>()
                .iter(app.world())
                .count(),
            0
        );

        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyS);
        app.update();

        assert_eq!(
            app.world_mut()
                .query::<&StatsScreen>()
                .iter(app.world())
                .count(),
            1
        );
    }

    #[test]
    fn stats_screen_renders_three_per_mode_bests_rows() {
        // Open the Stats overlay and assert three [`PerModeBestsRow`]
        // entities exist — one per supported [`GameMode`] (Classic, Zen,
        // Challenge — Time Attack and Daily are excluded by design).
        let mut app = headless_app();
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyS);
        app.update();

        let row_count = app
            .world_mut()
            .query::<&PerModeBestsRow>()
            .iter(app.world())
            .count();
        assert_eq!(
            row_count, 3,
            "expected three per-mode bests rows (Classic, Zen, Challenge), got {row_count}"
        );
    }

    #[test]
    fn classic_win_event_updates_classic_best_score() {
        // Default mode is Classic — a win event should populate the
        // Classic per-mode bests but leave Zen and Challenge at zero.
        let mut app = headless_app();
        app.world_mut().write_message(GameWonEvent {
            score: 1500,
            time_seconds: 180,
        });
        app.update();

        let stats = &app.world().resource::<StatsResource>().0;
        assert_eq!(stats.classic_best_score, 1500);
        assert_eq!(stats.classic_fastest_win_seconds, 180);
        assert_eq!(stats.zen_best_score, 0);
        assert_eq!(stats.challenge_best_score, 0);
    }

    #[test]
    fn zen_win_event_updates_zen_best_score_only() {
        let mut app = headless_app();
        app.world_mut()
            .resource_mut::<crate::resources::GameStateResource>()
            .0
            .mode = solitaire_core::game_state::GameMode::Zen;

        app.world_mut().write_message(GameWonEvent {
            score: 1800,
            time_seconds: 600,
        });
        app.update();

        let stats = &app.world().resource::<StatsResource>().0;
        assert_eq!(stats.zen_best_score, 1800);
        assert_eq!(stats.zen_fastest_win_seconds, 600);
        assert_eq!(stats.classic_best_score, 0);
        assert_eq!(stats.challenge_best_score, 0);
    }

    #[test]
    fn pressing_s_twice_closes_stats_screen() {
        let mut app = headless_app();

        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyS);
        app.update();

        // Release + clear + press: `press()` is a no-op if the key is already
        // in `pressed`, and MinimalPlugins doesn't include bevy_input's
        // per-frame updater to drain `just_pressed`, so we cycle manually.
        {
            let mut input = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            input.release(KeyCode::KeyS);
            input.clear();
            input.press(KeyCode::KeyS);
        }
        app.update();

        assert_eq!(
            app.world_mut()
                .query::<&StatsScreen>()
                .iter(app.world())
                .count(),
            0
        );
    }

    #[test]
    fn format_id_list_renders_empty_as_none() {
        assert_eq!(format_id_list(&[]), "None");
    }

    #[test]
    fn format_id_list_sorts_dedups_and_prefixes() {
        assert_eq!(format_id_list(&[3, 1, 1, 2]), "#1, #2, #3");
    }

    #[test]
    fn xp_to_next_level_label_at_zero_xp() {
        // Level 0, 0 XP: 500 needed, 0% done.
        assert_eq!(xp_to_next_level_label(0, 0), "500 XP (0%)");
    }

    #[test]
    fn xp_to_next_level_label_halfway_through_level_1() {
        // Level 1 starts at 500 XP, level 2 at 1000 XP.
        // At 750 XP: 250 done of 500, 50%, 250 remaining.
        assert_eq!(xp_to_next_level_label(750, 1), "250 XP (50%)");
    }

    #[test]
    fn xp_to_next_level_label_at_level_10_boundary() {
        // Level 10 starts at 5000 XP, level 11 at 6000 XP.
        // At 5000 XP: 0 done, 0%, 1000 remaining.
        assert_eq!(xp_to_next_level_label(5_000, 10), "1000 XP (0%)");
    }

    // -----------------------------------------------------------------------
    // format_duration
    // -----------------------------------------------------------------------

    #[test]
    fn format_duration_zero_seconds() {
        assert_eq!(format_duration(0), "0:00");
    }

    #[test]
    fn format_duration_pads_seconds_to_two_digits() {
        assert_eq!(format_duration(65), "1:05");
    }

    #[test]
    fn format_duration_exactly_one_hour() {
        assert_eq!(format_duration(3600), "60:00");
    }

    #[test]
    fn format_duration_handles_sub_minute() {
        assert_eq!(format_duration(59), "0:59");
    }

    // -----------------------------------------------------------------------
    // Task #65 — win rate and stat cell pure-function tests
    // -----------------------------------------------------------------------

    #[test]
    fn format_win_rate_zero() {
        // 0 wins, 0 played → "—"
        let s = StatsSnapshot::default();
        assert_eq!(format_win_rate(&s), "\u{2014}");
    }

    #[test]
    fn format_win_rate_half() {
        // 5 wins out of 10 played → "50%"
        let s = StatsSnapshot {
            games_played: 10,
            games_won: 5,
            ..StatsSnapshot::default()
        };
        assert_eq!(format_win_rate(&s), "50%");
    }

    #[test]
    fn format_stat_value_zero_returns_zero() {
        assert_eq!(format_stat_value(0u32), "0");
    }

    // -----------------------------------------------------------------------
    // Task #66 — fastest win, best score, streak pure-function tests
    // -----------------------------------------------------------------------

    #[test]
    fn format_fastest_win_unset() {
        // fastest_win_seconds == u64::MAX → "—"
        assert_eq!(format_fastest_win(u64::MAX), "\u{2014}");
    }

    #[test]
    fn format_fastest_win_90s() {
        // 90 seconds → "1:30"
        assert_eq!(format_fastest_win(90), "1:30");
    }

    #[test]
    fn best_score_display_zero() {
        // best_single_score == 0 → "—"
        assert_eq!(format_optional_u32(0), "\u{2014}");
    }

    // -----------------------------------------------------------------------
    // Task #38 — avg time pure-function tests
    // -----------------------------------------------------------------------

    #[test]
    fn format_avg_time_no_wins_shows_dash() {
        // games_won == 0 → "—"
        let s = StatsSnapshot::default();
        assert_eq!(format_avg_time(&s), "\u{2014}");
    }

    #[test]
    fn format_avg_time_after_single_win() {
        // After one win of 90 s avg should be "1:30"
        let s = StatsSnapshot {
            games_won: 1,
            avg_time_seconds: 90,
            ..StatsSnapshot::default()
        };
        assert_eq!(format_avg_time(&s), "1:30");
    }

    #[test]
    fn format_avg_time_after_multiple_wins() {
        // avg_time_seconds = 200 s → "3:20"
        let s = StatsSnapshot {
            games_won: 3,
            avg_time_seconds: 200,
            ..StatsSnapshot::default()
        };
        assert_eq!(format_avg_time(&s), "3:20");
    }

    // -----------------------------------------------------------------------
    // Task #49 — streak-broken toast on forfeit
    // -----------------------------------------------------------------------

    #[test]
    fn forfeit_with_streak_fires_streak_broken_toast() {
        let mut app = headless_app();

        // Set up a streak of 3 and at least one move so forfeit counts.
        {
            let mut stats = app.world_mut().resource_mut::<StatsResource>();
            stats.0.win_streak_current = 3;
        }
        app.world_mut()
            .resource_mut::<crate::resources::GameStateResource>()
            .0
            .move_count = 1;

        app.world_mut().write_message(ForfeitEvent);
        app.update();

        let events = app.world().resource::<Messages<InfoToastEvent>>();
        let mut reader = events.get_cursor();
        let messages: Vec<&str> = reader
            .read(events)
            .map(|e| e.0.as_str())
            .collect();

        assert!(
            messages.contains(&"Streak of 3 broken!"),
            "expected 'Streak of 3 broken!' in toasts, got: {messages:?}"
        );
    }

    #[test]
    fn forfeit_with_streak_of_one_does_not_fire_streak_broken_toast() {
        let mut app = headless_app();

        {
            let mut stats = app.world_mut().resource_mut::<StatsResource>();
            stats.0.win_streak_current = 1;
        }
        app.world_mut()
            .resource_mut::<crate::resources::GameStateResource>()
            .0
            .move_count = 1;

        app.world_mut().write_message(ForfeitEvent);
        app.update();

        let events = app.world().resource::<Messages<InfoToastEvent>>();
        let mut reader = events.get_cursor();
        let messages: Vec<&str> = reader
            .read(events)
            .map(|e| e.0.as_str())
            .collect();

        assert!(
            !messages.iter().any(|m| m.contains("broken")),
            "expected no streak-broken toast for streak of 1, got: {messages:?}"
        );
    }

    // -----------------------------------------------------------------------
    // Streak-milestone flourish — pure helper + event-firing tests
    // -----------------------------------------------------------------------

    /// Pure helper: every threshold in `STREAK_MILESTONES` (3, 5, 10) must
    /// fire when the streak crosses it from below.
    #[test]
    fn streak_milestone_helper_fires_at_each_threshold() {
        for &threshold in STREAK_MILESTONES {
            assert_eq!(
                streak_milestone_crossed(threshold - 1, threshold),
                Some(threshold),
                "expected milestone {threshold} to fire when crossed from below",
            );
        }
    }

    /// Pure helper: rising past 10 to 11, 12, … must NOT fire — the
    /// flourish is a threshold-crossing event, not a "every win past 10"
    /// event.
    #[test]
    fn streak_milestone_helper_does_not_fire_past_highest() {
        // prev=10 → new=11: above the highest threshold, no crossing.
        assert_eq!(streak_milestone_crossed(10, 11), None);
        // prev=15 → new=16: well past every threshold, no crossing.
        assert_eq!(streak_milestone_crossed(15, 16), None);
        // prev=2 → new=2: no change → no crossing.
        assert_eq!(streak_milestone_crossed(2, 2), None);
    }

    /// Pure helper: rising 1 → 2 stays below the lowest threshold (3),
    /// must NOT fire.
    #[test]
    fn streak_milestone_helper_does_not_fire_below_threshold() {
        assert_eq!(streak_milestone_crossed(1, 2), None);
        assert_eq!(streak_milestone_crossed(0, 1), None);
    }

    /// Integration: pre-set streak to 2, fire a win that bumps it to 3,
    /// assert exactly one `WinStreakMilestoneEvent { streak: 3 }` is
    /// written by the win handler.
    #[test]
    fn streak_milestone_event_fires_at_threshold_crossing() {
        let mut app = headless_app();
        {
            let mut stats = app.world_mut().resource_mut::<StatsResource>();
            stats.0.win_streak_current = 2;
        }
        app.world_mut().write_message(GameWonEvent {
            score: 500,
            time_seconds: 90,
        });
        app.update();

        let events = app.world().resource::<Messages<WinStreakMilestoneEvent>>();
        let mut reader = events.get_cursor();
        let collected: Vec<u32> = reader.read(events).map(|e| e.streak).collect();

        assert_eq!(
            collected,
            vec![3],
            "expected one WinStreakMilestoneEvent {{ streak: 3 }} after crossing 2 → 3",
        );
    }

    /// Integration: pre-set streak to 1, fire a win that bumps it to 2 —
    /// no threshold is crossed, no event must be fired.
    #[test]
    fn streak_milestone_event_does_not_fire_at_non_threshold() {
        let mut app = headless_app();
        {
            let mut stats = app.world_mut().resource_mut::<StatsResource>();
            stats.0.win_streak_current = 1;
        }
        app.world_mut().write_message(GameWonEvent {
            score: 500,
            time_seconds: 90,
        });
        app.update();

        let events = app.world().resource::<Messages<WinStreakMilestoneEvent>>();
        let mut reader = events.get_cursor();
        let collected: Vec<u32> = reader.read(events).map(|e| e.streak).collect();

        assert!(
            collected.is_empty(),
            "expected no WinStreakMilestoneEvent for non-threshold streak crossing 1 → 2, got {collected:?}",
        );
    }

    /// Integration: pre-set streak to 10, fire a win that bumps it to 11.
    /// Past the highest threshold, no event must fire — the flourish
    /// is reserved for the threshold crossing itself.
    #[test]
    fn streak_milestone_event_does_not_fire_past_10() {
        let mut app = headless_app();
        {
            let mut stats = app.world_mut().resource_mut::<StatsResource>();
            stats.0.win_streak_current = 10;
        }
        app.world_mut().write_message(GameWonEvent {
            score: 500,
            time_seconds: 90,
        });
        app.update();

        let events = app.world().resource::<Messages<WinStreakMilestoneEvent>>();
        let mut reader = events.get_cursor();
        let collected: Vec<u32> = reader.read(events).map(|e| e.streak).collect();

        assert!(
            collected.is_empty(),
            "expected no WinStreakMilestoneEvent past the highest threshold, got {collected:?}",
        );
    }
}
