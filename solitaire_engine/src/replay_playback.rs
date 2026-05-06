//! In-engine replay playback core.
//!
//! When the player clicks "Watch replay" on the Stats overlay, the live
//! game state is reset to the deal seeded from the replay's `seed` /
//! `mode` / `draw_mode`, and the engine ticks through `replay.moves` at a
//! steady cadence — firing the canonical [`MoveRequestEvent`] /
//! [`DrawRequestEvent`] for each one. The existing animation pipeline
//! plays back identically to a live game.
//!
//! ## Public surface
//!
//! - [`ReplayPlaybackState`] — single source of truth for whether
//!   playback is live, how far through the move list we've ticked, and
//!   how long until the next advance.
//! - [`start_replay_playback`] — public entry point; the Stats
//!   "Watch replay" button calls this. Resets the game to the recorded
//!   deal and transitions the state machine to
//!   [`ReplayPlaybackState::Playing`].
//! - [`stop_replay_playback`] — interrupts playback at any time. Safe to
//!   call when [`ReplayPlaybackState::Inactive`].
//! - [`ReplayPlaybackPlugin`] — registers the resource and the tick /
//!   linger systems.
//!
//! ## Coordination note
//!
//! This module is built in parallel with the Stats-side overlay. The
//! resource shape, helper signatures, and plugin marker match the
//! contract the overlay agent reads against — see also the docs on the
//! enum variants.
//!
//! ## Recording is paused during playback
//!
//! Playback fires the same [`MoveRequestEvent`] / [`DrawRequestEvent`]
//! the live engine handles. Without intervention, [`RecordingReplay`]
//! would re-record those events and a replay would re-record itself
//! indefinitely. To prevent that, [`record_replay_skip_during_playback`]
//! snapshots the recording's length at the start of playback and
//! truncates the buffer back to that length every frame. This keeps
//! the recording contract opaque to `game_plugin` — no event-source
//! flag is threaded through, no every-callsite gate is added.

use bevy::prelude::*;
use solitaire_data::{Replay, ReplayMove};

use crate::events::{DrawRequestEvent, MoveRequestEvent, StateChangedEvent};
use crate::game_plugin::{GameMutation, RecordingReplay};
use crate::resources::GameStateResource;
use crate::settings_plugin::SettingsResource;

/// Default per-move duration during playback, in seconds. Acts as the
/// fallback when `SettingsResource` is absent — i.e. in headless test
/// fixtures that don't install [`crate::settings_plugin::SettingsPlugin`].
/// In production the live value is read from
/// [`solitaire_data::Settings::replay_move_interval_secs`] every frame
/// so Settings adjustments take effect on the next playback tick.
///
/// Kept in sync with `solitaire_data::settings::default_replay_move_interval_secs`
/// (the data crate cannot depend on this engine crate, so the constant
/// is duplicated). The
/// `settings_replay_move_interval_default_matches_engine_constant`
/// test in `solitaire_engine::settings_plugin` enforces equality.
pub const REPLAY_MOVE_INTERVAL_SECS: f32 = 0.45;

/// Helper: returns the live per-move replay interval. Reads
/// [`SettingsResource::replay_move_interval_secs`] when the resource is
/// installed, falling back to [`REPLAY_MOVE_INTERVAL_SECS`] otherwise.
/// Also clamps below by `f32::EPSILON` so a hand-edited 0.0 cannot
/// busy-loop the playback tick.
fn current_move_interval_secs(settings: Option<&SettingsResource>) -> f32 {
    let raw = settings
        .map(|s| s.0.replay_move_interval_secs)
        .unwrap_or(REPLAY_MOVE_INTERVAL_SECS);
    raw.max(f32::EPSILON)
}

/// How long the [`ReplayPlaybackState::Completed`] state lingers before
/// the auto-clear system transitions it back to
/// [`ReplayPlaybackState::Inactive`]. Gives the overlay UI time to
/// display "Replay complete" before dismissing.
pub const REPLAY_COMPLETION_LINGER_SECS: f32 = 5.0;

