//! Card interaction visuals: hover scale, drag lift, and input buffering.
//!
//! # Hover
//!
//! [`HoverState`] tracks the entity currently under the cursor. A system
//! smoothly lerps `Transform.scale` toward `HOVER_SCALE` on the hovered card
//! and back to 1.0 when the cursor leaves. Scale is only written when no
//! [`CardAnimation`] is active on the entity (the animation takes priority).
//!
//! # Drag visual
//!
//! While [`DragState`] is non-idle, the dragged card entities receive a subtle
//! scale boost (`DRAG_LIFT_SCALE`) and their z-order is pushed up. The exact
//! translation is still controlled by the existing [`crate::input_plugin`] —
//! this system only applies the _visual_ enhancement without touching XY.
//!
//! # Input buffer
//!
//! [`InputBuffer`] stores move/draw/undo actions that arrived while cards are
//! still animating. Call [`InputBuffer::push`] from any system that wants
//! buffering. The drain system fires the oldest buffered action as soon as all
//! [`CardAnimation`] components have cleared, giving a responsive feel on
//! fast repeated clicks.
//!
//! # Visual priority
//!
//! Dragged cards always have the highest z. The existing [`crate::input_plugin`]
//! sets drag z; this module applies scale on top. The ordering constraint
//! `.after(crate::game_plugin::GameMutation)` ensures all game-state changes
//! settle before visual updates run.

use std::collections::VecDeque;

use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use super::animation::CardAnimation;
use crate::card_plugin::CardEntity;
use crate::events::{DrawRequestEvent, MoveRequestEvent, UndoRequestEvent};
use crate::layout::LayoutResource;
use crate::resources::DragState;

