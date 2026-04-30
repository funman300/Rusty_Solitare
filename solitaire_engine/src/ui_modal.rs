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

use crate::font_plugin::FontResource;
use crate::ui_theme::{
    ACCENT_PRIMARY, ACCENT_PRIMARY_HOVER, ACCENT_SECONDARY, BG_BASE, BG_ELEVATED, BG_ELEVATED_HI,
    BG_ELEVATED_PRESSED, BG_ELEVATED_TOP, BORDER_STRONG, BORDER_SUBTLE, RADIUS_LG, RADIUS_MD,
    SCRIM, TEXT_PRIMARY, TEXT_SECONDARY, TYPE_BODY_LG, TYPE_CAPTION, TYPE_HEADLINE, VAL_SPACE_2,
    VAL_SPACE_3, VAL_SPACE_4, VAL_SPACE_5,
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
pub fn spawn_modal<M: Component, F>(
    commands: &mut Commands,
    plugin_marker: M,
    z_panel: i32,
    build_card: F,
) -> Entity
where
    F: FnOnce(&mut ChildSpawnerCommands),
{
    commands
        .spawn((
            plugin_marker,
            ModalScrim,
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
            BackgroundColor(SCRIM),
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
                BackgroundColor(BG_ELEVATED),
                BorderColor::all(BORDER_STRONG),
            ))
            .with_children(build_card);
        })
        .id()
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
        app.add_systems(Update, paint_modal_buttons);
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
}

