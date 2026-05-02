//! Hover-tooltip infrastructure. Adds a one-shot, design-token-styled
//! popover that appears over any UI element carrying a [`Tooltip`]
//! component once the cursor has lingered for
//! [`crate::ui_theme::MOTION_TOOLTIP_DELAY_SECS`] seconds.
//!
//! ## Why a sibling overlay
//!
//! Like [`crate::ui_focus`], this module uses a single absolute-positioned
//! overlay entity that is never a descendant of any modal or HUD card. On
//! every frame, [`show_or_hide_tooltip`] reads the hovered target's
//! [`bevy::ui::UiGlobalTransform`] + [`bevy::ui::ComputedNode`] and writes
//! an absolute `Node.left` / `Node.top` so the overlay tracks the target
//! without inheriting modal scale-in or scroll-clipping. The pattern
//! mirrors [`crate::ui_focus::update_focus_overlay`] one-for-one.
//!
//! ## Public surface
//!
//! - [`Tooltip`] — component carrying the hover text. Add it to any
//!   interactive node and the rest is automatic.
//! - [`UiTooltipPlugin`] — registers the resource, startup spawn, and the
//!   per-frame tracking + display systems.
//!
//! ## Scope
//!
//! Phase 1 of the tooltip rollout — *infrastructure only*. No HUD or
//! Settings entity carries [`Tooltip`] yet; a follow-up commit applies
//! tooltips to specific readouts and buttons. Treat this module as the
//! library half of the feature.

use std::borrow::Cow;
use std::time::Duration;

use bevy::prelude::*;
use bevy::ui::{ComputedNode, UiGlobalTransform};

use crate::font_plugin::FontResource;
use crate::settings_plugin::SettingsResource;
use crate::ui_theme::{
    BG_ELEVATED_HI, BORDER_SUBTLE, MOTION_TOOLTIP_DELAY_SECS, RADIUS_SM, TEXT_PRIMARY,
    TYPE_CAPTION, VAL_SPACE_2, Z_TOOLTIP,
};

// ---------------------------------------------------------------------------
// Public component / plugin
// ---------------------------------------------------------------------------

/// Marker on a UI element that should display a tooltip when the cursor
/// hovers over it. The component carries the tooltip text — typically a
/// short caption explaining what the element does or what its number
/// represents.
///
/// Bevy UI hover detection requires the [`Interaction`] component (the
/// picking system writes `Interaction::Hovered` only on entities that
/// have it), so [`Tooltip`] declares it as a required component. Adding
/// `Tooltip` to a node automatically inserts a default [`Interaction`].
///
/// The owning entity must also be a UI [`Node`] for picking to pick it
/// up; that's a layout concern handled at the call site. Every interactive
/// HUD readout and modal button in this codebase already carries `Node`,
/// so in practice callers just attach `Tooltip::new("…")` and move on.
///
/// # Example
///
/// ```ignore
/// use solitaire_engine::ui_tooltip::Tooltip;
///
/// commands.spawn((
///     Node { /* ... */ ..default() },
///     Tooltip::new("Cards left in the stock"),
/// ));
/// ```
#[derive(Component, Debug, Clone)]
#[require(Interaction)]
pub struct Tooltip(pub Cow<'static, str>);

impl Tooltip {
    /// Builds a [`Tooltip`] from any string-like value. Prefer passing a
    /// `&'static str` for static labels — the underlying `Cow` keeps the
    /// allocation-free path open for the common case while still
    /// accepting owned `String`s for runtime-formatted text.
    pub fn new(text: impl Into<Cow<'static, str>>) -> Self {
        Self(text.into())
    }
}

/// Registers the tooltip overlay and the systems that drive it. Add this
/// plugin once, immediately after [`crate::ui_focus::UiFocusPlugin`], and
/// every entity carrying a [`Tooltip`] component gains hover-to-reveal
/// behaviour with no per-plugin wiring.
pub struct UiTooltipPlugin;

impl Plugin for UiTooltipPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TooltipState>()
            .add_systems(Startup, spawn_tooltip_overlay)
            .add_systems(
                Update,
                (track_tooltip_hover, show_or_hide_tooltip).chain(),
            );
    }
}

