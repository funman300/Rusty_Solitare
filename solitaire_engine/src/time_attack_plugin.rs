//! Time Attack mode runtime: 10-minute countdown wrapped around back-to-back
//! `GameMode::TimeAttack` games. Pressing **T** starts a session (gated by
//! level ≥ `CHALLENGE_UNLOCK_LEVEL`); each win during the session bumps the
//! counter and auto-deals a fresh game. When the timer expires the session
//! ends and `TimeAttackEndedEvent` fires.
//!
//! ## Persistence
//!
//! Classic / Zen / Challenge mid-deals already round-trip through
//! `game_state.json` (the file carries `mode: GameMode`, so the deal *and*
//! its mode flag both survive a window close). Time Attack additionally
//! has session-level state — the 10-minute window remaining and the running
//! win counter — that lives in [`TimeAttackResource`], not in `GameState`.
//! That extra state is persisted to the sibling file
//! `time_attack_session.json` via [`solitaire_data::TimeAttackSession`] so
//! closing the window mid-Time-Attack does not lose the session.
//!
//! The file is written periodically (every ~30 real seconds, mirroring the
//! game-state auto-save cadence) and on `AppExit`. It is deleted on session
//! end, on a fresh session start, and on quit-to-menu. Load happens once at
//! plugin startup; if the persisted window expired during the time the app
//! was closed, the file is treated as missing.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use bevy::prelude::*;
use solitaire_core::game_state::GameMode;
use solitaire_data::{
    delete_time_attack_session_at, load_time_attack_session_from, save_time_attack_session_to,
    time_attack_session_path, TimeAttackSession,
};

use crate::challenge_plugin::CHALLENGE_UNLOCK_LEVEL;
use crate::events::{
    GameWonEvent, InfoToastEvent, NewGameRequestEvent, StartTimeAttackRequestEvent,
};
use crate::game_plugin::GameMutation;
use crate::progress_plugin::ProgressResource;
use crate::resources::GameStateResource;

/// Length of a Time Attack session in real-world seconds (10 minutes).
pub const TIME_ATTACK_DURATION_SECS: f32 = 600.0;

/// Session state for an in-progress Time Attack run. Not persisted.
#[derive(Resource, Debug, Clone, Default)]
pub struct TimeAttackResource {
    pub active: bool,
    pub remaining_secs: f32,
    pub wins: u32,
}

/// Fired when the Time Attack timer expires. The summary toast in
/// `AnimationPlugin` consumes this; UI/stats consumers can also subscribe.
#[derive(Message, Debug, Clone, Copy)]
pub struct TimeAttackEndedEvent {
    pub wins: u32,
}

/// Real-world seconds between Time Attack session-state auto-saves.
///
/// Mirrors the game-state auto-save cadence in `game_plugin::AUTO_SAVE_INTERVAL_SECS`
/// so a crash loses at most ~30 s of session-timer progress.
const TIME_ATTACK_AUTO_SAVE_INTERVAL_SECS: f32 = 30.0;

/// Persistence path for `time_attack_session.json`. `None` disables I/O
/// (used in headless tests so they don't touch the real data dir).
#[derive(Resource, Debug, Clone)]
pub struct TimeAttackSessionPath(pub Option<PathBuf>);

/// Accumulated real-world seconds since the last Time Attack session save.
/// Exposed as a `Resource` so tests can pre-seed it past the threshold without
/// needing to control `Time::delta_secs()` (mirrors `game_plugin::AutoSaveTimer`).
#[derive(Resource, Default)]
pub struct TimeAttackAutoSaveTimer(pub f32);

/// Implements the 10-minute Time Attack mode: counts down the session timer, tracks wins per session, and fires `TimeAttackEndedEvent` when time expires.
pub struct TimeAttackPlugin;

impl TimeAttackPlugin {
    /// Plugin variant with persistence disabled. Use in headless tests to
    /// avoid touching the real `time_attack_session.json` on disk.
    pub fn headless() -> Self {
        Self
    }
}

