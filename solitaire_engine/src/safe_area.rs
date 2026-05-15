//! Safe-area insets.
//!
//! Reports the OS-reserved regions around the playable surface (status
//! bar at the top, gesture / navigation bar at the bottom on Android,
//! display cutouts, etc.) so UI anchored to a screen edge can avoid
//! collisions.
//!
//! On non-Android targets all four edges report `0.0`. On Android the
//! values come from `WindowInsets.getInsets(WindowInsets.Type.systemBars())`
//! via JNI; the call is retried for the first few frames because
//! `getRootWindowInsets()` only returns useful values after the decor
//! view has been laid out at least once.
//!
//! UI that wants to respect the top inset should tag itself with the
//! [`SafeAreaAnchoredTop`] marker carrying the layout's original top
//! offset; [`apply_safe_area_anchors`] re-applies `base_top + insets.top`
//! whenever the resource changes, so late inset arrival or orientation
//! changes flow through automatically.

use bevy::prelude::*;

use crate::ui_modal::ModalScrim;

/// Pixel sizes of the system-reserved regions on each edge of the
/// surface. Zero on desktop.
#[derive(Resource, Debug, Clone, Copy, Default, PartialEq)]
pub struct SafeAreaInsets {
    pub top: f32,
    pub bottom: f32,
    pub left: f32,
    pub right: f32,
}

impl SafeAreaInsets {
    /// `true` when any edge has a non-zero reservation. Used by the
    /// Android polling system to know it can stop querying.
    pub fn is_populated(&self) -> bool {
        self.top > 0.0 || self.bottom > 0.0 || self.left > 0.0 || self.right > 0.0
    }
}

/// Marker for `Node` entities whose `top` offset should be re-applied
/// as `base_top + SafeAreaInsets::top`.
///
/// `base_top` is the offset the layout would have used on a surface
/// with no system reservation (i.e. on desktop). The fix-up system
/// adds the current top inset on top of it whenever the resource
/// changes.
#[derive(Component, Debug, Clone, Copy)]
pub struct SafeAreaAnchoredTop {
    pub base_top: f32,
}

/// Marker for `Node` entities whose `bottom` offset should be re-applied
/// as `base_bottom + SafeAreaInsets::bottom / scale`.
///
/// Use this for elements anchored to the bottom edge (e.g. a bottom action
/// bar) so they clear the Android gesture-navigation zone automatically.
#[derive(Component, Debug, Clone, Copy)]
pub struct SafeAreaAnchoredBottom {
    pub base_bottom: f32,
}

pub struct SafeAreaInsetsPlugin;

impl Plugin for SafeAreaInsetsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SafeAreaInsets>()
            .add_systems(
                Update,
                (apply_safe_area_anchors, apply_safe_area_bottom_anchors, apply_safe_area_to_modal_scrims),
            );

        #[cfg(target_os = "android")]
        app.add_systems(Update, android::refresh_insets);
    }
}

/// Re-applies `base_top + insets.top` to every entity carrying the
/// [`SafeAreaAnchoredTop`] marker whenever [`SafeAreaInsets`] changes.
///
/// Bevy resource change detection (`Res::is_changed`) is `true` on the
/// frame the resource is inserted and every frame a `ResMut` borrow
/// occurs. Combined with the Android polling loop short-circuiting
/// once insets are populated, this runs at most a handful of times in
/// a session.
fn apply_safe_area_anchors(
    insets: Res<SafeAreaInsets>,
    windows: Query<&Window>,
    mut q: Query<(&SafeAreaAnchoredTop, &mut Node)>,
) {
    if !insets.is_changed() {
        return;
    }
    // Android's WindowInsets API returns physical pixels; Bevy UI's Val::Px
    // expects logical pixels (≈ dp). Divide by the window scale factor so
    // the HUD band shifts by the correct number of dp on high-DPI devices.
    let scale = windows.iter().next().map_or(1.0, |w| w.scale_factor());
    let top_logical = insets.top / scale;
    for (anchor, mut node) in &mut q {
        node.top = Val::Px(anchor.base_top + top_logical);
    }
}

/// Re-applies `base_bottom + insets.bottom / scale` to every entity carrying
/// [`SafeAreaAnchoredBottom`] whenever [`SafeAreaInsets`] changes.
fn apply_safe_area_bottom_anchors(
    insets: Res<SafeAreaInsets>,
    windows: Query<&Window>,
    mut q: Query<(&SafeAreaAnchoredBottom, &mut Node)>,
) {
    if !insets.is_changed() {
        return;
    }
    let scale = windows.iter().next().map_or(1.0, |w| w.scale_factor());
    let bottom_logical = insets.bottom / scale;
    for (anchor, mut node) in &mut q {
        node.bottom = Val::Px(anchor.base_bottom + bottom_logical);
    }
}

/// Pads the bottom of every [`ModalScrim`] by the logical bottom inset so
/// modal cards don't extend into the Android gesture-navigation zone.
///
/// Fires when [`SafeAreaInsets`] changes (covers the common case of insets
/// arriving a few frames after app start) AND when a new `ModalScrim` is
/// spawned (covers modals opened after insets have already settled).
fn apply_safe_area_to_modal_scrims(
    insets: Res<SafeAreaInsets>,
    windows: Query<&Window>,
    mut scrims: Query<&mut Node, With<ModalScrim>>,
    new_scrims: Query<(), (With<ModalScrim>, Added<ModalScrim>)>,
) {
    let has_new = !new_scrims.is_empty();
    if !insets.is_changed() && !has_new {
        return;
    }
    let scale = windows.iter().next().map_or(1.0, |w| w.scale_factor());
    let bottom_logical = insets.bottom / scale;
    for mut node in &mut scrims {
        node.padding.bottom = Val::Px(bottom_logical);
    }
}