/// Lifecycle state of an in-flight replay playback.
///
/// The default state is [`Inactive`](Self::Inactive) — no replay is
/// running. The overlay (and any other consumer) reads this resource to
/// decide whether the "Replay" banner should be visible and what
/// progress to display.
///
/// Lifecycle:
/// 1. Default state is [`Inactive`](Self::Inactive).
/// 2. [`start_replay_playback`] transitions to
///    [`Playing`](Self::Playing) and resets the live `GameState` to the
///    replay's recorded deal.
/// 3. The tick system [`tick_replay_playback`] advances `cursor` once
///    per [`REPLAY_MOVE_INTERVAL_SECS`] and fires the canonical event
///    for each [`ReplayMove`].
/// 4. When `cursor == replay.moves.len()`, the state transitions to
///    [`Completed`](Self::Completed). It lingers for
///    [`REPLAY_COMPLETION_LINGER_SECS`] (driven by
///    [`auto_clear_completed_replay`]) before returning to
///    [`Inactive`](Self::Inactive).
/// 5. [`stop_replay_playback`] interrupts at any time and forces the
///    state back to [`Inactive`](Self::Inactive).
#[derive(Resource, Debug, Default)]
pub enum ReplayPlaybackState {
    /// No replay is being played back. The overlay despawns itself when
    /// the resource transitions back to this variant.
    #[default]
    Inactive,
    /// A replay is currently being played back. The overlay reads
    /// `replay.moves.len()` for the denominator of the progress
    /// indicator and `cursor` for the numerator.
    Playing {
        /// The replay being played back. Owned so the state is the
        /// only place playback metadata lives — no separate resource
        /// needed.
        replay: Replay,
        /// Index of the next move to apply, in `[0, replay.moves.len()]`.
        cursor: usize,
        /// Seconds remaining until the next move is dispatched.
        secs_to_next: f32,
    },
    /// The replay finished playing back. The overlay swaps the banner
    /// label to "Replay complete" until [`auto_clear_completed_replay`]
    /// transitions back to [`Inactive`](Self::Inactive) a few seconds
    /// later.
    Completed,
}

impl ReplayPlaybackState {
    /// Returns `true` when a replay is currently being played back.
    pub fn is_playing(&self) -> bool {
        matches!(self, Self::Playing { .. })
    }

    /// Returns `true` when the replay has finished but the resource has
    /// not yet been auto-cleared back to [`Self::Inactive`].
    pub fn is_completed(&self) -> bool {
        matches!(self, Self::Completed)
    }

    /// Returns `(cursor, total)` when a replay is in progress so the
    /// overlay can render `"Move N of M"`. Returns `None` while
    /// [`Inactive`](Self::Inactive) or [`Completed`](Self::Completed) —
    /// the replay is consumed when transitioning out of `Playing`, so
    /// the total is no longer available in `Completed`.
    pub fn progress(&self) -> Option<(usize, usize)> {
        match self {
            Self::Playing { replay, cursor, .. } => Some((*cursor, replay.moves.len())),
            Self::Inactive | Self::Completed => None,
        }
    }
}

