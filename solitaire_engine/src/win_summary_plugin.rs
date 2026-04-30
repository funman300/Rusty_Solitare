//! Win summary modal overlay and screen-shake effect.
//!
//! # Task #33 — Win summary screen
//! On `GameWonEvent`, after a 0.5 s delay (so the cascade animation has
//! started), a full-screen modal is spawned showing score, time, XP, and a
//! "Play Again" button that fires `NewGameRequestEvent` and closes the modal.
//!
//! # Task #47 — Win fanfare screen-shake
//! When `GameWonEvent` fires, `ScreenShakeResource` is set. A system offsets
//! the `Camera2d` `Transform` each frame with a decaying oscillation until the
//! shake duration elapses.

use bevy::prelude::*;
use solitaire_core::game_state::GameMode;

use crate::achievement_plugin::display_name_for;
use crate::events::{
    AchievementUnlockedEvent, GameWonEvent, InfoToastEvent, NewGameRequestEvent, XpAwardedEvent,
};
use crate::game_plugin::GameMutation;
use crate::progress_plugin::ProgressResource;
use crate::resources::GameStateResource;
use crate::settings_plugin::SettingsResource;
use crate::stats_plugin::{StatsResource, StatsUpdate};
use crate::ui_theme::{
    scaled_duration, ACCENT_PRIMARY, BG_BASE, BG_ELEVATED, MOTION_WIN_SHAKE_AMPLITUDE,
    MOTION_WIN_SHAKE_SECS, RADIUS_LG, RADIUS_MD, SCRIM, STATE_INFO, STATE_SUCCESS, STATE_WARNING,
    TEXT_PRIMARY, TEXT_SECONDARY, TYPE_BODY_LG, TYPE_DISPLAY, TYPE_HEADLINE, VAL_SPACE_2,
    VAL_SPACE_3, Z_WIN_CASCADE,
};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Delay after `GameWonEvent` before the win-summary modal is spawned.
/// Chosen so the cascade animation has a moment to start first.
const WIN_SUMMARY_DELAY_SECS: f32 = 0.5;

/// Default duration of the screen-shake in seconds, before `AnimSpeed` scaling.
/// Sourced from `ui_theme::MOTION_WIN_SHAKE_SECS`.
const SHAKE_DURATION_SECS: f32 = MOTION_WIN_SHAKE_SECS;
/// Maximum camera displacement in world-space pixels at the start of the shake.
/// Sourced from `ui_theme::MOTION_WIN_SHAKE_AMPLITUDE`.
const SHAKE_INTENSITY: f32 = MOTION_WIN_SHAKE_AMPLITUDE;

// ---------------------------------------------------------------------------
// Resources
// ---------------------------------------------------------------------------

/// Accumulates win data while waiting for `XpAwardedEvent` to arrive.
///
/// The XP event fires shortly after `GameWonEvent`. We store both pieces of
/// data here so the modal can show the complete picture.
#[derive(Resource, Debug, Clone, Default)]
pub struct WinSummaryPending {
    /// Score from the most recent `GameWonEvent`.
    pub score: i32,
    /// Elapsed game time (seconds) from the most recent `GameWonEvent`.
    pub time_seconds: u64,
    /// XP awarded from the most recent `XpAwardedEvent` (0 until that event fires).
    pub xp: u64,
    /// Human-readable breakdown of the XP components for the most recent win,
    /// e.g. `"+50 base  +25 no-undo  +30 speed"`. Empty until `GameWonEvent`
    /// populates it.
    pub xp_detail: String,
    /// Whether this win beat the player's previous best score or fastest time.
    ///
    /// Captured from `StatsResource` **before** `StatsUpdate` mutates it so
    /// the comparison reflects the old personal-best values.
    pub new_record: bool,
    /// When the winning game was a Challenge-mode run, holds the 1-based
    /// human-readable level number that was just completed (e.g. `Some(3)`
    /// means "Challenge 3"). `None` for non-Challenge modes.
    pub challenge_level: Option<u32>,
}