impl Plugin for TimeAttackPlugin {
    fn build(&self, app: &mut App) {
        let path = time_attack_session_path();
        // Restore any saved session that hasn't yet expired in real time.
        // A missing file or an expired window both yield `None`, in which
        // case the resource keeps its default (inactive) value.
        let initial_session = path
            .as_deref()
            .and_then(load_time_attack_session_from)
            .map_or_else(TimeAttackResource::default, |s| TimeAttackResource {
                active: true,
                remaining_secs: s.remaining_secs,
                wins: s.wins,
            });

        app.insert_resource(initial_session)
            .insert_resource(TimeAttackSessionPath(path))
            .init_resource::<TimeAttackAutoSaveTimer>()
            .add_message::<TimeAttackEndedEvent>()
            .add_message::<GameWonEvent>()
            .add_message::<NewGameRequestEvent>()
            .add_message::<StartTimeAttackRequestEvent>()
            .add_message::<InfoToastEvent>()
            .add_systems(
                Update,
                handle_start_time_attack_request.before(GameMutation),
            )
            .add_systems(Update, advance_time_attack)
            .add_systems(Update, auto_deal_on_time_attack_win.after(GameMutation))
            .add_systems(Update, auto_save_time_attack_session)
            .add_systems(Last, save_time_attack_session_on_exit);
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_start_time_attack_request(
    keys: Res<ButtonInput<KeyCode>>,
    mut requests: MessageReader<StartTimeAttackRequestEvent>,
    progress: Res<ProgressResource>,
    mut session: ResMut<TimeAttackResource>,
    mut new_game: MessageWriter<NewGameRequestEvent>,
    mut info_toast: MessageWriter<InfoToastEvent>,
    path: Option<Res<TimeAttackSessionPath>>,
    mut auto_save_timer: ResMut<TimeAttackAutoSaveTimer>,
) {
    // Either T or the HUD Modes-popover "Time Attack" row triggers this.
    let button_clicked = requests.read().count() > 0;
    if !keys.just_pressed(KeyCode::KeyT) && !button_clicked {
        return;
    }
    if progress.0.level < CHALLENGE_UNLOCK_LEVEL {
        info_toast.write(InfoToastEvent(format!(
            "Time Attack unlocks at level {CHALLENGE_UNLOCK_LEVEL}"
        )));
        return;
    }
    *session = TimeAttackResource {
        active: true,
        remaining_secs: TIME_ATTACK_DURATION_SECS,
        wins: 0,
    };
    // Reset the auto-save accumulator so the first save lands a full
    // interval from now, not immediately because of an old residual value
    // left over from a previous session.
    auto_save_timer.0 = 0.0;
    // Delete any leftover persisted session file from a prior run so the
    // fresh window starts at exactly TIME_ATTACK_DURATION_SECS rather than
    // resuming whatever the disk happened to hold. Failures here are
    // logged but never fatal.
    if let Some(p) = path.as_ref().and_then(|r| r.0.as_deref())
        && let Err(e) = delete_time_attack_session_at(p) {
            warn!("time_attack_session: failed to delete stale session: {e}");
        }
    new_game.write(NewGameRequestEvent {
        seed: None,
        mode: Some(GameMode::TimeAttack),
        confirmed: false,
    });
}

fn advance_time_attack(
    time: Res<Time>,
    mut session: ResMut<TimeAttackResource>,
    mut ended: MessageWriter<TimeAttackEndedEvent>,
    paused: Option<Res<crate::pause_plugin::PausedResource>>,
    path: Option<Res<TimeAttackSessionPath>>,
) {
    if !session.active {
        return;
    }
    if paused.is_some_and(|p| p.0) {
        return;
    }
    session.remaining_secs -= time.delta_secs();
    if session.remaining_secs <= 0.0 {
        let wins = session.wins;
        session.active = false;
        session.remaining_secs = 0.0;
        ended.write(TimeAttackEndedEvent { wins });
        // Session ended naturally — delete the persisted file so the next
        // launch sees no in-progress session.
        if let Some(p) = path.as_ref().and_then(|r| r.0.as_deref())
            && let Err(e) = delete_time_attack_session_at(p) {
                warn!("time_attack_session: failed to delete on expiry: {e}");
            }
    }
}

fn auto_deal_on_time_attack_win(
    mut wins: MessageReader<GameWonEvent>,
    game: Res<GameStateResource>,
    mut session: ResMut<TimeAttackResource>,
    mut new_game: MessageWriter<NewGameRequestEvent>,
) {
    for _ in wins.read() {
        if !session.active || game.0.mode != GameMode::TimeAttack {
            continue;
        }
        session.wins = session.wins.saturating_add(1);
        new_game.write(NewGameRequestEvent {
            seed: None,
            mode: Some(GameMode::TimeAttack),
            confirmed: false,
        });
    }
}

/// Returns the current Unix-seconds wall-clock time, falling back to 0 if
/// the system time predates the epoch (impossible under any sane clock,
/// but the fallback keeps the function infallible).
fn current_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Periodically persists the live `TimeAttackResource` to
/// `time_attack_session.json` every 30 real-world seconds while a session
/// is active. The accumulator uses real-clock delta so it keeps ticking
/// even if the in-game timer is paused — the goal is "if the OS kills the
/// process now, how much do we lose?" and pause does not change that.
fn auto_save_time_attack_session(
    time: Res<Time>,
    session: Res<TimeAttackResource>,
    path: Option<Res<TimeAttackSessionPath>>,
    mut timer: ResMut<TimeAttackAutoSaveTimer>,
) {
    if !session.active {
        return;
    }
    timer.0 += time.delta_secs();
    if timer.0 < TIME_ATTACK_AUTO_SAVE_INTERVAL_SECS {
        return;
    }
    timer.0 -= TIME_ATTACK_AUTO_SAVE_INTERVAL_SECS;
    let Some(p) = path.as_ref().and_then(|r| r.0.as_deref()) else {
        return;
    };
    let payload = TimeAttackSession {
        remaining_secs: session.remaining_secs,
        wins: session.wins,
        saved_at_unix_secs: current_unix_secs(),
    };
    if let Err(e) = save_time_attack_session_to(p, &payload) {
        warn!("time_attack_session: auto-save failed: {e}");
    }
}

/// Last-schedule companion to `game_plugin::save_game_state_on_exit`:
/// flushes the live session resource to disk on `AppExit` so a graceful
/// quit does not lose the timer + win count. If the session is inactive
/// the persisted file is deleted instead, leaving a clean slate for the
/// next launch.
fn save_time_attack_session_on_exit(
    mut exit_events: MessageReader<AppExit>,
    session: Res<TimeAttackResource>,
    path: Res<TimeAttackSessionPath>,
) {
    if exit_events.is_empty() {
        return;
    }
    exit_events.clear();
    let Some(p) = path.0.as_deref() else { return };

    if !session.active {
        if let Err(e) = delete_time_attack_session_at(p) {
            warn!("time_attack_session: failed to delete on exit: {e}");
        }
        return;
    }

    let payload = TimeAttackSession {
        remaining_secs: session.remaining_secs,
        wins: session.wins,
        saved_at_unix_secs: current_unix_secs(),
    };
    if let Err(e) = save_time_attack_session_to(p, &payload) {
        warn!("time_attack_session: failed to save on exit: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game_plugin::GamePlugin;
    use crate::progress_plugin::ProgressPlugin;
    use crate::table_plugin::TablePlugin;
    use solitaire_core::game_state::{DrawMode, GameState};

    fn headless_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(GamePlugin)
            .add_plugins(TablePlugin)
            .add_plugins(ProgressPlugin::headless())
            .add_plugins(TimeAttackPlugin);
        app.init_resource::<ButtonInput<KeyCode>>();
        // Disable session persistence — tests must not touch the real
        // ~/.local/share/solitaire_quest/time_attack_session.json.
        app.insert_resource(TimeAttackSessionPath(None));
        // The plugin's startup-load hook may have populated TimeAttackResource
        // from a real on-disk session. Reset it so each test starts inactive.
        *app.world_mut().resource_mut::<TimeAttackResource>() = TimeAttackResource::default();
        app.update();
        app
    }

    fn press_t(app: &mut App) {
        let mut input = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        input.release(KeyCode::KeyT);
        input.clear();
        input.press(KeyCode::KeyT);
    }

    #[test]
    fn pressing_t_below_unlock_level_is_ignored() {
        let mut app = headless_app();
        press_t(&mut app);
        app.update();

        let session = app.world().resource::<TimeAttackResource>();
        assert!(!session.active);

        let events = app.world().resource::<Messages<NewGameRequestEvent>>();
        let mut cursor = events.get_cursor();
        assert!(cursor.read(events).next().is_none());
    }

    #[test]
    fn pressing_t_at_unlock_level_starts_session_and_deals_time_attack_game() {
        let mut app = headless_app();
        app.world_mut().resource_mut::<ProgressResource>().0.level = CHALLENGE_UNLOCK_LEVEL;

        press_t(&mut app);
        app.update();

        let session = app.world().resource::<TimeAttackResource>().clone();
        assert!(session.active);
        assert_eq!(session.wins, 0);
        assert!((session.remaining_secs - TIME_ATTACK_DURATION_SECS).abs() < 1.0);

        let events = app.world().resource::<Messages<NewGameRequestEvent>>();
        let mut cursor = events.get_cursor();
        let fired: Vec<_> = cursor.read(events).copied().collect();
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].mode, Some(GameMode::TimeAttack));
    }

    #[test]
    fn timer_expiry_fires_ended_event_and_clears_active() {
        let mut app = headless_app();
        // Set the session to an already-expired state (remaining < 0).
        // MinimalPlugins time delta is nonzero so we skip the intermediate
        // 0.001-remaining step to avoid a double-fire.
        *app.world_mut().resource_mut::<TimeAttackResource>() = TimeAttackResource {
            active: true,
            remaining_secs: -1.0,
            wins: 5,
        };
        app.update();

        let session = app.world().resource::<TimeAttackResource>();
        assert!(!session.active);
        assert_eq!(session.remaining_secs, 0.0);

        let events = app.world().resource::<Messages<TimeAttackEndedEvent>>();
        let mut cursor = events.get_cursor();
        let fired: Vec<_> = cursor.read(events).copied().collect();
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].wins, 5);
    }

    #[test]
    fn win_during_session_increments_wins_and_auto_deals() {
        let mut app = headless_app();
        // Start a session manually.
        *app.world_mut().resource_mut::<TimeAttackResource>() = TimeAttackResource {
            active: true,
            remaining_secs: 100.0,
            wins: 0,
        };
        // The current game must be in TimeAttack mode for auto-deal to fire.
        app.world_mut().resource_mut::<GameStateResource>().0 =
            GameState::new_with_mode(7, DrawMode::DrawOne, GameMode::TimeAttack);

        app.world_mut().write_message(GameWonEvent {
            score: 500,
            time_seconds: 60,
        });
        app.update();

        let session = app.world().resource::<TimeAttackResource>();
        assert_eq!(session.wins, 1);

        let events = app.world().resource::<Messages<NewGameRequestEvent>>();
        let mut cursor = events.get_cursor();
        let fired: Vec<_> = cursor.read(events).copied().collect();
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].mode, Some(GameMode::TimeAttack));
        assert!(fired[0].seed.is_none());
    }

    #[test]
    fn win_when_session_inactive_does_not_increment() {
        let mut app = headless_app();
        // Default session is inactive. Game is TimeAttack mode — still no count.
        app.world_mut().resource_mut::<GameStateResource>().0 =
            GameState::new_with_mode(7, DrawMode::DrawOne, GameMode::TimeAttack);

        app.world_mut().write_message(GameWonEvent {
            score: 500,
            time_seconds: 60,
        });
        app.update();

        let session = app.world().resource::<TimeAttackResource>();
        assert_eq!(session.wins, 0);
    }

    #[test]
    fn classic_win_during_session_does_not_increment() {
        let mut app = headless_app();
        *app.world_mut().resource_mut::<TimeAttackResource>() = TimeAttackResource {
            active: true,
            remaining_secs: 100.0,
            wins: 0,
        };
        // GameStateResource defaults to Classic mode.
        app.world_mut().write_message(GameWonEvent {
            score: 500,
            time_seconds: 60,
        });
        app.update();

        let session = app.world().resource::<TimeAttackResource>();
        assert_eq!(session.wins, 0);
    }

    #[test]
    fn paused_session_does_not_fire_ended_event() {
        // Insert PausedResource(true) so the advance system exits early.
        // Even with remaining_secs at -1 (which would normally trigger expiry),
        // the timer must not fire while the game is paused.
        let mut app = headless_app();
        app.insert_resource(crate::pause_plugin::PausedResource(true));
        *app.world_mut().resource_mut::<TimeAttackResource>() = TimeAttackResource {
            active: true,
            remaining_secs: -1.0, // would normally expire
            wins: 3,
        };
        app.update();

        // remaining_secs must not have been reset to 0.0 (pause blocked the update).
        let session = app.world().resource::<TimeAttackResource>();
        assert!(session.active, "session must still be active while paused");
        assert!(session.remaining_secs < 0.0, "remaining_secs must not change while paused");

        // No ended event must have been emitted.
        let events = app.world().resource::<Messages<TimeAttackEndedEvent>>();
        let mut cursor = events.get_cursor();
        assert!(
            cursor.read(events).next().is_none(),
            "TimeAttackEndedEvent must not fire while paused"
        );
    }

    // -----------------------------------------------------------------------
    // Persistence tests — closing the window mid-Time-Attack must not lose
    // the session timer or the running win count.
    // -----------------------------------------------------------------------

    fn tmp_ta_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("engine_test_ta_{name}.json"))
    }

    /// On `AppExit`, an active session must be flushed to disk so the next
    /// launch can restore it.
    #[test]
    fn exit_persists_active_session() {
        use solitaire_data::load_time_attack_session_from;

        let path = tmp_ta_path("exit_save");
        let _ = std::fs::remove_file(&path);

        let mut app = headless_app();
        app.insert_resource(TimeAttackSessionPath(Some(path.clone())));
        *app.world_mut().resource_mut::<TimeAttackResource>() = TimeAttackResource {
            active: true,
            remaining_secs: 240.0,
            wins: 4,
        };

        app.world_mut().write_message(AppExit::Success);
        app.update();

        // Plugin stamps `saved_at_unix_secs` with the current wall clock,
        // and we load immediately, so wall-clock elapsed is ~0 and the
        // restored remaining_secs should match what we wrote within a tiny
        // epsilon (allowing for the test taking a few seconds to run).
        let loaded =
            load_time_attack_session_from(&path).expect("file should exist after exit");
        assert!(
            (loaded.remaining_secs - 240.0).abs() < 5.0,
            "remaining_secs must round-trip within 5 s tolerance, got {}",
            loaded.remaining_secs,
        );
        assert_eq!(loaded.wins, 4, "wins must round-trip");

        let _ = std::fs::remove_file(&path);
    }

    /// On `AppExit` with no active session, any stale persisted file must
    /// be deleted so the next launch starts clean.
    #[test]
    fn exit_clears_persisted_file_when_no_active_session() {
        let path = tmp_ta_path("exit_clear");
        // Pre-create a stale file.
        std::fs::write(&path, b"{\"remaining_secs\":100.0,\"wins\":1,\"saved_at_unix_secs\":0}")
            .expect("write stale");
        assert!(path.exists());

        let mut app = headless_app();
        app.insert_resource(TimeAttackSessionPath(Some(path.clone())));
        // Default = inactive session.
        app.world_mut().write_message(AppExit::Success);
        app.update();

        assert!(!path.exists(), "stale file must be deleted on exit when session is inactive");
    }

    /// `auto_save_time_attack_session` writes the session once the
    /// accumulator crosses 30 s while the session is active.
    #[test]
    fn auto_save_writes_after_30_seconds() {
        use solitaire_data::load_time_attack_session_from;

        let path = tmp_ta_path("auto_save_30s");
        let _ = std::fs::remove_file(&path);

        let mut app = headless_app();
        app.insert_resource(TimeAttackSessionPath(Some(path.clone())));
        *app.world_mut().resource_mut::<TimeAttackResource>() = TimeAttackResource {
            active: true,
            remaining_secs: 500.0,
            wins: 2,
        };
        // Pre-seed the timer past the threshold so the very next update fires the save.
        app.insert_resource(TimeAttackAutoSaveTimer(TIME_ATTACK_AUTO_SAVE_INTERVAL_SECS + 0.1));
        app.update();

        assert!(path.exists(), "auto-save file must exist after timer crosses threshold");
        let loaded = load_time_attack_session_from(&path).expect("session must load");
        assert_eq!(loaded.wins, 2);

        let _ = std::fs::remove_file(&path);
    }

    /// Auto-save is a no-op when no session is active — we should not be
    /// littering the user's data dir with empty session files just because
    /// the app was running.
    #[test]
    fn auto_save_is_noop_when_session_inactive() {
        let path = tmp_ta_path("auto_save_noop");
        let _ = std::fs::remove_file(&path);

        let mut app = headless_app();
        app.insert_resource(TimeAttackSessionPath(Some(path.clone())));
        // Session stays at default (inactive). Timer is past threshold.
        app.insert_resource(TimeAttackAutoSaveTimer(TIME_ATTACK_AUTO_SAVE_INTERVAL_SECS + 0.1));
        app.update();

        assert!(!path.exists(), "auto-save must not fire when session is inactive");
    }

    /// Starting a fresh session must delete any stale persisted file so a
    /// player who quit Time Attack mid-window, came back, then started a
    /// brand-new session begins at exactly TIME_ATTACK_DURATION_SECS.
    #[test]
    fn starting_new_session_deletes_stale_persisted_file() {
        let path = tmp_ta_path("start_clears");
        // Pre-create a stale file.
        std::fs::write(&path, b"{\"remaining_secs\":42.0,\"wins\":99,\"saved_at_unix_secs\":0}")
            .expect("write stale");

        let mut app = headless_app();
        app.insert_resource(TimeAttackSessionPath(Some(path.clone())));
        // Player must be at unlock level for the start-handler to act.
        app.world_mut().resource_mut::<ProgressResource>().0.level = CHALLENGE_UNLOCK_LEVEL;

        press_t(&mut app);
        app.update();

        assert!(!path.exists(), "stale persisted file must be cleared at session start");

        // And the live resource must reflect a fresh session, not the stale data.
        let session = app.world().resource::<TimeAttackResource>();
        assert!(session.active);
        assert_eq!(session.wins, 0, "wins must reset to 0, not the stale 99");
        assert!(
            (session.remaining_secs - TIME_ATTACK_DURATION_SECS).abs() < 1.0,
            "remaining_secs must reset to TIME_ATTACK_DURATION_SECS, not the stale 42; got {}",
            session.remaining_secs,
        );
    }

    /// Natural session expiry (timer reaches 0) must delete the persisted
    /// file so the next launch does not see an "active" session that has
    /// already ended.
    #[test]
    fn session_expiry_deletes_persisted_file() {
        let path = tmp_ta_path("expiry_clears");
        // Pre-create a file that simulates the auto-save's prior write.
        std::fs::write(&path, b"{\"remaining_secs\":1.0,\"wins\":7,\"saved_at_unix_secs\":0}")
            .expect("write");
        assert!(path.exists());

        let mut app = headless_app();
        app.insert_resource(TimeAttackSessionPath(Some(path.clone())));
        // Session about to expire on the next update tick.
        *app.world_mut().resource_mut::<TimeAttackResource>() = TimeAttackResource {
            active: true,
            remaining_secs: -1.0,
            wins: 7,
        };

        app.update();

        assert!(!path.exists(), "persisted file must be deleted on natural expiry");
        let session = app.world().resource::<TimeAttackResource>();
        assert!(!session.active);
    }
}
