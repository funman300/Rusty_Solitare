//! Analytics plugin — buffers game-play events and flushes them to the
//! configured server in the background.
//!
//! Disabled by default (opt-in via Settings → Privacy). Only active when
//! `settings.analytics_enabled` is `true` AND `sync_backend` is a
//! `SolitaireServer` with a URL to send to.

use std::sync::Arc;

use bevy::prelude::*;
use bevy::tasks::AsyncComputeTaskPool;
use solitaire_core::game_state::GameMode;
use solitaire_data::{analytics_client::AnalyticsClient, settings::SyncBackend, Settings};

use crate::events::{AchievementUnlockedEvent, ForfeitEvent, GameWonEvent, NewGameRequestEvent};
use crate::resources::GameStateResource;
use crate::settings_plugin::{SettingsChangedEvent, SettingsResource};

// ---------------------------------------------------------------------------
// Resource
// ---------------------------------------------------------------------------

/// Holds the active analytics client. `None` when the feature is disabled.
#[derive(Resource)]
pub struct AnalyticsResource {
    pub client: Option<Arc<AnalyticsClient>>,
    flush_timer: Timer,
}

impl Default for AnalyticsResource {
    fn default() -> Self {
        Self {
            client: None,
            flush_timer: Timer::from_seconds(60.0, TimerMode::Repeating),
        }
    }
}

// ---------------------------------------------------------------------------
// Plugin
// ---------------------------------------------------------------------------

/// Registers analytics systems. Add after `SettingsPlugin` in the app.
pub struct AnalyticsPlugin;

impl Plugin for AnalyticsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AnalyticsResource>()
            .add_systems(Startup, init_analytics)
            .add_systems(
                Update,
                (
                    react_to_settings_change,
                    on_game_won,
                    on_forfeit,
                    on_new_game,
                    on_achievement_unlocked,
                    tick_flush_timer,
                ),
            );
    }
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

fn init_analytics(settings: Res<SettingsResource>, mut analytics: ResMut<AnalyticsResource>) {
    analytics.client = client_for(&settings.0);
}

fn react_to_settings_change(
    mut events: MessageReader<SettingsChangedEvent>,
    mut analytics: ResMut<AnalyticsResource>,
) {
    for ev in events.read() {
        analytics.client = client_for(&ev.0);
    }
}

fn on_game_won(
    mut wins: MessageReader<GameWonEvent>,
    analytics: Res<AnalyticsResource>,
    settings: Res<SettingsResource>,
) {
    let Some(client) = analytics.client.clone() else {
        return;
    };
    for ev in wins.read() {
        client.record(
            "game_won",
            serde_json::json!({
                "score": ev.score,
                "time_seconds": ev.time_seconds,
            }),
        );
        fire_flush(client.clone(), &settings.0);
    }
}

fn on_forfeit(
    mut forfeits: MessageReader<ForfeitEvent>,
    analytics: Res<AnalyticsResource>,
    settings: Res<SettingsResource>,
) {
    let Some(client) = analytics.client.clone() else {
        return;
    };
    for _ev in forfeits.read() {
        client.record("game_forfeit", serde_json::json!({}));
        fire_flush(client.clone(), &settings.0);
    }
}

fn on_new_game(
    mut requests: MessageReader<NewGameRequestEvent>,
    analytics: Res<AnalyticsResource>,
    game: Res<GameStateResource>,
) {
    let Some(client) = analytics.client.clone() else {
        return;
    };
    for ev in requests.read() {
        // Only record confirmed starts — skip the first unconfirmed request
        // that spawns the "abandon game?" modal.
        if !ev.confirmed {
            continue;
        }
        // mode = None means "reuse current game mode". Reading from
        // GameStateResource at this point gives the still-active game's mode,
        // which is exactly what the new game will inherit.
        let mode = ev.mode.unwrap_or(game.0.mode);
        client.record(
            "game_start",
            serde_json::json!({ "mode": mode_str(mode) }),
        );
    }
}

fn on_achievement_unlocked(
    mut achievements: MessageReader<AchievementUnlockedEvent>,
    analytics: Res<AnalyticsResource>,
) {
    let Some(client) = analytics.client.clone() else {
        return;
    };
    for ev in achievements.read() {
        client.record(
            "achievement_unlocked",
            serde_json::json!({ "achievement_id": ev.0.id }),
        );
    }
}

fn tick_flush_timer(
    time: Res<Time>,
    mut analytics: ResMut<AnalyticsResource>,
    settings: Res<SettingsResource>,
) {
    analytics.flush_timer.tick(time.delta());
    if !analytics.flush_timer.just_finished() {
        return;
    }
    if let Some(client) = analytics.client.clone() {
        fire_flush(client, &settings.0);
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn client_for(settings: &Settings) -> Option<Arc<AnalyticsClient>> {
    if !settings.analytics_enabled {
        return None;
    }
    match &settings.sync_backend {
        SyncBackend::SolitaireServer { url, .. } => {
            Some(Arc::new(AnalyticsClient::new(url.clone())))
        }
        SyncBackend::Local => None,
    }
}

fn fire_flush(client: Arc<AnalyticsClient>, settings: &Settings) {
    let user_id = match &settings.sync_backend {
        SyncBackend::SolitaireServer { username, .. } => Some(username.clone()),
        SyncBackend::Local => None,
    };
    AsyncComputeTaskPool::get()
        .spawn(async move {
            if let Ok(rt) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                rt.block_on(client.flush(user_id));
            }
        })
        .detach();
}

fn mode_str(mode: GameMode) -> &'static str {
    match mode {
        GameMode::Classic => "classic",
        GameMode::Zen => "zen",
        GameMode::Challenge => "challenge",
        GameMode::TimeAttack => "time_attack",
        GameMode::Difficulty(_) => "difficulty",
    }
}
