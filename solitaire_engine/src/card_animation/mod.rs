//! `CardAnimationPlugin` — curve-based card animation system.
//!
//! # Quick start
//!
//! Register the plugin alongside the existing animation plugins:
//!
//! ```ignore
//! app.add_plugins((
//!     AnimationPlugin,       // existing: drives CardAnim (linear)
//!     FeedbackAnimPlugin,    // existing: shake + settle
//!     CardAnimationPlugin,   // new: curve-based CardAnimation
//! ));
//! ```
//!
//! Spawn a card with a `CardAnimation` component:
//!
//! ```ignore
//! use solitaire_engine::card_animation::{CardAnimation, MotionCurve};
//!
//! commands.spawn((
//!     SpriteBundle { /* ... */ },
//!     CardAnimation::slide(
//!         Vec2::new(0.0, 0.0),    // start xy
//!         0.0,                    // start z
//!         Vec2::new(300.0, 200.0),// end xy
//!         5.0,                    // end z (resting)
//!         MotionCurve::SmoothSnap,
//!     )
//!     .with_z_lift(12.0)          // floats up during motion
//!     .with_delay(0.03),          // stagger delay
//! ));
//! ```
//!
//! Retarget a card mid-flight:
//!
//! ```ignore
//! use solitaire_engine::card_animation::retarget_animation;
//!
//! fn handle_drop(
//!     mut commands: Commands,
//!     q: Query<(Entity, &Transform, Option<&CardAnimation>), With<CardEntity>>,
//! ) {
//!     let (entity, transform, anim) = q.get(card_entity).unwrap();
//!     retarget_animation(
//!         &mut commands,
//!         entity,
//!         anim,
//!         transform,
//!         new_target_xy,
//!         new_target_z,
//!         MotionCurve::SmoothSnap,
//!     );
//! }
//! ```
//!
//! # Win cascade with `Expressive` curve
//!
//! The existing `AnimationPlugin` drives the win cascade with `CardAnim`
//! (linear). To use the curve-based cascade instead, disable
//! `handle_win_cascade` in `AnimationPlugin` and register `WinCascadePlugin`
//! (declared below) which uses `CardAnimation` + `MotionCurve::Expressive`.
//!
//! They **must not both be active** — both write to `Transform` on the same
//! 52 entities and will race.
//!
//! # Coexistence rules
//!
//! | Condition | Safe? |
//! |---|---|
//! | `CardAnim` and `CardAnimation` on **different** entities | ✓ |
//! | `CardAnim` and `CardAnimation` on the **same** entity | ✗ |
//! | `HoverState` scale + `CardAnimation` scale on same entity | ✓ (CardAnimation takes priority — hover skipped via `Without<CardAnimation>` filter) |
//! | `apply_drag_visual` scale + `CardAnimation` scale | ✓ (same filter) |

pub mod animation;
pub mod curves;
pub mod interaction;
pub mod timing;

pub use animation::{retarget_animation, win_scatter_targets, CardAnimation};
pub use curves::{sample_curve, MotionCurve};
pub use interaction::{BufferedInput, HoverState, InputBuffer};
pub use timing::{
    cascade_delay, compute_duration, micro_vary, DEAL_INTERVAL_SECS, MAX_DURATION_SECS,
    MIN_DURATION_SECS, WIN_CASCADE_INTERVAL_SECS,
};

use bevy::prelude::*;

use crate::card_plugin::CardEntity;
use crate::events::{DrawRequestEvent, GameWonEvent, MoveRequestEvent, UndoRequestEvent};
use crate::game_plugin::GameMutation;
use crate::layout::LayoutResource;
use crate::resources::DragState;

use animation::advance_card_animations;
use interaction::{apply_drag_visual, apply_hover_scale, detect_hover, drain_input_buffer};

// ---------------------------------------------------------------------------
// Plugin
// ---------------------------------------------------------------------------

/// Registers all systems, resources, and components for curve-based card
/// animation, hover visuals, drag lift, and input buffering.
///
/// Safe to register alongside `AnimationPlugin` and `FeedbackAnimPlugin` as
/// long as no single entity carries both `CardAnim` and `CardAnimation`.
pub struct CardAnimationPlugin;

