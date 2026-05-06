//! Loads, updates, and persists `StatsSnapshot` in response to game events,
//! and provides a toggleable full-window stats overlay (press `S`).
//!
//! The persistence path is configurable via `StatsPlugin::storage_path`.
//! In production, `StatsPlugin::default()` loads/saves from the platform
//! data dir. In tests, use `StatsPlugin::headless()` to disable all file
//! I/O so the user's real stats file is neither read nor overwritten.

use std::path::PathBuf;

use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::input::ButtonInput;
use bevy::prelude::*;
use solitaire_data::{
    load_replay_history_from, load_stats_from, replay_history_path, save_stats_to,
    stats_file_path, PlayerProgress, Replay, ReplayHistory, StatsExt, StatsSnapshot, WEEKLY_GOALS,
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
    ScrimDismissible,
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

/// Resource holding the rolling [`ReplayHistory`] of recent winning
/// replays.
///
/// Populated from `<data_dir>/solitaire_quest/replays.json` at startup
/// and refreshed in-place whenever the engine writes a new winning
/// replay so the Stats overlay's selector always reflects the current
/// on-disk history.
///
/// `replays[0]` is the most recent win — the Stats overlay's selector
/// defaults to that entry and lets the player step backwards through
/// up to [`solitaire_data::REPLAY_HISTORY_CAP`] older entries.
#[derive(Resource, Debug, Default, Clone)]
pub struct ReplayHistoryResource(pub ReplayHistory);

/// Most recent shareable replay URL written by `sync_plugin` after the
/// `SyncProvider::push_replay` task completes successfully. `None`
/// until the player wins a game on a server-backed sync backend;
/// repopulated on each subsequent win.
///
/// The Stats overlay's "Copy share link" button reads from here and
/// writes the URL to the OS clipboard via `arboard`. Not persisted to
/// disk — the URL is recoverable by re-uploading the same replay
/// (still in `replays.json`), so the session-bound lifetime is fine
/// for a v1 share affordance.
#[derive(Resource, Debug, Default, Clone)]
pub struct LastSharedReplayUrl(pub Option<String>);

/// Marker on the "Copy share link" button inside the Stats modal.
/// Click writes [`LastSharedReplayUrl`] to the OS clipboard via
/// `arboard` and surfaces a confirmation toast. Hidden / disabled
/// when no shareable URL is available.
#[derive(Component, Debug)]
pub struct CopyShareLinkButton;

/// Currently-selected index into [`ReplayHistoryResource::0`].`replays`.
///
/// `0` is the most recent win and is the default on every modal open.
/// The Prev / Next chips wrap-around within the bounds of the current
/// history so the selector is always sat on a valid replay (or on `0`
/// when the history is empty — the chips paint disabled in that case).
#[derive(Resource, Debug, Default, Clone, Copy)]
pub struct SelectedReplayIndex(pub usize);

/// Persistence path for the rolling replay history file
/// (`replays.json`). `None` disables I/O — used by tests and by
/// `StatsPlugin::headless`.
#[derive(Resource, Debug, Clone)]
pub struct LatestReplayPath(pub Option<PathBuf>);

/// Marker on the "Watch replay" button inside the Stats modal. Clicking
/// it starts in-engine playback of the selected replay — see
/// [`handle_watch_replay_button`].
#[derive(Component, Debug)]
pub struct WatchReplayButton;

/// Marker on the selector's "Previous replay" chip — steps the
/// selection backwards (toward older replays) within
/// [`ReplayHistoryResource`].
#[derive(Component, Debug)]
pub struct ReplayPrevButton;

/// Marker on the selector's "Next replay" chip — steps the selection
/// forwards (toward more recent replays).
#[derive(Component, Debug)]
pub struct ReplayNextButton;

/// Marker on the selector's `"Replay N / M"` caption text node so the
/// repaint system can rewrite the label as the selection changes.
#[derive(Component, Debug)]
pub struct ReplaySelectorCaption;

/// Marker component on each per-mode bests row in the stats overlay.
///
/// One row per supported [`solitaire_core::game_state::GameMode`] (Classic,
/// Zen, Challenge — Time Attack and Daily are intentionally excluded; see
/// `StatsSnapshot` doc comments). Tests query by this marker to assert the
/// per-mode section rendered.
#[derive(Component, Debug)]
pub struct PerModeBestsRow;

/// Marker on the scrollable body Node inside the Stats modal.
///
/// The Stats panel renders an 8-cell primary grid, three per-mode bests
/// rows, a five-cell progression grid, weekly goals, an unlocks line,
/// optional Time Attack readout, and the latest replay caption — enough
/// content to overflow the modal on the 800x600 minimum window. This
/// marker tags the inner container that carries `Overflow::scroll_y()`
/// plus a `max_height` constraint. Mirrors the `SettingsPanelScrollable`
/// pattern.
#[derive(Component, Debug)]
pub struct StatsScrollable;

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
        // platform-default path via `replay_history_path()`.
        let replay_path = self.storage_path.as_ref().and(replay_history_path());
        let initial_history = replay_path
            .as_deref()
            .and_then(load_replay_history_from)
            .unwrap_or_default();
        app.insert_resource(StatsResource(loaded))
            .insert_resource(StatsStoragePath(self.storage_path.clone()))
            .insert_resource(ReplayHistoryResource(initial_history))
            .init_resource::<SelectedReplayIndex>()
            .insert_resource(LatestReplayPath(replay_path))
            .init_resource::<LastSharedReplayUrl>()
            .add_message::<GameWonEvent>()
            .add_message::<NewGameRequestEvent>()
            .add_message::<ForfeitEvent>()
            .add_message::<InfoToastEvent>()
            .add_message::<ToggleStatsRequestEvent>()
            .add_message::<WinStreakMilestoneEvent>()
            // `MouseWheel` is emitted by Bevy's input plugin under
            // `DefaultPlugins`; register it explicitly so the stats-scroll
            // system also runs cleanly under `MinimalPlugins` in tests.
            .add_message::<MouseWheel>()
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
                refresh_replay_history_on_win.after(GameMutation),
            )
            .add_systems(Update, handle_watch_replay_button)
            .add_systems(Update, handle_copy_share_link_button)
            .add_systems(
                Update,
                (handle_replay_selector_buttons, repaint_replay_selector_caption).chain(),
            )
            .add_systems(Update, scroll_stats_panel);
    }
}

