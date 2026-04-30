//! Reusable modal-overlay primitive: a uniform scrim + centred card with
//! header / body / actions slots, plus a button variant system that maps
//! to the design tokens in [`crate::ui_theme`].
//!
//! The audit found that the 11 existing overlay screens used three
//! different visual styles (card-centred dialog, bare full-screen, and
//! one outlier) with scrim alpha drift between 0.60 and 0.92. Every
//! overlay built its own root `Node` and its own colour decisions.
//!
//! This module collapses all of that into a single helper. Each
//! conversion commit replaces an overlay's bespoke spawn function with
//! a call to [`spawn_modal`] plus body content built in a closure.
//!
//! # Example
//!
//! ```ignore
//! spawn_modal(
//!     &mut commands,
//!     ConfirmNewGameScreen,
//!     ui_theme::Z_MODAL_PANEL,
//!     |card| {
//!         spawn_modal_header(card, "Abandon current game?", font_res);
//!         spawn_modal_body_text(
//!             card,
//!             "Your progress will be lost.",
//!             ui_theme::TEXT_SECONDARY,
//!             font_res,
//!         );
//!         spawn_modal_actions(card, |actions| {
//!             spawn_modal_button(
//!                 actions,
//!                 CancelButton,
//!                 "Cancel",
//!                 Some("Esc"),
//!                 ButtonVariant::Secondary,
//!                 font_res,
//!             );
//!             spawn_modal_button(
//!                 actions,
//!                 ConfirmButton,
//!                 "Yes, abandon",
//!                 Some("Y"),
//!                 ButtonVariant::Primary,
//!                 font_res,
//!             );
//!         });
//!     },
//! );
//! ```

use bevy::prelude::*;
use solitaire_data::AnimSpeed;

use crate::font_plugin::FontResource;
use crate::settings_plugin::SettingsResource;
use crate::ui_theme::{
    scaled_duration, ACCENT_PRIMARY, ACCENT_PRIMARY_HOVER, ACCENT_SECONDARY, BG_BASE, BG_ELEVATED,
    BG_ELEVATED_HI, BG_ELEVATED_PRESSED, BG_ELEVATED_TOP, BORDER_STRONG, BORDER_SUBTLE,
    MOTION_MODAL_SECS, RADIUS_LG, RADIUS_MD, SCRIM, TEXT_PRIMARY, TEXT_SECONDARY, TYPE_BODY_LG,
    TYPE_CAPTION, TYPE_HEADLINE, VAL_SPACE_2, VAL_SPACE_3, VAL_SPACE_4, VAL_SPACE_5,
};

// ---------------------------------------------------------------------------
// Marker components — let click handlers query / paint systems target /
// despawn helpers find every part of a standard modal.
// ---------------------------------------------------------------------------

/// Marker on the full-screen scrim entity. Carries `BackgroundColor`
/// `SCRIM` and the modal's z-index.
#[derive(Component, Debug)]
pub struct ModalScrim;

/// Marker on the centred card entity. Child of the scrim.
#[derive(Component, Debug)]
pub struct ModalCard;

/// Marker on a header `Text` (`TYPE_HEADLINE` + `TEXT_PRIMARY`).
#[derive(Component, Debug)]
pub struct ModalHeader;

/// Marker on a body paragraph `Text`.
#[derive(Component, Debug)]
pub struct ModalBody;

/// Marker on the actions row (flex-row, justify-end).
#[derive(Component, Debug)]
pub struct ModalActions;

/// Marker on a button inside a modal. Carries its variant so the paint
/// system can recolour it on hover / press.
#[derive(Component, Debug, Clone, Copy)]
pub struct ModalButton(pub ButtonVariant);

/// Drives the modal open animation. Inserted on the scrim entity by
/// [`spawn_modal`]; advanced and removed by [`advance_modal_enter`] once
/// `elapsed >= duration`.
///
/// During the animation the scrim's `BackgroundColor` alpha lerps from
/// 0 → `SCRIM`'s native alpha and the card's `Transform` scale lerps from
/// `MODAL_ENTER_START_SCALE` → 1.0. Under `AnimSpeed::Instant`,
/// `duration == 0.0` and the system snaps everything to the final state on
/// the first tick so no half-state is ever shown.
#[derive(Component, Debug, Clone, Copy)]
pub struct ModalEntering {
    /// Seconds elapsed since the animation started.
    pub elapsed: f32,
    /// Total duration in seconds. May be zero (`AnimSpeed::Instant`).
    pub duration: f32,
}