// ---------------------------------------------------------------------------
// Private resource + markers
// ---------------------------------------------------------------------------

/// Internal state for the singleton tooltip overlay. Tracks which
/// [`Tooltip`]-bearing entity the cursor is currently hovering and the
/// `Time::elapsed()` timestamp at which the hover started, so the display
/// system can fire only once the dwell threshold has elapsed.
#[derive(Resource, Debug, Default)]
struct TooltipState {
    /// `(target_entity, hover_started_at)` — populated by
    /// [`track_tooltip_hover`] when an entity transitions to
    /// [`Interaction::Hovered`], cleared when the cursor leaves.
    hovered: Option<(Entity, Duration)>,
    /// The singleton overlay entity, populated by
    /// [`spawn_tooltip_overlay`] on Startup. Read by
    /// [`show_or_hide_tooltip`] to skip a `single_mut` query.
    overlay: Option<Entity>,
}

/// Marker on the singleton tooltip-overlay container.
#[derive(Component, Debug)]
struct TooltipOverlay;

/// Marker on the overlay's [`Text`] child, so the display system can
/// rewrite the tooltip string without despawning the whole overlay.
#[derive(Component, Debug)]
struct TooltipText;

// ---------------------------------------------------------------------------
// Tunables
// ---------------------------------------------------------------------------

/// Vertical gap between the target and the tooltip overlay, in logical
/// pixels. Small enough to read as "attached"; big enough to clear the
/// target's own border.
const TOOLTIP_GAP_PX: f32 = 4.0;

/// Pure helper: returns `true` once `elapsed_secs` has met or exceeded
/// the player-configured `delay_secs`, so the tooltip should be revealed.
///
/// Treating "elapsed >= delay" as the show condition (rather than
/// strictly greater than) is what makes a `delay_secs == 0.0` setting
/// behave as advertised: on the very first tick after hover starts,
/// `elapsed_secs` is `0.0` and the tooltip appears immediately. With a
/// strict `>` the zero-delay case would still wait one tick.
///
/// Extracted so the comparison can be unit-tested without spinning up
/// a Bevy `App` — `Time<Virtual>` clamps each tick to 250 ms under
/// `MinimalPlugins`, which makes precise sub-second timing assertions
/// awkward.
pub(crate) fn tooltip_should_show(elapsed_secs: f32, delay_secs: f32) -> bool {
    elapsed_secs >= delay_secs
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// Spawns the singleton tooltip-overlay entity at Startup. Hidden until a
/// [`Tooltip`]-bearing target is hovered for [`MOTION_TOOLTIP_DELAY_SECS`]
/// seconds, then repositioned and revealed by [`show_or_hide_tooltip`].
fn spawn_tooltip_overlay(
    mut commands: Commands,
    mut state: ResMut<TooltipState>,
    font_res: Option<Res<FontResource>>,
) {
    let font_handle = font_res.map(|f| f.0.clone()).unwrap_or_default();
    let font = TextFont {
        font: font_handle,
        font_size: TYPE_CAPTION,
        ..default()
    };

    let overlay = commands
        .spawn((
            TooltipOverlay,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                top: Val::Px(0.0),
                padding: UiRect::axes(VAL_SPACE_2, VAL_SPACE_2),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(RADIUS_SM)),
                // Auto width/height so the overlay tracks its text content.
                ..default()
            },
            BackgroundColor(BG_ELEVATED_HI),
            BorderColor::all(BORDER_SUBTLE),
            Visibility::Hidden,
            // Pin above the focus ring so a tooltip on a focused element
            // is never occluded by the focus outline.
            GlobalZIndex(Z_TOOLTIP),
        ))
        .with_children(|root| {
            root.spawn((
                TooltipText,
                Text::new(String::new()),
                font,
                TextColor(TEXT_PRIMARY),
            ));
        })
        .id();

    state.overlay = Some(overlay);
}

