//! Keyboard focus ring for modal buttons (Phase 1).
//!
//! Solitaire Quest's 11 modals (Help, Stats, Achievements, Settings,
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
//! Phase 1 is modal buttons only. The HUD action bar (Phase 2), Home
//! mode cards (Phase 2), and Settings bespoke buttons + arrow-key
//! handling (Phase 3) remain out of scope. When no modal is open and no
//! HUD button is hovered, every system here no-ops so
//! [`crate::selection_plugin`]'s Tab/Enter card-selection still works.

use bevy::ecs::query::Has;
use bevy::input::ButtonInput;
use bevy::prelude::*;
use bevy::ui::{ComputedNode, UiGlobalTransform};

use crate::ui_modal::{ButtonVariant, ModalButton, ModalScrim};
use crate::ui_theme::{FOCUS_RING, RADIUS_MD, Z_FOCUS_RING};

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
            .add_systems(
                Update,
                (
                    attach_focusable_to_modal_buttons,
                    auto_focus_on_modal_open,
                    sync_focus_on_mouse_click,
                    handle_focus_keys,
                    update_focus_overlay,
                )
                    .chain(),
            );
    }
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

/// Handles Tab / Shift+Tab / Enter / Space when a modal is open (the
/// only active focus group in Phase 1). Consumed keys are cleared from
/// `ButtonInput<KeyCode>` so [`crate::selection_plugin`] doesn't also
/// treat them as card-selection input.
///
/// When no modal is open this system is a no-op — card-selection Tab
/// keeps working exactly as it did before Phase 1.
fn handle_focus_keys(
    mut keys: ResMut<ButtonInput<KeyCode>>,
    scrims: Query<Entity, With<ModalScrim>>,
    children_q: Query<&Children>,
    focusables: Query<(&Focusable, Has<Disabled>)>,
    mut focused: ResMut<FocusedButton>,
    mut writes: Commands,
) {
    if scrims.iter().next().is_none() {
        // No modal open ⇒ Phase 1 stays out of the way. Phase 2 will
        // extend this with a Hud-group active path.
        return;
    }

    let tab_pressed = keys.just_pressed(KeyCode::Tab);
    let activate_pressed =
        keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::Space);

    if !tab_pressed && !activate_pressed {
        return;
    }

    // Pick the topmost modal as the active group. With multiple modals
    // stacked (Pause + Forfeit confirm) the most-recently-spawned scrim
    // has the highest entity index. Bevy entity indices grow on each
    // spawn, so this is a stable proxy for "topmost modal" in Phase 1.
    let active_scrim = scrims
        .iter()
        .max_by_key(|e| e.index())
        .expect("scrims iter was non-empty above");
    let active_group = FocusGroup::Modal(active_scrim);

    // Walk the scrim's hierarchy in `Children` order so the cycle
    // matches the visual document order (left → right inside
    // `spawn_modal_actions`). Using `Children` traversal — not entity
    // index — sidesteps the fact that ECS entity indices don't track
    // spawn order under deferred command application.
    let mut group: Vec<Entity> = Vec::new();
    let mut stack: Vec<Entity> = vec![active_scrim];
    while let Some(entity) = stack.pop() {
        if let Ok(children) = children_q.get(entity) {
            // Push in reverse so the first child is popped first —
            // gives us a depth-first walk in Children order.
            for child in children.iter().collect::<Vec<_>>().into_iter().rev() {
                stack.push(child);
            }
        }
        if let Ok((focusable, disabled)) = focusables.get(entity)
            && !disabled
            && focusable.group == active_group
        {
            group.push(entity);
        }
    }
    // Stable sort by `Focusable::order` (Phase 1 keeps every value at
    // 0 so this is effectively a no-op, but it lets future phases give
    // explicit priorities — e.g. a "primary first" override — without
    // changing the tab walk).
    group.sort_by_key(|e| {
        focusables
            .get(*e)
            .map(|(f, _)| f.order)
            .unwrap_or(i32::MAX)
    });

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
}