/// Builds a human-readable XP breakdown string for the win modal.
///
/// Mirrors the logic in `solitaire_data::xp_for_win` so the breakdown always
/// matches the total shown on the `XpAwardedEvent`.
///
/// Examples:
/// - slow win, no undo  → `"+50 base  +25 no-undo"`
/// - fast win, undo     → `"+50 base  +30 speed"`
/// - fast win, no undo  → `"+50 base  +25 no-undo  +30 speed"`
fn build_xp_detail(time_seconds: u64, used_undo: bool) -> String {
    let speed_bonus: u64 = if time_seconds >= 120 {
        0
    } else {
        let scaled = 50_u64.saturating_sub(time_seconds.saturating_mul(40) / 120);
        scaled.max(10)
    };
    let no_undo_bonus: u64 = if used_undo { 0 } else { 25 };

    let mut parts = vec!["+50 base".to_string()];
    if no_undo_bonus > 0 {
        parts.push("+25 no-undo".to_string());
    }
    if speed_bonus > 0 {
        parts.push(format!("+{speed_bonus} speed"));
    }
    parts.join("  ")
}

/// Drives the camera shake effect after a win.
///
/// While `remaining > 0` a system applies a decaying sinusoidal offset to the
/// main camera's `Transform`.  The system resets the camera to the origin when
/// `remaining` reaches zero.
#[derive(Resource, Debug, Clone, Default)]
pub struct ScreenShakeResource {
    /// Seconds of shake remaining.
    pub remaining: f32,
    /// Total duration the shake was armed for, used to compute the
    /// `remaining / total` decay factor. Tracked separately from `remaining`
    /// because the duration is now scaled by `AnimSpeed`, so a fixed
    /// divisor would be wrong on Fast.
    pub total: f32,
    /// Peak displacement in world-space pixels (decays to zero over `remaining`).
    pub intensity: f32,
}

/// Tracks the human-readable names of every achievement unlocked during the
/// current game session.
///
/// Populated by `collect_session_achievements` from `AchievementUnlockedEvent`s
/// and cleared whenever `NewGameRequestEvent` fires so each new game starts
/// with a fresh list. This includes all implicit game-context resets triggered
/// by mode-switch keys:
///
/// | Key | Mode | Event fired |
/// |-----|------|-------------|
/// | Z   | Zen                   | `NewGameRequestEvent { mode: Some(Zen), .. }` |
/// | X   | Challenge             | `NewGameRequestEvent { mode: Some(Challenge), .. }` |
/// | C   | Daily Challenge       | `NewGameRequestEvent { seed: Some(..), mode: None }` |
/// | T   | Time Attack           | `NewGameRequestEvent { mode: Some(TimeAttack), .. }` |
///
/// Because every mode switch routes through `NewGameRequestEvent`,
/// `collect_session_achievements` clears this list for all of them.
/// The win-summary modal reads this resource to display an
/// "Achievements Unlocked" section.
#[derive(Resource, Debug, Clone, Default)]
pub struct SessionAchievements {
    /// Display names (not IDs) of achievements unlocked this session, in
    /// unlock order.
    pub names: Vec<String>,
}

// ---------------------------------------------------------------------------
// Components
// ---------------------------------------------------------------------------

/// Marker on the win-summary modal root entity.
#[derive(Component, Debug)]
pub struct WinSummaryOverlay;

/// Marker on the "Play Again" button inside the win-summary modal.
#[derive(Component, Debug)]
enum WinSummaryButton {
    PlayAgain,
}

// ---------------------------------------------------------------------------
// Plugin
// ---------------------------------------------------------------------------

/// Registers the win-summary modal and screen-shake systems.
pub struct WinSummaryPlugin;

impl Plugin for WinSummaryPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<WinSummaryPending>()
            .init_resource::<ScreenShakeResource>()
            .init_resource::<SessionAchievements>()
            .add_message::<GameWonEvent>()
            .add_message::<XpAwardedEvent>()
            .add_message::<NewGameRequestEvent>()
            .add_message::<InfoToastEvent>()
            .add_message::<AchievementUnlockedEvent>()
            // `cache_win_data` must run BEFORE `StatsUpdate` so it can compare
            // the player's old personal-best values before `StatsPlugin` overwrites them.
            .add_systems(
                Update,
                cache_win_data
                    .after(GameMutation)
                    .before(StatsUpdate),
            )
            .add_systems(
                Update,
                (
                    collect_session_achievements,
                    spawn_win_summary_after_delay,
                    handle_win_summary_buttons,
                    apply_screen_shake,
                )
                    .after(GameMutation),
            );
    }
}