/// Watches every interactive entity for `Changed<Interaction>` and
/// updates [`TooltipState::hovered`] accordingly:
///
/// * Hovering a [`Tooltip`]-bearing entity records the start time so the
///   display system can apply the dwell delay.
/// * Leaving the currently-hovered entity (transition away from
///   `Hovered`) clears the state so the overlay hides on the next tick.
///
/// Hovering a different `Tooltip` entity simply replaces the prior
/// `(entity, t0)` pair — the dwell timer restarts, matching native
/// tooltip behaviour where moving across multiple targets resets the
/// reveal delay.
fn track_tooltip_hover(
    time: Res<Time>,
    interactions: Query<
        (Entity, &Interaction, Option<&Tooltip>),
        Changed<Interaction>,
    >,
    mut state: ResMut<TooltipState>,
) {
    for (entity, interaction, tooltip) in &interactions {
        match interaction {
            Interaction::Hovered => {
                if tooltip.is_some() {
                    // Record the hover start. If the same entity is
                    // already recorded, leave the original timestamp so
                    // a re-emitted Hovered (e.g. pointer wiggle) doesn't
                    // reset the dwell timer.
                    let already = matches!(state.hovered, Some((e, _)) if e == entity);
                    if !already {
                        state.hovered = Some((entity, time.elapsed()));
                    }
                }
            }
            Interaction::Pressed | Interaction::None => {
                // Clear iff this is the entity we were tracking. Other
                // changed-interaction events on unrelated entities must
                // not blow away an in-flight hover.
                if matches!(state.hovered, Some((e, _)) if e == entity) {
                    state.hovered = None;
                }
            }
        }
    }
}

