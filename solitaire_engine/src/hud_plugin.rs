//! Persistent in-game HUD: score, move count, elapsed time, and mode badge.
//!
//! The HUD spawns once at startup and lives for the app's lifetime. Text is
//! refreshed whenever `GameStateResource` changes (which happens on every move
//! and every elapsed-time tick), so score, moves, and timer all stay current
//! without a separate tick system.

use bevy::prelude::*;
use solitaire_core::game_state::{DrawMode, GameMode};

use crate::game_plugin::GameMutation;
use crate::resources::GameStateResource;
use crate::time_attack_plugin::TimeAttackResource;

/// Marker on the score text node.
#[derive(Component, Debug)]
struct HudScore;

/// Marker on the move-count text node.
#[derive(Component, Debug)]
struct HudMoves;

/// Marker on the elapsed-time text node.
#[derive(Component, Debug)]
struct HudTime;

/// Marker on the mode badge text node.
#[derive(Component, Debug)]
struct HudMode;

/// HUD Z-layer — above cards (which start at z=0) but below overlay screens.
const Z_HUD: i32 = 50;

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_hud)
            .add_systems(Update, update_hud.after(GameMutation));
    }
}

fn spawn_hud(mut commands: Commands) {
    let white = TextColor(Color::srgba(1.0, 1.0, 1.0, 0.80));
    let font = TextFont { font_size: 18.0, ..default() };
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(12.0),
                top: Val::Px(8.0),
                flex_direction: FlexDirection::Row,
                column_gap: Val::Px(20.0),
                align_items: AlignItems::Center,
                ..default()
            },
            ZIndex(Z_HUD),
        ))
        .with_children(|b| {
            b.spawn((HudScore, Text::new("Score: 0"), font.clone(), white));
            b.spawn((HudMoves, Text::new("Moves: 0"), font.clone(), white));
            b.spawn((HudTime, Text::new("0:00"), font, white));
            b.spawn((
                HudMode,
                Text::new(""),
                TextFont { font_size: 17.0, ..default() },
                TextColor(Color::srgb(1.0, 0.85, 0.25)),
            ));
        });
}

