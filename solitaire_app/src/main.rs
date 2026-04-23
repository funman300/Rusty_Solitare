use bevy::prelude::*;
use solitaire_engine::{CardPlugin, GamePlugin, InputPlugin, TablePlugin};

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
        .run();
}