/// Initial card scale at `t = 0` for the modal open animation. The card
/// grows from this value to `1.0` over `MOTION_MODAL_SECS`.
pub const MODAL_ENTER_START_SCALE: f32 = 0.96;

// ---------------------------------------------------------------------------
// Button variants — three rungs of emphasis. A single overlay should have
// at most one Primary; Secondary and Tertiary fill out the rest.
// ---------------------------------------------------------------------------

/// Visual emphasis tier applied to a [`ModalButton`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonVariant {
    /// Loud yellow CTA — Confirm, Play Again. One per modal; right-aligned.
    Primary,
    /// Mid-emphasis — Cancel, Close, Done.
    Secondary,
    /// Low-emphasis — Quit, secondary navigation.
    Tertiary,
}

// ---------------------------------------------------------------------------
// Spawn helpers
// ---------------------------------------------------------------------------

/// Spawns a full-screen scrim and a centred card. The closure populates
/// the card's children — typically `spawn_modal_header`,
/// `spawn_modal_body_text`, and `spawn_modal_actions`.
///
/// Returns the scrim entity so callers can despawn the whole modal with
/// a single `commands.entity(scrim).despawn()` call (Bevy's hierarchy
/// despawn cascades to the card and its descendants).
///
/// `plugin_marker` is the overlay's plugin-specific marker
/// (`ConfirmNewGameScreen`, `HelpScreen`, etc.) so plugin click handlers
/// can find their own modal.
///
/// **Open animation.** The scrim is spawned with alpha 0 and the card
/// with `Transform::scale = MODAL_ENTER_START_SCALE`; a [`ModalEntering`]
/// component on the scrim drives the scrim alpha → `SCRIM`'s native
/// alpha and the card scale → 1.0 lerps via [`advance_modal_enter`]. The
/// duration is `scaled_duration(MOTION_MODAL_SECS, settings.animation_speed)`
/// so the open animation respects the player's `AnimSpeed` preference;
/// under `AnimSpeed::Instant` the duration is zero and the very first
/// tick snaps to the final state. The animate-OUT path is intentionally
/// out of scope — modals despawn instantly.
pub fn spawn_modal<M: Component, F>(
    commands: &mut Commands,
    plugin_marker: M,
    z_panel: i32,
    build_card: F,
) -> Entity
where
    F: FnOnce(&mut ChildSpawnerCommands),
{
    // The duration here is the `AnimSpeed::Normal` baseline; the
    // `apply_modal_enter_speed` system rescales it (or zeroes it for
    // `AnimSpeed::Instant`) on the first frame after spawn by reading
    // `SettingsResource`. Doing it that way keeps `spawn_modal` a free
    // function with no resource dependencies — every existing call site
    // (~11 plugins) continues to work without a signature change.
    let duration = MOTION_MODAL_SECS;
    let initial_scrim = scrim_with_alpha(0.0);
    commands
        .spawn((
            plugin_marker,
            ModalScrim,
            ModalEntering { elapsed: 0.0, duration },
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                top: Val::Px(0.0),
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(initial_scrim),
            // GlobalZIndex pins this root modal at `z_panel` regardless
            // of any sibling stacking-context quirks in Bevy 0.18 — the
            // ordinary `ZIndex` is preserved as a fallback for nested
            // contexts. Without GlobalZIndex, a confirmation modal at
            // `Z_PAUSE_DIALOG` (225) was rendering *behind* the pause
            // modal at `Z_PAUSE` (220) in some scenes.
            GlobalZIndex(z_panel),
            ZIndex(z_panel),
        ))
        .with_children(|root| {
            root.spawn((
                ModalCard,
                Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: VAL_SPACE_4,
                    padding: UiRect::all(VAL_SPACE_5),
                    border: UiRect::all(Val::Px(1.0)),
                    border_radius: BorderRadius::all(Val::Px(RADIUS_LG)),
                    max_width: Val::Px(720.0),
                    min_width: Val::Px(360.0),
                    align_items: AlignItems::Stretch,
                    ..default()
                },
                // Card UI nodes carry a Transform; the open animation
                // lerps `scale` from MODAL_ENTER_START_SCALE → 1.0.
                Transform::from_scale(Vec3::splat(MODAL_ENTER_START_SCALE)),
                BackgroundColor(BG_ELEVATED),
                BorderColor::all(BORDER_STRONG),
            ))
            .with_children(build_card);
        })
        .id()
}

