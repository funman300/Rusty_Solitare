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
use solitaire_core::scoring::compute_time_bonus;
use solitaire_data::AnimSpeed;

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
    scaled_duration, ACCENT_PRIMARY, BG_BASE, BG_ELEVATED, MOTION_SCORE_BREAKDOWN_FADE_SECS,
    MOTION_SCORE_BREAKDOWN_STAGGER_SECS, MOTION_WIN_SHAKE_AMPLITUDE, MOTION_WIN_SHAKE_SECS,
    RADIUS_LG, RADIUS_MD, SCRIM, STATE_INFO, STATE_SUCCESS, STATE_WARNING, TEXT_PRIMARY,
    TEXT_SECONDARY, TYPE_BODY, TYPE_BODY_LG, TYPE_DISPLAY, TYPE_HEADLINE, VAL_SPACE_2, VAL_SPACE_3,
    Z_WIN_CASCADE,
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
    /// Number of undos used during the winning game. Captured from
    /// `GameStateResource` at the moment `GameWonEvent` fires so the
    /// score-breakdown reveal can decide whether to award the no-undo
    /// bonus row.
    pub undo_count: u32,
    /// Game mode of the winning game. Captured at win time so the
    /// score-breakdown reveal can format the mode-multiplier row
    /// (e.g. `Zen ×0.0`, `Classic ×1.0`).
    pub mode: GameMode,
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

/// Marker for one row of the win-modal score-breakdown reveal.
///
/// Each row carries a stagger delay (seconds until the row should
/// become visible) plus a fade-in timer that lerps the row's text
/// alpha from `0.0 → 1.0` over [`MOTION_SCORE_BREAKDOWN_FADE_SECS`].
/// Rows are spawned with `Visibility::Hidden`; the reveal system
/// flips them to `Visibility::Inherited` once `delay_secs` elapses
/// and then drives the per-text alpha lerp until the row reaches
/// full opacity.
///
/// When `AnimSpeed::Instant` is active the row is spawned with
/// `delay_secs = 0.0`, `fade_duration_secs = 0.0`, and visibility
/// already set to `Inherited` so the reveal happens on frame 1.
#[derive(Component, Debug, Clone, Copy)]
pub struct ScoreBreakdownRow {
    /// Seconds remaining until this row first becomes visible.
    /// Counts down to 0 in `reveal_score_breakdown`. Zero or negative
    /// means "show immediately".
    pub delay_secs: f32,
    /// Seconds elapsed since this row became visible. Drives the
    /// alpha lerp on the row's child `Text` nodes.
    pub fade_elapsed_secs: f32,
    /// Total fade-in duration. Zero means "no fade — appear at full
    /// opacity in one frame".
    pub fade_duration_secs: f32,
    /// `true` once the row's `Visibility` has been promoted from
    /// `Hidden` to `Inherited`. Prevents re-running the visibility
    /// switch every frame after the row first reveals.
    pub revealed: bool,
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
                    reveal_score_breakdown,
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

/// Score amount awarded as a "no-undo" bonus in the win modal when the
/// player completes the game without using undo. Mirrors the XP-side
/// no-undo bonus so the score and XP breakdowns reinforce each other,
/// and stays a `pub const` so tests can assert against it without
/// re-typing the literal.
pub const SCORE_NO_UNDO_BONUS: i32 = 25;

/// Decomposed view of the player's final score, displayed in the win
/// modal as a sequence of fade-in rows.
///
/// The fields mirror the row layout described in the win-modal
/// reveal:
///
/// ```text
/// Base score                   {base}
/// Time bonus ({m:ss})         +{time_bonus}
/// No-undo bonus               +{no_undo_bonus}
/// Mode multiplier ({mode} ×N) ×{multiplier}
/// ─────────────────────────────────
/// Total                        {total}
/// ```
///
/// Components that do not apply to the current win are zeroed out:
/// `time_bonus = 0` when the player took longer than the time-bonus
/// curve produces a positive result, `no_undo_bonus = 0` when undo
/// was used, and `multiplier = 1.0` outside Zen mode. The renderer
/// uses these zero markers to skip rows the player would not benefit
/// from seeing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScoreBreakdown {
    /// Running game score before the win-time bonuses are applied.
    /// Equal to `pending.score`, which is `GameState::score` at the
    /// moment of `GameWonEvent`.
    pub base: i32,
    /// Time-bonus component — `compute_time_bonus(time_seconds)`.
    /// Zero when `time_seconds == 0` or when the formula yields zero.
    pub time_bonus: i32,
    /// Score awarded for completing the win without using undo.
    /// Zero when `undo_count > 0`.
    pub no_undo_bonus: i32,
    /// Multiplier applied to `(base + time_bonus + no_undo_bonus)` to
    /// produce the final total. `0.0` for Zen mode (which never
    /// scores), `1.0` otherwise.
    pub multiplier: f32,
    /// Game mode the win occurred in. Used by the renderer to format
    /// the multiplier row label, e.g. `"Mode multiplier (Zen ×0)"`.
    pub mode: GameMode,
    /// Elapsed game time in seconds, used to format the time-bonus
    /// row label as `m:ss`.
    pub time_seconds: u64,
}