// ---------------------------------------------------------------------------
// Pure helpers
// ---------------------------------------------------------------------------

/// Formats `seconds` as `m:ss`.
///
/// ```
/// # use solitaire_engine::win_summary_plugin::format_win_time;
/// assert_eq!(format_win_time(0),   "0:00");
/// assert_eq!(format_win_time(65),  "1:05");
/// assert_eq!(format_win_time(3661), "61:01");
/// ```
pub fn format_win_time(seconds: u64) -> String {
    let m = seconds / 60;
    let s = seconds % 60;
    format!("{m}:{s:02}")
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// Caches score/time from `GameWonEvent` and XP from `XpAwardedEvent` into
/// `WinSummaryPending` so they are available when the modal spawns.
///
/// Also compares the win result against the player's previous personal bests
/// **before** `StatsUpdate` overwrites them, setting `WinSummaryPending::new_record`
/// and queuing an `InfoToastEvent` when the player sets a new record.
///
/// When the winning game is in `GameMode::Challenge`, the current
/// `challenge_index` (before `ChallengePlugin` advances it) is captured as the
/// 1-based level number and stored in `WinSummaryPending::challenge_level`.
///
/// This system is scheduled `.before(StatsUpdate)` so the comparison always
/// sees the old best values.
fn cache_win_data(
    mut won: MessageReader<GameWonEvent>,
    mut xp: MessageReader<XpAwardedEvent>,
    mut pending: ResMut<WinSummaryPending>,
    stats: Res<StatsResource>,
    game: Res<GameStateResource>,
    progress: Res<ProgressResource>,
    mut toast: MessageWriter<InfoToastEvent>,
) {
    for ev in won.read() {
        // Compare against old personal bests BEFORE StatsPlugin updates them.
        // `best_single_score == 0` means no wins yet — any positive score is a record.
        // `fastest_win_seconds == u64::MAX` is the sentinel for "no wins yet".
        let beats_score = ev.score > 0 && ev.score as u32 > stats.0.best_single_score;
        let beats_time = stats.0.fastest_win_seconds == u64::MAX
            || ev.time_seconds < stats.0.fastest_win_seconds;
        let is_new_record = beats_score || beats_time;

        // Capture the challenge level (1-based) before ChallengePlugin advances
        // the index. Only populated for Challenge-mode wins.
        let challenge_level = if game.0.mode == GameMode::Challenge {
            Some(progress.0.challenge_index.saturating_add(1))
        } else {
            None
        };

        let used_undo = game.0.undo_count > 0;
        pending.score = ev.score;
        pending.time_seconds = ev.time_seconds;
        pending.xp = 0; // reset; XP event follows
        pending.xp_detail = build_xp_detail(ev.time_seconds, used_undo);
        pending.new_record = is_new_record;
        pending.challenge_level = challenge_level;

        if is_new_record {
            toast.write(InfoToastEvent("New Record!".to_string()));
        }
    }
    for ev in xp.read() {
        pending.xp = ev.amount;
    }
}

/// Accumulates achievement names unlocked this session and resets them on a new game.
///
/// Listens for `AchievementUnlockedEvent` and appends the human-readable name
/// of each newly unlocked achievement to `SessionAchievements`. Clears the list
/// whenever `NewGameRequestEvent` fires so each fresh game starts clean.
///
/// All mode-switch keys (Z → Zen, X → Challenge, C → Daily Challenge,
/// T → Time Attack) route through `NewGameRequestEvent`, so this single
/// reader covers every implicit game-context reset in addition to the
/// explicit N / "Play Again" new-game requests.
fn collect_session_achievements(
    mut unlocks: MessageReader<AchievementUnlockedEvent>,
    mut new_games: MessageReader<NewGameRequestEvent>,
    mut session: ResMut<SessionAchievements>,
) {
    // Reset on any new-game request (including mode switches via Z/X/C/T) so
    // achievements from the previous session are not carried into the next one.
    if new_games.read().last().is_some() {
        session.names.clear();
    }
    for ev in unlocks.read() {
        session.names.push(display_name_for(&ev.0.id));
    }
}

/// After `GameWonEvent`, arms the screen-shake resource.
///
/// This system shares the `GameWonEvent` stream with `cache_win_data` through
/// the delay timer stored in `Local` — the shake fires immediately, while the
/// modal waits 0.5 s.
///
/// Just before the overlay is spawned the system also drains any pending
/// `XpAwardedEvent`s and folds their amounts into `pending.xp`.  This guards
/// against the edge case where `XpAwardedEvent` arrives in the same frame as
/// the timer fires but `cache_win_data` runs *after* this system in that
/// frame's schedule, which would otherwise leave `pending.xp` at 0 when
/// `spawn_overlay` reads it.
#[allow(clippy::too_many_arguments)]
fn spawn_win_summary_after_delay(
    mut commands: Commands,
    mut won: MessageReader<GameWonEvent>,
    mut xp_events: MessageReader<XpAwardedEvent>,
    mut shake: ResMut<ScreenShakeResource>,
    mut pending: ResMut<WinSummaryPending>,
    session: Res<SessionAchievements>,
    settings: Option<Res<SettingsResource>>,
    time: Res<Time>,
    overlays: Query<Entity, With<WinSummaryOverlay>>,
    mut delay: Local<Option<f32>>,
) {
    // Process new win events.
    for _ in won.read() {
        // Arm the screen shake immediately. Duration scales with the
        // player's `AnimSpeed` preference via `ui_theme::scaled_duration`;
        // intensity is left at its design-token value because amplitude
        // does not benefit from "fast" / "instant" scaling — at Instant
        // speed the duration is zero anyway, suppressing the shake.
        let speed = settings.as_ref().map_or(
            solitaire_data::AnimSpeed::Normal,
            |s| s.0.animation_speed,
        );
        let scaled = scaled_duration(SHAKE_DURATION_SECS, speed);
        shake.remaining = scaled;
        shake.total = scaled;
        shake.intensity = SHAKE_INTENSITY;
        // Start the delay timer (overwrite if a second win arrives).
        *delay = Some(WIN_SUMMARY_DELAY_SECS);
        // Clear any stale overlay from a previous win.
        for entity in &overlays {
            commands.entity(entity).despawn();
        }
    }

    // Tick the delay timer.
    if let Some(remaining) = delay.as_mut() {
        *remaining -= time.delta_secs();
        if *remaining <= 0.0 {
            *delay = None;
            // Only spawn if there is no overlay already.
            if overlays.is_empty() {
                // Drain any XpAwardedEvents that arrived this frame but were
                // not yet consumed by `cache_win_data` (which may run later in
                // the same schedule).  Accumulating here ensures the modal
                // never shows "XP: +0" due to a same-frame ordering race.
                for ev in xp_events.read() {
                    pending.xp = pending.xp.saturating_add(ev.amount);
                }
                let challenge_level = pending.challenge_level;
                spawn_overlay(&mut commands, &pending, &session, challenge_level);
            }
        }
    }
}

/// Despawns the win-summary modal and fires `NewGameRequestEvent` when
/// the player presses "Play Again".
fn handle_win_summary_buttons(
    interaction_query: Query<(&Interaction, &WinSummaryButton), Changed<Interaction>>,
    overlays: Query<Entity, With<WinSummaryOverlay>>,
    mut commands: Commands,
    mut new_game: MessageWriter<NewGameRequestEvent>,
) {
    for (interaction, button) in &interaction_query {
        if *interaction != Interaction::Pressed {
            continue;
        }
        match button {
            WinSummaryButton::PlayAgain => {
                // Despawn the modal.
                for entity in &overlays {
                    commands.entity(entity).despawn();
                }
                new_game.write(NewGameRequestEvent::default());
            }
        }
    }
}

/// Applies a decaying sinusoidal offset to the main `Camera2d` each frame
/// while `ScreenShakeResource::remaining > 0`.
///
/// Uses a deterministic oscillation (`sin`/`cos` of total elapsed time) to
/// avoid a dependency on a random-number crate in this crate.
fn apply_screen_shake(
    mut shake: ResMut<ScreenShakeResource>,
    time: Res<Time>,
    mut cameras: Query<&mut Transform, With<Camera2d>>,
) {
    let dt = time.delta_secs();
    if shake.remaining <= 0.0 {
        // Ensure the camera is back at origin whenever shake is idle.
        for mut t in &mut cameras {
            t.translation.x = 0.0;
            t.translation.y = 0.0;
        }
        return;
    }

    shake.remaining = (shake.remaining - dt).max(0.0);
    // Decay factor: 1.0 at start, 0.0 at end. Falls back to the design-token
    // duration if `total` is zero (older armings or test setups that bypass
    // `spawn_win_summary_after_delay`) so we never divide by zero.
    let total = if shake.total > 0.0 { shake.total } else { SHAKE_DURATION_SECS };
    let decay = shake.remaining / total;
    let elapsed = time.elapsed_secs();
    let offset_x = (elapsed * 47.0).sin() * shake.intensity * decay;
    let offset_y = (elapsed * 31.0).cos() * shake.intensity * decay;

    for mut t in &mut cameras {
        t.translation.x = offset_x;
        t.translation.y = offset_y;
    }
}

// ---------------------------------------------------------------------------
// UI construction
// ---------------------------------------------------------------------------

/// Spawns the full-screen win-summary modal.
///
/// `challenge_level` is `Some(N)` when the win was a Challenge-mode completion;
/// a "Challenge N complete!" annotation is added to the modal header in that case.
fn spawn_overlay(
    commands: &mut Commands,
    pending: &WinSummaryPending,
    session: &SessionAchievements,
    challenge_level: Option<u32>,
) {
    commands
        .spawn((
            WinSummaryOverlay,
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
            BackgroundColor(SCRIM),
            ZIndex(Z_WIN_CASCADE),
        ))
        .with_children(|root| {
            root.spawn((
                Node {
                    flex_direction: FlexDirection::Column,
                    padding: UiRect::all(Val::Px(36.0)),
                    row_gap: Val::Px(18.0),
                    min_width: Val::Px(320.0),
                    align_items: AlignItems::Center,
                    border_radius: BorderRadius::all(Val::Px(RADIUS_LG)),
                    ..default()
                },
                BackgroundColor(BG_ELEVATED),
            ))
            .with_children(|card| {
                // Heading
                card.spawn((
                    Text::new("You Won!"),
                    TextFont { font_size: TYPE_DISPLAY, ..default() },
                    TextColor(ACCENT_PRIMARY),
                ));

                // Challenge-mode annotation — shown only for Challenge wins.
                if let Some(level) = challenge_level {
                    card.spawn((
                        Text::new(format!("Challenge {level} complete!")),
                        TextFont { font_size: TYPE_HEADLINE, ..default() },
                        TextColor(STATE_INFO),
                    ));
                }

                // New Record badge — shown only when the player beats their
                // previous best score or fastest win time.
                if pending.new_record {
                    card.spawn((
                        Text::new("New Record!"),
                        TextFont { font_size: TYPE_HEADLINE, ..default() },
                        TextColor(STATE_WARNING),
                    ));
                }

                // Score
                card.spawn((
                    Text::new(format!("Score: {}", pending.score)),
                    TextFont { font_size: TYPE_HEADLINE, ..default() },
                    TextColor(TEXT_PRIMARY),
                ));

                // Time
                card.spawn((
                    Text::new(format!("Time: {}", format_win_time(pending.time_seconds))),
                    TextFont { font_size: TYPE_HEADLINE, ..default() },
                    TextColor(TEXT_PRIMARY),
                ));

                // XP total
                card.spawn((
                    Text::new(format!("XP earned: +{}", pending.xp)),
                    TextFont { font_size: TYPE_BODY_LG, ..default() },
                    TextColor(STATE_SUCCESS),
                ));

                // XP breakdown (smaller, dimmer text)
                if !pending.xp_detail.is_empty() {
                    card.spawn((
                        Text::new(pending.xp_detail.clone()),
                        TextFont { font_size: 15.0, ..default() },
                        TextColor(TEXT_SECONDARY),
                    ));
                }

                // Achievements unlocked this game — at most 3 shown explicitly;
                // excess is summarised with "...and N more".
                if !session.names.is_empty() {
                    spawn_achievements_section(card, &session.names);
                }

                // Play Again button
                card.spawn((
                    WinSummaryButton::PlayAgain,
                    Button,
                    Node {
                        padding: UiRect::axes(Val::Px(28.0), VAL_SPACE_3),
                        justify_content: JustifyContent::Center,
                        margin: UiRect::top(VAL_SPACE_2),
                        border_radius: BorderRadius::all(Val::Px(RADIUS_MD)),
                        ..default()
                    },
                    BackgroundColor(ACCENT_PRIMARY),
                ))
                .with_children(|b| {
                    b.spawn((
                        Text::new("Play Again"),
                        TextFont { font_size: TYPE_BODY_LG, ..default() },
                        TextColor(BG_BASE),
                    ));
                });
            });
        });
}

/// Maximum number of achievement names shown explicitly in the win modal before
/// the overflow "...and N more" line is shown instead.
const MAX_ACHIEVEMENTS_SHOWN: usize = 3;

/// Spawns the "Achievements Unlocked" sub-section inside the win modal card.
///
/// Shows at most [`MAX_ACHIEVEMENTS_SHOWN`] names. When more achievements were
/// unlocked than the cap, appends a "...and N more" line so the player knows
/// there are additional unlocks visible on the achievements screen.
fn spawn_achievements_section(card: &mut ChildSpawnerCommands, names: &[String]) {
    card.spawn((
        Text::new("Achievements Unlocked"),
        TextFont { font_size: TYPE_BODY_LG, ..default() },
        TextColor(ACCENT_PRIMARY),
    ));

    let shown = names.len().min(MAX_ACHIEVEMENTS_SHOWN);
    for name in &names[..shown] {
        card.spawn((
            Text::new(format!("  {name}")),
            TextFont { font_size: 16.0, ..default() },
            TextColor(TEXT_PRIMARY),
        ));
    }

    let overflow = names.len().saturating_sub(MAX_ACHIEVEMENTS_SHOWN);
    if overflow > 0 {
        card.spawn((
            Text::new(format!("  ...and {overflow} more")),
            TextFont { font_size: 15.0, ..default() },
            TextColor(TEXT_SECONDARY),
        ));
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use solitaire_core::game_state::GameState;
    use solitaire_data::{PlayerProgress, StatsSnapshot};

    /// Build a minimal app with `WinSummaryPlugin` and all resources required
    /// by `cache_win_data`: `StatsResource`, `GameStateResource`, and
    /// `ProgressResource`.
    fn make_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(WinSummaryPlugin)
            .insert_resource(StatsResource(StatsSnapshot::default()))
            .insert_resource(GameStateResource(GameState::new(0, solitaire_core::game_state::DrawMode::DrawOne)))
            .insert_resource(ProgressResource(PlayerProgress::default()));
        app.update();
        app
    }

    #[test]
    fn format_win_time_zero() {
        assert_eq!(format_win_time(0), "0:00");
    }

    #[test]
    fn format_win_time_one_minute_five_seconds() {
        assert_eq!(format_win_time(65), "1:05");
    }

    #[test]
    fn format_win_time_exact_minute() {
        assert_eq!(format_win_time(120), "2:00");
    }

    #[test]
    fn format_win_time_large() {
        // 3661 s = 61 min 1 s
        assert_eq!(format_win_time(3661), "61:01");
    }

    #[test]
    fn format_win_time_59_seconds() {
        assert_eq!(format_win_time(59), "0:59");
    }

    #[test]
    fn screen_shake_resource_default_is_idle() {
        let shake = ScreenShakeResource::default();
        assert!(shake.remaining <= 0.0);
    }

    #[test]
    fn win_summary_pending_default_is_zeroed() {
        let p = WinSummaryPending::default();
        assert_eq!(p.score, 0);
        assert_eq!(p.time_seconds, 0);
        assert_eq!(p.xp, 0);
        assert!(p.xp_detail.is_empty());
        assert!(!p.new_record);
        assert!(p.challenge_level.is_none());
    }

    #[test]
    fn build_xp_detail_slow_win_with_undo() {
        // 300s >= 120s → no speed bonus; undo used → no no-undo bonus.
        let detail = build_xp_detail(300, true);
        assert_eq!(detail, "+50 base");
    }

    #[test]
    fn build_xp_detail_slow_win_no_undo() {
        let detail = build_xp_detail(300, false);
        assert_eq!(detail, "+50 base  +25 no-undo");
    }

    #[test]
    fn build_xp_detail_fast_win_with_undo() {
        // 0s → speed bonus 50.
        let detail = build_xp_detail(0, true);
        assert_eq!(detail, "+50 base  +50 speed");
    }

    #[test]
    fn build_xp_detail_fast_win_no_undo() {
        let detail = build_xp_detail(0, false);
        assert_eq!(detail, "+50 base  +25 no-undo  +50 speed");
    }

    #[test]
    fn win_summary_plugin_inserts_resources() {
        let app = make_app();
        assert!(app.world().get_resource::<WinSummaryPending>().is_some());
        assert!(app.world().get_resource::<ScreenShakeResource>().is_some());
        assert!(app.world().get_resource::<SessionAchievements>().is_some());
    }

    #[test]
    fn session_achievements_accumulates_unlock_events() {
        let mut app = make_app();

        use solitaire_data::AchievementRecord;
        let record = AchievementRecord::locked("first_win");
        app.world_mut()
            .write_message(AchievementUnlockedEvent(record));
        app.update();

        let session = app.world().resource::<SessionAchievements>();
        assert_eq!(session.names.len(), 1);
        // display_name_for("first_win") == "First Win"
        assert_eq!(session.names[0], "First Win");
    }

    #[test]
    fn session_achievements_resets_on_new_game_request() {
        let mut app = make_app();

        use solitaire_data::AchievementRecord;
        let record = AchievementRecord::locked("first_win");
        app.world_mut()
            .write_message(AchievementUnlockedEvent(record));
        app.update();

        // Confirm it was recorded.
        assert_eq!(
            app.world().resource::<SessionAchievements>().names.len(),
            1
        );

        // Fire NewGameRequestEvent — should clear the list.
        app.world_mut().write_message(NewGameRequestEvent::default());
        app.update();

        assert!(
            app.world().resource::<SessionAchievements>().names.is_empty(),
            "session achievements must be cleared on NewGameRequestEvent"
        );
    }

    /// Verifies that mode-switch new-game requests (Z/X/C/T keys) also clear
    /// `SessionAchievements`. All mode switches route through
    /// `NewGameRequestEvent` with a non-`None` `mode` or `seed` field, so
    /// this test uses `GameMode::Zen` as a representative case; the same path
    /// is taken for Challenge, Daily Challenge, and Time Attack.
    #[test]
    fn session_achievements_resets_on_mode_switch_new_game_request() {
        let mut app = make_app();

        use solitaire_core::game_state::GameMode;
        use solitaire_data::AchievementRecord;

        // Simulate an achievement unlock during the current session.
        let record = AchievementRecord::locked("first_win");
        app.world_mut()
            .write_message(AchievementUnlockedEvent(record));
        app.update();

        assert_eq!(
            app.world().resource::<SessionAchievements>().names.len(),
            1,
            "achievement should be recorded before the mode switch"
        );

        // Simulate pressing Z (Zen mode switch) — fires NewGameRequestEvent
        // with mode = Some(Zen). Same event shape used by X (Challenge),
        // C (Daily Challenge), and T (Time Attack).
        app.world_mut().write_message(NewGameRequestEvent {
            seed: None,
            mode: Some(GameMode::Zen),
            confirmed: false,
        });
        app.update();

        assert!(
            app.world().resource::<SessionAchievements>().names.is_empty(),
            "session achievements must be cleared when a mode-switch NewGameRequestEvent fires"
        );
    }

    #[test]
    fn cache_win_data_sets_score_and_time() {
        let mut app = make_app();

        app.world_mut()
            .write_message(GameWonEvent { score: 1234, time_seconds: 90 });
        app.update();

        let pending = app.world().resource::<WinSummaryPending>();
        assert_eq!(pending.score, 1234);
        assert_eq!(pending.time_seconds, 90);
        // 90s < 120s → speed bonus present; default game has undo_count=0 → no-undo bonus present.
        assert!(!pending.xp_detail.is_empty(), "xp_detail must be populated on GameWonEvent");
        assert!(pending.xp_detail.contains("+50 base"));
    }

    #[test]
    fn cache_win_data_sets_xp_from_xp_awarded_event() {
        let mut app = make_app();

        app.world_mut().write_message(GameWonEvent { score: 0, time_seconds: 0 });
        app.world_mut().write_message(XpAwardedEvent { amount: 75 });
        app.update();

        let pending = app.world().resource::<WinSummaryPending>();
        assert_eq!(pending.xp, 75);
    }

    #[test]
    fn game_won_event_arms_screen_shake() {
        let mut app = make_app();

        app.world_mut()
            .write_message(GameWonEvent { score: 0, time_seconds: 0 });
        app.update();

        let shake = app.world().resource::<ScreenShakeResource>();
        assert!(shake.remaining > 0.0, "shake must be armed after GameWonEvent");
    }

    // -----------------------------------------------------------------------
    // New Record detection tests
    // -----------------------------------------------------------------------

    #[test]
    fn first_win_is_always_a_new_record() {
        // Default stats: best_single_score=0, fastest_win_seconds=u64::MAX.
        // Any positive-score win should be flagged as a new record.
        let mut app = make_app();

        app.world_mut()
            .write_message(GameWonEvent { score: 500, time_seconds: 120 });
        app.update();

        let pending = app.world().resource::<WinSummaryPending>();
        assert!(pending.new_record, "first win should always set new_record");
    }

    #[test]
    fn win_that_beats_best_score_sets_new_record() {
        let mut app = make_app();
        {
            let mut stats = app.world_mut().resource_mut::<StatsResource>();
            stats.0.best_single_score = 400;
            stats.0.fastest_win_seconds = 200;
        }

        // Score 500 beats previous best of 400.
        app.world_mut()
            .write_message(GameWonEvent { score: 500, time_seconds: 300 });
        app.update();

        let pending = app.world().resource::<WinSummaryPending>();
        assert!(pending.new_record, "beating best score should set new_record");
    }

    #[test]
    fn win_that_beats_fastest_time_sets_new_record() {
        let mut app = make_app();
        {
            let mut stats = app.world_mut().resource_mut::<StatsResource>();
            stats.0.best_single_score = 800;
            stats.0.fastest_win_seconds = 200;
        }

        // Score 500 does not beat 800, but time 100 < 200.
        app.world_mut()
            .write_message(GameWonEvent { score: 500, time_seconds: 100 });
        app.update();

        let pending = app.world().resource::<WinSummaryPending>();
        assert!(pending.new_record, "beating fastest time should set new_record");
    }

    #[test]
    fn win_below_personal_bests_does_not_set_new_record() {
        let mut app = make_app();
        {
            let mut stats = app.world_mut().resource_mut::<StatsResource>();
            stats.0.best_single_score = 800;
            stats.0.fastest_win_seconds = 60;
        }

        // Score 500 < 800 and time 120 > 60 — neither record broken.
        app.world_mut()
            .write_message(GameWonEvent { score: 500, time_seconds: 120 });
        app.update();

        let pending = app.world().resource::<WinSummaryPending>();
        assert!(
            !pending.new_record,
            "win below both personal bests must not set new_record"
        );
    }

    // -----------------------------------------------------------------------
    // Challenge-level capture tests
    // -----------------------------------------------------------------------

    #[test]
    fn challenge_win_captures_level_number() {
        let mut app = make_app();

        // Set challenge_index = 4 so the completed level is 5 (1-based).
        app.world_mut()
            .resource_mut::<ProgressResource>()
            .0
            .challenge_index = 4;
        // Switch game mode to Challenge.
        {
            use solitaire_core::game_state::DrawMode;
            app.world_mut().resource_mut::<GameStateResource>().0 =
                GameState::new_with_mode(1, DrawMode::DrawOne, GameMode::Challenge);
        }

        app.world_mut()
            .write_message(GameWonEvent { score: 0, time_seconds: 0 });
        app.update();

        let pending = app.world().resource::<WinSummaryPending>();
        assert_eq!(
            pending.challenge_level,
            Some(5),
            "challenge_level must be 1-based index of the completed challenge"
        );
    }

    #[test]
    fn classic_win_leaves_challenge_level_none() {
        let mut app = make_app();
        // Default game mode is Classic — challenge_level should stay None.
        app.world_mut()
            .write_message(GameWonEvent { score: 0, time_seconds: 0 });
        app.update();

        let pending = app.world().resource::<WinSummaryPending>();
        assert!(
            pending.challenge_level.is_none(),
            "challenge_level must be None for non-Challenge wins"
        );
    }
}