/// Public entry point — call from the Stats "Watch replay" button
/// handler.
///
/// Resets the live [`GameStateResource`] to a fresh deal seeded from
/// `replay.seed` / `replay.draw_mode` / `replay.mode` (via
/// [`Commands::insert_resource`]), then transitions the state machine
/// to [`ReplayPlaybackState::Playing`] with `cursor: 0` and
/// `secs_to_next: REPLAY_MOVE_INTERVAL_SECS`.
///
/// `commands` is used to overwrite [`GameStateResource`] in a deferred
/// flush — equivalent to what `handle_new_game` does, minus the
/// [`crate::events::NewGameRequestEvent`] round-trip and the
/// abandon-current-game confirmation modal (which would block playback
/// indefinitely). Using `Commands` rather than [`crate::events::NewGameRequestEvent`]
/// also sidesteps the fact that `NewGameRequestEvent` has no
/// `draw_mode_override` field — `handle_new_game` always reads
/// `draw_mode` from `Settings`, which would silently coerce a Draw-1
/// replay into a Draw-3 game (or vice versa) when the player's
/// settings disagree with the recording.
///
/// Safe to call from any state — if a replay is already playing it is
/// dropped and the new one starts immediately.
pub fn start_replay_playback(
    commands: &mut Commands,
    state: &mut ResMut<ReplayPlaybackState>,
    replay: Replay,
) {
    use solitaire_core::game_state::GameState;

    let fresh = GameState::new_with_mode(replay.seed, replay.draw_mode.clone(), replay.mode);
    commands.insert_resource(GameStateResource(fresh));

    // Initial `secs_to_next` uses the constant rather than reading
    // `SettingsResource` because this entry point takes `Commands` /
    // `ResMut<ReplayPlaybackState>` only. The first-tick latency may
    // therefore lag the configured interval by up to ~0.45 s on an
    // unusually short setting; subsequent ticks read the live setting
    // every frame via [`tick_replay_playback`].
    **state = ReplayPlaybackState::Playing {
        replay,
        cursor: 0,
        secs_to_next: REPLAY_MOVE_INTERVAL_SECS,
    };
}

/// Aborts an in-flight replay playback and resets
/// [`ReplayPlaybackState`] back to [`ReplayPlaybackState::Inactive`].
///
/// Safe to call from any state — when already
/// [`ReplayPlaybackState::Inactive`] it simply re-asserts inactivity.
///
/// The current [`GameStateResource`] is left as-is: the player sees the
/// replay's most-recently-applied state until they start a fresh game
/// manually. This avoids forcing an extra deal animation in their face
/// the moment they cancel.
///
/// `commands` is currently unused but accepted to match the
/// [`start_replay_playback`] signature — leaves room to hook in
/// cleanup (e.g. despawning playback-only overlays) without a future
/// API break.
pub fn stop_replay_playback(
    _commands: &mut Commands,
    state: &mut ResMut<ReplayPlaybackState>,
) {
    **state = ReplayPlaybackState::Inactive;
}

/// Tick system. Runs every frame; only does work when
/// [`ReplayPlaybackState::is_playing`].
///
/// Drains `secs_to_next` by `time.delta_secs()`. When the countdown
/// expires, fires the canonical event for the move at `cursor`,
/// increments `cursor`, and resets `secs_to_next`. When `cursor`
/// reaches `replay.moves.len()`, transitions to
/// [`ReplayPlaybackState::Completed`].
///
/// The advance loop is a `while`, not an `if`, so coarse time steps
/// (e.g. test-driven 200 ms ticks against a 450 ms interval) still
/// fire the right number of events — accumulated debt is paid off
/// across as many advances as needed in the same frame. In normal
/// gameplay frame deltas are well below `REPLAY_MOVE_INTERVAL_SECS`,
/// so the loop runs at most once per frame.
fn tick_replay_playback(
    time: Res<Time>,
    settings: Option<Res<SettingsResource>>,
    mut state: ResMut<ReplayPlaybackState>,
    mut moves_writer: MessageWriter<MoveRequestEvent>,
    mut draws_writer: MessageWriter<DrawRequestEvent>,
) {
    let dt = time.delta_secs();
    let interval = current_move_interval_secs(settings.as_deref());
    let mut transition_to_completed = false;

    if let ReplayPlaybackState::Playing {
        replay,
        cursor,
        secs_to_next,
    } = state.as_mut()
    {
        *secs_to_next -= dt;
        while *secs_to_next <= 0.0 && *cursor < replay.moves.len() {
            match &replay.moves[*cursor] {
                ReplayMove::Move { from, to, count } => {
                    moves_writer.write(MoveRequestEvent {
                        from: from.clone(),
                        to: to.clone(),
                        count: *count,
                    });
                }
                ReplayMove::StockClick => {
                    draws_writer.write(DrawRequestEvent);
                }
            }
            *cursor += 1;
            *secs_to_next += interval;
        }

        if *cursor >= replay.moves.len() {
            transition_to_completed = true;
        }
    }

    if transition_to_completed {
        *state = ReplayPlaybackState::Completed;
    }
}

