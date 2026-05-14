//! Keyboard focus ring for modal buttons (Phase 1).
//!
//! Ferrous Solitaire's 11 modals (Help, Stats, Achievements, Settings,
//! Profile, Leaderboard, Pause, Forfeit confirm, GameOver, Confirm new
//! game, Onboarding) ship without any keyboard focus support. Phase 1
//! gives every button spawned via [`crate::ui_modal::spawn_modal_button`]
//! a real, visible focus state:
//!
//! - **Tab / Shift+Tab** cycles forward / backward through buttons
//! - **Enter / Space** activates the focused button (writes
//!   `Interaction::Pressed` so existing click handlers fire)
//! - **Mouse click** on a focusable transfers keyboard focus to it
//! - **Modal open** auto-focuses the Primary button (or the first
//!   focusable in spawn order if no Primary exists)
//!
//! ## Architecture: sibling overlay entity
//!
//! Rather than attach a `BorderColor` / `Outline` to the focused button —
//! which would inherit the modal card's open-animation scale and clip to
//! any scroll container — Phase 1 uses a single overlay entity that is
//! never a descendant of any modal. Each frame, [`update_focus_overlay`]
//! reads the focused button's [`bevy::ui::UiGlobalTransform`] and
//! [`bevy::ui::ComputedNode`] and positions the overlay's absolute
//! `Node` to wrap the button with a 4 px halo.
//!
//! This sidesteps:
//! - Modal card scale-in (the overlay is a sibling, not a child)
//! - `Overflow::scroll_y()` clipping (no ancestor enforces a clip rect)
//!
//! ## Phase scope
//!
//! Phase 1 is modal buttons only. Phase 2 extended the same component
//! to the HUD action bar (on hover) and Home mode cards. Phase 3 closes
//! out the engine: Settings bespoke buttons opt-in via the same
//! ancestry-walk pattern, picker rows inside Settings get [`FocusRow`]
//! so Left/Right cycle within the row, and the focused button is
//! auto-scrolled into the visible Settings viewport (see the
//! `scroll_focus_into_view` system in `settings_plugin`).
//!
//! When no modal is open and no HUD button is hovered, every system
//! here no-ops so [`crate::selection_plugin`]'s Tab/Enter
//! card-selection still works.

use std::f32::consts::TAU;

use bevy::ecs::query::Has;
use bevy::input::ButtonInput;
use bevy::prelude::*;
use bevy::ui::{ComputedNode, UiGlobalTransform};
use solitaire_data::AnimSpeed;

use crate::settings_plugin::SettingsResource;
use crate::ui_modal::{ButtonVariant, ModalButton, ModalScrim};
use crate::ui_theme::{FOCUS_RING, MOTION_FOCUS_PULSE_SECS, RADIUS_MD, Z_FOCUS_RING};

// ---------------------------------------------------------------------------
// Public component / resource API
// ---------------------------------------------------------------------------

/// Marker on every interactive entity that participates in keyboard
/// focus. Phase 1 inserts this on every [`ModalButton`]; future phases
/// will extend the same component to HUD buttons and Home mode cards.
#[derive(Component, Debug, Clone, Copy)]
pub struct Focusable {
    /// Group this focusable belongs to. Tab cycles inside a single
    /// group at a time — buttons in different modals don't interleave.
    pub group: FocusGroup,
    /// Lower numbers visited first within a group. Phase 1 keeps every
    /// modal button at `0` and uses spawn-order (entity index) as the
    /// tiebreaker, which matches `spawn_modal_actions`'s document order.
    pub order: i32,
}

/// Logical grouping for keyboard focus. Tab cycles only within the
/// active group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FocusGroup {
    /// Bound to a specific scrim entity — modal-scoped. Two stacked
    /// modals (e.g. Pause + Forfeit confirm) maintain independent
    /// focus rings; Tab cycles inside the topmost one.
    Modal(Entity),
    /// Top-right action bar. Phase 2 will populate this — Phase 1
    /// declares the variant so the surface is stable.
    Hud,
}

/// Marker that suppresses Tab navigation and Enter / Space activation
/// for an otherwise-focusable entity. Public so callers can opt buttons
/// in or out at runtime without removing [`Focusable`] (which would
/// also break the spawn-order ordering).
#[derive(Component, Debug, Clone, Copy)]
pub struct Disabled;

/// Marker on a parent container whose direct [`Focusable`] children
/// form a horizontal row navigable by Left / Right arrow keys.
///
/// Tab / Shift+Tab still escape the row to the next focusable outside
/// it (the row's children participate in their group's normal cycle
/// just like any other focusable). Arrow keys are scoped to the row:
/// pressing Left/Right wraps within the row's children only, skipping
/// any child marked [`Disabled`].
///
/// Used by Settings picker rows (card-back swatches, background
/// swatches) to give players a familiar "select-from-options" feel
/// without leaving the keyboard.
#[derive(Component, Debug, Clone, Copy)]
pub struct FocusRow;

/// Globally-focused button entity, or `None` if nothing is focused.
/// Read-only in steady state; written by the focus systems on Tab,
/// mouse click, and modal open / close.
#[derive(Resource, Debug, Default)]
pub struct FocusedButton(pub Option<Entity>);

/// Registers the keyboard-focus ring system. Add this plugin once,
/// after [`crate::ui_modal::UiModalPlugin`], so every modal button
/// gains keyboard navigation without per-plugin wiring.
pub struct UiFocusPlugin;

impl Plugin for UiFocusPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<FocusedButton>()
            .add_systems(Startup, spawn_focus_overlay)
            // Attach + auto-focus run in `PostUpdate` so they see entities
            // a click-handler in `Update` queued via `Commands` earlier in
            // the same frame. If they ran in `Update` they'd race the
            // click handler: there's no ordering edge between an arbitrary
            // modal-spawning system and the focus chain, so Bevy's
            // `auto_insert_apply_deferred` pass cannot synchronise them.
            // Pushing the attach / auto-focus pair into `PostUpdate` puts
            // the natural schedule-boundary sync point between every
            // modal spawn and focus arrival — `FocusedButton` is always
            // populated before the same `app.update()` returns.
            //
            // The remaining systems stay in `Update` so they keep
            // observing input on the frame it occurs. They read
            // `FocusedButton` written during the *previous* tick's
            // `PostUpdate`, which is exactly what we want: the very next
            // user keypress after a modal opens lands on a populated
            // resource.
            .add_systems(
                PostUpdate,
                (
                    attach_focusable_to_modal_buttons,
                    auto_focus_on_modal_open,
                )
                    .chain(),
            )
            .add_systems(
                Update,
                (
                    sync_focus_on_mouse_click,
                    clear_hud_focus_on_unhover,
                    handle_focus_keys,
                    update_focus_overlay,
                    pulse_focus_overlay,
                )
                    .chain(),
            );
    }
}

