//! Tracks the per-date daily challenge: a deterministic seed every player
//! sees on a given calendar day, plus completion bookkeeping.
//!
//! When the player wins a game whose seed matches today's daily seed and
//! today's date hasn't been completed yet, this plugin:
//!   - calls `PlayerProgress::record_daily_completion`
//!   - awards a fixed XP bonus (`DAILY_BONUS_XP`)
//!   - persists progress
//!   - emits `DailyChallengeCompletedEvent`
//!
//! Pressing **C** fires a `NewGameRequestEvent` with today's daily seed so
//! the player can start a fresh attempt.

use bevy::input::ButtonInput;
use bevy::prelude::*;
use bevy::tasks::{futures_lite::future, AsyncComputeTaskPool, Task};
use chrono::{Local, NaiveDate};
use solitaire_data::{daily_seed_for, save_progress_to};
use solitaire_sync::ChallengeGoal;

use crate::events::{GameWonEvent, InfoToastEvent, NewGameRequestEvent, XpAwardedEvent};
use crate::game_plugin::GameMutation;
use crate::progress_plugin::{ProgressResource, ProgressStoragePath, ProgressUpdate};
use crate::resources::GameStateResource;
use crate::sync_plugin::SyncProviderResource;

/// Bonus XP awarded for completing today's daily challenge.
pub const DAILY_BONUS_XP: u64 = 100;

/// The active daily challenge — date + RNG seed for that date's deal,
/// plus optional goal metadata fetched from the server.
#[derive(Resource, Debug, Clone)]
pub struct DailyChallengeResource {
    pub date: NaiveDate,
    pub seed: u64,
    /// Human-readable goal description from the server, e.g. "Win in under 5 minutes".
    pub goal_description: Option<String>,
    /// Optional target score the server requires for this challenge.
    pub target_score: Option<i32>,
    /// Optional time limit in seconds the server imposes.
    pub max_time_secs: Option<u64>,
}

/// Fired when the player presses C to start the daily challenge.
/// Carries the current goal description so it can be displayed as a toast.
#[derive(Message, Debug, Clone)]
pub struct DailyGoalAnnouncementEvent(pub String);

impl DailyChallengeResource {
    pub fn for_today() -> Self {
        let date = Local::now().date_naive();
        Self {
            date,
            seed: daily_seed_for(date),
            goal_description: None,
            target_score: None,
            max_time_secs: None,
        }
    }
}

/// Fired when the player has just completed today's daily challenge.
#[derive(Message, Debug, Clone, Copy)]
pub struct DailyChallengeCompletedEvent {
    pub date: NaiveDate,
    pub streak: u32,
}

/// Holds the in-flight server challenge fetch so the result can be polled
/// each frame without blocking the main thread.
#[derive(Resource, Default)]
struct DailyChallengeTask(Option<Task<Option<ChallengeGoal>>>);

/// Fetches today's daily challenge seed and goal from the sync server on startup and tracks completion.
/// Fires `DailyChallengeCompletedEvent` when the player wins a matching game.
pub struct DailyChallengePlugin;

impl Plugin for DailyChallengePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(DailyChallengeResource::for_today())
            .init_resource::<DailyChallengeTask>()
            .add_message::<DailyChallengeCompletedEvent>()
            .add_message::<DailyGoalAnnouncementEvent>()
            .add_message::<GameWonEvent>()
            .add_message::<NewGameRequestEvent>()
            .add_message::<XpAwardedEvent>()
            .add_systems(Startup, fetch_server_challenge)
            .add_systems(Update, poll_server_challenge)
            // record/award after the base ProgressUpdate so we don't fight
            // ProgressPlugin's add_xp on the same frame.
            .add_systems(Update, handle_daily_completion.after(ProgressUpdate))
            .add_systems(Update, handle_start_daily_request.before(GameMutation));
    }
}

/// Startup system: spawns an async task to fetch the server's daily challenge.
///
/// Only runs when `SyncProviderResource` is present (i.e. `SyncPlugin` is
/// installed). The endpoint is public so authentication is not required.
fn fetch_server_challenge(
    provider: Option<Res<SyncProviderResource>>,
    mut task_res: ResMut<DailyChallengeTask>,
) {
    let Some(provider) = provider else { return };
    let provider = provider.0.clone();
    let task = AsyncComputeTaskPool::get()
        .spawn(async move { provider.fetch_daily_challenge().await.ok().flatten() });
    task_res.0 = Some(task);
}