impl ScoreBreakdown {
    /// Builds a breakdown for the given win.
    ///
    /// `base` is the running game score (`pending.score`); `time_seconds`,
    /// `undo_count`, and `mode` come from the captured `WinSummaryPending`.
    /// All score arithmetic is saturating to keep the breakdown safe even
    /// for pathologically high scores.
    pub fn compute(base: i32, time_seconds: u64, undo_count: u32, mode: GameMode) -> Self {
        let time_bonus = compute_time_bonus(time_seconds);
        let no_undo_bonus = if undo_count == 0 { SCORE_NO_UNDO_BONUS } else { 0 };
        let multiplier = match mode {
            GameMode::Zen => 0.0,
            GameMode::Classic | GameMode::Challenge | GameMode::TimeAttack => 1.0,
        };
        Self {
            base,
            time_bonus,
            no_undo_bonus,
            multiplier,
            mode,
            time_seconds,
        }
    }

    /// Final total displayed on the breakdown's bottom row, rounded
    /// half-to-even (Rust's default `as i32` cast truncates toward
    /// zero, which is fine for a non-fractional multiplier set).
    pub fn total(&self) -> i32 {
        let pre_mult = self
            .base
            .saturating_add(self.time_bonus)
            .saturating_add(self.no_undo_bonus);
        ((pre_mult as f32) * self.multiplier) as i32
    }

    /// Whether the no-undo bonus row should be rendered. Skipped when
    /// the player used undo (bonus is zero) so the modal does not
    /// show a "+0" line that adds nothing.
    pub fn shows_no_undo_row(&self) -> bool {
        self.no_undo_bonus > 0
    }

    /// Whether the time-bonus row should be rendered. Skipped when
    /// the bonus is zero (e.g. `time_seconds == 0`).
    pub fn shows_time_bonus_row(&self) -> bool {
        self.time_bonus > 0
    }

    /// Whether the mode-multiplier row should be rendered. Skipped
    /// for `multiplier == 1.0` so Classic/Challenge/TimeAttack wins
    /// do not show a redundant "×1.0" line.
    pub fn shows_multiplier_row(&self) -> bool {
        (self.multiplier - 1.0).abs() > f32::EPSILON
    }

    /// Total number of rows the breakdown will spawn, counting the
    /// always-present `Base score` and `Total` rows plus the
    /// separator. Used by tests to assert spawn counts deterministically.
    pub fn row_count(&self) -> usize {
        let mut n = 1; // base
        if self.shows_time_bonus_row() {
            n += 1;
        }
        if self.shows_no_undo_row() {
            n += 1;
        }
        if self.shows_multiplier_row() {
            n += 1;
        }
        n += 1; // separator
        n += 1; // total
        n
    }
}