/// Type alias to reduce complexity in hover/drag query signatures.
type CardTransformQuery<'w, 's> =
    Query<'w, 's, (Entity, &'static mut Transform), (With<CardEntity>, Without<CardAnimation>)>;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Scale applied to the card currently under the cursor (1.0 = no change).
const HOVER_SCALE: f32 = 1.04;

/// Additional scale applied to dragged cards while in flight.
const DRAG_LIFT_SCALE: f32 = 1.08;

/// Lerp speed for hover scale interpolation (higher = snappier).
const HOVER_LERP_SPEED: f32 = 14.0;

/// Lerp speed for drag scale interpolation.
const DRAG_LERP_SPEED: f32 = 20.0;

/// Maximum number of buffered inputs retained.
const INPUT_BUFFER_CAPACITY: usize = 4;

// ---------------------------------------------------------------------------
// Resources
// ---------------------------------------------------------------------------

/// Tracks the entity currently under the cursor and the interpolated hover scale.
#[derive(Resource, Debug, Default)]
pub struct HoverState {
    /// Entity currently hovered (`None` when cursor is off all cards or dragging).
    pub entity: Option<Entity>,
    /// Current interpolated scale applied to the hovered card.
    pub scale: f32,
}

/// Describes a user action that arrived while cards were still animating.
#[derive(Debug, Clone)]
pub enum BufferedInput {
    Move { from: crate::events::MoveRequestEvent },
    Draw,
    Undo,
}

/// FIFO queue of inputs deferred until ongoing animations complete.
///
/// Populate via [`InputBuffer::push`] and consume via the drain system.
/// Capped at [`INPUT_BUFFER_CAPACITY`] — further pushes when full are silently
/// dropped to prevent stale action pileup.
#[derive(Resource, Debug, Default)]
pub struct InputBuffer {
    pub(crate) queue: VecDeque<BufferedInput>,
}

impl InputBuffer {
    /// Enqueues an input if the buffer is not full.
    pub fn push(&mut self, input: BufferedInput) {
        if self.queue.len() < INPUT_BUFFER_CAPACITY {
            self.queue.push_back(input);
        }
    }

    /// Returns `true` when no inputs are pending.
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// Returns how many inputs are queued.
    pub fn len(&self) -> usize {
        self.queue.len()
    }
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// Detects which card is under the cursor and updates [`HoverState`].
///
/// Clears hover when [`DragState`] is active (dragging takes visual priority).
/// Picks the topmost card (highest `translation.z`) when multiple cards overlap.
pub(crate) fn detect_hover(
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Camera, &GlobalTransform)>,
    drag: Option<Res<DragState>>,
    layout: Option<Res<LayoutResource>>,
    cards: Query<(Entity, &Transform), With<CardEntity>>,
    mut hover: ResMut<HoverState>,
) {
    let is_dragging = drag.as_ref().is_some_and(|d| !d.is_idle());
    if is_dragging {
        hover.entity = None;
        return;
    }

    let Some(layout) = layout else { return };
    let Some(cursor_world) = cursor_world(&windows, &cameras) else {
        hover.entity = None;
        return;
    };

    let half_w = layout.0.card_size.x * 0.5;
    let half_h = layout.0.card_size.y * 0.5;

    let mut best: Option<(Entity, f32)> = None;
    for (entity, transform) in &cards {
        let pos = transform.translation.truncate();
        if (cursor_world.x - pos.x).abs() < half_w
            && (cursor_world.y - pos.y).abs() < half_h
        {
            let z = transform.translation.z;
            if best.is_none_or(|(_, bz)| z > bz) {
                best = Some((entity, z));
            }
        }
    }

    hover.entity = best.map(|(e, _)| e);
}

/// Applies the hover scale to the currently hovered card via smooth lerp.
///
/// Only runs on cards that have **no active [`CardAnimation`]** — animated
/// cards control their own scale. When hover changes entities, the previous
/// entity's scale is snapped back to 1.0 to avoid leaving a permanently
/// enlarged card.
pub(crate) fn apply_hover_scale(
    time: Res<Time>,
    mut hover_state: ResMut<HoverState>,
    mut cards: CardTransformQuery,
) {
    let dt = time.delta_secs();
    let target_entity = hover_state.entity;

    for (entity, mut transform) in &mut cards {
        let target_scale = if Some(entity) == target_entity {
            HOVER_SCALE
        } else {
            1.0
        };

        let current = transform.scale.x;
        let new_scale = current + (target_scale - current) * (HOVER_LERP_SPEED * dt).min(1.0);
        transform.scale = Vec3::splat(new_scale);
    }

    // Update the tracked scale for external inspection.
    hover_state.scale = if let Some(entity) = target_entity {
        cards
            .get(entity)
            .map(|(_, t)| t.scale.x)
            .unwrap_or(HOVER_SCALE)
    } else {
        1.0
    };
}

/// Applies a scale boost and z-lift to dragged card entities.
///
/// Reads [`DragState`] for the list of card IDs being dragged. Does **not**
/// modify `translation.xy` — the existing `InputPlugin` owns drag translation.
/// Only writes `scale` and `translation.z` so the two systems are disjoint.
pub(crate) fn apply_drag_visual(
    time: Res<Time>,
    drag: Option<Res<DragState>>,
    mut cards: Query<(Entity, &CardEntity, &mut Transform), (Without<CardAnimation>,)>,
) {
    let dt = time.delta_secs();
    let empty: Vec<u32> = Vec::new();
    let dragged_ids: &[u32] = drag.as_ref().map_or(empty.as_slice(), |d| &d.cards);

    for (_, card, mut transform) in &mut cards {
        let is_dragged = dragged_ids.contains(&card.card_id);
        let target_scale = if is_dragged { DRAG_LIFT_SCALE } else { 1.0 };
        let current = transform.scale.x;
        let new_scale = current + (target_scale - current) * (DRAG_LERP_SPEED * dt).min(1.0);
        transform.scale = Vec3::splat(new_scale);
    }
}

/// Fires the oldest buffered input when no [`CardAnimation`] components remain.
///
/// Call this system late in the `Update` schedule so freshly-removed animations
/// are already gone before the drain runs.
pub(crate) fn drain_input_buffer(
    mut buffer: ResMut<InputBuffer>,
    anims: Query<&CardAnimation>,
    mut move_events: EventWriter<MoveRequestEvent>,
    mut draw_events: EventWriter<DrawRequestEvent>,
    mut undo_events: EventWriter<UndoRequestEvent>,
) {
    if !anims.is_empty() {
        return;
    }
    match buffer.queue.pop_front() {
        Some(BufferedInput::Move { from }) => {
            move_events.write(from);
        }
        Some(BufferedInput::Draw) => {
            draw_events.write(DrawRequestEvent);
        }
        Some(BufferedInput::Undo) => {
            undo_events.write(UndoRequestEvent);
        }
        None => {}
    }
}

// ---------------------------------------------------------------------------
// Cursor helper (mirrors the pattern used by input_plugin)
// ---------------------------------------------------------------------------

/// Converts the cursor screen position to 2-D world coordinates.
///
/// Returns `None` when the cursor is outside the window or no camera is found.
fn cursor_world(
    windows: &Query<&Window, With<PrimaryWindow>>,
    cameras: &Query<(&Camera, &GlobalTransform)>,
) -> Option<Vec2> {
    let window = windows.single().ok()?;
    let cursor = window.cursor_position()?;
    let (camera, camera_transform) = cameras.single().ok()?;
    camera.viewport_to_world_2d(camera_transform, cursor).ok()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_buffer_capacity_is_respected() {
        let mut buf = InputBuffer::default();
        for _ in 0..INPUT_BUFFER_CAPACITY + 5 {
            buf.push(BufferedInput::Draw);
        }
        assert_eq!(
            buf.len(),
            INPUT_BUFFER_CAPACITY,
            "buffer must not exceed capacity"
        );
    }

    #[test]
    fn input_buffer_is_fifo() {
        let mut buf = InputBuffer::default();
        buf.push(BufferedInput::Draw);
        buf.push(BufferedInput::Undo);

        matches!(buf.queue.pop_front().unwrap(), BufferedInput::Draw);
        matches!(buf.queue.pop_front().unwrap(), BufferedInput::Undo);
    }

    #[test]
    fn input_buffer_empty_initially() {
        let buf = InputBuffer::default();
        assert!(buf.is_empty());
        assert_eq!(buf.len(), 0);
    }

    #[test]
    fn input_buffer_len_increments() {
        let mut buf = InputBuffer::default();
        buf.push(BufferedInput::Draw);
        assert_eq!(buf.len(), 1);
        buf.push(BufferedInput::Undo);
        assert_eq!(buf.len(), 2);
    }

    #[test]
    fn hover_state_default_has_no_entity() {
        let state = HoverState::default();
        assert!(state.entity.is_none());
        assert_eq!(state.scale, 0.0);
    }
}