/// Per-frame display driver. Reads [`TooltipState::hovered`] and:
///
/// * If `None`, hides the overlay.
/// * If `Some((entity, t0))` and `time.elapsed() - t0 < delay`, hides the
///   overlay (still in the dwell window).
/// * If `Some((entity, t0))` and the dwell has elapsed, copies the
///   target's [`Tooltip`] string into the overlay's [`TooltipText`] child,
///   positions the overlay above the target (or below, if above would
///   clip the screen top), and reveals it.
///
/// Positioning math mirrors
/// [`crate::ui_focus::update_focus_overlay`]: `ComputedNode.size` and
/// `UiGlobalTransform.translation` are converted from physical to
/// logical pixels via `inverse_scale_factor` before being written into
/// `Val::Px` slots on the overlay's `Node`. Headless tests run under
/// `MinimalPlugins` and don't execute the layout schedule, so
/// `ComputedNode` is `Vec2::ZERO` there — the test asserts the
/// visibility-and-text invariant rather than position.
#[allow(clippy::type_complexity)]
fn show_or_hide_tooltip(
    time: Res<Time>,
    state: Res<TooltipState>,
    settings: Option<Res<SettingsResource>>,
    tooltips: Query<(&Tooltip, &UiGlobalTransform, &ComputedNode)>,
    tooltip_text_only: Query<&Tooltip>,
    mut overlay_q: Query<(&mut Node, &mut Visibility, &Children), With<TooltipOverlay>>,
    mut text_q: Query<&mut Text, With<TooltipText>>,
) {
    let Ok((mut node, mut visibility, children)) = overlay_q.single_mut() else {
        // Overlay not yet spawned — first frame before Startup ran, or a
        // test harness without Startup. Nothing to do.
        return;
    };

    // Helper: hide the overlay if not already hidden.
    let hide = |visibility: &mut Visibility| {
        if !matches!(*visibility, Visibility::Hidden) {
            *visibility = Visibility::Hidden;
        }
    };

    let Some((target, started_at)) = state.hovered else {
        hide(&mut visibility);
        return;
    };

    // Player-configurable dwell delay; falls back to the design-token
    // default when `SettingsResource` is absent (test harnesses running
    // `UiTooltipPlugin` under `MinimalPlugins` without `SettingsPlugin`).
    let delay_secs = settings
        .as_ref()
        .map(|s| s.0.tooltip_delay_secs)
        .unwrap_or(MOTION_TOOLTIP_DELAY_SECS);
    let elapsed = time.elapsed().saturating_sub(started_at);
    if !tooltip_should_show(elapsed.as_secs_f32(), delay_secs) {
        hide(&mut visibility);
        return;
    }

    // Past the dwell threshold. Pull the target's tooltip text and write
    // it into the overlay's Text child. The wider query
    // (`UiGlobalTransform + ComputedNode`) may miss in headless tests
    // where layout doesn't run; fall back to the text-only query so test
    // assertions on visibility + text content still pass even when
    // positioning data is unavailable.
    let label: Option<Cow<'static, str>> = tooltips
        .get(target)
        .ok()
        .map(|(t, _, _)| t.0.clone())
        .or_else(|| tooltip_text_only.get(target).ok().map(|t| t.0.clone()));

    let Some(text) = label else {
        // Target despawned or no longer carries Tooltip — hide and bail.
        // We don't write back to the resource here because it's `Res`,
        // not `ResMut`; `track_tooltip_hover` will clear it the next
        // frame the entity changes interaction.
        hide(&mut visibility);
        return;
    };

    // Update the visible text. Skip the write if it already matches so
    // we don't churn the change-detection flag every frame.
    for child in children.iter() {
        if let Ok(mut t) = text_q.get_mut(child)
            && t.0 != text
        {
            t.0 = text.clone().into_owned();
        }
    }

    // Compute placement. ComputedNode.size is in physical pixels;
    // inverse_scale_factor multiplies physical → logical so the result
    // matches the Val::Px logical-pixel coordinate space every other
    // Node uses.
    if let Ok((_, transform, computed)) = tooltips.get(target) {
        let inv = computed.inverse_scale_factor;
        let size_logical = computed.size() * inv;
        let center_logical = transform.translation * inv;

        // Default placement: above the target, centered horizontally.
        // Tooltip width isn't known until layout — use a small assumed
        // width via auto sizing; we centre on the target's centre and
        // let the overlay's auto Node width do the rest. For the X
        // coordinate we still need to anchor *something*: place the
        // overlay's left edge at the target's centre minus half of the
        // target's width, then rely on auto-Node sizing. That's a small
        // approximation; the follow-up phase that wires real entities
        // will measure overlay width via ComputedNode and re-centre.
        let half = size_logical * 0.5;

        let left_above = center_logical.x - half.x;
        let top_above = center_logical.y - half.y - TOOLTIP_GAP_PX;
        // If the tooltip would render above the screen top (top < 0),
        // flip below the target. We don't know overlay height yet, so
        // use the target's bottom edge plus the gap.
        let (left, top) = if top_above < 0.0 {
            (left_above, center_logical.y + half.y + TOOLTIP_GAP_PX)
        } else {
            (left_above, top_above)
        };

        node.left = Val::Px(left);
        node.top = Val::Px(top);
    }

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
    use bevy::time::TimeUpdateStrategy;

    /// Builds a headless `App` with `MinimalPlugins + UiTooltipPlugin`.
    /// Ticks once so the Startup spawn system has run and the singleton
    /// overlay exists in the world before the first asserting `update`.
    fn headless_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins).add_plugins(UiTooltipPlugin);
        app.update();
        app
    }

    /// Tells `TimePlugin` to advance the clock by `secs` on the next
    /// `app.update()`. Mirrors the helper in `ui_modal::tests` and
    /// `hud_plugin::tests`.
    fn set_manual_time_step(app: &mut App, secs: f32) {
        app.insert_resource(TimeUpdateStrategy::ManualDuration(
            Duration::from_secs_f32(secs),
        ));
    }

    /// Reads the current overlay visibility. Panics if the singleton is
    /// missing — that would indicate a bug in `spawn_tooltip_overlay`.
    fn overlay_visibility(app: &mut App) -> Visibility {
        let mut q = app
            .world_mut()
            .query_filtered::<&Visibility, With<TooltipOverlay>>();
        *q.iter(app.world())
            .next()
            .expect("TooltipOverlay singleton should exist")
    }

    /// Reads the current tooltip text content from the overlay's Text
    /// child.
    fn overlay_text(app: &mut App) -> String {
        let mut q = app.world_mut().query_filtered::<&Text, With<TooltipText>>();
        q.iter(app.world())
            .next()
            .expect("TooltipText child should exist")
            .0
            .clone()
    }

    /// Spawns a synthetic interactive node with a `Tooltip` component,
    /// pre-set to `Interaction::Hovered`. The picking pipeline doesn't
    /// run under `MinimalPlugins`, so we write `Hovered` directly.
    fn spawn_hovered_tooltip(app: &mut App, label: &'static str) -> Entity {
        let id = app
            .world_mut()
            .spawn((
                Node::default(),
                Interaction::Hovered,
                Tooltip::new(label),
            ))
            .id();
        // Mark the Interaction Changed by re-inserting it. `Changed`
        // requires component mutation since the previous tick; spawn
        // already counts, but a follow-up insert is the explicit signal.
        app.world_mut()
            .entity_mut(id)
            .insert(Interaction::Hovered);
        id
    }

    /// Test 1: nothing is shown before the dwell delay elapses.
    #[test]
    fn tooltip_does_not_show_before_delay() {
        let mut app = headless_app();
        // Manual step well under the dwell delay. A handful of ticks
        // accumulates to far less than `MOTION_TOOLTIP_DELAY_SECS` so
        // the overlay must stay hidden the whole time.
        set_manual_time_step(&mut app, MOTION_TOOLTIP_DELAY_SECS * 0.1);

        spawn_hovered_tooltip(&mut app, "Test");
        // Two ticks: track_tooltip_hover records the hover start on
        // tick #1; show_or_hide_tooltip on tick #2 sees a non-zero but
        // sub-threshold elapsed. Both must keep the overlay hidden.
        app.update();
        app.update();

        assert!(
            matches!(overlay_visibility(&mut app), Visibility::Hidden),
            "overlay must stay hidden before MOTION_TOOLTIP_DELAY_SECS elapses"
        );
    }

    /// Advances Bevy's virtual clock far enough that any
    /// `Time::elapsed()` reader observes more than
    /// `MOTION_TOOLTIP_DELAY_SECS` of progress since the last
    /// `track_tooltip_hover` recorded a hover start.
    ///
    /// `Time<Virtual>` clamps each tick's delta to `max_delta`
    /// (default 250 ms) regardless of how big the underlying
    /// `TimeUpdateStrategy::ManualDuration` is, so a single oversized
    /// step doesn't actually advance virtual time by that much. We
    /// instead set a small per-tick step (200 ms — well under the
    /// 250 ms clamp) and call `app.update()` enough times to exceed
    /// the dwell threshold by a comfortable margin.
    fn advance_past_tooltip_delay(app: &mut App) {
        set_manual_time_step(app, 0.2);
        // 5 ticks × 200 ms = 1.0 s — comfortably past the 0.5 s delay
        // even after subtracting the first tick (when the hover gets
        // recorded; that tick's elapsed-since-hover is zero).
        for _ in 0..5 {
            app.update();
        }
    }

    /// Test 2: after the dwell delay, the overlay reveals and the
    /// tooltip text matches the hovered entity's `Tooltip` string.
    /// Position is intentionally not asserted: layout doesn't run under
    /// `MinimalPlugins`, so `ComputedNode.size` is `Vec2::ZERO`. The
    /// invariants we *can* check headlessly are visibility and text.
    #[test]
    fn tooltip_shows_after_delay() {
        let mut app = headless_app();
        spawn_hovered_tooltip(&mut app, "Test");
        advance_past_tooltip_delay(&mut app);

        assert!(
            matches!(overlay_visibility(&mut app), Visibility::Visible),
            "overlay must be visible after the dwell delay"
        );
        assert_eq!(
            overlay_text(&mut app),
            "Test",
            "overlay text must reflect the hovered entity's Tooltip string"
        );
    }

    /// Test 3: after the tooltip is shown, transitioning the target's
    /// `Interaction` away from `Hovered` hides the overlay on the next
    /// tick.
    #[test]
    fn tooltip_hides_on_unhover() {
        let mut app = headless_app();
        let target = spawn_hovered_tooltip(&mut app, "Test");
        advance_past_tooltip_delay(&mut app);
        assert!(
            matches!(overlay_visibility(&mut app), Visibility::Visible),
            "precondition: tooltip should be visible before un-hover"
        );

        // Unhover. `track_tooltip_hover` clears the state on the next
        // tick because the entity transitions Hovered → None.
        app.world_mut()
            .entity_mut(target)
            .insert(Interaction::None);
        app.update();

        assert!(
            matches!(overlay_visibility(&mut app), Visibility::Hidden),
            "overlay must hide once the target is no longer hovered"
        );
    }

    /// Test 4: when the cursor switches from one tooltip entity to
    /// another with different text, the overlay's text updates to match
    /// the new target's string after the dwell delay.
    #[test]
    fn tooltip_text_updates_when_hovered_target_changes() {
        let mut app = headless_app();

        // Phase A: hover entity A and let its tooltip appear.
        let a = spawn_hovered_tooltip(&mut app, "A label");
        advance_past_tooltip_delay(&mut app);
        assert_eq!(overlay_text(&mut app), "A label");

        // Phase B: unhover A, hover B with a different label. Then
        // advance time past the dwell delay again so B's tooltip can
        // take over the overlay.
        app.world_mut().entity_mut(a).insert(Interaction::None);
        let _b = spawn_hovered_tooltip(&mut app, "B label");
        advance_past_tooltip_delay(&mut app);

        assert!(
            matches!(overlay_visibility(&mut app), Visibility::Visible),
            "B's tooltip must be visible after switching hover"
        );
        assert_eq!(
            overlay_text(&mut app),
            "B label",
            "overlay text must update to the new hovered entity's Tooltip string"
        );
    }

    /// Test 5: `tooltip_should_show` is the pure helper that the system
    /// uses to gate the reveal — exercising it directly avoids the
    /// `Time<Virtual>` 250 ms clamp that makes precise sub-second
    /// timing assertions in `MinimalPlugins` fiddly. The four cases
    /// below cover the boundary semantics:
    ///
    /// * `delay = 0.0` ("Instant") must show on the first tick.
    /// * `elapsed < delay` must NOT show.
    /// * `elapsed == delay` must show (boundary inclusive).
    /// * `elapsed > delay` must show.
    #[test]
    fn tooltip_should_show_respects_delay() {
        // delay == 0 ("Instant"): any elapsed (including zero) shows.
        assert!(tooltip_should_show(0.0, 0.0), "instant delay must show on first tick");
        assert!(tooltip_should_show(0.5, 0.0));

        // Standard non-zero delay.
        assert!(!tooltip_should_show(0.4, 0.5), "elapsed < delay must hide");
        assert!(tooltip_should_show(0.5, 0.5), "elapsed == delay must show (boundary)");
        assert!(tooltip_should_show(0.6, 0.5), "elapsed > delay must show");

        // Larger delay (max-end of the slider).
        assert!(!tooltip_should_show(1.0, 1.5));
        assert!(tooltip_should_show(1.5, 1.5));
    }
}
