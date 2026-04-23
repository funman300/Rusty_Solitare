//! Keyboard + mouse input for the game board.
//!
//! - `U` → `UndoRequestEvent`
//! - `N` → `NewGameRequestEvent { seed: None }`
//! - `D` → `DrawRequestEvent`
//! - `Esc` → logged as a pause placeholder (no event yet; wired up when the
//!   pause screen lands in a later phase)
//! - Left-click on the stock pile → `DrawRequestEvent`
//!
//! Drag-and-drop for tableau/waste/foundation moves is handled in Phase 3E.

use bevy::input::ButtonInput;
use bevy::math::Vec2;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use solitaire_core::pile::PileType;

use crate::events::{DrawRequestEvent, NewGameRequestEvent, UndoRequestEvent};
use crate::layout::LayoutResource;

/// Registers the keyboard + mouse input systems.
pub struct InputPlugin;

impl Plugin for InputPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (handle_keyboard, handle_mouse_clicks));
    }
}

fn handle_keyboard(
    keys: Res<ButtonInput<KeyCode>>,
    mut undo: EventWriter<UndoRequestEvent>,
    mut new_game: EventWriter<NewGameRequestEvent>,
    mut draw: EventWriter<DrawRequestEvent>,
) {
    if keys.just_pressed(KeyCode::KeyU) {
        undo.send(UndoRequestEvent);
    }
    if keys.just_pressed(KeyCode::KeyN) {
        new_game.send(NewGameRequestEvent { seed: None });
    }
    if keys.just_pressed(KeyCode::KeyD) {
        draw.send(DrawRequestEvent);
    }
    if keys.just_pressed(KeyCode::Escape) {
        // Pause placeholder — the pause screen hooks this up in a later phase.
        info!("pause requested (not yet wired)");
    }
}

fn handle_mouse_clicks(
    buttons: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Camera, &GlobalTransform)>,
    layout: Option<Res<LayoutResource>>,
    mut draw: EventWriter<DrawRequestEvent>,
) {
    if !buttons.just_pressed(MouseButton::Left) {
        return;
    }
    let Some(layout) = layout else {
        return;
    };
    let Ok(window) = windows.get_single() else {
        return;
    };
    let Some(cursor) = window.cursor_position() else {
        return;
    };
    let Ok((camera, camera_transform)) = cameras.get_single() else {
        return;
    };
    let Ok(world) = camera.viewport_to_world_2d(camera_transform, cursor) else {
        return;
    };

    let Some(&stock_pos) = layout.0.pile_positions.get(&PileType::Stock) else {
        return;
    };
    if point_in_rect(world, stock_pos, layout.0.card_size) {
        draw.send(DrawRequestEvent);
    }
}

/// Axis-aligned rectangle hit-test with a center and full size.
fn point_in_rect(point: Vec2, center: Vec2, size: Vec2) -> bool {
    let half = size / 2.0;
    point.x >= center.x - half.x
        && point.x <= center.x + half.x
        && point.y >= center.y - half.y
        && point.y <= center.y + half.y
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn point_in_rect_inside_returns_true() {
        let center = Vec2::new(10.0, 20.0);
        let size = Vec2::new(40.0, 60.0);
        assert!(point_in_rect(Vec2::new(10.0, 20.0), center, size));
        assert!(point_in_rect(Vec2::new(29.0, 49.0), center, size));
        assert!(point_in_rect(Vec2::new(-9.0, -9.0), center, size));
    }

    #[test]
    fn point_in_rect_on_edge_returns_true() {
        let center = Vec2::ZERO;
        let size = Vec2::new(10.0, 10.0);
        assert!(point_in_rect(Vec2::new(5.0, 5.0), center, size));
        assert!(point_in_rect(Vec2::new(-5.0, -5.0), center, size));
    }

    #[test]
    fn point_in_rect_outside_returns_false() {
        let center = Vec2::ZERO;
        let size = Vec2::new(10.0, 10.0);
        assert!(!point_in_rect(Vec2::new(6.0, 0.0), center, size));
        assert!(!point_in_rect(Vec2::new(0.0, 6.0), center, size));
        assert!(!point_in_rect(Vec2::new(-100.0, 0.0), center, size));
    }
}