/// Computes the focus-ring breathing factor for a given elapsed time.
///
/// Returns a value in `[0.65, 1.0]` following a sin curve over
/// [`MOTION_FOCUS_PULSE_SECS`]. Multiply [`FOCUS_RING`]'s native alpha by
/// this factor each frame to produce the breathing effect.
///
/// Pure helper so the curve can be unit-tested without a Bevy app.
pub fn focus_ring_pulse_factor(elapsed_secs: f32) -> f32 {
    let phase = (elapsed_secs * TAU / MOTION_FOCUS_PULSE_SECS).sin();
    // 0.825 mid-point ± 0.175 amplitude → range [0.65, 1.0]. Multiplicative
    // factor against FOCUS_RING's static alpha so the brightest tick is
    // exactly the original colour, not a brighter one.
    0.825 + 0.175 * phase
}

/// Modulates the focus overlay's border alpha with a slow sin-curve
/// breathing pulse so the indicator catches the eye without competing
/// with gameplay motion. Skipped under `AnimSpeed::Instant` — the static
/// border colour is restored so reduced-motion users see no animation.
fn pulse_focus_overlay(
    time: Res<Time>,
    settings: Option<Res<SettingsResource>>,
    focused: Res<FocusedButton>,
    mut overlay: Query<&mut BorderColor, With<FocusOverlay>>,
) {
    let Ok(mut border) = overlay.single_mut() else {
        return;
    };

    let instant = settings
        .as_deref()
        .is_some_and(|s| matches!(s.0.animation_speed, AnimSpeed::Instant));

    let factor = if instant || focused.0.is_none() {
        1.0
    } else {
        focus_ring_pulse_factor(time.elapsed_secs())
    };

    let mut colour = FOCUS_RING;
    colour.set_alpha(FOCUS_RING.alpha() * factor);
    *border = BorderColor::all(colour);
}

// ---------------------------------------------------------------------------
// Private marker for the single overlay entity
// ---------------------------------------------------------------------------

/// Marker on the singleton overlay entity. Spawned once at startup;
/// repositioned every frame to track the focused button.
#[derive(Component, Debug)]
struct FocusOverlay;

/// Padding (logical px) added around the focused button's bounding box.
/// 4 px on every edge — enough to clear the button's own border without
/// crowding adjacent content.
const FOCUS_OVERLAY_PADDING: f32 = 4.0;

/// Width of the focus ring border in logical pixels.
const FOCUS_OVERLAY_BORDER: f32 = 2.0;

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// Spawns the singleton focus-overlay entity. Hidden until a button is
/// focused; its `Node` is repositioned each frame by
/// [`update_focus_overlay`].
fn spawn_focus_overlay(mut commands: Commands) {
    commands.spawn((
        FocusOverlay,
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(0.0),
            top: Val::Px(0.0),
            width: Val::Px(0.0),
            height: Val::Px(0.0),
            border: UiRect::all(Val::Px(FOCUS_OVERLAY_BORDER)),
            border_radius: BorderRadius::all(Val::Px(RADIUS_MD)),
            ..default()
        },
        // No background — only the border is visible, so the focused
        // button itself stays fully readable underneath.
        BackgroundColor(Color::NONE),
        BorderColor::all(FOCUS_RING),
        Visibility::Hidden,
        // Pin above every modal layer so the ring is never occluded by
        // a card's hover / press recolour.
        GlobalZIndex(Z_FOCUS_RING),
    ));
}

/// Attaches a [`Focusable`] to any [`ModalButton`] that doesn't yet
/// carry one. This is the **zero-churn path**: existing modal plugins
/// don't need any code changes — they get keyboard focus for free as
/// soon as `UiFocusPlugin` is registered. Walks ancestors via
/// [`ChildOf`] to find the [`ModalScrim`] that owns the button so the
/// new [`Focusable`]'s group is bound to that specific scrim.
fn attach_focusable_to_modal_buttons(
    mut commands: Commands,
    new_buttons: Query<Entity, (With<ModalButton>, Without<Focusable>)>,
    parents: Query<&ChildOf>,
    scrims: Query<(), With<ModalScrim>>,
) {
    for button in &new_buttons {
        // Walk ancestors until we find the ModalScrim. Bounded loop so
        // a malformed hierarchy can't hang the system.
        let mut current = button;
        let mut scrim_entity: Option<Entity> = None;
        for _ in 0..32 {
            if scrims.get(current).is_ok() {
                scrim_entity = Some(current);
                break;
            }
            match parents.get(current) {
                Ok(parent) => current = parent.parent(),
                Err(_) => break,
            }
        }
        if let Some(scrim) = scrim_entity {
            commands.entity(button).insert(Focusable {
                group: FocusGroup::Modal(scrim),
                order: 0,
            });
        }
    }
}

/// Auto-focuses a modal's Primary button (or the first focusable in
/// spawn order if no Primary exists) the frame it appears. Triggered
/// by `Added<ModalScrim>` so it fires exactly once per modal.
///
/// `Added<ModalScrim>` is preferred over `Added<ModalEntering>` because
/// the entering animation may already have been removed on the same
/// tick under `AnimSpeed::Instant`; the scrim itself lives for the
/// modal's full lifetime.
fn auto_focus_on_modal_open(
    new_scrims: Query<Entity, Added<ModalScrim>>,
    children_q: Query<&Children>,
    buttons: Query<(&ModalButton, Has<Disabled>), With<Focusable>>,
    mut focused: ResMut<FocusedButton>,
) {
    for scrim in &new_scrims {
        let mut primary: Option<Entity> = None;
        let mut first: Option<Entity> = None;

        // Recursive descendants walk — collected via a small stack so
        // we don't need a depth-bounded recursive function.
        let mut stack: Vec<Entity> = vec![scrim];
        while let Some(entity) = stack.pop() {
            if let Ok((button, disabled)) = buttons.get(entity)
                && !disabled
            {
                if first.is_none() {
                    first = Some(entity);
                }
                if button.0 == ButtonVariant::Primary && primary.is_none() {
                    primary = Some(entity);
                }
            }
            if let Ok(children) = children_q.get(entity) {
                for child in children.iter() {
                    stack.push(child);
                }
            }
        }

        if let Some(target) = primary.or(first) {
            focused.0 = Some(target);
        }
    }
}

/// Mouse click on a focusable transfers keyboard focus to it. The
/// existing click handler still fires; this just keeps the keyboard
/// ring in sync so a Tab afterwards advances from the clicked button.
#[allow(clippy::type_complexity)]
fn sync_focus_on_mouse_click(
    interactions: Query<
        (Entity, &Interaction, Has<Disabled>),
        (Changed<Interaction>, With<Focusable>),
    >,
    mut focused: ResMut<FocusedButton>,
) {
    for (entity, interaction, disabled) in &interactions {
        if disabled {
            continue;
        }
        if matches!(interaction, Interaction::Pressed) {
            focused.0 = Some(entity);
        }
    }
}