/// Returns `SCRIM` with its alpha multiplied by `factor` (0.0–1.0). The
/// open animation lerps `factor` from 0 → 1 over the modal-enter
/// duration so the scrim fades in instead of popping.
fn scrim_with_alpha(factor: f32) -> Color {
    let mut c = SCRIM;
    let target = SCRIM.alpha();
    c.set_alpha(target * factor.clamp(0.0, 1.0));
    c
}

/// Spawns the standard modal header — `TYPE_HEADLINE` + `TEXT_PRIMARY`.
pub fn spawn_modal_header(
    parent: &mut ChildSpawnerCommands,
    title: impl Into<String>,
    font_res: Option<&FontResource>,
) {
    let font = TextFont {
        font: font_res.map(|f| f.0.clone()).unwrap_or_default(),
        font_size: TYPE_HEADLINE,
        ..default()
    };
    parent.spawn((
        ModalHeader,
        Text::new(title.into()),
        font,
        TextColor(TEXT_PRIMARY),
    ));
}

/// Spawns a body paragraph at `TYPE_BODY_LG`. Pass `TEXT_PRIMARY` for
/// primary copy, `TEXT_SECONDARY` for caption-style supporting copy.
pub fn spawn_modal_body_text(
    parent: &mut ChildSpawnerCommands,
    text: impl Into<String>,
    color: Color,
    font_res: Option<&FontResource>,
) {
    let font = TextFont {
        font: font_res.map(|f| f.0.clone()).unwrap_or_default(),
        font_size: TYPE_BODY_LG,
        ..default()
    };
    parent.spawn((
        ModalBody,
        Text::new(text.into()),
        font,
        TextColor(color),
    ));
}

/// Spawns the bottom actions row — flex-row with primary right-aligned.
/// The closure populates the row's buttons via `spawn_modal_button`.
pub fn spawn_modal_actions<F>(parent: &mut ChildSpawnerCommands, build_buttons: F)
where
    F: FnOnce(&mut ChildSpawnerCommands),
{
    parent
        .spawn((
            ModalActions,
            Node {
                flex_direction: FlexDirection::Row,
                column_gap: VAL_SPACE_3,
                justify_content: JustifyContent::FlexEnd,
                margin: UiRect::top(VAL_SPACE_2),
                ..default()
            },
        ))
        .with_children(build_buttons);
}

/// Spawns a real `Button` entity with consistent geometry, colours, and
/// optional hotkey-hint chip.
///
/// `marker` is the click-handler-targeting component (e.g.
/// `ConfirmYesButton`); plugin systems query for it on
/// `Changed<Interaction>` to detect clicks.
pub fn spawn_modal_button<M: Component>(
    parent: &mut ChildSpawnerCommands,
    marker: M,
    label: impl Into<String>,
    hotkey: Option<&'static str>,
    variant: ButtonVariant,
    font_res: Option<&FontResource>,
) {
    let font_handle = font_res.map(|f| f.0.clone()).unwrap_or_default();
    let font_label = TextFont {
        font: font_handle.clone(),
        font_size: TYPE_BODY_LG,
        ..default()
    };
    let font_caption = TextFont {
        font: font_handle,
        font_size: TYPE_CAPTION,
        ..default()
    };

    let label_color = match variant {
        // Primary buttons sit on the loud yellow accent — dark text on
        // top reads well and passes AAA contrast.
        ButtonVariant::Primary => BG_BASE,
        ButtonVariant::Secondary | ButtonVariant::Tertiary => TEXT_PRIMARY,
    };
    let caption_color = match variant {
        // Use a slightly muted version of the label colour so the chip
        // reads as a secondary detail without disappearing.
        ButtonVariant::Primary => Color::srgba(0.0, 0.0, 0.0, 0.55),
        ButtonVariant::Secondary | ButtonVariant::Tertiary => TEXT_SECONDARY,
    };

    parent
        .spawn((
            marker,
            ModalButton(variant),
            Button,
            Node {
                padding: UiRect::axes(VAL_SPACE_4, VAL_SPACE_3),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                column_gap: VAL_SPACE_2,
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(RADIUS_MD)),
                ..default()
            },
            BackgroundColor(idle_bg(variant)),
            BorderColor::all(BORDER_SUBTLE),
        ))
        .with_children(|b| {
            b.spawn((Text::new(label.into()), font_label, TextColor(label_color)));
            if let Some(key) = hotkey {
                b.spawn((Text::new(key), font_caption, TextColor(caption_color)));
            }
        });
}

