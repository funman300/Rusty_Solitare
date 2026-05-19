//! Matomo analytics plugin — buffers game-play events and flushes them to
//! the configured Matomo instance in the background.
//!
//! Disabled by default (opt-in via Settings → Privacy). Only active when
//! `settings.analytics_enabled` is `true` AND `settings.matomo_url` is set.

use std::sync::Arc;

use bevy::prelude::*;
use bevy::tasks::AsyncComputeTaskPool;
use solitaire_core::game_state::GameMode;
use solitaire_data::{matomo_client::MatomoClient, settings::SyncBackend, Settings};

use crate::events::{AchievementUnlockedEvent, ForfeitEvent, GameWonEvent, NewGameRequestEvent};
use crate::resources::{GameStateResource, TokioRuntimeResource};
use crate::settings_plugin::{SettingsChangedEvent, SettingsResource};

// ---------------------------------------------------------------------------
// Resource
// ---------------------------------------------------------------------------

/// Holds the active Matomo client. `None` when the feature is disabled.
#[derive(Resource)]
pub struct AnalyticsResource {
    pub client: Option<Arc<MatomoClient>>,
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
                    on_new_game,
                    on_achievement_unlocked,
                ),
            );

        // Build the shared Tokio runtime; skip network flush systems if the OS
        // refuses to create threads (resource-limited / sandboxed environments).
        match TokioRuntimeResource::new() {
            Ok(rt) => {
                app.insert_resource(rt).add_systems(
                    Update,
                    (on_game_won, on_forfeit, tick_flush_timer),
                );
            }
            Err(e) => {
                bevy::log::warn!("analytics_plugin: Tokio runtime unavailable — analytics flush disabled: {e}");
            }
        }
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
    rt: Res<TokioRuntimeResource>,
) {
    let Some(client) = analytics.client.clone() else {
        return;
    };
    for ev in wins.read() {
        client.event("Game", "Won", None, Some(ev.score as f64));
        fire_flush(client.clone(), rt.0.clone());
    }
}

fn on_forfeit(
    mut forfeits: MessageReader<ForfeitEvent>,
    analytics: Res<AnalyticsResource>,
    rt: Res<TokioRuntimeResource>,
) {
    let Some(client) = analytics.client.clone() else {
        return;
    };
    for _ev in forfeits.read() {
        client.event("Game", "Forfeit", None, None);
        fire_flush(client.clone(), rt.0.clone());
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
        if !ev.confirmed {
            continue;
        }
        let mode = ev.mode.unwrap_or(game.0.mode);
        client.event("Game", "Start", Some(mode_str(mode)), None);
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
        client.event("Achievement", "Unlocked", Some(&ev.0.id), None);
    }
}

fn tick_flush_timer(
    time: Res<Time>,
    mut analytics: ResMut<AnalyticsResource>,
    rt: Res<TokioRuntimeResource>,
) {
    analytics.flush_timer.tick(time.delta());
    if !analytics.flush_timer.just_finished() {
        return;
    }
    if let Some(client) = analytics.client.clone() {
        fire_flush(client, rt.0.clone());
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn client_for(settings: &Settings) -> Option<Arc<MatomoClient>> {
    if !settings.analytics_enabled {
        return None;
    }
    let url = settings.matomo_url.as_deref()?;
    let uid = match &settings.sync_backend {
        SyncBackend::SolitaireServer { username, .. } => Some(username.clone()),
        SyncBackend::Local => None,
    };
    Some(Arc::new(MatomoClient::new(url, settings.matomo_site_id, uid)))
}

fn fire_flush(client: Arc<MatomoClient>, rt: Arc<tokio::runtime::Runtime>) {
    AsyncComputeTaskPool::get()
        .spawn(async move {
            rt.block_on(client.flush());
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
