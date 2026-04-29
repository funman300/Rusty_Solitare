//! Platform-adaptive animation tuning.
//!
//! [`AnimationTuning`] is a Bevy resource that provides animation parameters
//! adapted to the currently detected input platform. Systems and components
//! that need animation timing should read from this resource instead of using
//! hardcoded constants, so the same binary behaves appropriately on both a
//! touchscreen phone and a desktop with a mouse.
//!
//! # Platform detection
//!
//! [`update_input_platform`] runs every frame. When a touch event is detected
//! the resource switches to [`InputPlatform::Touch`] (mobile defaults); when a
//! mouse event is detected it switches back to [`InputPlatform::Mouse`]
//! (desktop defaults). The transition is immediate.
//!
//! # Usage
//!
//! ```ignore
//! fn my_system(tuning: Res<AnimationTuning>, time: Res<Time>) {
//!     let duration = tuning.scale_duration(0.25); // 0.25 s on desktop, 0.19 s on mobile
//!     let scale = tuning.drag_scale;              // platform-appropriate lift
//! }
//! ```

use bevy::input::touch::Touches;
use bevy::prelude::*;

// ---------------------------------------------------------------------------
// InputPlatform
// ---------------------------------------------------------------------------

/// The most recently detected input platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InputPlatform {
    /// Mouse / keyboard — desktop behaviour (richer motion, hover states).
    #[default]
    Mouse,
    /// Touchscreen — mobile behaviour (faster, tighter, no hover).
    Touch,
}

// ---------------------------------------------------------------------------
// AnimationTuning resource
// ---------------------------------------------------------------------------

/// Animation and interaction parameters adapted to the active [`InputPlatform`].
///
/// Mobile (touch) defaults are faster and less bouncy than desktop (mouse)
/// defaults. Read this resource wherever you previously used animation
/// constants to get correct behaviour across both platforms.
#[derive(Resource, Debug, Clone)]
pub struct AnimationTuning {
    /// Currently detected input platform.
    pub platform: InputPlatform,

    /// Multiplier applied to all computed animation durations.
    ///
    /// `1.0` on desktop; `0.75` on mobile (25 % faster).
    pub duration_scale: f32,

    /// Multiplier applied to spring-curve overshoot amplitude.
    ///
    /// `1.0` on desktop (full bounce); `0.5` on mobile (half — tighter feel
    /// on small screens where large overshoots look incorrect).
    pub overshoot_scale: f32,

    /// Minimum pointer/finger movement in **screen pixels** before a drag
    /// is committed.
    ///
    /// Prevents accidental drags from quick taps. Desktop = 4 px; mobile
    /// = 10 px (fingers are less precise than a mouse cursor).
    pub drag_threshold_px: f32,

    /// `Transform.scale` applied to a card while it is being dragged.
    pub drag_scale: f32,

    /// `Transform.scale` applied to the card under the cursor (desktop only).
    ///
    /// Always `1.0` on touch because there is no hover concept on a
    /// touchscreen — applying hover to the card under the last touch
    /// would feel wrong.
    pub hover_scale: f32,

    /// Lerp speed (per second) for the hover scale interpolation.
    ///
    /// Higher values make the hover pop in/out faster.
    pub hover_lerp_speed: f32,

    /// Per-card stagger interval (seconds) for cascade / deal animations.
    ///
    /// Mobile gets a slightly tighter stagger so the full cascade finishes
    /// more quickly.
    pub cascade_stagger_secs: f32,
}

impl AnimationTuning {
    /// Desktop (mouse) defaults — richer motion, more expressive curves.
    pub fn desktop() -> Self {
        Self {
            platform: InputPlatform::Mouse,
            duration_scale: 1.0,
            overshoot_scale: 1.0,
            drag_threshold_px: 4.0,
            drag_scale: 1.08,
            hover_scale: 1.04,
            hover_lerp_speed: 14.0,
            cascade_stagger_secs: 0.018,
        }
    }