/// Clears [`FocusedButton`] when the focused entity is a Hud-grouped
/// button and the mouse has moved off the entire HUD bar (no Hud
/// `Focusable` is currently `Interaction::Hovered`). Without this, the
/// focus ring would persist around a HUD button after the cursor
/// leaves — visually confusing because the player has nothing to
/// activate at that point.
///
/// Modal focus is unaffected: a focused modal button stays focused
/// while the modal is open, regardless of mouse position.
fn clear_hud_focus_on_unhover(
    mut focused: ResMut<FocusedButton>,
    focusables: Query<&Focusable>,
    hud_interactions: Query<(&Interaction, &Focusable), Without<Disabled>>,
) {
    let Some(target) = focused.0 else {
        return;
    };
    // Only act when the current focus is a Hud focusable. Modal focus
    // is sticky.
    let Ok(target_focusable) = focusables.get(target) else {
        return;
    };
    if target_focusable.group != FocusGroup::Hud {
        return;
    }
    let any_hud_hovered = hud_interactions.iter().any(|(interaction, focusable)| {
        matches!(interaction, Interaction::Hovered) && focusable.group == FocusGroup::Hud
    });
    if !any_hud_hovered {
        focused.0 = None;
    }
}

/// Handles Tab / Shift+Tab / Enter / Space when a focus group is
/// active. Two activation paths exist:
///
/// 1. **Modal** — if any [`ModalScrim`] entity exists, the topmost
///    scrim's group becomes active. Tab cycles only buttons inside that
///    scrim's hierarchy (matches Phase 1).
/// 2. **Hud** — else, if at least one `Focusable { group: Hud }`
///    entity is currently `Interaction::Hovered`, the HUD bar engages.
///    Tab cycles through every Hud-grouped focusable, sorted by
///    `(order, spawn_index)`.
///
/// When neither path is active this system is a no-op — card-selection
/// Tab in [`crate::selection_plugin`] keeps working exactly as before.
///
/// Consumed keys are cleared from `ButtonInput<KeyCode>` so the
/// selection plugin doesn't double-handle them.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn handle_focus_keys(
    mut keys: ResMut<ButtonInput<KeyCode>>,
    scrims: Query<Entity, With<ModalScrim>>,
    children_q: Query<&Children>,
    parents_q: Query<&ChildOf>,
    rows: Query<(), With<FocusRow>>,
    focusables: Query<(&Focusable, Has<Disabled>)>,
    hud_interactions: Query<(Entity, &Interaction, &Focusable), Without<Disabled>>,
    mut focused: ResMut<FocusedButton>,
    mut writes: Commands,
) {
    // Arrow-key navigation inside a `FocusRow` is a separate, scoped
    // path that must run before the Tab / activation logic so a focused
    // swatch responds to Left / Right without falling through to the
    // group cycle. Only acts when the currently-focused entity's direct
    // parent carries `FocusRow`; otherwise the keys are a no-op
    // (explicit semantics — we don't want Left/Right doubling as Tab).
    let arrow_left = keys.just_pressed(KeyCode::ArrowLeft);
    let arrow_right = keys.just_pressed(KeyCode::ArrowRight);
    if (arrow_left || arrow_right)
        && let Some(target) = focused.0
        && let Ok(parent) = parents_q.get(target)
        && rows.get(parent.parent()).is_ok()
        && let Ok(siblings) = children_q.get(parent.parent())
    {
        // Build the row's enabled-focusable cycle in Children order so
        // it matches the visual left → right layout.
        let row_cycle: Vec<Entity> = siblings
            .iter()
            .filter(|e| {
                focusables
                    .get(*e)
                    .is_ok_and(|(_, disabled)| !disabled)
            })
            .collect();
        if !row_cycle.is_empty()
            && let Some(idx) = row_cycle.iter().position(|e| *e == target)
        {
            let n = row_cycle.len();
            let next = if arrow_right {
                (idx + 1) % n
            } else {
                (idx + n - 1) % n
            };
            focused.0 = Some(row_cycle[next]);
        }
        // Always consume the arrow key when we engage — even if the
        // cycle was empty — so downstream systems don't double-handle.
        if arrow_left {
            keys.clear_just_pressed(KeyCode::ArrowLeft);
        }
        if arrow_right {
            keys.clear_just_pressed(KeyCode::ArrowRight);
        }
        return;
    }

    // Resolve the active focus group:
    //   1. Any modal open  ⇒ Modal(topmost scrim)
    //   2. Any Hud-grouped focusable hovered ⇒ Hud
    //   3. Otherwise ⇒ no-op
    let active_group: FocusGroup = if let Some(active_scrim) = scrims.iter().max_by_key(|e| e.index()) {
        // Pick the topmost modal as the active group. With multiple
        // modals stacked (Pause + Forfeit confirm) the most-recently-
        // spawned scrim has the highest entity index — same heuristic
        // Phase 1 used.
        FocusGroup::Modal(active_scrim)
    } else if hud_interactions.iter().any(|(_, interaction, focusable)| {
        matches!(interaction, Interaction::Hovered) && focusable.group == FocusGroup::Hud
    }) {
        FocusGroup::Hud
    } else {
        return;
    };

    let tab_pressed = keys.just_pressed(KeyCode::Tab);
    let activate_pressed =
        keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::Space);

    if !tab_pressed && !activate_pressed {
        return;
    }

    // Build the cycle list for the active group.
    let mut group: Vec<Entity> = match active_group {
        FocusGroup::Modal(scrim) => {
            // Walk the scrim's hierarchy in `Children` order so the
            // cycle matches the visual document order (left → right
            // inside `spawn_modal_actions`). Using `Children`
            // traversal — not entity index — sidesteps the fact that
            // ECS entity indices don't track spawn order under
            // deferred command application.
            let mut found: Vec<Entity> = Vec::new();
            let mut stack: Vec<Entity> = vec![scrim];
            while let Some(entity) = stack.pop() {
                if let Ok(children) = children_q.get(entity) {
                    // Push in reverse so the first child is popped
                    // first — gives us a depth-first walk in Children
                    // order.
                    for child in children.iter().collect::<Vec<_>>().into_iter().rev() {
                        stack.push(child);
                    }
                }
                if let Ok((focusable, disabled)) = focusables.get(entity)
                    && !disabled
                    && focusable.group == active_group
                {
                    found.push(entity);
                }
            }
            found
        }
        FocusGroup::Hud => {
            // The HUD action bar isn't a single subtree we can walk —
            // each button is spawned independently — so collect every
            // Hud-grouped, non-disabled focusable directly.
            // `hud_interactions` already filters out `Disabled` and
            // exposes the entity id we need.
            let mut found: Vec<Entity> = hud_interactions
                .iter()
                .filter_map(|(entity, _interaction, focusable)| {
                    (focusable.group == FocusGroup::Hud).then_some(entity)
                })
                .collect();
            // Tiebreak by entity index so a deterministic spawn-order
            // sort falls out of the secondary key.
            found.sort_by_key(|e| e.index());
            found
        }
    };
    // Stable sort by `Focusable::order` so explicit priorities (e.g.
    // HUD spawn-order: 0..5) drive the cycle. The pre-sort by entity
    // index above is the tiebreaker for entries sharing an `order`.
    group.sort_by_key(|e| focusables.get(*e).map_or(i32::MAX, |(f, _)| f.order));

    if group.is_empty() {
        // Still consume the key so the card-selection plugin doesn't
        // treat Tab as a pile cycle while a (button-less) modal is
        // open. Without this guard, opening an information-only modal
        // would accidentally let Tab navigate the table behind it.
        if tab_pressed {
            keys.clear_just_pressed(KeyCode::Tab);
        }
        if activate_pressed {
            keys.clear_just_pressed(KeyCode::Enter);
            keys.clear_just_pressed(KeyCode::Space);
        }
        return;
    }

    if tab_pressed {
        let backwards = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
        let current_idx = focused.0.and_then(|e| group.iter().position(|g| *g == e));
        let n = group.len();
        let next_idx = match current_idx {
            Some(i) if backwards => (i + n - 1) % n,
            Some(i) => (i + 1) % n,
            None if backwards => n - 1,
            None => 0,
        };
        focused.0 = Some(group[next_idx]);
        keys.clear_just_pressed(KeyCode::Tab);
    }

    if activate_pressed {
        if let Some(target) = focused.0
            && focusables.get(target).is_ok()
        {
            // Write `Interaction::Pressed` so the existing click
            // handlers (`Changed<Interaction>` queries on
            // `Interaction::Pressed`) fire on the next system run.
            writes.entity(target).insert(Interaction::Pressed);
        }
        keys.clear_just_pressed(KeyCode::Enter);
        keys.clear_just_pressed(KeyCode::Space);
    }
}

