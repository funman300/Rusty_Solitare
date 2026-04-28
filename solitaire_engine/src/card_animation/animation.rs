//! `CardAnimation` component and the system that drives it.
//!
//! # Design
//!
//! `CardAnimation` is a **drop-in upgrade** for the existing linear `CardAnim`.
//! It targets `Transform` (the current sprite-based architecture). Swapping to
//! Bevy UI requires only changing the four write lines in `advance_card_animations`
//! to write `Style.left` / `Style.top` via a `Style` component query instead.
//!
//! # Z-lift
//!
//! During motion, `translation.z` follows a parabolic arc:
//!
//! ```text
//! z(t) = lerp(start_z, end_z, t) + z_lift × sin(t × π)
//! ```
//!
//! The sine term is 0 at `t = 0` and `t = 1` and peaks at `t = 0.5`, so the
//! card "floats up" in the middle of its travel and lands at its correct rest z.
//!
//! # Retargeting
//!
//! When a card is redirected mid-flight, call [`retarget_animation`]. It reads
//! the current interpolated position so the card never snaps.
//!
//! # Coexistence with `CardAnim`
//!
//! `CardAnimation` and the legacy `CardAnim` can coexist in the same world but
//! **must never be on the same entity** — both write to `Transform`. When
//! migrating, replace `CardAnim` insertions with `CardAnimation` insertions and
//! register `CardAnimationPlugin` alongside `AnimationPlugin`.

use std::f32::consts::PI;

use bevy::prelude::*;

use super::curves::{sample_curve, MotionCurve};
use super::timing::compute_duration;
use crate::pause_plugin::PausedResource;

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

/// Curve-based card animation.
///
/// Drives `Transform` XY translation via a [`MotionCurve`], with optional
/// z-lift and scale interpolation. Removes itself when the animation completes.
#[derive(Component, Debug, Clone)]
pub struct CardAnimation {
    /// 2-D start position (world space).
    pub start: Vec2,
    /// 2-D destination (world space).
    pub end: Vec2,
    /// Seconds elapsed since the delay expired.
    pub elapsed: f32,
    /// Total animation duration in seconds (excluding delay).
    pub duration: f32,
    /// Easing curve applied to the interpolation factor.
    pub curve: MotionCurve,
    /// Seconds to wait before starting movement.
    pub delay: f32,
    /// Z coordinate at animation start (used for parabolic lift calculation).
    pub start_z: f32,
    /// Z coordinate at animation end — the card's resting z after completion.
    pub end_z: f32,
    /// Extra Z added at the midpoint of motion (`z(0.5) = base_z + z_lift`).
    /// Set to 0.0 to disable the depth arc.
    pub z_lift: f32,
    /// Transform scale at `t = 0`.
    pub scale_start: f32,
    /// Transform scale at `t = 1`.
    pub scale_end: f32,
}

impl CardAnimation {
    /// Convenience constructor: slide from `start` to `end` with auto-computed
    /// duration based on pixel distance. No z-lift or scale change.
    pub fn slide(start: Vec2, start_z: f32, end: Vec2, end_z: f32, curve: MotionCurve) -> Self {
        Self {
            start,
            end,
            elapsed: 0.0,
            duration: compute_duration(start.distance(end)),
            curve,
            delay: 0.0,
            start_z,
            end_z,
            z_lift: 0.0,
            scale_start: 1.0,
            scale_end: 1.0,
        }
    }

    /// Sets the pre-animation delay in seconds.
    #[must_use]
    pub fn with_delay(mut self, secs: f32) -> Self {
        self.delay = secs;
        self
    }

    /// Overrides the auto-computed duration.
    #[must_use]
    pub fn with_duration(mut self, secs: f32) -> Self {
        self.duration = secs;
        self
    }

    /// Enables the parabolic z-lift arc with the given peak offset.
    #[must_use]
    pub fn with_z_lift(mut self, lift: f32) -> Self {
        self.z_lift = lift;
        self
    }

    /// Interpolates `Transform.scale` from `start` to `end` over the animation.
    #[must_use]
    pub fn with_scale(mut self, start: f32, end: f32) -> Self {
        self.scale_start = start;
        self.scale_end = end;
        self
    }