    /// Mobile (touch) defaults — faster, tighter, no hover.
    pub fn mobile() -> Self {
        Self {
            platform: InputPlatform::Touch,
            duration_scale: 0.75,
            overshoot_scale: 0.5,
            drag_threshold_px: 10.0,
            drag_scale: 1.12,
            hover_scale: 1.0, // no hover affordance on touch
            hover_lerp_speed: 20.0,
            cascade_stagger_secs: 0.014,
        }
    }

    /// Scales `base_duration` by [`Self::duration_scale`].
    ///
    /// Use this wherever you compute an animation duration to respect the
    /// current platform's speed preference.
    #[inline]
    pub fn scale_duration(&self, base_duration: f32) -> f32 {
        base_duration * self.duration_scale
    }
}

impl Default for AnimationTuning {
    fn default() -> Self {
        Self::desktop()
    }
}

// ---------------------------------------------------------------------------
// Detection system
// ---------------------------------------------------------------------------

/// Detects the active input platform and updates [`AnimationTuning`] to match.
///
/// Called every frame. Uses `Option<Res<Touches>>` so the system is safe when
/// running under `MinimalPlugins` (which does not register the touch subsystem).
pub(crate) fn update_input_platform(
    touches: Option<Res<Touches>>,
    mouse_buttons: Option<Res<ButtonInput<MouseButton>>>,
    mut tuning: ResMut<AnimationTuning>,
) {
    let touch_active = touches.as_ref().is_some_and(|t| {
        t.iter().next().is_some()
            || t.iter_just_pressed().next().is_some()
            || t.iter_just_released().next().is_some()
    });

    let mouse_active = mouse_buttons.as_ref().is_some_and(|mb| {
        mb.get_just_pressed().next().is_some() || mb.get_pressed().next().is_some()
    });

    if touch_active && tuning.platform != InputPlatform::Touch {
        *tuning = AnimationTuning::mobile();
    } else if mouse_active && tuning.platform != InputPlatform::Mouse {
        *tuning = AnimationTuning::desktop();
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_defaults_are_sane() {
        let t = AnimationTuning::desktop();
        assert_eq!(t.duration_scale, 1.0);
        assert_eq!(t.platform, InputPlatform::Mouse);
        assert!(t.hover_scale > 1.0, "desktop hover must lift the card");
        assert!(t.drag_threshold_px < 10.0, "desktop threshold must be smaller than mobile");
    }

    #[test]
    fn mobile_is_faster_than_desktop() {
        let d = AnimationTuning::desktop();
        let m = AnimationTuning::mobile();
        assert!(m.duration_scale < d.duration_scale, "mobile must animate faster");
        assert!(m.overshoot_scale < d.overshoot_scale, "mobile must bounce less");
    }

    #[test]
    fn mobile_has_no_hover() {
        // On touch, `hover_scale = 1.0` means no visible hover effect.
        assert_eq!(AnimationTuning::mobile().hover_scale, 1.0);
    }

    #[test]
    fn mobile_drag_threshold_larger_than_desktop() {
        assert!(
            AnimationTuning::mobile().drag_threshold_px
                > AnimationTuning::desktop().drag_threshold_px,
            "mobile needs a larger threshold because touch is less precise"
        );
    }

    #[test]
    fn scale_duration_applies_multiplier() {
        let mut t = AnimationTuning::default();
        t.duration_scale = 0.5;
        assert!((t.scale_duration(1.0) - 0.5).abs() < 1e-6);
        assert!((t.scale_duration(0.25) - 0.125).abs() < 1e-6);
    }

    #[test]
    fn mobile_cascade_stagger_tighter_than_desktop() {
        assert!(
            AnimationTuning::mobile().cascade_stagger_secs
                < AnimationTuning::desktop().cascade_stagger_secs
        );
    }

    #[test]
    fn default_is_desktop() {
        assert_eq!(AnimationTuning::default().platform, InputPlatform::Mouse);
    }
}