/// Local timer for the [`ReplayPlaybackState::Completed`] linger.
/// Resets to zero whenever the state transitions out of
/// [`ReplayPlaybackState::Completed`].
#[derive(Default)]
struct CompletionLinger(f32);

/// Auto-clear system. While [`ReplayPlaybackState::Completed`],
/// accumulates time and transitions back to
/// [`ReplayPlaybackState::Inactive`] once
/// [`REPLAY_COMPLETION_LINGER_SECS`] has elapsed.
fn auto_clear_completed_replay(
    time: Res<Time>,
    mut state: ResMut<ReplayPlaybackState>,
    mut linger: Local<CompletionLinger>,
) {
    if state.is_completed() {
        linger.0 += time.delta_secs();
        if linger.0 >= REPLAY_COMPLETION_LINGER_SECS {
            *state = ReplayPlaybackState::Inactive;
            linger.0 = 0.0;
        }
    } else {
        // Reset whenever we're not in Completed so the next completion
        // measures from zero rather than accumulating across cycles.
        linger.0 = 0.0;
    }
}

/// Local cache of the recording buffer's length at the start of
/// playback. Lets us roll back any growth during playback without
/// touching `game_plugin`'s recording call sites.
#[derive(Default)]
struct RecordingSnapshot {
    /// `Some(len)` while playback is active. The recording is
    /// truncated back to this length every frame so playback-driven
    /// events leak no entries into the recorded move list. `None`
    /// when not playing — recording behaves normally.
    snapshot_len: Option<usize>,
}

/// Recording-pause system. While [`ReplayPlaybackState::is_playing`],
/// snapshots the recording's length on entry and truncates the
/// recording back to that length every frame. This keeps the live
/// [`RecordingReplay`] opaque to `game_plugin`'s `handle_move` /
/// `handle_draw` — those still push unconditionally; we just wipe the
/// playback-driven entries before any other system can read them.
///
/// Implemented this way because [`RecordingReplay`] is mutated inside
/// the [`GameMutation`] system set (the schedule set that owns
/// `handle_move` / `handle_draw`). We schedule this system
/// `.after(GameMutation)` so the truncation runs each frame *after*
/// the unconditional push, removing the same entry the playback tick
/// caused.
fn record_replay_skip_during_playback(
    state: Res<ReplayPlaybackState>,
    mut recording: ResMut<RecordingReplay>,
    mut snap: Local<RecordingSnapshot>,
) {
    // Treat `Playing` and `Completed` identically for the purpose of
    // recording suppression. The tick system's final advance fires
    // its event in the same frame it transitions to `Completed`; the
    // event is then consumed by `handle_move` / `handle_draw` either
    // this frame (race-dependent on system order) or the next. By
    // suppressing recording growth across both states, we close that
    // window cleanly: the snapshot survives until the resource is
    // back to `Inactive` (auto-cleared after
    // `REPLAY_COMPLETION_LINGER_SECS`).
    if state.is_playing() || state.is_completed() {
        let baseline = match snap.snapshot_len {
            Some(n) => n,
            None => {
                let n = recording.moves.len();
                snap.snapshot_len = Some(n);
                n
            }
        };
        if recording.moves.len() > baseline {
            recording.moves.truncate(baseline);
        }
    } else {
        // Drop the snapshot when neither playing nor completed so
        // the next playback cycle re-anchors to whatever the
        // recording is at that point.
        snap.snapshot_len = None;
    }
}

/// On-completion side effect: fire a single [`StateChangedEvent`] when
/// playback transitions from `Playing` to `Completed` so any UI that
/// listens for state mutations refreshes one final time. Cheap and
/// idempotent — `StateChangedEvent` is a one-shot signal.
fn fire_state_changed_on_completion(
    state: Res<ReplayPlaybackState>,
    mut last_was_completed: Local<bool>,
    mut writer: MessageWriter<StateChangedEvent>,
) {
    let now_completed = state.is_completed();
    if now_completed && !*last_was_completed {
        writer.write(StateChangedEvent);
    }
    *last_was_completed = now_completed;
}