/// Update system: polls the server-challenge fetch task.
///
/// On success, replaces the locally-computed seed in `DailyChallengeResource`
/// with the server's authoritative seed — ensuring all players worldwide get
/// the same deal on a given date regardless of their local clock hash.
///
/// Silently no-ops if the task is still in flight, already consumed, or
/// if the server returned a challenge for a different date.
fn poll_server_challenge(
    mut task_res: ResMut<DailyChallengeTask>,
    mut daily: ResMut<DailyChallengeResource>,
) {
    let Some(task) = task_res.0.as_mut() else {
        return;
    };
    let Some(result) = future::block_on(future::poll_once(task)) else {
        return;
    };
    task_res.0 = None;
    let Some(goal) = result else { return };
    let Ok(date) = NaiveDate::parse_from_str(&goal.date, "%Y-%m-%d") else {
        return;
    };
    if date == daily.date {
        let old_seed = daily.seed;
        daily.seed = goal.seed;
        daily.goal_description = Some(goal.description.clone());
        daily.target_score = goal.target_score;
        daily.max_time_secs = goal.max_time_secs;
        info!(
            "daily challenge seed updated from server: {old_seed} → {} ({})",
            goal.seed,
            goal.description
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_daily_completion(
    mut wins: MessageReader<GameWonEvent>,
    daily: Res<DailyChallengeResource>,
    game: Res<GameStateResource>,
    mut progress: ResMut<ProgressResource>,
    path: Res<ProgressStoragePath>,
    mut completed: MessageWriter<DailyChallengeCompletedEvent>,
    mut xp_awarded: MessageWriter<XpAwardedEvent>,
    mut toast: MessageWriter<InfoToastEvent>,
) {
    for ev in wins.read() {
        if game.0.seed != daily.seed {
            continue;
        }
        // Enforce server-supplied goal constraints when present.
        if let Some(target) = daily.target_score
            && ev.score < target {
                continue; // score goal not met
            }
        if let Some(max_secs) = daily.max_time_secs
            && ev.time_seconds > max_secs {
                continue; // time limit exceeded
            }
        if !progress.0.record_daily_completion(daily.date) {
            // Already counted today — no-op.
            continue;
        }
        progress.0.add_xp(DAILY_BONUS_XP);
        xp_awarded.write(XpAwardedEvent { amount: DAILY_BONUS_XP });
        if let Some(target) = &path.0
            && let Err(e) = save_progress_to(target, &progress.0) {
                warn!("failed to save progress after daily completion: {e}");
            }
        completed.write(DailyChallengeCompletedEvent {
            date: daily.date,
            streak: progress.0.daily_challenge_streak,
        });
        toast.write(InfoToastEvent("Daily challenge complete! +100 XP".to_string()));
    }
}

fn handle_start_daily_request(
    keys: Res<ButtonInput<KeyCode>>,
    daily: Res<DailyChallengeResource>,
    mut new_game: MessageWriter<NewGameRequestEvent>,
    mut announce: MessageWriter<DailyGoalAnnouncementEvent>,
) {
    if keys.just_pressed(KeyCode::KeyC) {
        new_game.write(NewGameRequestEvent {
            seed: Some(daily.seed),
            mode: None,
        });
        let desc = daily
            .goal_description
            .clone()
            .unwrap_or_else(|| "Daily Challenge".to_string());
        announce.write(DailyGoalAnnouncementEvent(desc));
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
            .add_plugins(DailyChallengePlugin);
        app.init_resource::<ButtonInput<KeyCode>>();
        app.update();
        app
    }

    #[test]
    fn resource_uses_today() {
        let app = headless_app();
        let r = app.world().resource::<DailyChallengeResource>();
        assert_eq!(r.date, Local::now().date_naive());
        assert_eq!(r.seed, daily_seed_for(r.date));
    }

    #[test]
    fn winning_with_daily_seed_completes_and_fires_event() {
        let mut app = headless_app();
        let daily_seed = app.world().resource::<DailyChallengeResource>().seed;

        // Replace the GameState with one whose seed matches the daily seed.
        app.world_mut().resource_mut::<GameStateResource>().0 =
            GameState::new(daily_seed, DrawMode::DrawOne);

        app.world_mut().write_message(GameWonEvent {
            score: 500,
            time_seconds: 200,
        });
        app.update();

        let progress = &app.world().resource::<ProgressResource>().0;
        assert_eq!(progress.daily_challenge_streak, 1);
        // +100 from the daily bonus
        assert!(progress.total_xp >= DAILY_BONUS_XP);

        let events = app.world().resource::<Messages<DailyChallengeCompletedEvent>>();
        let mut cursor = events.get_cursor();
        let fired: Vec<_> = cursor.read(events).copied().collect();
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].streak, 1);
    }

    #[test]
    fn winning_with_unrelated_seed_does_not_complete_daily() {
        let mut app = headless_app();
        let daily_seed = app.world().resource::<DailyChallengeResource>().seed;
        // Use a deliberately different seed.
        app.world_mut().resource_mut::<GameStateResource>().0 =
            GameState::new(daily_seed.wrapping_add(7777), DrawMode::DrawOne);

        app.world_mut().write_message(GameWonEvent {
            score: 500,
            time_seconds: 200,
        });
        app.update();

        let progress = &app.world().resource::<ProgressResource>().0;
        assert_eq!(progress.daily_challenge_streak, 0);

        let events = app.world().resource::<Messages<DailyChallengeCompletedEvent>>();
        let mut cursor = events.get_cursor();
        assert!(cursor.read(events).next().is_none());
    }

    #[test]
    fn second_win_same_day_is_idempotent() {
        let mut app = headless_app();
        let daily_seed = app.world().resource::<DailyChallengeResource>().seed;
        app.world_mut().resource_mut::<GameStateResource>().0 =
            GameState::new(daily_seed, DrawMode::DrawOne);

        app.world_mut().write_message(GameWonEvent {
            score: 500,
            time_seconds: 200,
        });
        app.update();
        // Re-send win.
        app.world_mut().write_message(GameWonEvent {
            score: 500,
            time_seconds: 200,
        });
        app.update();

        let progress = &app.world().resource::<ProgressResource>().0;
        assert_eq!(progress.daily_challenge_streak, 1, "streak does not double-count");
    }

    #[test]
    fn pressing_c_fires_new_game_with_daily_seed() {
        let mut app = headless_app();
        let daily_seed = app.world().resource::<DailyChallengeResource>().seed;

        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyC);
        app.update();

        let events = app.world().resource::<Messages<NewGameRequestEvent>>();
        let mut cursor = events.get_cursor();
        let fired: Vec<_> = cursor.read(events).copied().collect();
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].seed, Some(daily_seed));
    }

    #[test]
    fn pressing_c_fires_announcement_event_with_description() {
        let mut app = headless_app();
        // Inject a goal description.
        app.world_mut()
            .resource_mut::<DailyChallengeResource>()
            .goal_description = Some("Win in under 5 minutes".to_string());

        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyC);
        app.update();

        let events = app.world().resource::<Messages<DailyGoalAnnouncementEvent>>();
        let mut cursor = events.get_cursor();
        let fired: Vec<_> = cursor.read(events).cloned().collect();
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].0, "Win in under 5 minutes");
    }

    #[test]
    fn pressing_c_with_no_description_uses_fallback() {
        let mut app = headless_app();
        // Ensure no description is set.
        assert!(app.world().resource::<DailyChallengeResource>().goal_description.is_none());

        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyC);
        app.update();

        let events = app.world().resource::<Messages<DailyGoalAnnouncementEvent>>();
        let mut cursor = events.get_cursor();
        let fired: Vec<_> = cursor.read(events).cloned().collect();
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].0, "Daily Challenge");
    }

    #[test]
    fn goal_fields_stored_from_server_fetch() {
        let mut app = headless_app();
        // Simulate what poll_server_challenge does when the server responds.
        {
            let mut daily = app.world_mut().resource_mut::<DailyChallengeResource>();
            daily.goal_description = Some("Win without undo".to_string());
            daily.target_score = Some(1_000);
            daily.max_time_secs = Some(300);
        }
        let r = app.world().resource::<DailyChallengeResource>();
        assert_eq!(r.goal_description.as_deref(), Some("Win without undo"));
        assert_eq!(r.target_score, Some(1_000));
        assert_eq!(r.max_time_secs, Some(300));
    }
}