#[cfg(target_os = "android")]
mod android {
    use super::SafeAreaInsets;
    use bevy::prelude::*;

    /// Polls Android for safe-area insets until we get a non-zero
    /// reading, then stops. `getRootWindowInsets()` returns `null` (or
    /// all-zero `Insets`) until the decor view has been laid out, which
    /// is typically frame 1–3 of a fresh launch.
    pub(super) fn refresh_insets(
        mut insets: ResMut<SafeAreaInsets>,
        mut tries: Local<u32>,
    ) {
        // Cap retries so we don't burn CPU forever on edge-to-edge
        // devices that genuinely report zero insets.
        const MAX_TRIES: u32 = 120; // ~2 seconds @ 60 fps

        if *tries >= MAX_TRIES || insets.is_populated() {
            return;
        }
        *tries += 1;

        match query_insets() {
            Ok(v) if v.is_populated() => {
                info!(
                    "safe_area: insets resolved top={} bottom={} left={} right={} (after {} frames)",
                    v.top, v.bottom, v.left, v.right, *tries
                );
                *insets = v;
            }
            Ok(_) => {
                // Layout not ready yet; try again next frame.
            }
            Err(e) => {
                // Don't spam — log once and let polling continue silently.
                if *tries == 1 {
                    warn!("safe_area: JNI query failed (will retry): {e}");
                }
            }
        }
    }

    fn query_insets() -> Result<SafeAreaInsets, String> {
        use bevy::android::ANDROID_APP;
        use jni::{objects::JObject, JavaVM};

        let app = ANDROID_APP
            .get()
            .ok_or_else(|| "ANDROID_APP not initialized".to_string())?;

        // SAFETY: `vm_as_ptr()` returns the JavaVM* set up by the Android
        // runtime; valid for the lifetime of the process.
        let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr().cast()) }
            .map_err(|e| format!("JavaVM::from_raw: {e}"))?;

        let mut env = vm
            .attach_current_thread_permanently()
            .map_err(|e| format!("attach_current_thread: {e}"))?;

        // SAFETY: `activity_as_ptr()` returns the NativeActivity jobject
        // pointer — valid for the lifetime of the process.
        let activity = unsafe { JObject::from_raw(app.activity_as_ptr() as _) };

        (|| -> jni::errors::Result<SafeAreaInsets> {
            // Window window = activity.getWindow();
            let window = env
                .call_method(&activity, "getWindow", "()Landroid/view/Window;", &[])?
                .l()?;

            // View decor = window.getDecorView();
            let decor = env
                .call_method(&window, "getDecorView", "()Landroid/view/View;", &[])?
                .l()?;

            // WindowInsets insets = decor.getRootWindowInsets();
            let raw_insets = env
                .call_method(
                    &decor,
                    "getRootWindowInsets",
                    "()Landroid/view/WindowInsets;",
                    &[],
                )?
                .l()?;
            if raw_insets.is_null() {
                return Ok(SafeAreaInsets::default());
            }

            // int types = WindowInsets.Type.systemBars();
            // (Static method on the WindowInsets$Type inner class.
            // Available since API 30 / Android 11.)
            let type_class = env.find_class("android/view/WindowInsets$Type")?;
            let bars_type = env
                .call_static_method(&type_class, "systemBars", "()I", &[])?
                .i()?;

            // Insets bars = insets.getInsets(types);
            let bars = env
                .call_method(
                    &raw_insets,
                    "getInsets",
                    "(I)Landroid/graphics/Insets;",
                    &[bars_type.into()],
                )?
                .l()?;

            // `Insets` exposes `top`, `bottom`, `left`, `right` as public
            // `int` fields (pixel values, not dp).
            let top = env.get_field(&bars, "top", "I")?.i()? as f32;
            let bottom = env.get_field(&bars, "bottom", "I")?.i()? as f32;
            let left = env.get_field(&bars, "left", "I")?.i()? as f32;
            let right = env.get_field(&bars, "right", "I")?.i()? as f32;

            Ok(SafeAreaInsets {
                top,
                bottom,
                left,
                right,
            })
        })()
        .map_err(|e| format!("safe-area JNI: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_zero_and_not_populated() {
        let i = SafeAreaInsets::default();
        assert_eq!(i.top, 0.0);
        assert_eq!(i.bottom, 0.0);
        assert!(!i.is_populated());
    }

    #[test]
    fn is_populated_returns_true_for_any_nonzero_edge() {
        assert!(SafeAreaInsets {
            top: 24.0,
            ..Default::default()
        }
        .is_populated());
        assert!(SafeAreaInsets {
            bottom: 16.0,
            ..Default::default()
        }
        .is_populated());
        assert!(SafeAreaInsets {
            left: 8.0,
            ..Default::default()
        }
        .is_populated());
        assert!(SafeAreaInsets {
            right: 8.0,
            ..Default::default()
        }
        .is_populated());
    }
}