/// Repositions the focus-overlay entity each frame to wrap the
/// focused button. Reads the focused button's `UiGlobalTransform`
/// (window-space center, physical pixels) and `ComputedNode.size`
/// (physical pixels), converts to logical pixels via
/// `inverse_scale_factor`, and writes the result into the overlay's
/// absolute `Node` position + size.
///
/// If the focused entity has been despawned (e.g. its modal closed)
/// or no button is focused at all, the overlay is hidden and
/// `FocusedButton` is cleared — keeps the resource self-healing
/// without needing a `RemovedComponents` hook.
fn update_focus_overlay(
    mut focused: ResMut<FocusedButton>,
    targets: Query<(&UiGlobalTransform, &ComputedNode), With<Focusable>>,
    mut overlay: Query<(&mut Node, &mut Visibility), With<FocusOverlay>>,
) {
    let Ok((mut node, mut visibility)) = overlay.single_mut() else {
        // Overlay entity not yet spawned (first frame before Startup
        // ran, or running under a test harness that didn't include
        // `Startup`). Nothing to do.
        return;
    };

    let Some(target) = focused.0 else {
        if !matches!(*visibility, Visibility::Hidden) {
            *visibility = Visibility::Hidden;
        }
        return;
    };

    let Ok((transform, computed)) = targets.get(target) else {
        // Focused entity disappeared (e.g. modal despawned). Clear
        // the resource and hide the overlay so the next modal open
        // gets a clean slate.
        focused.0 = None;
        if !matches!(*visibility, Visibility::Hidden) {
            *visibility = Visibility::Hidden;
        }
        return;
    };

    // ComputedNode.size is in physical pixels; inverse_scale_factor
    // multiplies physical → logical. The overlay's Val::Px values are
    // logical pixels (matching every other Node in the engine), so we
    // convert before writing.
    let inv = computed.inverse_scale_factor;
    let size_logical = computed.size() * inv;
    let center_logical = transform.translation * inv;

    let half = size_logical * 0.5;
    let pad = FOCUS_OVERLAY_PADDING;

    let left = center_logical.x - half.x - pad;
    let top = center_logical.y - half.y - pad;
    let width = size_logical.x + pad * 2.0;
    let height = size_logical.y + pad * 2.0;

    node.left = Val::Px(left);
    node.top = Val::Px(top);
    node.width = Val::Px(width);
    node.height = Val::Px(height);

    if !matches!(*visibility, Visibility::Visible) {
        *visibility = Visibility::Visible;
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui_modal::{
        spawn_modal, spawn_modal_actions, spawn_modal_button, ButtonVariant, UiModalPlugin,
    };

    #[test]
    fn focus_ring_pulse_factor_at_zero_is_mid_point() {
        // sin(0) = 0 → factor = 0.825 (mid of [0.65, 1.0]).
        let f = focus_ring_pulse_factor(0.0);
        assert!((f - 0.825).abs() < 1e-5, "factor at t=0 should be 0.825, got {f}");
    }

    #[test]
    fn focus_ring_pulse_factor_peaks_at_quarter_period() {
        // sin(τ/4) = 1 → factor = 1.0.
        let f = focus_ring_pulse_factor(MOTION_FOCUS_PULSE_SECS / 4.0);
        assert!((f - 1.0).abs() < 1e-4, "factor at peak should be 1.0, got {f}");
    }

    #[test]
    fn focus_ring_pulse_factor_troughs_at_three_quarter_period() {
        // sin(3τ/4) = -1 → factor = 0.65.
        let f = focus_ring_pulse_factor(MOTION_FOCUS_PULSE_SECS * 3.0 / 4.0);
        assert!((f - 0.65).abs() < 1e-4, "factor at trough should be 0.65, got {f}");
    }

    #[test]
    fn focus_ring_pulse_factor_stays_in_brightness_range() {
        // Sweep across two full periods; factor must stay within [0.65, 1.0].
        for i in 0..200 {
            let t = i as f32 * MOTION_FOCUS_PULSE_SECS * 0.01;
            let f = focus_ring_pulse_factor(t);
            assert!(
                (0.649..=1.001).contains(&f),
                "factor at t={t} out of range: {f}"
            );
        }
    }

    /// Plugin-marker for the synthetic test modal — `spawn_modal`
    /// requires a `Component` on the scrim.
    #[derive(Component, Debug)]
    struct TestModal;

    /// Marker on each test button so per-button assertions can target
    /// the right entity.
    #[derive(Component, Debug)]
    struct TestButtonA;
    #[derive(Component, Debug)]
    struct TestButtonB;
    #[derive(Component, Debug)]
    struct TestButtonC;

    fn headless_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(UiModalPlugin)
            .add_plugins(UiFocusPlugin);
        app.init_resource::<ButtonInput<KeyCode>>();
        // Run Startup so `spawn_focus_overlay` has executed before the
        // first asserting `update`.
        app.update();
        app
    }

    /// Spawns a 2-button modal (Primary "A" + Secondary "B"). Returns
    /// (scrim, button_a, button_b). Buttons are looked up after the
    /// `attach_focusable_to_modal_buttons` system has run by querying
    /// the marker components.
    fn spawn_two_button_modal(app: &mut App) -> (Entity, Entity, Entity) {
        let scrim = {
            let world = app.world_mut();
            let mut commands = world.commands();
            let id = spawn_modal(&mut commands, TestModal, 0, |card| {
                spawn_modal_actions(card, |actions| {
                    spawn_modal_button(
                        actions,
                        TestButtonB,
                        "B",
                        None,
                        ButtonVariant::Secondary,
                        None,
                    );
                    spawn_modal_button(
                        actions,
                        TestButtonA,
                        "A",
                        None,
                        ButtonVariant::Primary,
                        None,
                    );
                });
            });
            world.flush();
            id
        };
        // Run one tick so `attach_focusable_to_modal_buttons` and
        // `auto_focus_on_modal_open` execute.
        app.update();

        let mut a_query = app.world_mut().query_filtered::<Entity, With<TestButtonA>>();
        let a = a_query
            .iter(app.world())
            .next()
            .expect("button A should have been spawned");
        let mut b_query = app.world_mut().query_filtered::<Entity, With<TestButtonB>>();
        let b = b_query
            .iter(app.world())
            .next()
            .expect("button B should have been spawned");
        (scrim, a, b)
    }

    /// Spawns a 3-button modal (A primary, B secondary, C tertiary, in
    /// that spawn order) so Tab cycle order can be observed.
    fn spawn_three_button_modal(app: &mut App) -> (Entity, Entity, Entity, Entity) {
        let scrim = {
            let world = app.world_mut();
            let mut commands = world.commands();
            let id = spawn_modal(&mut commands, TestModal, 0, |card| {
                spawn_modal_actions(card, |actions| {
                    spawn_modal_button(
                        actions,
                        TestButtonA,
                        "A",
                        None,
                        ButtonVariant::Primary,
                        None,
                    );
                    spawn_modal_button(
                        actions,
                        TestButtonB,
                        "B",
                        None,
                        ButtonVariant::Secondary,
                        None,
                    );
                    spawn_modal_button(
                        actions,
                        TestButtonC,
                        "C",
                        None,
                        ButtonVariant::Tertiary,
                        None,
                    );
                });
            });
            world.flush();
            id
        };
        app.update();

        let mut q_a = app.world_mut().query_filtered::<Entity, With<TestButtonA>>();
        let a = q_a.iter(app.world()).next().expect("A spawned");
        let mut q_b = app.world_mut().query_filtered::<Entity, With<TestButtonB>>();
        let b = q_b.iter(app.world()).next().expect("B spawned");
        let mut q_c = app.world_mut().query_filtered::<Entity, With<TestButtonC>>();
        let c = q_c.iter(app.world()).next().expect("C spawned");
        (scrim, a, b, c)
    }

    /// Drives a fresh `just_pressed` event for `key`.
    ///
    /// `ButtonInput::press` is a no-op for `just_pressed` if the key is
    /// already in `pressed` — and `MinimalPlugins` doesn't run the
    /// frame tick that drains `pressed`, so a second call would be
    /// silent. Releasing first, clearing the just-pressed/released
    /// flags, then pressing reproduces a real keystroke per frame.
    fn press_key(app: &mut App, key: KeyCode) {
        let mut input = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        input.release_all();
        input.clear();
        input.press(key);
    }

    /// Same as [`press_key`] but for chord-style multi-key presses
    /// (e.g. Shift+Tab). All keys land as `just_pressed` on the next
    /// system tick.
    fn press_keys(app: &mut App, keys: &[KeyCode]) {
        let mut input = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        input.release_all();
        input.clear();
        for k in keys {
            input.press(*k);
        }
    }

    #[test]
    fn auto_focus_picks_primary_on_modal_open() {
        let mut app = headless_app();
        let (_scrim, a, _b) = spawn_two_button_modal(&mut app);
        let focused = app.world().resource::<FocusedButton>().0;
        assert_eq!(focused, Some(a), "Primary button A should auto-focus");
    }

    /// One-shot trigger resource consumed by the production-shaped test
    /// system [`spawn_modal_via_system`]. When set to `true`, the system
    /// queues a `spawn_modal` call on the next `Update` and clears the
    /// flag. Mirrors the real production flow where a click-handler
    /// system queues the modal spawn via `Commands` rather than the
    /// test fixture using `world.flush()` ahead of time.
    #[derive(Resource, Default)]
    struct SpawnModalTrigger(bool);

    /// Production-shaped modal spawner: a regular Bevy `System` that
    /// reads a trigger flag and queues a 2-button modal via `Commands`.
    /// Crucially this system has **no** ordering relationship with
    /// `UiFocusPlugin`'s chain — exactly the situation that surfaces the
    /// "focus arrives one frame late" bug in production.
    fn spawn_modal_via_system(
        mut commands: Commands,
        mut trigger: ResMut<SpawnModalTrigger>,
    ) {
        if !trigger.0 {
            return;
        }
        trigger.0 = false;
        spawn_modal(&mut commands, TestModal, 0, |card| {
            spawn_modal_actions(card, |actions| {
                spawn_modal_button(
                    actions,
                    TestButtonB,
                    "B",
                    None,
                    ButtonVariant::Secondary,
                    None,
                );
                spawn_modal_button(
                    actions,
                    TestButtonA,
                    "A",
                    None,
                    ButtonVariant::Primary,
                    None,
                );
            });
        });
    }

    /// Same-frame-focus contract: when a modal is spawned by an
    /// independent system during the same `Update` as the focus chain,
    /// `FocusedButton` must be populated with the primary button by the
    /// time `handle_focus_keys` runs in that **same** update — so a Tab
    /// pressed in the very next tick advances focus rather than
    /// landing on "nothing focused → primary".
    #[test]
    fn primary_button_is_focused_on_modal_spawn_same_frame() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(UiModalPlugin)
            .add_plugins(UiFocusPlugin);
        app.init_resource::<ButtonInput<KeyCode>>();
        app.init_resource::<SpawnModalTrigger>();
        // Register the production-shaped spawn system in `Update` with
        // no chain relationship to `UiFocusPlugin`.
        app.add_systems(Update, spawn_modal_via_system);
        // Initial Startup pass.
        app.update();

        // Trigger the spawn and run exactly ONE update — the same
        // `Update` cycle that the focus chain runs in. By the end of
        // this update, `FocusedButton` must already point at the
        // primary button.
        app.world_mut().resource_mut::<SpawnModalTrigger>().0 = true;
        app.update();

        let primary = app
            .world_mut()
            .query_filtered::<Entity, With<TestButtonA>>()
            .iter(app.world())
            .next()
            .expect("Primary button should exist after the spawn update");

        assert_eq!(
            app.world().resource::<FocusedButton>().0,
            Some(primary),
            "FocusedButton must be populated with the primary on the same frame the modal spawns"
        );
    }

    /// Tab pressed on the very next tick after a modal opens must
    /// advance focus from the primary to the secondary — not from
    /// "nothing focused" to the primary. The latter would mean focus
    /// arrived a frame late and Tab was wasted on first-focus instead
    /// of advancing.
    #[test]
    fn first_tab_after_modal_open_advances_to_secondary() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(UiModalPlugin)
            .add_plugins(UiFocusPlugin);
        app.init_resource::<ButtonInput<KeyCode>>();
        app.init_resource::<SpawnModalTrigger>();
        app.add_systems(Update, spawn_modal_via_system);
        app.update();

        // Spawn the modal in update N.
        app.world_mut().resource_mut::<SpawnModalTrigger>().0 = true;
        app.update();

        // Press Tab on update N+1. If focus arrived correctly in N,
        // Tab advances primary → secondary. If focus arrived late,
        // Tab promotes "no focus" to primary (the bug).
        let primary = app
            .world_mut()
            .query_filtered::<Entity, With<TestButtonA>>()
            .iter(app.world())
            .next()
            .expect("primary spawned");
        let secondary = app
            .world_mut()
            .query_filtered::<Entity, With<TestButtonB>>()
            .iter(app.world())
            .next()
            .expect("secondary spawned");

        press_key(&mut app, KeyCode::Tab);
        app.update();

        let focused_after_tab = app.world().resource::<FocusedButton>().0;
        assert_ne!(
            focused_after_tab,
            Some(primary),
            "first Tab after modal open should advance off the primary, not land on it (focus arrived late)"
        );
        assert_eq!(
            focused_after_tab,
            Some(secondary),
            "first Tab from primary should land on the secondary"
        );
    }

    #[test]
    fn tab_advances_focus_in_spawn_order() {
        let mut app = headless_app();
        let (_scrim, a, b, c) = spawn_three_button_modal(&mut app);

        // Auto-focused on A (primary).
        assert_eq!(app.world().resource::<FocusedButton>().0, Some(a));

        // Tab → B (next in spawn order after A).
        press_key(&mut app, KeyCode::Tab);
        app.update();
        assert_eq!(app.world().resource::<FocusedButton>().0, Some(b));

        // Tab → C.
        press_key(&mut app, KeyCode::Tab);
        app.update();
        assert_eq!(app.world().resource::<FocusedButton>().0, Some(c));

        // Tab wraps back to A.
        press_key(&mut app, KeyCode::Tab);
        app.update();
        assert_eq!(app.world().resource::<FocusedButton>().0, Some(a));
    }

    #[test]
    fn shift_tab_reverses() {
        let mut app = headless_app();
        let (_scrim, a, b, c) = spawn_three_button_modal(&mut app);
        assert_eq!(app.world().resource::<FocusedButton>().0, Some(a));

        // Shift+Tab from A wraps backward to C.
        press_keys(&mut app, &[KeyCode::ShiftLeft, KeyCode::Tab]);
        app.update();
        assert_eq!(app.world().resource::<FocusedButton>().0, Some(c));

        // Shift+Tab from C → B.
        press_keys(&mut app, &[KeyCode::ShiftLeft, KeyCode::Tab]);
        app.update();
        assert_eq!(app.world().resource::<FocusedButton>().0, Some(b));
    }

    #[test]
    fn enter_activates_focused_button() {
        let mut app = headless_app();
        let (_scrim, a, _b) = spawn_two_button_modal(&mut app);
        assert_eq!(app.world().resource::<FocusedButton>().0, Some(a));

        // Confirm the focused button is NOT pressed before Enter — the
        // baseline state matters because we're going to assert the
        // post-Enter component value, not a count delta.
        let pre = app.world().entity(a).get::<Interaction>().copied();
        assert_ne!(
            pre,
            Some(Interaction::Pressed),
            "focused button should not be pressed before Enter"
        );

        press_key(&mut app, KeyCode::Enter);
        app.update();

        // After Enter, `handle_focus_keys` inserts `Interaction::Pressed`
        // on the focused button so existing click handlers
        // (`Changed<Interaction>` queries matching `Pressed`) fire on
        // the next system tick — exactly the same signal a real mouse
        // click produces.
        let post = app
            .world()
            .entity(a)
            .get::<Interaction>()
            .copied()
            .expect("focused button should carry an Interaction after activation");
        assert_eq!(
            post,
            Interaction::Pressed,
            "Enter on focused button A should leave its Interaction at Pressed"
        );
    }

    #[test]
    fn focus_clears_when_modal_despawns() {
        let mut app = headless_app();
        let (scrim, a, _b) = spawn_two_button_modal(&mut app);
        assert_eq!(app.world().resource::<FocusedButton>().0, Some(a));

        // Despawn the scrim — Bevy's hierarchy cascade despawns the
        // card and every button under it.
        app.world_mut().entity_mut(scrim).despawn();
        app.update();

        assert!(
            app.world().resource::<FocusedButton>().0.is_none(),
            "FocusedButton should clear once the focused entity is despawned"
        );
    }

    #[test]
    fn focus_overlay_visible_when_focus_set() {
        let mut app = headless_app();
        let (_scrim, _a, _b) = spawn_two_button_modal(&mut app);
        // One more update so `update_focus_overlay` runs *after* the
        // auto-focus side-effect and writes a non-Hidden Visibility.
        app.update();

        let mut q = app
            .world_mut()
            .query_filtered::<&Visibility, With<FocusOverlay>>();
        let v = q
            .iter(app.world())
            .next()
            .expect("FocusOverlay singleton should exist");
        assert!(
            matches!(v, Visibility::Visible),
            "overlay should be visible while a button has focus, got {v:?}"
        );
    }

    #[test]
    fn mouse_click_transfers_focus() {
        let mut app = headless_app();
        let (_scrim, a, b) = spawn_two_button_modal(&mut app);
        assert_eq!(app.world().resource::<FocusedButton>().0, Some(a));

        // Simulate a real click landing on B by directly inserting
        // `Interaction::Pressed` — the same write `bevy_ui::focus`
        // would emit on a real mouse press.
        app.world_mut().entity_mut(b).insert(Interaction::Pressed);
        app.update();

        assert_eq!(
            app.world().resource::<FocusedButton>().0,
            Some(b),
            "mouse-pressed focusable should take focus"
        );
    }

    /// Tab is consumed by `handle_focus_keys` while a modal is open,
    /// so a downstream system reading `ButtonInput<KeyCode>` (e.g.
    /// `selection_plugin::handle_selection_keys`) sees no Tab press.
    /// Verifies the simpler invariant from the brief: the key is no
    /// longer `just_pressed` after the focus system runs.
    #[test]
    fn selection_plugin_does_not_see_tab_when_modal_is_open() {
        let mut app = headless_app();
        let (_scrim, _a, _b) = spawn_two_button_modal(&mut app);

        press_key(&mut app, KeyCode::Tab);
        app.update();

        let keys = app.world().resource::<ButtonInput<KeyCode>>();
        assert!(
            !keys.just_pressed(KeyCode::Tab),
            "handle_focus_keys must clear Tab so selection_plugin can't double-handle it"
        );
    }

    // -----------------------------------------------------------------------
    // Phase 2 — HUD-on-hover focus path
    // -----------------------------------------------------------------------

    /// Spawns three synthetic Hud-tagged focusable buttons (orders
    /// 0, 1, 2) without involving the real HUD bar — keeps the test
    /// independent of `HudPlugin`'s layout. Every button gets a
    /// `Button` widget (so `Interaction` is present) and `Node` so the
    /// query in `handle_focus_keys` matches.
    fn spawn_three_hud_buttons(app: &mut App) -> (Entity, Entity, Entity) {
        let world = app.world_mut();
        let a = world
            .spawn((
                Button,
                Node::default(),
                Interaction::default(),
                Focusable {
                    group: FocusGroup::Hud,
                    order: 0,
                },
                TestButtonA,
            ))
            .id();
        let b = world
            .spawn((
                Button,
                Node::default(),
                Interaction::default(),
                Focusable {
                    group: FocusGroup::Hud,
                    order: 1,
                },
                TestButtonB,
            ))
            .id();
        let c = world
            .spawn((
                Button,
                Node::default(),
                Interaction::default(),
                Focusable {
                    group: FocusGroup::Hud,
                    order: 2,
                },
                TestButtonC,
            ))
            .id();
        app.update();
        (a, b, c)
    }

    #[test]
    fn hud_tab_engages_only_when_a_hud_button_is_hovered() {
        let mut app = headless_app();
        let (a, _b, _c) = spawn_three_hud_buttons(&mut app);

        // No hover, no modal ⇒ Tab is a no-op. (Phase 1 contract still
        // holds when nothing is hovered.)
        press_key(&mut app, KeyCode::Tab);
        app.update();
        assert!(
            app.world().resource::<FocusedButton>().0.is_none(),
            "Tab without hover must not engage the HUD focus ring"
        );

        // Hover button A → Tab must engage and focus a Hud entity.
        // With no current focus, the cycle starts at index 0 (order
        // 0), which is button A.
        app.world_mut().entity_mut(a).insert(Interaction::Hovered);
        press_key(&mut app, KeyCode::Tab);
        app.update();

        assert_eq!(
            app.world().resource::<FocusedButton>().0,
            Some(a),
            "first Tab on Hud-engaged group should focus the order=0 button"
        );
    }

    #[test]
    fn hud_tab_advances_within_hud_group() {
        let mut app = headless_app();
        let (a, b, c) = spawn_three_hud_buttons(&mut app);

        // Engage by hovering A, then Tab to land on A.
        app.world_mut().entity_mut(a).insert(Interaction::Hovered);
        press_key(&mut app, KeyCode::Tab);
        app.update();
        assert_eq!(app.world().resource::<FocusedButton>().0, Some(a));

        // Subsequent Tabs cycle by `Focusable::order`.
        press_key(&mut app, KeyCode::Tab);
        app.update();
        assert_eq!(app.world().resource::<FocusedButton>().0, Some(b));

        press_key(&mut app, KeyCode::Tab);
        app.update();
        assert_eq!(app.world().resource::<FocusedButton>().0, Some(c));

        // Wrap-around back to A.
        press_key(&mut app, KeyCode::Tab);
        app.update();
        assert_eq!(app.world().resource::<FocusedButton>().0, Some(a));
    }

    #[test]
    fn hud_enter_activates_focused_hud_button() {
        let mut app = headless_app();
        let (a, _b, _c) = spawn_three_hud_buttons(&mut app);

        app.world_mut().entity_mut(a).insert(Interaction::Hovered);
        press_key(&mut app, KeyCode::Tab);
        app.update();
        assert_eq!(app.world().resource::<FocusedButton>().0, Some(a));

        // Enter while A is focused inserts `Interaction::Pressed`.
        // Note: A also still has `Interaction::Hovered` from earlier;
        // the activation system overwrites it with `Pressed`.
        press_key(&mut app, KeyCode::Enter);
        app.update();

        let post = app
            .world()
            .entity(a)
            .get::<Interaction>()
            .copied()
            .expect("focused HUD button should carry an Interaction after activation");
        assert_eq!(
            post,
            Interaction::Pressed,
            "Enter on focused HUD button A should leave its Interaction at Pressed"
        );
    }

    #[test]
    fn hud_focus_clears_when_mouse_leaves_bar() {
        let mut app = headless_app();
        let (a, b, c) = spawn_three_hud_buttons(&mut app);

        // Engage by hovering A, then Tab to focus A.
        app.world_mut().entity_mut(a).insert(Interaction::Hovered);
        press_key(&mut app, KeyCode::Tab);
        app.update();
        assert_eq!(app.world().resource::<FocusedButton>().0, Some(a));

        // Mouse leaves the bar entirely — every Hud button drops back
        // to `Interaction::None`. After the next update,
        // `clear_hud_focus_on_unhover` must clear `FocusedButton`.
        for entity in [a, b, c] {
            app.world_mut().entity_mut(entity).insert(Interaction::None);
        }
        app.update();

        assert!(
            app.world().resource::<FocusedButton>().0.is_none(),
            "FocusedButton must clear once no Hud button is hovered"
        );
    }

    // -----------------------------------------------------------------------
    // Phase 3 — FocusRow arrow-key navigation
    // -----------------------------------------------------------------------

    /// Spawns a synthetic modal scrim with a single [`FocusRow`] parent
    /// containing three focusable swatches (A, B, C) bound to the scrim.
    /// Returns `(scrim, row, a, b, c)`. No real `ModalScrim` ancestry —
    /// just a `ModalScrim` marker on the scrim entity so the active-group
    /// resolver in `handle_focus_keys` picks it up.
    fn spawn_modal_with_focus_row(app: &mut App) -> (Entity, Entity, Entity, Entity, Entity) {
        let world = app.world_mut();
        let scrim = world.spawn((ModalScrim, Node::default())).id();
        let row = world.spawn((FocusRow, Node::default())).id();
        world.entity_mut(scrim).add_child(row);

        let make_swatch = |w: &mut World, marker: fn(&mut bevy::ecs::world::EntityWorldMut)| {
            let mut e = w.spawn((
                Button,
                Node::default(),
                Interaction::default(),
                Focusable {
                    group: FocusGroup::Modal(scrim),
                    order: 0,
                },
            ));
            marker(&mut e);
            e.id()
        };
        let a = make_swatch(world, |e| {
            e.insert(TestButtonA);
        });
        let b = make_swatch(world, |e| {
            e.insert(TestButtonB);
        });
        let c = make_swatch(world, |e| {
            e.insert(TestButtonC);
        });
        for child in [a, b, c] {
            world.entity_mut(row).add_child(child);
        }
        // One tick so the focus systems observe the new hierarchy.
        app.update();
        (scrim, row, a, b, c)
    }

    #[test]
    fn arrow_right_advances_focus_within_focus_row() {
        let mut app = headless_app();
        let (_scrim, _row, a, b, _c) = spawn_modal_with_focus_row(&mut app);

        // Focus child A explicitly so we know the starting state.
        app.world_mut().resource_mut::<FocusedButton>().0 = Some(a);

        press_key(&mut app, KeyCode::ArrowRight);
        app.update();

        assert_eq!(
            app.world().resource::<FocusedButton>().0,
            Some(b),
            "ArrowRight should advance focus from A → B inside the row"
        );
    }

    #[test]
    fn arrow_left_at_first_wraps_to_last() {
        let mut app = headless_app();
        let (_scrim, _row, a, _b, c) = spawn_modal_with_focus_row(&mut app);

        app.world_mut().resource_mut::<FocusedButton>().0 = Some(a);

        press_key(&mut app, KeyCode::ArrowLeft);
        app.update();

        assert_eq!(
            app.world().resource::<FocusedButton>().0,
            Some(c),
            "ArrowLeft from the first child must wrap to the last"
        );
    }

    #[test]
    fn arrow_keys_outside_focus_row_are_noop() {
        let mut app = headless_app();
        // Modal with two buttons, but no FocusRow — the standard 2-button
        // modal fixture is exactly this shape.
        let (_scrim, a, _b) = spawn_two_button_modal(&mut app);
        // Auto-focus picked Primary (A). Arrow keys must not change it.
        assert_eq!(app.world().resource::<FocusedButton>().0, Some(a));

        press_key(&mut app, KeyCode::ArrowRight);
        app.update();

        assert_eq!(
            app.world().resource::<FocusedButton>().0,
            Some(a),
            "ArrowRight outside a FocusRow must leave focus unchanged"
        );
    }

    #[test]
    fn tab_escapes_focus_row_to_next_section() {
        // Build a synthetic modal with two FocusRows of two children
        // each — first row with order=0 children, second row with
        // order=10 — then focus the last child of row 1 and press Tab.
        // The cycle must advance into row 2 rather than wrap back inside
        // row 1.
        let mut app = headless_app();

        let (scrim, _row1_a, row1_b, _row2_a, _row2_b) = {
            let world = app.world_mut();
            let scrim = world.spawn((ModalScrim, Node::default())).id();
            let row1 = world.spawn((FocusRow, Node::default())).id();
            let row2 = world.spawn((FocusRow, Node::default())).id();
            world.entity_mut(scrim).add_child(row1);
            world.entity_mut(scrim).add_child(row2);

            let r1a = world
                .spawn((
                    Button,
                    Node::default(),
                    Interaction::default(),
                    Focusable {
                        group: FocusGroup::Modal(scrim),
                        order: 0,
                    },
                ))
                .id();
            let r1b = world
                .spawn((
                    Button,
                    Node::default(),
                    Interaction::default(),
                    Focusable {
                        group: FocusGroup::Modal(scrim),
                        order: 0,
                    },
                ))
                .id();
            let r2a = world
                .spawn((
                    Button,
                    Node::default(),
                    Interaction::default(),
                    Focusable {
                        group: FocusGroup::Modal(scrim),
                        order: 10,
                    },
                ))
                .id();
            let r2b = world
                .spawn((
                    Button,
                    Node::default(),
                    Interaction::default(),
                    Focusable {
                        group: FocusGroup::Modal(scrim),
                        order: 10,
                    },
                ))
                .id();
            world.entity_mut(row1).add_child(r1a);
            world.entity_mut(row1).add_child(r1b);
            world.entity_mut(row2).add_child(r2a);
            world.entity_mut(row2).add_child(r2b);
            (scrim, r1a, r1b, r2a, r2b)
        };
        app.update();

        // Focus the last child of row 1.
        app.world_mut().resource_mut::<FocusedButton>().0 = Some(row1_b);

        press_key(&mut app, KeyCode::Tab);
        app.update();

        // After Tab the cycle must move out of row 1 — either to a child
        // of row 2 (preferred behaviour) or, in a wrap, to the first
        // child of row 1. The test enforces the stronger contract:
        // Tab must escape the row so the next focusable is in row 2.
        let focused = app
            .world()
            .resource::<FocusedButton>()
            .0
            .expect("Tab should leave focus on some entity");
        // `focused` must NOT be `row1_b` itself (Tab clearly should advance)
        assert_ne!(focused, row1_b, "Tab must advance off the current focus");
        // And it must be a descendant of row 2's parent (i.e. a Focusable
        // with order >= 10) — our row 1 children all have order 0.
        let order = app
            .world()
            .entity(focused)
            .get::<Focusable>()
            .expect("focused entity must carry Focusable")
            .order;
        assert_eq!(
            order, 10,
            "Tab from the end of row 1 should land in row 2 (order=10), not wrap inside row 1 (order=0); landed on order={order}"
        );
        // Sanity: scrim entity isn't the focus.
        assert_ne!(focused, scrim);
    }

    #[test]
    fn disabled_swatch_skipped_by_arrow_keys() {
        let mut app = headless_app();
        let (_scrim, _row, a, b, c) = spawn_modal_with_focus_row(&mut app);

        // Disable the middle swatch.
        app.world_mut().entity_mut(b).insert(Disabled);
        // Focus the first swatch and press Right — should jump over B
        // straight to C.
        app.world_mut().resource_mut::<FocusedButton>().0 = Some(a);

        press_key(&mut app, KeyCode::ArrowRight);
        app.update();

        assert_eq!(
            app.world().resource::<FocusedButton>().0,
            Some(c),
            "ArrowRight should skip the Disabled middle swatch and land on C"
        );
    }
}