// ---------------------------------------------------------------------------
// Helpers + paint system
// ---------------------------------------------------------------------------

/// Idle-state background colour for a button variant.
fn idle_bg(variant: ButtonVariant) -> Color {
    match variant {
        ButtonVariant::Primary => ACCENT_PRIMARY,
        // Secondary sits at a higher elevation than Tertiary at idle so
        // the hierarchy reads even before hover; the paint system then
        // bumps each variant one rung on hover.
        ButtonVariant::Secondary => BG_ELEVATED_HI,
        ButtonVariant::Tertiary => BG_ELEVATED,
    }
}

/// Hover-state background colour. Each variant steps up one rung from
/// its idle colour so idle / hover / pressed are visually distinct.
fn hover_bg(variant: ButtonVariant) -> Color {
    match variant {
        ButtonVariant::Primary => ACCENT_PRIMARY_HOVER,
        ButtonVariant::Secondary => BG_ELEVATED_TOP,
        ButtonVariant::Tertiary => BG_ELEVATED_HI,
    }
}

/// Pressed-state background colour. Primary swaps to the magenta
/// secondary accent for a moment of celebration; Secondary darkens to
/// the base elevation; Tertiary darkens further.
fn pressed_bg(variant: ButtonVariant) -> Color {
    match variant {
        ButtonVariant::Primary => ACCENT_SECONDARY,
        ButtonVariant::Secondary => BG_ELEVATED,
        ButtonVariant::Tertiary => BG_ELEVATED_PRESSED,
    }
}

// ---------------------------------------------------------------------------
// Modal open animation
// ---------------------------------------------------------------------------

/// Patches the `ModalEntering::duration` of newly-spawned modals against
/// the player's `AnimSpeed` setting. Runs on `Added<ModalEntering>` so it
/// only fires once per modal, immediately after [`spawn_modal`] inserts
/// the component.
///
/// Under `AnimSpeed::Instant` this drops the duration to 0; the next
/// frame [`advance_modal_enter`] sees `t >= 1.0` and snaps the modal to
/// its final state, so no half-state is ever shown.
pub fn apply_modal_enter_speed(
    settings: Option<Res<SettingsResource>>,
    mut q: Query<&mut ModalEntering, Added<ModalEntering>>,
) {
    let speed = settings
        .as_ref()
        .map(|s| s.0.animation_speed)
        .unwrap_or(AnimSpeed::Normal);
    for mut entering in &mut q {
        entering.duration = scaled_duration(MOTION_MODAL_SECS, speed);
    }
}