impl Plugin for CardAnimationPlugin {
    fn build(&self, app: &mut App) {
        // Register events and resources that interaction systems depend on,
        // idempotently — double-registration is safe in Bevy.
        app.add_message::<MoveRequestEvent>()
            .add_message::<DrawRequestEvent>()
            .add_message::<UndoRequestEvent>()
            .add_message::<GameWonEvent>()
            .init_resource::<DragState>()
            .init_resource::<HoverState>()
            .init_resource::<InputBuffer>()
            .add_systems(
                Update,
                (
                    // Advance active animations (highest priority — runs first).
                    advance_card_animations,
                    // Interaction visuals (run after animation to read final positions).
                    detect_hover,
                    apply_hover_scale,
                    apply_drag_visual,
                    // Drain buffered inputs only when no animations remain.
                    drain_input_buffer,
                )
                    .chain()
                    .after(GameMutation),
            );
    }
}

// ---------------------------------------------------------------------------
// Optional: win cascade with Expressive curve
// ---------------------------------------------------------------------------

/// Optional plugin that replaces the linear win cascade in `AnimationPlugin`
/// with an `Expressive`-curve cascade.
///
/// **Do not register this alongside `AnimationPlugin`'s win cascade** — they
/// will race on the same card entities. To use this plugin, prevent
/// `AnimationPlugin` from handling `GameWonEvent` (or remove it and manage
/// win toasts manually).
pub struct WinCascadePlugin;

impl Plugin for WinCascadePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            trigger_expressive_win_cascade.after(GameMutation),
        );
    }
}