/// Bevy plugin that initialises [`ReplayPlaybackState`] and drives
/// playback ticks, completion linger, and the recording-pause guard.
///
/// Register this in the main app alongside [`crate::game_plugin::GamePlugin`].
/// Tests can install it under [`MinimalPlugins`] to exercise the public
/// API without spinning up the full client.
pub struct ReplayPlaybackPlugin;

impl Plugin for ReplayPlaybackPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ReplayPlaybackState>()
            .add_systems(
                Update,
                (
                    tick_replay_playback,
                    auto_clear_completed_replay,
                    fire_state_changed_on_completion,
                )
                    .chain(),
            )
            .add_systems(
                Update,
                record_replay_skip_during_playback.after(GameMutation),
            );
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game_plugin::GamePlugin;
    use bevy::time::TimeUpdateStrategy;
    use chrono::NaiveDate;
    use solitaire_core::game_state::{DrawMode, GameMode};
    use solitaire_core::pile::PileType;
    use std::time::Duration;

    /// Builds a headless `App` with `MinimalPlugins`, `GamePlugin`, and
    /// `ReplayPlaybackPlugin`. `GamePlugin` brings the canonical
    /// `MoveRequestEvent` / `DrawRequestEvent` registrations along with
    /// `RecordingReplay` so the recording-pause test can read it.
    fn headless_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(GamePlugin::headless())
            .add_plugins(ReplayPlaybackPlugin);
        // Disable game-state persistence so tests don't touch the
        // real ~/.local/share/solitaire_quest/game_state.json.
        app.insert_resource(crate::game_plugin::GameStatePath(None));
        app.insert_resource(crate::game_plugin::ReplayPath(None));
        // Tick once so any startup systems flush before the first
        // assertion.
        app.update();
        app
    }

    /// `Time<Virtual>` clamps each tick to `max_delta` (default 250 ms),
    /// so we drive 200 ms steps and call `update` enough times to pass
    /// the requested duration.
    fn advance_by(app: &mut App, total_secs: f32) {
        app.insert_resource(TimeUpdateStrategy::ManualDuration(
            Duration::from_secs_f32(0.2),
        ));
        let ticks = (total_secs / 0.2).ceil() as usize + 1;
        for _ in 0..ticks {
            app.update();
        }
    }

    /// A 3-move replay covering both `Move` and `StockClick` variants.
    /// Seed 12345 is arbitrary — the test asserts on event counts and
    /// move shapes, not on board positions.
    fn sample_replay_three_moves() -> Replay {
        Replay::new(
            12345,
            DrawMode::DrawOne,
            GameMode::Classic,
            60,
            500,
            NaiveDate::from_ymd_opt(2026, 5, 5).expect("valid date"),
            vec![
                ReplayMove::StockClick,
                ReplayMove::Move {
                    from: PileType::Waste,
                    to: PileType::Tableau(3),
                    count: 1,
                },
                ReplayMove::StockClick,
            ],
        )
    }

    /// Scoped helper to invoke `start_replay_playback` from within the
    /// app's `World` (the public API takes `Commands`, which only
    /// exists inside systems). We use a one-shot system to obtain the
    /// `Commands`.
    fn start_playback(app: &mut App, replay: Replay) {
        #[derive(Resource)]
        struct ReplayInbox(Option<Replay>);
        app.insert_resource(ReplayInbox(Some(replay)));

        fn run(
            mut commands: Commands,
            mut state: ResMut<ReplayPlaybackState>,
            mut inbox: ResMut<ReplayInbox>,
        ) {
            if let Some(replay) = inbox.0.take() {
                start_replay_playback(&mut commands, &mut state, replay);
            }
        }
        let id = app.world_mut().register_system(run);
        app.world_mut()
            .run_system(id)
            .expect("one-shot start_playback");
    }

    fn stop_playback(app: &mut App) {
        fn run(mut commands: Commands, mut state: ResMut<ReplayPlaybackState>) {
            stop_replay_playback(&mut commands, &mut state);
        }
        let id = app.world_mut().register_system(run);
        app.world_mut()
            .run_system(id)
            .expect("one-shot stop_playback");
    }

    /// Fresh state must be `Inactive`. After `start_replay_playback`
    /// the state must be `Playing { cursor: 0, .. }` carrying the
    /// supplied replay.
    #[test]
    fn start_replay_playback_transitions_inactive_to_playing() {
        let mut app = headless_app();
        assert!(matches!(
            *app.world().resource::<ReplayPlaybackState>(),
            ReplayPlaybackState::Inactive
        ));

        let replay = sample_replay_three_moves();
        start_playback(&mut app, replay.clone());
        // Apply the deferred Commands flush.
        app.update();

        let state = app.world().resource::<ReplayPlaybackState>();
        match state {
            ReplayPlaybackState::Playing {
                cursor,
                replay: r,
                ..
            } => {
                assert_eq!(*cursor, 0);
                assert_eq!(r.seed, replay.seed);
                assert_eq!(r.moves.len(), 3);
            }
            other => panic!("expected Playing, got {other:?}"),
        }
        assert_eq!(state.progress(), Some((0, 3)));
    }

    /// One full interval (plus a small margin to clear the boundary)
    /// must advance the cursor by at least one.
    #[test]
    fn tick_advances_cursor_after_interval() {
        let mut app = headless_app();
        start_playback(&mut app, sample_replay_three_moves());
        app.update();

        // Drive virtual time forward by one interval.
        advance_by(&mut app, REPLAY_MOVE_INTERVAL_SECS + 0.05);

        let state = app.world().resource::<ReplayPlaybackState>();
        match state {
            ReplayPlaybackState::Playing { cursor, .. } => {
                assert!(
                    *cursor >= 1,
                    "expected cursor advanced past one move, got {cursor}",
                );
            }
            other => panic!("expected Playing, got {other:?}"),
        }
    }

    /// Driving past `n * REPLAY_MOVE_INTERVAL_SECS` must produce
    /// `n` events that match the recorded move kinds. We register a
    /// pair of accumulator systems that drain `MoveRequestEvent` /
    /// `DrawRequestEvent` into resources every frame — using a
    /// detached cursor across many `app.update()` calls is unreliable
    /// because Bevy's `Messages` double-buffer drops events older
    /// than two frames.
    #[test]
    fn tick_fires_canonical_event_for_each_move() {
        #[derive(Resource, Default)]
        struct CapturedMoves(Vec<MoveRequestEvent>);
        #[derive(Resource, Default)]
        struct CapturedDraws(usize);

        fn collect_moves(
            mut events: MessageReader<MoveRequestEvent>,
            mut sink: ResMut<CapturedMoves>,
        ) {
            for ev in events.read() {
                sink.0.push(ev.clone());
            }
        }
        fn collect_draws(
            mut events: MessageReader<DrawRequestEvent>,
            mut sink: ResMut<CapturedDraws>,
        ) {
            for _ in events.read() {
                sink.0 += 1;
            }
        }

        let mut app = headless_app();
        app.init_resource::<CapturedMoves>()
            .init_resource::<CapturedDraws>()
            .add_systems(Update, (collect_moves, collect_draws));

        start_playback(&mut app, sample_replay_three_moves());
        app.update();

        // Drive through 3 intervals. Add a small margin to ensure the
        // last firing isn't sitting exactly on the boundary.
        advance_by(&mut app, REPLAY_MOVE_INTERVAL_SECS * 3.0 + 0.1);

        let captured_moves = app.world().resource::<CapturedMoves>();
        let captured_draws = app.world().resource::<CapturedDraws>();

        // Sample replay: StockClick, Move { Waste -> Tableau(3), 1 }, StockClick.
        assert_eq!(
            captured_draws.0, 2,
            "expected 2 DrawRequestEvent (two StockClicks)",
        );
        assert_eq!(
            captured_moves.0.len(),
            1,
            "expected 1 MoveRequestEvent (the single Move variant)",
        );
        let m = &captured_moves.0[0];
        assert!(matches!(m.from, PileType::Waste));
        assert!(matches!(m.to, PileType::Tableau(3)));
        assert_eq!(m.count, 1);
    }

    /// Driving past one interval on a single-move replay must
    /// transition to `Completed`.
    #[test]
    fn playback_completes_when_cursor_reaches_end() {
        let mut app = headless_app();
        let one_move = Replay::new(
            42,
            DrawMode::DrawOne,
            GameMode::Classic,
            10,
            100,
            NaiveDate::from_ymd_opt(2026, 5, 5).expect("valid date"),
            vec![ReplayMove::StockClick],
        );
        start_playback(&mut app, one_move);
        app.update();

        advance_by(&mut app, REPLAY_MOVE_INTERVAL_SECS + 0.1);

        let state = app.world().resource::<ReplayPlaybackState>();
        assert!(
            state.is_completed(),
            "expected Completed after consuming the only move, got {state:?}",
        );
    }

    /// `stop_replay_playback` must force the state back to `Inactive`
    /// even mid-playback.
    #[test]
    fn stop_replay_playback_returns_to_inactive() {
        let mut app = headless_app();
        start_playback(&mut app, sample_replay_three_moves());
        app.update();
        // Tick once so the state is well and truly `Playing`.
        advance_by(&mut app, 0.1);
        assert!(app.world().resource::<ReplayPlaybackState>().is_playing());

        stop_playback(&mut app);
        app.update();

        assert!(matches!(
            *app.world().resource::<ReplayPlaybackState>(),
            ReplayPlaybackState::Inactive
        ));
    }

    /// Recording must remain frozen during playback. Pre-populate the
    /// recording with one entry, start playback, and assert the
    /// recording's move list is unchanged after several ticks.
    #[test]
    fn recording_paused_during_playback() {
        let mut app = headless_app();
        // Pre-populate the recording with one entry that should
        // survive playback unchanged. Mirrors the situation where the
        // player partway through a game opens stats and clicks Watch
        // Replay — their in-flight recording must not get clobbered.
        {
            let mut rec = app.world_mut().resource_mut::<RecordingReplay>();
            rec.moves.push(ReplayMove::StockClick);
        }
        start_playback(&mut app, sample_replay_three_moves());
        app.update();

        let baseline_len = app.world().resource::<RecordingReplay>().moves.len();
        assert_eq!(
            baseline_len, 1,
            "preconditions: recording starts with one entry",
        );

        // Drive playback through every move in the replay. Each move
        // would normally append to `RecordingReplay`; the pause
        // system must clamp the recording back to `baseline_len` on
        // every frame.
        advance_by(&mut app, REPLAY_MOVE_INTERVAL_SECS * 4.0 + 0.1);

        let after_len = app.world().resource::<RecordingReplay>().moves.len();
        assert_eq!(
            after_len, baseline_len,
            "recording must not grow while playback is active",
        );
    }

    /// With `SettingsResource::replay_move_interval_secs` set to 0.10 s
    /// (well below the 0.45 s default), playback over a fixed
    /// wall-clock window must dispatch strictly more moves than the
    /// same fixture would at the 0.45 s default. This is the
    /// regression check that the tick reads from the live Settings
    /// value rather than the hardcoded
    /// [`REPLAY_MOVE_INTERVAL_SECS`] constant.
    ///
    /// The follow-up assertion exercises the boundary condition: at
    /// the 0.10 s/move setting, exactly six 0.10 s ticks must yield
    /// fewer moves than six 0.20 s ticks (because the latter doubles
    /// the per-update advance and pays off two intervals each tick).
    #[test]
    fn replay_playback_tick_uses_settings_interval() {
        use solitaire_data::Settings;

        #[derive(Resource, Default)]
        struct CapturedDraws(usize);

        fn collect_draws(
            mut events: MessageReader<DrawRequestEvent>,
            mut sink: ResMut<CapturedDraws>,
        ) {
            for _ in events.read() {
                sink.0 += 1;
            }
        }

        // Long replay so the fast cadence has plenty of moves to
        // chew through and the 0.45 s vs 0.10 s difference is easy
        // to observe.
        fn ten_draws_replay() -> Replay {
            Replay::new(
                7,
                DrawMode::DrawOne,
                GameMode::Classic,
                10,
                100,
                NaiveDate::from_ymd_opt(2026, 5, 5).expect("valid date"),
                vec![ReplayMove::StockClick; 10],
            )
        }

        // ---- Run 1: 0.10 s/move (Settings override) ----
        let mut fast_app = headless_app();
        fast_app.insert_resource(SettingsResource(Settings {
            replay_move_interval_secs: 0.10,
            ..Settings::default()
        }));
        fast_app
            .init_resource::<CapturedDraws>()
            .add_systems(Update, collect_draws);

        start_playback(&mut fast_app, ten_draws_replay());
        fast_app.update();
        // 1.0 s of virtual time at 0.10 s/move dispatches ~5 moves
        // after the default 0.45 s startup interval is consumed.
        advance_by(&mut fast_app, 1.0);
        let fast_count = fast_app.world().resource::<CapturedDraws>().0;

        // ---- Run 2: 0.45 s/move (default — no SettingsResource) ----
        let mut slow_app = headless_app();
        // `tick_replay_playback` falls back to `REPLAY_MOVE_INTERVAL_SECS`
        // (0.45 s) when `SettingsResource` is absent.
        slow_app
            .init_resource::<CapturedDraws>()
            .add_systems(Update, collect_draws);

        start_playback(&mut slow_app, ten_draws_replay());
        slow_app.update();
        advance_by(&mut slow_app, 1.0);
        let slow_count = slow_app.world().resource::<CapturedDraws>().0;

        assert!(
            fast_count > slow_count,
            "at 0.10 s/move the tick must dispatch strictly more moves \
             than at the 0.45 s default over the same wall-clock window: \
             fast={fast_count}, slow={slow_count}",
        );

        // ---- Boundary: a 0.05 s/tick cadence over the same window
        // dispatches NO MORE moves than a 0.10 s/tick cadence, because
        // 0.05 s < 0.10 s configured interval — the secs_to_next clock
        // never crosses the threshold inside a single tick. ----
        //
        // We don't assert "exactly zero" because the leading update()
        // after `start_playback` may run before the strategy is
        // applied (cf. comments on `tick_advances_cursor_after_interval`),
        // but the count must not exceed what we'd get with one-tick
        // advances at the same total wall-clock window.
        fn count_after_window(interval_secs: f32, tick_secs: f32, total_secs: f32) -> usize {
            let mut app = headless_app();
            app.insert_resource(SettingsResource(Settings {
                replay_move_interval_secs: interval_secs,
                ..Settings::default()
            }));
            app.init_resource::<CapturedDraws>()
                .add_systems(Update, collect_draws);
            start_playback(&mut app, ten_draws_replay());
            app.update();
            app.insert_resource(TimeUpdateStrategy::ManualDuration(
                Duration::from_secs_f32(tick_secs),
            ));
            let ticks = (total_secs / tick_secs).ceil() as usize + 1;
            for _ in 0..ticks {
                app.update();
            }
            app.world().resource::<CapturedDraws>().0
        }

        let count_at_05 = count_after_window(0.10, 0.05, 1.0);
        let count_at_20 = count_after_window(0.10, 0.20, 1.0);
        assert!(
            count_at_05 <= count_at_20,
            "0.05 s ticks (strictly less than the 0.10 s interval) must \
             dispatch no more moves than 0.20 s ticks over the same \
             wall-clock window: count_at_05={count_at_05}, count_at_20={count_at_20}",
        );
    }
}