/// Drives the modal open animation. For each scrim entity carrying
/// [`ModalEntering`] this system increments `elapsed`, computes
/// `t = (elapsed / duration).clamp(0, 1)`, applies an ease-out
/// (`t * (2 - t)`) curve to both the scrim alpha and the card scale,
/// and removes the component plus any leftover transform offset once
/// `t >= 1.0`.
///
/// The card scale is patched on the modal's `ModalCard` child rather
/// than on the scrim — the scrim is full-window and any scale on it
/// would visibly squash the layout. The card carries its own
/// `Transform`, started at `Vec3::splat(MODAL_ENTER_START_SCALE)` by
/// [`spawn_modal`].
pub fn advance_modal_enter(
    time: Res<Time>,
    mut commands: Commands,
    mut scrims: Query<(Entity, &mut ModalEntering, &mut BackgroundColor, &Children), With<ModalScrim>>,
    mut cards: Query<&mut Transform, With<ModalCard>>,
) {
    let dt = time.delta_secs();
    for (scrim_entity, mut entering, mut bg, children) in &mut scrims {
        // Zero-duration path (AnimSpeed::Instant): snap to the final
        // state on the very first tick so the modal is fully visible
        // immediately and we never expose the 0.96 / alpha-0 starting
        // pose to the player.
        let t = if entering.duration <= 0.0 {
            1.0
        } else {
            entering.elapsed += dt;
            (entering.elapsed / entering.duration).clamp(0.0, 1.0)
        };

        // Ease-out: t * (2 - t). Reaches 1.0 at t=1, derivative is 0
        // at the endpoint so the animation settles instead of snapping.
        let eased = t * (2.0 - t);

        bg.0 = scrim_with_alpha(eased);

        let scale = MODAL_ENTER_START_SCALE + (1.0 - MODAL_ENTER_START_SCALE) * eased;
        for child in children.iter() {
            if let Ok(mut transform) = cards.get_mut(child) {
                transform.scale = Vec3::splat(scale);
            }
        }

        if t >= 1.0 {
            // Pin scrim and card to their final exact values so any
            // float drift from the lerp doesn't survive into normal
            // use (downstream paint systems read these later).
            bg.0 = SCRIM;
            for child in children.iter() {
                if let Ok(mut transform) = cards.get_mut(child) {
                    transform.scale = Vec3::ONE;
                }
            }
            commands.entity(scrim_entity).remove::<ModalEntering>();
        }
    }
}

/// Repaints every `ModalButton` on `Changed<Interaction>` so hover and
/// press states are visible without each overlay registering its own
/// paint system.
#[allow(clippy::type_complexity)]
pub fn paint_modal_buttons(
    mut buttons: Query<
        (&Interaction, &ModalButton, &mut BackgroundColor),
        Changed<Interaction>,
    >,
) {
    for (interaction, modal_button, mut bg) in &mut buttons {
        bg.0 = match interaction {
            Interaction::Pressed => pressed_bg(modal_button.0),
            Interaction::Hovered => hover_bg(modal_button.0),
            Interaction::None => idle_bg(modal_button.0),
        };
    }
}

// ---------------------------------------------------------------------------
// Plugin registration
// ---------------------------------------------------------------------------

/// Registers `paint_modal_buttons` so every `ModalButton` automatically
/// gets hover / press feedback. Add this plugin to the app once;
/// individual overlay plugins don't need their own paint systems.
pub struct UiModalPlugin;

impl Plugin for UiModalPlugin {
    fn build(&self, app: &mut App) {
        // Order: `apply_modal_enter_speed` patches the duration on the
        // first frame after spawn (Added<ModalEntering>), then
        // `advance_modal_enter` ticks. Running them in a tuple keeps
        // them in the same stage so a freshly-spawned modal lands on
        // the correct duration before its first frame of advance —
        // important for AnimSpeed::Instant where duration must be 0
        // before advance computes `t`.
        app.add_systems(
            Update,
            (apply_modal_enter_speed, advance_modal_enter, paint_modal_buttons).chain(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Idle / hover / pressed cycle through three distinct colours per
    /// variant — guards against a future refactor accidentally mapping
    /// two states to the same colour.
    #[test]
    fn paint_states_are_distinct_per_variant() {
        for variant in [
            ButtonVariant::Primary,
            ButtonVariant::Secondary,
            ButtonVariant::Tertiary,
        ] {
            let i = idle_bg(variant);
            let h = hover_bg(variant);
            let p = pressed_bg(variant);
            assert_ne!(i, h, "idle and hover must differ for {variant:?}");
            assert_ne!(h, p, "hover and pressed must differ for {variant:?}");
            assert_ne!(i, p, "idle and pressed must differ for {variant:?}");
        }
    }

    #[test]
    fn ui_modal_plugin_registers_paint_system() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins).add_plugins(UiModalPlugin);
        // App built without panic — paint_modal_buttons is registered.
        app.update();
    }

    // -----------------------------------------------------------------------
    // Modal open animation (G1)
    // -----------------------------------------------------------------------

    /// Marker component for the test modal — `spawn_modal` requires a
    /// `Component` so tests need their own dummy.
    #[derive(Component, Debug)]
    struct TestModal;

    /// `spawn_modal` inserts `ModalEntering` carrying the full
    /// `MOTION_MODAL_SECS` duration (`AnimSpeed::Normal` baseline) plus
    /// a card child sized at the start scale. The
    /// `apply_modal_enter_speed` system rescales later under
    /// `SettingsResource`; absent that resource the baseline stands.
    #[test]
    fn spawn_modal_inserts_entering_with_full_duration_and_start_scale() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins).add_plugins(UiModalPlugin);