    /// Returns the current interpolated XY position without advancing time.
    ///
    /// Used by [`retarget_animation`] to read mid-flight position cleanly.
    pub fn current_xy(&self) -> Vec2 {
        if self.duration <= 0.0 {
            return self.end;
        }
        let t = (self.elapsed / self.duration).clamp(0.0, 1.0);
        let s = sample_curve(self.curve, t);
        self.start.lerp(self.end, s)
    }
}

// ---------------------------------------------------------------------------
// Retarget helper
// ---------------------------------------------------------------------------

/// Redirects a card to a new destination without snapping or interrupting motion.
///
/// Reads the card's current interpolated position (from a live `CardAnimation` if
/// present, or from `Transform` if the card is stationary) and starts a fresh
/// `CardAnimation` from that position. Duration is recalculated from the remaining
/// distance so short remaining paths feel appropriately quick.
///
/// # Example
///
/// ```ignore
/// // Inside a system that decides to move a card to a new target:
/// let (entity, transform, anim) = cards.get(card_entity)?;
/// retarget_animation(
///     &mut commands,
///     entity,
///     anim,            // Option<&CardAnimation>
///     transform,
///     Vec2::new(400.0, 200.0),
///     resting_z,
///     MotionCurve::SmoothSnap,
/// );
/// ```
pub fn retarget_animation(
    commands: &mut Commands,
    entity: Entity,
    current_anim: Option<&CardAnimation>,
    transform: &Transform,
    new_end: Vec2,
    new_end_z: f32,
    curve: MotionCurve,
) {
    let (current_xy, current_z) = match current_anim {
        Some(anim) => (anim.current_xy(), transform.translation.z),
        None => (transform.translation.truncate(), transform.translation.z),
    };

    let distance = current_xy.distance(new_end);
    commands.entity(entity).insert(CardAnimation {
        start: current_xy,
        end: new_end,
        elapsed: 0.0,
        duration: compute_duration(distance),
        curve,
        delay: 0.0,
        start_z: current_z,
        end_z: new_end_z,
        z_lift: 8.0,
        scale_start: 1.0,
        scale_end: 1.0,
    });
}

// ---------------------------------------------------------------------------
// System
// ---------------------------------------------------------------------------

/// Advances all [`CardAnimation`] components each frame.
///
/// Skipped while the game is paused. On completion the component is removed
/// and `Transform` is snapped to the exact destination to prevent floating-point
/// drift.
pub(crate) fn advance_card_animations(
    mut commands: Commands,
    time: Res<Time>,
    paused: Option<Res<PausedResource>>,
    mut q: Query<(Entity, &mut Transform, &mut CardAnimation)>,
) {
    if paused.is_some_and(|p| p.0) {
        return;
    }
    let dt = time.delta_secs();

    for (entity, mut transform, mut anim) in &mut q {
        // Honour pre-animation delay.
        if anim.delay > 0.0 {
            anim.delay = (anim.delay - dt).max(0.0);
            continue;
        }

        // Zero-duration: instant snap.
        if anim.duration <= 0.0 {
            transform.translation = anim.end.extend(anim.end_z);
            transform.scale = Vec3::splat(anim.scale_end);
            commands.entity(entity).remove::<CardAnimation>();
            continue;
        }

        anim.elapsed += dt;
        let t = (anim.elapsed / anim.duration).min(1.0);
        let s = sample_curve(anim.curve, t);

        // --- XY via curve ---
        let xy = anim.start.lerp(anim.end, s);
        transform.translation.x = xy.x;
        transform.translation.y = xy.y;

        // --- Z: linear base interpolation + parabolic lift arc ---
        //
        // The sine arch is 0 at t=0 and t=1, peaking at t=0.5.
        // This keeps the card's resting Z correct at both ends.
        let base_z = anim.start_z + (anim.end_z - anim.start_z) * t;
        let lift = anim.z_lift * (t * PI).sin();
        transform.translation.z = base_z + lift;

        // --- Scale ---
        let scale = anim.scale_start + (anim.scale_end - anim.scale_start) * s;
        transform.scale = Vec3::splat(scale);

        // --- Completion ---
        if t >= 1.0 {
            transform.translation = anim.end.extend(anim.end_z);
            transform.scale = Vec3::splat(anim.scale_end);
            commands.entity(entity).remove::<CardAnimation>();
        }
    }
}