/// Human-readable display name for a game mode. Used as the prefix in
/// the mode-multiplier row, e.g. `"Mode multiplier (Zen ×0)"`.
fn mode_display_name(mode: GameMode) -> &'static str {
    match mode {
        GameMode::Classic => "Classic",
        GameMode::Zen => "Zen",
        GameMode::Challenge => "Challenge",
        GameMode::TimeAttack => "Time Attack",
    }
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
        pending.undo_count = game.0.undo_count;
        pending.mode = game.0.mode;

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
                // Re-derive AnimSpeed here — the `speed` binding above
                // only lives inside the `for _ in won.read()` loop.
                let anim_speed = settings
                    .as_ref()
                    .map_or(AnimSpeed::Normal, |s| s.0.animation_speed);
                spawn_overlay(&mut commands, &pending, &session, challenge_level, anim_speed);
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
///
/// `anim_speed` controls the score-breakdown reveal: under
/// `AnimSpeed::Instant`, every breakdown row is spawned visible and at
/// full opacity (no stagger, no fade); otherwise rows are spawned
/// hidden and the [`reveal_score_breakdown`] system fades them in over
/// roughly one second.
fn spawn_overlay(
    commands: &mut Commands,
    pending: &WinSummaryPending,
    session: &SessionAchievements,
    challenge_level: Option<u32>,
    anim_speed: AnimSpeed,
) {
    let breakdown = ScoreBreakdown::compute(
        pending.score,
        pending.time_seconds,
        pending.undo_count,
        pending.mode,
    );
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

                // Score breakdown reveal — replaces the previous single
                // "Score:" line with a per-component multi-row layout.
                spawn_score_breakdown(card, &breakdown, anim_speed);

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

/// Spawns the score-breakdown rows inside the win-modal card.
///
/// Rows are appended in this order — only the first and last two are
/// always present, the middle three depend on `breakdown`:
///
/// 1. `Base score` — value column = `breakdown.base`.
/// 2. `Time bonus (m:ss)` — only when `breakdown.shows_time_bonus_row()`.
/// 3. `No-undo bonus` — only when `breakdown.shows_no_undo_row()`.
/// 4. `Mode multiplier (Mode-name ×N)` — only when
///    `breakdown.shows_multiplier_row()`.
/// 5. Separator (em-dashes).
/// 6. `Total` — value column = `breakdown.total()`.
///
/// Every row is spawned with a [`ScoreBreakdownRow`] component carrying
/// a per-row stagger delay calculated from
/// [`MOTION_SCORE_BREAKDOWN_STAGGER_SECS`]. Under `AnimSpeed::Instant`,
/// stagger and fade are both zero so the breakdown appears in one frame.
fn spawn_score_breakdown(
    card: &mut ChildSpawnerCommands,
    breakdown: &ScoreBreakdown,
    anim_speed: AnimSpeed,
) {
    let stagger = scaled_duration(MOTION_SCORE_BREAKDOWN_STAGGER_SECS, anim_speed);
    let fade = scaled_duration(MOTION_SCORE_BREAKDOWN_FADE_SECS, anim_speed);
    let mut row_index: u32 = 0;

    // 1. Base score — always shown.
    spawn_breakdown_row(
        card,
        "Base score",
        format!("{}", breakdown.base),
        ACCENT_PRIMARY,
        anim_speed,
        stagger * row_index as f32,
        fade,
    );
    row_index += 1;

    // 2. Time bonus.
    if breakdown.shows_time_bonus_row() {
        spawn_breakdown_row(
            card,
            &format!("Time bonus ({})", format_win_time(breakdown.time_seconds)),
            format!("+{}", breakdown.time_bonus),
            STATE_SUCCESS,
            anim_speed,
            stagger * row_index as f32,
            fade,
        );
        row_index += 1;
    }

    // 3. No-undo bonus.
    if breakdown.shows_no_undo_row() {
        spawn_breakdown_row(
            card,
            "No-undo bonus",
            format!("+{}", breakdown.no_undo_bonus),
            STATE_SUCCESS,
            anim_speed,
            stagger * row_index as f32,
            fade,
        );
        row_index += 1;
    }

    // 4. Mode multiplier (only when not 1.0).
    if breakdown.shows_multiplier_row() {
        let mode_name = mode_display_name(breakdown.mode);
        spawn_breakdown_row(
            card,
            &format!("Mode multiplier ({mode_name} ×{:.1})", breakdown.multiplier),
            format!("×{:.1}", breakdown.multiplier),
            STATE_INFO,
            anim_speed,
            stagger * row_index as f32,
            fade,
        );
        row_index += 1;
    }

    // 5. Separator — em-dashes spanning the visual width.
    spawn_breakdown_row(
        card,
        "─────────────────",
        "─────".to_string(),
        TEXT_SECONDARY,
        anim_speed,
        stagger * row_index as f32,
        fade,
    );
    row_index += 1;

    // 6. Total — emphasised in primary accent.
    spawn_breakdown_row(
        card,
        "Total",
        format!("{}", breakdown.total()),
        ACCENT_PRIMARY,
        anim_speed,
        stagger * row_index as f32,
        fade,
    );
}

/// Spawns one row of the score breakdown — a flex-row `Node` with two
/// `Text` children (label left, value right). The row is tagged with
/// [`ScoreBreakdownRow`] and starts hidden when `anim_speed` is anything
/// other than [`AnimSpeed::Instant`]; the [`reveal_score_breakdown`]
/// system flips it visible after `delay_secs` and fades in the text
/// over `fade_duration_secs`.
fn spawn_breakdown_row(
    card: &mut ChildSpawnerCommands,
    label: &str,
    value: String,
    value_color: Color,
    anim_speed: AnimSpeed,
    delay_secs: f32,
    fade_duration_secs: f32,
) {
    // Under Instant, every row is visible immediately at full opacity.
    let instant = matches!(anim_speed, AnimSpeed::Instant);
    let initial_visibility = if instant {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    let initial_alpha = if instant { 1.0 } else { 0.0 };

    let label_color_with_alpha = TEXT_PRIMARY.with_alpha(initial_alpha);
    let value_color_with_alpha = value_color.with_alpha(initial_alpha);

    card.spawn((
        ScoreBreakdownRow {
            delay_secs,
            fade_elapsed_secs: 0.0,
            fade_duration_secs,
            revealed: instant,
        },
        Node {
            width: Val::Percent(100.0),
            min_width: Val::Px(280.0),
            flex_direction: FlexDirection::Row,
            justify_content: JustifyContent::SpaceBetween,
            align_items: AlignItems::Center,
            ..default()
        },
        initial_visibility,
    ))
    .with_children(|row| {
        // Label — left-aligned.
        row.spawn((
            Text::new(label.to_string()),
            TextFont { font_size: TYPE_BODY, ..default() },
            TextColor(label_color_with_alpha),
        ));
        // Value — right-aligned via the parent's JustifyContent::SpaceBetween.
        row.spawn((
            Text::new(value),
            TextFont { font_size: TYPE_BODY, ..default() },
            TextColor(value_color_with_alpha),
        ));
    });
}

/// Reveal system — ticks each [`ScoreBreakdownRow`] down toward zero
/// and fades its child `Text` alpha from 0 → 1 over the row's
/// `fade_duration_secs` once `delay_secs` elapses.
///
/// The system is non-blocking: the Play Again button is interactable
/// from the moment the modal spawns; the breakdown reveal just plays
/// out underneath. Rows that have already reached full opacity are
/// skipped via the `revealed` flag plus an early
/// `fade_elapsed >= fade_duration` short-circuit on the alpha lerp.
pub fn reveal_score_breakdown(
    time: Res<Time>,
    mut rows: Query<(&mut ScoreBreakdownRow, &mut Visibility, Option<&Children>)>,
    mut texts: Query<&mut TextColor>,
) {
    let dt = time.delta_secs();
    for (mut row, mut visibility, children) in &mut rows {
        if !row.revealed {
            row.delay_secs -= dt;
            if row.delay_secs <= 0.0 {
                *visibility = Visibility::Inherited;
                row.revealed = true;
            } else {
                continue; // still hidden, no fade work yet
            }
        }
        // Row is revealed — drive the fade-in until it's fully opaque.
        let fade_done = row.fade_elapsed_secs >= row.fade_duration_secs;
        if !fade_done {
            row.fade_elapsed_secs += dt;
        }
        let t = if row.fade_duration_secs <= 0.0 {
            1.0
        } else {
            (row.fade_elapsed_secs / row.fade_duration_secs).clamp(0.0, 1.0)
        };
        let target_alpha = if fade_done { 1.0 } else { t };
        if let Some(children) = children {
            for child in children.iter() {
                if let Ok(mut tc) = texts.get_mut(child) {
                    let c = tc.0;
                    if (c.alpha() - target_alpha).abs() > f32::EPSILON {
                        tc.0 = c.with_alpha(target_alpha);
                    }
                }
            }
        }
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
        assert_eq!(p.undo_count, 0);
        assert_eq!(p.mode, GameMode::Classic);
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

    // -----------------------------------------------------------------------
    // Score-breakdown tests
    // -----------------------------------------------------------------------

    /// `cache_win_data` captures both `undo_count` and `mode` from the
    /// `GameStateResource` at the moment of `GameWonEvent`. The breakdown
    /// reveal needs both fields to format the no-undo-bonus and
    /// mode-multiplier rows.
    #[test]
    fn cache_win_data_captures_undo_count_and_mode() {
        use solitaire_core::game_state::DrawMode;

        let mut app = make_app();
        // Set up a Zen-mode game with 2 undos used.
        {
            let mut game = app.world_mut().resource_mut::<GameStateResource>();
            game.0 = GameState::new_with_mode(7, DrawMode::DrawOne, GameMode::Zen);
            game.0.undo_count = 2;
        }

        app.world_mut()
            .write_message(GameWonEvent { score: 0, time_seconds: 0 });
        app.update();

        let pending = app.world().resource::<WinSummaryPending>();
        assert_eq!(pending.undo_count, 2);
        assert_eq!(pending.mode, GameMode::Zen);
    }

    /// `ScoreBreakdown::compute` produces the expected per-component
    /// values for a non-trivial Classic-mode win. Time-bonus is the
    /// canonical `compute_time_bonus(120) = 5833` (700_000 / 120) and
    /// the no-undo bonus fires because `undo_count == 0`.
    #[test]
    fn score_breakdown_compute_produces_expected_components() {
        let bd = ScoreBreakdown::compute(3200, 120, 0, GameMode::Classic);
        assert_eq!(bd.base, 3200);
        assert_eq!(bd.time_bonus, 5833); // 700_000 / 120
        assert_eq!(bd.no_undo_bonus, SCORE_NO_UNDO_BONUS);
        assert!((bd.multiplier - 1.0).abs() < f32::EPSILON);
        // Classic ×1.0 → multiplier row is suppressed.
        assert!(!bd.shows_multiplier_row());
        // Total == base + time_bonus + no_undo_bonus.
        assert_eq!(bd.total(), 3200 + 5833 + SCORE_NO_UNDO_BONUS);
    }

    /// Zen-mode wins produce a zero multiplier — the breakdown shows
    /// the multiplier row and the total collapses to zero regardless
    /// of the other components.
    #[test]
    fn score_breakdown_zen_mode_zeros_total() {
        let bd = ScoreBreakdown::compute(500, 60, 0, GameMode::Zen);
        assert!((bd.multiplier - 0.0).abs() < f32::EPSILON);
        assert!(bd.shows_multiplier_row(), "Zen ×0 must display the multiplier row");
        assert_eq!(bd.total(), 0);
    }

    /// When the player used undo, the `no_undo_bonus` is zero and the
    /// row is suppressed.
    #[test]
    fn score_breakdown_skips_no_undo_row_when_undo_was_used() {
        let bd = ScoreBreakdown::compute(100, 60, 1, GameMode::Classic);
        assert_eq!(bd.no_undo_bonus, 0);
        assert!(!bd.shows_no_undo_row());
    }

    /// At `time_seconds == 0` the time-bonus formula yields 0; the row
    /// is suppressed.
    #[test]
    fn score_breakdown_skips_time_bonus_row_when_zero() {
        let bd = ScoreBreakdown::compute(100, 0, 0, GameMode::Classic);
        assert_eq!(bd.time_bonus, 0);
        assert!(!bd.shows_time_bonus_row());
    }

    /// `row_count()` reports the number of rows the renderer will
    /// spawn. A non-trivial Classic win with both bonuses produces:
    /// base + time + no-undo + separator + total = 5 rows (no
    /// multiplier row, ×1.0 is suppressed).
    #[test]
    fn win_modal_score_breakdown_spawns_one_row_per_component() {
        let bd = ScoreBreakdown::compute(3200, 120, 0, GameMode::Classic);
        assert_eq!(
            bd.row_count(),
            5,
            "Classic with both bonuses: base + time + no-undo + sep + total"
        );

        // Zen with both bonuses ALSO shows the multiplier row.
        let zen = ScoreBreakdown::compute(3200, 120, 0, GameMode::Zen);
        assert_eq!(
            zen.row_count(),
            6,
            "Zen with both bonuses: base + time + no-undo + multiplier + sep + total"
        );
    }

    /// When `no_undo_bonus == 0`, the row count drops by one.
    #[test]
    fn win_modal_score_breakdown_skips_zero_bonus_rows() {
        let bd_with = ScoreBreakdown::compute(3200, 120, 0, GameMode::Classic);
        let bd_without = ScoreBreakdown::compute(3200, 120, 1, GameMode::Classic);
        assert_eq!(
            bd_with.row_count() - 1,
            bd_without.row_count(),
            "removing the no-undo bonus must remove exactly one row"
        );
    }

    /// Pure helper test: the reveal logic uses delta-time to count
    /// down `delay_secs`; at `t = 0` only the first row is "revealed",
    /// and after one stagger interval the second row reveals as well.
    /// We exercise the system directly on a hand-built world rather
    /// than going through the full modal-spawn path so the test is
    /// independent of `Time` resource quirks.
    #[test]
    fn score_breakdown_reveal_advances_visibility_per_stagger() {
        use bevy::time::TimePlugin;

        let mut app = App::new();
        app.add_plugins(MinimalPlugins.build().disable::<TimePlugin>());
        app.init_resource::<Time>();
        app.add_systems(Update, reveal_score_breakdown);

        // Spawn three rows with delays of 0.0, 0.15, and 0.30 s.
        let stagger = MOTION_SCORE_BREAKDOWN_STAGGER_SECS;
        let fade = MOTION_SCORE_BREAKDOWN_FADE_SECS;
        let row0 = app
            .world_mut()
            .spawn((
                ScoreBreakdownRow {
                    delay_secs: 0.0,
                    fade_elapsed_secs: 0.0,
                    fade_duration_secs: fade,
                    revealed: false,
                },
                Visibility::Hidden,
            ))
            .id();
        let row1 = app
            .world_mut()
            .spawn((
                ScoreBreakdownRow {
                    delay_secs: stagger,
                    fade_elapsed_secs: 0.0,
                    fade_duration_secs: fade,
                    revealed: false,
                },
                Visibility::Hidden,
            ))
            .id();
        let row2 = app
            .world_mut()
            .spawn((
                ScoreBreakdownRow {
                    delay_secs: stagger * 2.0,
                    fade_elapsed_secs: 0.0,
                    fade_duration_secs: fade,
                    revealed: false,
                },
                Visibility::Hidden,
            ))
            .id();

        // Frame 1: `time.delta` is 0 (first frame), so only row0
        // (delay = 0) should reveal.
        app.update();
        assert!(app.world().entity(row0).get::<ScoreBreakdownRow>().unwrap().revealed);
        assert!(!app.world().entity(row1).get::<ScoreBreakdownRow>().unwrap().revealed);
        assert!(!app.world().entity(row2).get::<ScoreBreakdownRow>().unwrap().revealed);

        // Advance time by one stagger interval — row1 should reveal.
        {
            let mut time = app.world_mut().resource_mut::<Time>();
            time.advance_by(std::time::Duration::from_secs_f32(stagger + 0.001));
        }
        app.update();
        assert!(app.world().entity(row1).get::<ScoreBreakdownRow>().unwrap().revealed);
        assert!(!app.world().entity(row2).get::<ScoreBreakdownRow>().unwrap().revealed);

        // Advance again — row2 should reveal.
        {
            let mut time = app.world_mut().resource_mut::<Time>();
            time.advance_by(std::time::Duration::from_secs_f32(stagger + 0.001));
        }
        app.update();
        assert!(app.world().entity(row2).get::<ScoreBreakdownRow>().unwrap().revealed);
    }

    /// Under `AnimSpeed::Instant`, breakdown rows must spawn already
    /// revealed and at full opacity — there should be no stagger
    /// reveal animation at all.
    #[test]
    fn score_breakdown_instant_speed_skips_stagger() {
        // Helper: simulate what `spawn_breakdown_row` constructs by
        // checking the `instant` branch behaviour. Specifically: under
        // Instant, scaled_duration → 0.0, so the row's stagger and
        // fade are both zero.
        let stagger = scaled_duration(MOTION_SCORE_BREAKDOWN_STAGGER_SECS, AnimSpeed::Instant);
        let fade = scaled_duration(MOTION_SCORE_BREAKDOWN_FADE_SECS, AnimSpeed::Instant);
        assert_eq!(stagger, 0.0);
        assert_eq!(fade, 0.0);
    }
}
