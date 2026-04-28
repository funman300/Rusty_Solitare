use bevy::prelude::*;
use solitaire_data::{load_settings_from, provider_for_backend, settings_file_path, Settings};
use solitaire_engine::{
    AchievementPlugin, AnimationPlugin, AudioPlugin, AutoCompletePlugin, CardPlugin,
    ChallengePlugin, CursorPlugin, DailyChallengePlugin, FeedbackAnimPlugin, GamePlugin,
    HelpPlugin, HomePlugin, HudPlugin, InputPlugin, LeaderboardPlugin, OnboardingPlugin,
    PausePlugin, ProfilePlugin, ProgressPlugin, SettingsPlugin, StatsPlugin, SyncPlugin,
    TablePlugin, TimeAttackPlugin, WeeklyGoalsPlugin,
};

fn main() {
    // Load settings before building the app so we can construct the right
    // sync provider. Falls back to defaults if no settings file exists yet.
    let settings: Settings = settings_file_path()
        .map(|p| load_settings_from(&p))
        .unwrap_or_default();
    let sync_provider = provider_for_backend(&settings.sync_backend);

    App::new()
        .add_plugins(
            DefaultPlugins.set(WindowPlugin {
                primary_window: Some(Window {
                    title: "Solitaire Quest".into(),
                    resolution: (1280u32, 800u32).into(),
                    ..default()
                }),
                ..default()
            }),
        )
        .add_plugins(GamePlugin)
        .add_plugins(TablePlugin)
        .add_plugins(CardPlugin)
        .add_plugins(CursorPlugin)
        .add_plugins(InputPlugin)
        .add_plugins(AnimationPlugin)
        .add_plugins(FeedbackAnimPlugin)
        .add_plugins(AutoCompletePlugin)
        .add_plugins(StatsPlugin::default())
        .add_plugins(ProgressPlugin::default())
        .add_plugins(AchievementPlugin::default())
        .add_plugins(DailyChallengePlugin)
        .add_plugins(WeeklyGoalsPlugin)
        .add_plugins(ChallengePlugin)
        .add_plugins(TimeAttackPlugin)
        .add_plugins(HudPlugin)
        .add_plugins(HelpPlugin)
        .add_plugins(HomePlugin)
        .add_plugins(ProfilePlugin)
        .add_plugins(PausePlugin)
        .add_plugins(SettingsPlugin::default())
        .add_plugins(AudioPlugin)
        .add_plugins(OnboardingPlugin)
        .add_plugins(SyncPlugin::new(sync_provider))
        .add_plugins(LeaderboardPlugin)
        .run();
}