// ---------------------------------------------------------------------------
// Win cascade
// ---------------------------------------------------------------------------

/// Win-cascade scatter targets — 8 points beyond the window edges.
///
/// Scaled by `radius` (pass `layout.card_size.x * 8.0` for a good result).
pub fn win_scatter_targets(radius: f32) -> [Vec2; 8] {
    let r = radius;
    [
        Vec2::new(r, r),
        Vec2::new(-r, r),
        Vec2::new(r, -r),
        Vec2::new(-r, -r),
        Vec2::new(0.0, r),
        Vec2::new(0.0, -r),
        Vec2::new(r, 0.0),
        Vec2::new(-r, 0.0),
    ]
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_anim(start: Vec2, end: Vec2, elapsed: f32, duration: f32) -> CardAnimation {
        CardAnimation {
            start,
            end,
            elapsed,
            duration,
            curve: MotionCurve::Responsive, // linear-ish for easy assertion
            delay: 0.0,
            start_z: 0.0,
            end_z: 0.0,
            z_lift: 0.0,
            scale_start: 1.0,
            scale_end: 1.0,
        }
    }

    #[test]
    fn current_xy_at_start() {
        let anim = make_anim(Vec2::ZERO, Vec2::new(100.0, 0.0), 0.0, 1.0);
        let pos = anim.current_xy();
        assert!(pos.x < 5.0, "at t=0 position should be near start, got {pos:?}");
    }

    #[test]
    fn current_xy_at_end() {
        let anim = make_anim(Vec2::ZERO, Vec2::new(100.0, 0.0), 1.0, 1.0);
        let pos = anim.current_xy();
        assert!(
            (pos.x - 100.0).abs() < 1e-3,
            "at t=1 position should be at end, got {pos:?}"
        );
    }

    #[test]
    fn current_xy_zero_duration_returns_end() {
        let anim = make_anim(Vec2::ZERO, Vec2::new(50.0, 0.0), 0.0, 0.0);
        let pos = anim.current_xy();
        assert!(
            (pos.x - 50.0).abs() < 1e-3,
            "zero-duration must return end immediately, got {pos:?}"
        );
    }

    #[test]
    fn slide_constructor_auto_computes_duration() {
        let start = Vec2::ZERO;
        let end = Vec2::new(300.0, 0.0);
        let anim = CardAnimation::slide(start, 0.0, end, 0.0, MotionCurve::SmoothSnap);
        let distance = 300.0_f32;
        let expected = compute_duration(distance);
        assert!(
            (anim.duration - expected).abs() < 1e-5,
            "slide() duration mismatch: got {}, expected {}",
            anim.duration,
            expected
        );
    }

    #[test]
    fn with_delay_sets_delay() {
        let anim = CardAnimation::slide(Vec2::ZERO, 0.0, Vec2::ONE, 0.0, MotionCurve::SmoothSnap)
            .with_delay(0.5);
        assert!((anim.delay - 0.5).abs() < 1e-6);
    }

    #[test]
    fn with_z_lift_sets_z_lift() {
        let anim = CardAnimation::slide(Vec2::ZERO, 0.0, Vec2::ONE, 0.0, MotionCurve::SmoothSnap)
            .with_z_lift(12.0);
        assert!((anim.z_lift - 12.0).abs() < 1e-6);
    }

    #[test]
    fn win_scatter_has_eight_targets() {
        let targets = win_scatter_targets(800.0);
        assert_eq!(targets.len(), 8);
    }

    #[test]
    fn win_scatter_targets_are_off_center() {
        for t in win_scatter_targets(400.0) {
            let dist = t.length();
            assert!(dist > 100.0, "scatter target should be well off-center: {t:?}");
        }
    }
}
