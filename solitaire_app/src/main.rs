use bevy::prelude::*;
use solitaire_engine::{
    AchievementPlugin, AnimationPlugin, CardPlugin, GamePlugin, InputPlugin, ProgressPlugin,
    StatsPlugin, TablePlugin,
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
        .add_plugins(AchievementPlugin::default())
        .add_plugins(ProgressPlugin::default())
        .run();
}