/// Routes mouse-wheel events into the Stats modal's scrollable body
/// while the panel is open. No-op when no `StatsScrollable` exists in
/// the world (modal closed). Mirrors `scroll_settings_panel`.
fn scroll_stats_panel(
    mut scroll_evr: MessageReader<MouseWheel>,
    mut scrollables: Query<&mut ScrollPosition, With<StatsScrollable>>,
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

/// After a win, the engine has just appended a fresh winning replay to
/// the rolling history file. Re-load it so the next time the player
/// opens the Stats overlay the selector reflects the new entry, and
/// reset [`SelectedReplayIndex`] to `0` so the default selection is the
/// just-recorded win.
fn refresh_replay_history_on_win(
    mut wins: MessageReader<GameWonEvent>,
    mut history: ResMut<ReplayHistoryResource>,
    mut selected: ResMut<SelectedReplayIndex>,
    path: Res<LatestReplayPath>,
) {
    // Only re-load when at least one win actually fired.
    if wins.read().next().is_none() {
        return;
    }
    let Some(p) = path.0.as_deref() else {
        return;
    };
    history.0 = load_replay_history_from(p).unwrap_or_default();
    // Snap the selector back to the most recent win — that's the one
    // the player just earned.
    selected.0 = 0;
}

/// Click handler for the "Watch replay" button.
///
/// Starts in-engine replay playback for the currently-selected entry in
/// [`ReplayHistoryResource`] (per [`SelectedReplayIndex`]). If the
/// history is empty or the selector points past the end (defensive
/// guard), surfaces an [`InfoToastEvent`] instead. The playback path
/// resets the live game to the recorded deal and ticks through the
/// move list via [`crate::replay_playback`]; the
/// [`crate::replay_overlay`] banner surfaces while playback runs.
/// Copies [`LastSharedReplayUrl`] to the OS clipboard via `arboard`
/// and surfaces a confirmation toast. When no URL is in hand (no win
/// yet on a server-backed sync backend, local-only mode, or upload
/// failed) the button still acknowledges the click but explains why
/// the clipboard wasn't written. `arboard::Clipboard::new()` failures
/// are logged + surfaced as a generic "couldn't reach the clipboard"
/// toast rather than swallowed — they're rare but worth diagnosing.
fn handle_copy_share_link_button(
    buttons: Query<&Interaction, (With<CopyShareLinkButton>, Changed<Interaction>)>,
    last_url: Res<LastSharedReplayUrl>,
    mut toast: MessageWriter<InfoToastEvent>,
) {
    if !buttons.iter().any(|i| *i == Interaction::Pressed) {
        return;
    }
    let Some(url) = last_url.0.as_ref() else {
        toast.write(InfoToastEvent(
            "No share link yet \u{2014} win a game on a server-backed sync to upload one.".to_string(),
        ));
        return;
    };
    match arboard::Clipboard::new() {
        Ok(mut cb) => match cb.set_text(url.clone()) {
            Ok(()) => {
                toast.write(InfoToastEvent(format!("Copied: {url}")));
            }
            Err(e) => {
                warn!("clipboard write failed: {e}");
                toast.write(InfoToastEvent(
                    "Couldn't write to clipboard \u{2014} share link wasn't copied.".to_string(),
                ));
            }
        },
        Err(e) => {
            warn!("clipboard init failed: {e}");
            toast.write(InfoToastEvent(
                "Couldn't reach the clipboard \u{2014} share link wasn't copied.".to_string(),
            ));
        }
    }
}

fn handle_watch_replay_button(
    mut commands: Commands,
    buttons: Query<&Interaction, (With<WatchReplayButton>, Changed<Interaction>)>,
    history: Res<ReplayHistoryResource>,
    selected: Res<SelectedReplayIndex>,
    playback: Option<ResMut<crate::replay_playback::ReplayPlaybackState>>,
    mut toast: MessageWriter<InfoToastEvent>,
) {
    if !buttons.iter().any(|i| *i == Interaction::Pressed) {
        return;
    }
    let chosen = history.0.replays.get(selected.0);
    match (chosen, playback) {
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

/// Click handler for the Prev / Next chips on the Stats overlay's
/// replay selector. Steps [`SelectedReplayIndex`] within the bounds of
/// the current [`ReplayHistoryResource`]; selection wraps so the
/// chooser is always sat on a valid replay.
///
/// No-op when the history is empty — the selector chips paint disabled
/// in that case but a defensive bounds check here keeps things tidy if
/// the click somehow lands.
fn handle_replay_selector_buttons(
    prev: Query<&Interaction, (With<ReplayPrevButton>, Changed<Interaction>)>,
    next: Query<&Interaction, (With<ReplayNextButton>, Changed<Interaction>)>,
    history: Res<ReplayHistoryResource>,
    mut selected: ResMut<SelectedReplayIndex>,
) {
    let len = history.0.replays.len();
    if len == 0 {
        return;
    }
    let prev_pressed = prev.iter().any(|i| *i == Interaction::Pressed);
    let next_pressed = next.iter().any(|i| *i == Interaction::Pressed);
    if prev_pressed {
        // Step toward older replays — wrap to the oldest when at the
        // newest (index 0).
        selected.0 = if selected.0 == 0 { len - 1 } else { selected.0 - 1 };
    }
    if next_pressed {
        // Step toward more recent replays — wrap to the newest when at
        // the oldest.
        selected.0 = (selected.0 + 1) % len;
    }
}

/// Live-update the `"Replay N / M"` caption text as the selector
/// changes. The caption sits next to the Prev / Next chips above the
/// Watch button so the player can see at a glance which replay they're
/// about to watch.
fn repaint_replay_selector_caption(
    history: Res<ReplayHistoryResource>,
    selected: Res<SelectedReplayIndex>,
    mut q: Query<&mut Text, With<ReplaySelectorCaption>>,
) {
    if !history.is_changed() && !selected.is_changed() {
        return;
    }
    for mut text in &mut q {
        **text = replay_selector_caption(selected.0, history.0.replays.len());
    }
}

/// Pure helper: render the selector caption shown next to the Prev /
/// Next chips. Returns `"No replays"` when the history is empty,
/// otherwise `"Replay {1-based index} / {total}"`.
///
/// `index` is zero-based as it's stored in [`SelectedReplayIndex`].
/// The display flips it to a one-based ordinal so "Replay 1" reads as
/// "the most recent win" — matching the mental model the chooser
/// surfaces.
pub fn replay_selector_caption(index: usize, total: usize) -> String {
    if total == 0 {
        return "No replays".to_string();
    }
    // Defensive clamp — the caller is supposed to keep `index` in
    // range, but a stale selector after a cap-driven truncation
    // shouldn't crash the renderer.
    let one_based = index.min(total.saturating_sub(1)) + 1;
    format!("Replay {one_based} / {total}")
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
    latest_replay: Res<ReplayHistoryResource>,
    selected_index: Res<SelectedReplayIndex>,
    screens: Query<Entity, With<StatsScreen>>,
) {
    let button_clicked = requests.read().count() > 0;
    if !keys.just_pressed(KeyCode::KeyS) && !button_clicked {
        return;
    }
    if let Ok(entity) = screens.single() {
        commands.entity(entity).despawn();
    } else {
        let selected = latest_replay.0.replays.get(selected_index.0);
        spawn_stats_screen(
            &mut commands,
            &stats.0,
            progress.as_deref().map(|p| &p.0),
            time_attack.as_deref(),
            font_res.as_deref(),
            selected,
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

    let scrim = spawn_modal(commands, StatsScreen, Z_MODAL_PANEL, |card| {
        spawn_modal_header(card, "Statistics", font_res);

        // Scrollable body — the Stats panel renders an 8-cell grid plus
        // multiple sections (per-mode bests, progression, weekly goals,
        // unlocks, optional Time Attack, latest replay caption) and
        // overflows the modal on the 800x600 minimum window. Wrapping
        // in an `Overflow::scroll_y()` Node with a constrained
        // `max_height` keeps every cell reachable; the Watch Replay /
        // Done action row stays fixed outside the scroll.
        card.spawn((
            StatsScrollable,
            ScrollPosition::default(),
            Node {
                flex_direction: FlexDirection::Column,
                row_gap: VAL_SPACE_3,
                max_height: Val::Vh(70.0),
                overflow: Overflow::scroll_y(),
                ..default()
            },
        ))
        .with_children(|body| {
            // First-launch caption — sits above the grid as gentle nudge so
            // the wall of em-dashes reads as "nothing to track yet" rather
            // than as broken state.
            if is_first_launch {
                body.spawn((
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
            body.spawn(Node {
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
            body.spawn((
                Text::new("Per-mode bests"),
                font_section.clone(),
                TextColor(STATE_INFO),
            ));
            body.spawn(Node {
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
                body.spawn((
                    Text::new("Progression"),
                    font_section.clone(),
                    TextColor(STATE_INFO),
                ));

                let level_str     = format_stat_value(p.level);
                let xp_str        = format_stat_value(p.total_xp as u32);
                let next_label    = xp_to_next_level_label(p.total_xp, p.level);
                let daily_str     = format_stat_value(p.daily_challenge_streak);
                let challenge_str = challenge_progress_label(p.challenge_index);

                body.spawn(Node {
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
                body.spawn((
                    Text::new("Weekly Goals"),
                    font_section.clone(),
                    TextColor(TEXT_SECONDARY),
                ));
                for goal in WEEKLY_GOALS {
                    let pv = p.weekly_goal_progress.get(goal.id).copied().unwrap_or(0);
                    body.spawn((
                        Text::new(format!("  {}: {}/{}", goal.description, pv, goal.target)),
                        font_row.clone(),
                        TextColor(TEXT_PRIMARY),
                    ));
                }

                // Unlocks line
                body.spawn((
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
                    body.spawn((
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
            body.spawn((
                Text::new(replay_caption),
                font_row.clone(),
                TextColor(TEXT_SECONDARY),
            ));
        });

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
            // Copy share link only renders when a sharable URL is in
            // hand. The button is intentionally absent (rather than
            // disabled) when no upload has happened yet — keeps the
            // action bar free of dead controls in the local-only and
            // first-launch cases.
            spawn_modal_button(
                actions,
                CopyShareLinkButton,
                "Copy share link",
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
    // Stats is read-only — opt into click-outside-to-dismiss.
    commands.entity(scrim).insert(ScrimDismissible);
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
    fn stats_modal_body_is_scrollable() {
        let mut app = headless_app();
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyS);
        app.update();

        let count = app
            .world_mut()
            .query::<&StatsScrollable>()
            .iter(app.world())
            .count();
        assert_eq!(
            count, 1,
            "Stats modal must spawn exactly one StatsScrollable body"
        );

        let mut q = app
            .world_mut()
            .query_filtered::<&Node, With<StatsScrollable>>();
        let nodes: Vec<&Node> = q.iter(app.world()).collect();
        assert_ne!(
            nodes[0].max_height,
            Val::Auto,
            "scrollable body must set a non-default max_height"
        );
        assert_eq!(nodes[0].overflow, Overflow::scroll_y());
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