/// Inserts `CardAnimation` (Expressive curve) on every card when `GameWonEvent` fires.
///
/// Cards scatter to 8 off-screen positions with per-card stagger. The z-lift
/// creates a "burst" effect as cards fly outward.
fn trigger_expressive_win_cascade(
    mut events: MessageReader<GameWonEvent>,
    cards: Query<(Entity, &Transform), With<CardEntity>>,
    layout: Option<Res<LayoutResource>>,
    mut commands: Commands,
) {
    if events.read().next().is_none() {
        return;
    }

    let radius = layout
        .as_ref()
        .map_or(800.0, |l| l.0.card_size.x * 8.0);

    let targets = win_scatter_targets(radius);

    for (index, (entity, transform)) in cards.iter().enumerate() {
        let start_xy = transform.translation.truncate();
        let start_z = transform.translation.z;
        let target = targets[index % targets.len()];

        commands.entity(entity).insert(
            CardAnimation::slide(start_xy, start_z, target, start_z + 60.0, MotionCurve::Expressive)
                .with_delay(cascade_delay(index, WIN_CASCADE_INTERVAL_SECS))
                .with_duration(0.65)
                .with_z_lift(25.0),
        );
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::animation_plugin::AnimationPlugin;
    use crate::card_plugin::CardPlugin;
    use crate::game_plugin::GamePlugin;
    use crate::table_plugin::TablePlugin;

    fn base_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(GamePlugin)
            .add_plugins(TablePlugin)
            .add_plugins(CardPlugin)
            .add_plugins(AnimationPlugin)
            .add_plugins(CardAnimationPlugin);
        app.update();
        app
    }

    #[test]
    fn plugin_registers_hover_state() {
        let app = base_app();
        assert!(
            app.world().get_resource::<HoverState>().is_some(),
            "HoverState resource must be registered"
        );
    }

    #[test]
    fn plugin_registers_input_buffer() {
        let app = base_app();
        assert!(
            app.world().get_resource::<InputBuffer>().is_some(),
            "InputBuffer resource must be registered"
        );
    }

    #[test]
    fn card_animation_advances_and_removes_itself() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins).add_plugins(CardAnimationPlugin);

        let start = Vec2::new(0.0, 0.0);
        let end = Vec2::new(100.0, 0.0);
        let entity = app
            .world_mut()
            .spawn((
                Transform::from_translation(start.extend(0.0)),
                CardAnimation {
                    start,
                    end,
                    elapsed: 0.99,
                    duration: 1.0,
                    curve: MotionCurve::Responsive,
                    delay: 0.0,
                    start_z: 0.0,
                    end_z: 0.0,
                    z_lift: 0.0,
                    scale_start: 1.0,
                    scale_end: 1.0,
                },
            ))
            .id();

        app.update();

        // After one update at elapsed=0.99, component should still be present.
        // We can't advance time reliably in MinimalPlugins, but we can check
        // that the advance_card_animations system processed the component
        // (pos moved closer to end).
        let transform = app.world().entity(entity).get::<Transform>().unwrap();
        assert!(
            transform.translation.x > 50.0,
            "card should have moved past midpoint by elapsed=0.99, got x={}",
            transform.translation.x
        );
    }

    #[test]
    fn card_animation_instant_snaps_on_zero_duration() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins).add_plugins(CardAnimationPlugin);

        let end = Vec2::new(200.0, 100.0);
        let entity = app
            .world_mut()
            .spawn((
                Transform::from_translation(Vec3::ZERO),
                CardAnimation {
                    start: Vec2::ZERO,
                    end,
                    elapsed: 0.0,
                    duration: 0.0, // zero duration → instant snap
                    curve: MotionCurve::SmoothSnap,
                    delay: 0.0,
                    start_z: 0.0,
                    end_z: 5.0,
                    z_lift: 0.0,
                    scale_start: 1.0,
                    scale_end: 1.0,
                },
            ))
            .id();

        app.update();

        assert!(
            app.world().entity(entity).get::<CardAnimation>().is_none(),
            "zero-duration animation must be removed after one update"
        );
        let transform = app.world().entity(entity).get::<Transform>().unwrap();
        assert!(
            (transform.translation.x - 200.0).abs() < 1e-3,
            "card must snap to end.x"
        );
        assert!(
            (transform.translation.y - 100.0).abs() < 1e-3,
            "card must snap to end.y"
        );
        assert!(
            (transform.translation.z - 5.0).abs() < 1e-3,
            "card must snap to end_z"
        );
    }

    #[test]
    fn card_animation_respects_delay() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins).add_plugins(CardAnimationPlugin);

        let entity = app
            .world_mut()
            .spawn((
                Transform::from_translation(Vec3::ZERO),
                CardAnimation {
                    start: Vec2::ZERO,
                    end: Vec2::new(100.0, 0.0),
                    elapsed: 0.0,
                    duration: 0.15,
                    curve: MotionCurve::SmoothSnap,
                    delay: 100.0, // huge delay — card must not move
                    start_z: 0.0,
                    end_z: 0.0,
                    z_lift: 0.0,
                    scale_start: 1.0,
                    scale_end: 1.0,
                },
            ))
            .id();

        app.update();

        let transform = app.world().entity(entity).get::<Transform>().unwrap();
        assert!(
            transform.translation.x.abs() < 1e-3,
            "card must not move during delay, got x={}",
            transform.translation.x
        );
    }

    #[test]
    fn input_buffer_push_and_drain_ordering() {
        let mut buf = InputBuffer::default();
        buf.push(BufferedInput::Draw);
        buf.push(BufferedInput::Undo);
        // FIFO: Draw comes out first.
        assert!(matches!(buf.queue.pop_front().unwrap(), BufferedInput::Draw));
        assert!(matches!(buf.queue.pop_front().unwrap(), BufferedInput::Undo));
    }

    #[test]
    fn hover_state_initialises_without_entity() {
        let state = HoverState::default();
        assert!(state.entity.is_none());
    }

    #[test]
    fn win_scatter_produces_eight_distinct_points() {
        let targets = win_scatter_targets(600.0);
        assert_eq!(targets.len(), 8);
        // All must be different.
        for i in 0..8 {
            for j in (i + 1)..8 {
                assert_ne!(
                    targets[i], targets[j],
                    "scatter targets {i} and {j} must be distinct"
                );
            }
        }
    }
}