#[allow(clippy::type_complexity)]
fn update_hud(
    game: Res<GameStateResource>,
    time_attack: Option<Res<TimeAttackResource>>,
    mut score_q: Query<&mut Text, (With<HudScore>, Without<HudMoves>, Without<HudTime>, Without<HudMode>)>,
    mut moves_q: Query<&mut Text, (With<HudMoves>, Without<HudScore>, Without<HudTime>, Without<HudMode>)>,
    mut time_q: Query<&mut Text, (With<HudTime>, Without<HudScore>, Without<HudMoves>, Without<HudMode>)>,
    mut mode_q: Query<&mut Text, (With<HudMode>, Without<HudScore>, Without<HudMoves>, Without<HudTime>)>,
) {
    let ta_active = time_attack.as_ref().is_some_and(|ta| ta.active);

    // Score, moves, and mode only need updating when the game state changes.
    if game.is_changed() {
        let g = &game.0;
        let is_zen = g.mode == GameMode::Zen;
        if let Ok(mut t) = score_q.get_single_mut() {
            // Zen mode suppresses score display per spec ("No score display").
            **t = if is_zen {
                String::new()
            } else {
                format!("Score: {}", g.score)
            };
        }
        if let Ok(mut t) = moves_q.get_single_mut() {
            **t = format!("Moves: {}", g.move_count);
        }
        if let Ok(mut t) = mode_q.get_single_mut() {
            **t = match g.mode {
                GameMode::Classic => match g.draw_mode {
                    DrawMode::DrawOne => String::new(),
                    DrawMode::DrawThree => "Draw 3".to_string(),
                },
                GameMode::Zen => "ZEN".to_string(),
                GameMode::Challenge => "CHALLENGE".to_string(),
                GameMode::TimeAttack => "TIME ATTACK".to_string(),
            };
        }
    }

    // Time display: show Time Attack countdown every frame when active;
    // Zen mode suppresses the timer per spec ("No timer").
    // Otherwise show game elapsed time (updates once per second via game.is_changed()).
    let is_zen = game.0.mode == GameMode::Zen;
    let update_time = (ta_active || game.is_changed()) && !is_zen;
    if update_time {
        if let Ok(mut t) = time_q.get_single_mut() {
            if let Some(ta) = time_attack.as_ref().filter(|ta| ta.active) {
                let remaining = ta.remaining_secs.max(0.0) as u64;
                let m = remaining / 60;
                let s = remaining % 60;
                **t = format!("{m}:{s:02}");
            } else {
                let secs = game.0.elapsed_seconds;
                let m = secs / 60;
                let s = secs % 60;
                **t = format!("{m}:{s:02}");
            }
        }
    } else if is_zen && game.is_changed() {
        // Clear the time display when entering Zen mode.
        if let Ok(mut t) = time_q.get_single_mut() {
            **t = String::new();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game_plugin::GamePlugin;
    use crate::table_plugin::TablePlugin;
    use solitaire_core::game_state::{DrawMode, GameState};

    fn headless_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(GamePlugin)
            .add_plugins(TablePlugin)
            .add_plugins(HudPlugin);
        app.update();
        app
    }

    #[test]
    fn hud_plugin_registers_without_panic() {
        let _app = headless_app();
    }

    #[test]
    fn update_hud_runs_after_game_mutation_without_panic() {
        let mut app = headless_app();
        app.world_mut().resource_mut::<GameStateResource>().0 =
            GameState::new(42, DrawMode::DrawOne);
        app.update();
    }

    fn read_hud_text<M: Component>(app: &mut App) -> String {
        app.world_mut()
            .query_filtered::<&Text, With<M>>()
            .iter(app.world())
            .next()
            .map(|t| t.0.clone())
            .unwrap_or_default()
    }

    #[test]
    fn score_reflects_game_state() {
        let mut app = headless_app();
        app.world_mut().resource_mut::<GameStateResource>().0.score = 750;
        app.update();
        assert_eq!(read_hud_text::<HudScore>(&mut app), "Score: 750");
    }

    #[test]
    fn moves_reflects_game_state() {
        let mut app = headless_app();
        app.world_mut().resource_mut::<GameStateResource>().0.move_count = 42;
        app.update();
        assert_eq!(read_hud_text::<HudMoves>(&mut app), "Moves: 42");
    }

    #[test]
    fn draw_three_mode_shows_draw_3_badge() {
        use solitaire_core::game_state::GameMode;
        let mut app = headless_app();
        app.world_mut().resource_mut::<GameStateResource>().0 =
            GameState::new_with_mode(42, DrawMode::DrawThree, GameMode::Classic);
        app.update();
        assert_eq!(read_hud_text::<HudMode>(&mut app), "Draw 3");
    }

    #[test]
    fn zen_mode_hides_score() {
        use solitaire_core::game_state::GameMode;
        let mut app = headless_app();
        app.world_mut().resource_mut::<GameStateResource>().0 =
            GameState::new_with_mode(42, DrawMode::DrawOne, GameMode::Zen);
        app.world_mut().resource_mut::<GameStateResource>().0.score = 999;
        app.update();
        // Zen mode spec: "No score display" → text must be empty.
        assert_eq!(read_hud_text::<HudScore>(&mut app), "");
    }

    #[test]
    fn time_display_uses_mm_ss_format() {
        let mut app = headless_app();
        app.world_mut().resource_mut::<GameStateResource>().0.elapsed_seconds = 125;
        app.update();
        // 125 seconds = 2 minutes 5 seconds → "2:05"
        assert_eq!(read_hud_text::<HudTime>(&mut app), "2:05");
    }
}