        let scrim = {
            let world = app.world_mut();
            let mut commands = world.commands();
            let id = spawn_modal(&mut commands, TestModal, 0, |_| {});
            world.flush();
            id
        };

        let entering = app
            .world()
            .entity(scrim)
            .get::<ModalEntering>()
            .expect("ModalEntering should be inserted on spawn");
        assert!(
            (entering.duration - MOTION_MODAL_SECS).abs() < 1e-6,
            "duration should be the AnimSpeed::Normal baseline before apply_modal_enter_speed runs; got {}",
            entering.duration
        );
        assert_eq!(entering.elapsed, 0.0);

        // The card child carries Transform with scale at the start value.
        let card_scale = card_scale_of(&mut app, scrim);
        assert!(
            (card_scale - MODAL_ENTER_START_SCALE).abs() < 1e-6,
            "card should spawn at MODAL_ENTER_START_SCALE; got {card_scale}"
        );
    }

    /// After enough simulated ticks for `elapsed >= duration`, the
    /// `ModalEntering` component is removed and the card scale is back
    /// at 1.0. Uses `Time<Virtual>` advance to push elapsed past the
    /// duration without waiting for real wall-clock time.
    #[test]
    fn advance_modal_enter_removes_component_and_settles_scale() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins).add_plugins(UiModalPlugin);

        let scrim = {
            let world = app.world_mut();
            let mut commands = world.commands();
            let id = spawn_modal(&mut commands, TestModal, 0, |_| {});
            world.flush();
            id
        };

        // Tick once with delta well beyond MOTION_MODAL_SECS — the
        // ease-out clamps t at 1.0 so a single oversized tick is enough
        // to settle the animation. `ManualDuration` makes
        // `Time::delta_secs()` deterministic inside the test.
        set_manual_time_step(&mut app, MOTION_MODAL_SECS * 2.0 + 0.1);
        // Two updates: the first sets up `Time` with the manual delta;
        // the second runs the advance system with non-zero dt. The
        // `Added<ModalEntering>` filter survives across these updates
        // because `apply_modal_enter_speed` writes the duration on
        // whichever frame the entity first appears.
        app.update();
        app.update();

        assert!(
            app.world().entity(scrim).get::<ModalEntering>().is_none(),
            "ModalEntering should be removed once elapsed >= duration"
        );
        let card_scale = card_scale_of(&mut app, scrim);
        assert!(
            (card_scale - 1.0).abs() < 1e-3,
            "card scale should settle at 1.0 after the open animation; got {card_scale}"
        );
    }

    /// Returns the X-component of the first `ModalCard` child of the
    /// given scrim's `Transform::scale`. All three components are kept
    /// in sync by the system so reading X is sufficient.
    fn card_scale_of(app: &mut App, scrim: Entity) -> f32 {
        let world = app.world();
        let children = world
            .entity(scrim)
            .get::<Children>()
            .expect("scrim should have a card child");
        for child in children.iter() {
            if let Some(t) = world.entity(child).get::<Transform>()
                && world.entity(child).get::<ModalCard>().is_some()
            {
                return t.scale.x;
            }
        }
        panic!("no ModalCard child with a Transform under scrim {scrim:?}");
    }

    /// Tells `TimePlugin` to advance the clock by `secs` on the next
    /// `app.update()`. Inside a unit test no real wall-clock time has
    /// passed between ticks, so the default `Automatic` strategy gives
    /// `delta_secs() == 0`. `ManualDuration` makes the next tick
    /// observe exactly `secs` of elapsed time.
    fn set_manual_time_step(app: &mut App, secs: f32) {
        use bevy::time::TimeUpdateStrategy;
        use std::time::Duration;
        app.insert_resource(TimeUpdateStrategy::ManualDuration(
            Duration::from_secs_f32(secs),
        ));
    }
}

