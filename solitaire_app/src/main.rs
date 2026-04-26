use bevy::prelude::*;
use solitaire_engine::{
    AchievementPlugin, AnimationPlugin, AudioPlugin, CardPlugin, ChallengePlugin,
    DailyChallengePlugin, GamePlugin, HelpPlugin, InputPlugin, PausePlugin, ProgressPlugin,
    StatsPlugin, TablePlugin, TimeAttackPlugin, WeeklyGoalsPlugin,
};

fn main() {
    App::new()
        .add_plugins(
            DefaultPlugins.set(WindowPlugin {
                primary_window: Some(Window {
                    title: "Solitaire Quest".into(),
                    resolution: (1280.0, 800.0).into(),
                    ..default()
                }),
                ..default()
            }),
        )
        .add_plugins(GamePlugin)
        .add_plugins(TablePlugin)
        .add_plugins(CardPlugin)
        .add_plugins(InputPlugin)
        .add_plugins(AnimationPlugin)
        .add_plugins(StatsPlugin::default())
        .add_plugins(ProgressPlugin::default())
        .add_plugins(AchievementPlugin::default())
        .add_plugins(DailyChallengePlugin)
        .add_plugins(WeeklyGoalsPlugin)
        .add_plugins(ChallengePlugin)
        .add_plugins(TimeAttackPlugin)
        .add_plugins(HelpPlugin)
        .add_plugins(PausePlugin)
        .add_plugins(AudioPlugin)
        .run();
}
