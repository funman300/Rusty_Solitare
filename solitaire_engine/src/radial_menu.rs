//! Right-click radial menu for power-user quick-drops.
//!
//! Holding the right mouse button on a face-up draggable card pops up a
//! small radial menu of icons, one per legal destination pile, arranged in
//! a ring around the cursor. Releasing the button while the cursor is
//! over an icon dispatches a [`MoveRequestEvent`] to that destination —
//! the player skips the drag entirely. Releasing in empty space, or
//! pressing `Esc`, cancels.
//!
//! # Relationship to [`crate::card_plugin::handle_right_click`]
//!
//! This plugin **augments** rather than replaces the legacy
//! right-click-highlight tint. On the press frame `handle_right_click`
//! still tints legal pile markers via [`RightClickHighlight`]; the radial
//! overlay sits on top (Z = [`Z_RADIAL_MENU`]) and disappears with the
//! release. The two paths read the same legal-destination set, so what
//! the radial offers always matches what the highlights show.
//!
//! # State machine
//!
//! ```text
//!  ┌──────────────────┐  RMB press on face-up card
//!  │ Idle             │ ──────────────────────────────────► Active
//!  └──────────────────┘
//!                       Esc OR RMB release outside any icon
//!                       OR pause / state change
//!  ┌──────────────────┐ ◄──────────────────────────────────┐
//!  │ Active           │                                    │
//!  │   source_pile    │  RMB release while hovered_index   │
//!  │   count          │  = Some(i)                         │
//!  │   cards          │ ─── fire MoveRequestEvent ─────────┘
//!  │   destinations[] │
//!  │   hovered_index  │
//!  └──────────────────┘
//! ```
//!
//! # Tests
//!
//! Tests live alongside the implementation. The cursor-tracking and
//! release-confirm systems take a [`RadialCursorOverride`] resource that
//! lets tests inject a world-space cursor position without spinning up a
//! real `PrimaryWindow` / camera, since `MinimalPlugins` provides
//! neither.

use bevy::input::touch::Touches;
use bevy::input::ButtonInput;
use bevy::math::Vec2;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use solitaire_core::card::Card;
use solitaire_core::game_state::GameState;
use solitaire_core::pile::PileType;
use solitaire_core::rules::{can_place_on_foundation, can_place_on_tableau};

use crate::card_plugin::{TABLEAU_FACEDOWN_FAN_FRAC, TABLEAU_FAN_FRAC};
use crate::events::MoveRequestEvent;
use crate::layout::{Layout, LayoutResource};
use crate::pause_plugin::PausedResource;
use crate::resources::{DragState, GameStateResource};
use crate::settings_plugin::SettingsResource;
use crate::ui_theme::{ACCENT_PRIMARY, BORDER_STRONG, BORDER_SUBTLE, BORDER_SUBTLE_HC, STATE_SUCCESS};

/// Seconds a finger must be held on a face-up card (without crossing the
/// drag threshold) before the radial menu opens. Matches Android's long-press
/// gesture recogniser default.
const LONG_PRESS_SECS: f32 = 0.5;

/// Sprite-space `Transform.z` for radial-menu overlay sprites.
///
/// One rung above [`crate::ui_theme::Z_DROP_OVERLAY`] (`50.0`) so the radial icons render
/// in front of any drop-target wash that might still be active from a
/// concurrent drag, but well below the lifted card stack at `DRAG_Z`.
pub const Z_RADIAL_MENU: f32 = 60.0;

/// Pixel radius (world space) of the ring on which radial icons are
/// placed, measured from the cursor centre.
pub const RADIAL_RADIUS_PX: f32 = 80.0;

/// Side length (world-space pixels) of each radial icon's hit-box.
///
/// Sprites are rendered at this size; the cursor is considered "over" an
/// icon when it lies within the axis-aligned square of this side length
/// centred on the icon anchor.
pub const RADIAL_ICON_SIZE_PX: f32 = 48.0;

/// Scale factor applied to the focused (hovered) icon for emphasis.
pub const RADIAL_HOVER_SCALE: f32 = 1.15;

// ---------------------------------------------------------------------------
// State resource
// ---------------------------------------------------------------------------

/// Right-click radial-menu state machine.
///
/// `Idle` is the resting state. `Active` is entered when right-mouse is
/// just-pressed on a face-up draggable card with at least one legal
/// destination; it is exited on right-mouse release, on `Escape`, or on
/// any external state change (game mutation, pause).
#[derive(Resource, Debug, Default, Clone, PartialEq)]
pub enum RightClickRadialState {
    /// Resting state — the radial is closed and no overlay sprites exist.
    #[default]
    Idle,
    /// Radial is open. The player is holding right-mouse on
    /// `source_pile` and the cursor is currently over icon
    /// `hovered_index` (or none).
    Active {
        /// Pile the right-clicked card came from.
        source_pile: PileType,
        /// Number of cards that would be moved (always `1` — only the
        /// top face-up card is ever offered for a quick-drop, since the
        /// radial is built around single-card foundation/tableau
        /// shortcuts and that matches the right-click highlight set).
        count: usize,
        /// Card ids that would be moved (bottom-to-top order). Length
        /// always equals `count`. Currently always one element.
        cards: Vec<u32>,
        /// Pre-computed `(destination, icon_anchor_world_pos)` pairs.
        ///
        /// Anchors are evenly spaced around a ring of radius
        /// [`RADIAL_RADIUS_PX`] centred on the press position. A single
        /// destination is placed directly above the cursor; multiple
        /// destinations span an arc.
        legal_destinations: Vec<(PileType, Vec2)>,
        /// Cursor position (world space) the radial was opened at —
        /// used as the centre of the ring for cursor-hover hit testing.
        centre: Vec2,
        /// Index into `legal_destinations` the cursor is currently
        /// hovering over, or `None` when the cursor is outside every
        /// icon's hit-box.
        hovered_index: Option<usize>,
    },
}

impl RightClickRadialState {
    /// Returns `true` when the radial is currently open.
    pub fn is_active(&self) -> bool {
        matches!(self, Self::Active { .. })
    }
}

/// Optional override resource for tests: when present and `Some`, every
/// system that would normally read `Window::cursor_position()` reads this
/// world-space coordinate instead.
///
/// Tests insert this resource so the radial systems can run under
/// `MinimalPlugins`, which has no `PrimaryWindow` and no `Camera`.
/// Production builds never insert this resource.
#[derive(Resource, Debug, Clone, Copy, Default)]
pub struct RadialCursorOverride(pub Option<Vec2>);

// ---------------------------------------------------------------------------
// Visual marker components
// ---------------------------------------------------------------------------

/// Marker on a radial icon parent entity. Wraps the icon's index into
/// [`RightClickRadialState::Active::legal_destinations`] so the
/// hover-state system can find the right anchor / pile.
#[derive(Component, Debug)]
pub struct RadialIcon {
    /// Index into `RightClickRadialState::Active::legal_destinations`.
    pub index: usize,
}

/// Marker on the centre dot drawn at the cursor / source position.
#[derive(Component, Debug)]
pub struct RadialCentre;

// ---------------------------------------------------------------------------
// Plugin
// ---------------------------------------------------------------------------

/// Registers [`RightClickRadialState`] and the systems that drive it.
///
/// All systems run in the `Update` schedule. `RadialCursorOverride` is
/// **not** registered by default — production never needs it; tests
/// insert it manually.
pub struct RadialMenuPlugin;

impl Plugin for RadialMenuPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<RightClickRadialState>()
            // Tests inject `RadialCursorOverride` themselves; production
            // never touches it. We do not `init_resource` here so the
            // cursor-from-window path is the default.
            .add_systems(
                Update,
                (
                    radial_open_on_right_click,
                    radial_open_on_long_press,
                    radial_track_cursor,
                    radial_handle_release_or_cancel,
                    radial_redraw_overlay,
                )
                    .chain(),
            );
    }
}

// ---------------------------------------------------------------------------
// Pure helpers (testable without a Bevy World)
// ---------------------------------------------------------------------------

/// Returns the world-space anchor for radial icon `index` of `count`,
/// arranged on a ring of `radius` centred at `centre`.
///
/// One destination places the icon directly above the cursor (12 o'clock).
/// Multiple destinations spread evenly around a circle, with index 0 at
/// 12 o'clock and remaining indices winding clockwise.
pub fn radial_anchor_for_index(centre: Vec2, count: usize, index: usize, radius: f32) -> Vec2 {
    if count == 0 {
        return centre;
    }
    if count == 1 {
        // Single destination → straight above the cursor for maximum legibility.
        return centre + Vec2::new(0.0, radius);
    }
    // Spread evenly. Angle is measured from the +Y axis, clockwise, so
    // index 0 sits at 12 o'clock and increasing indices sweep right.
    let frac = (index as f32) / (count as f32);
    let angle = std::f32::consts::TAU * frac;
    Vec2::new(centre.x + radius * angle.sin(), centre.y + radius * angle.cos())
}

/// Returns `(hit?, index)` — whether `cursor` falls within any icon's
/// hit-box, and if so the index of the first match. Hit-boxes are
/// axis-aligned squares of side [`RADIAL_ICON_SIZE_PX`] centred on each
/// anchor. If multiple icons overlap (impossible at the default radius +
/// icon size combination, but defensively checked) the lowest index wins.
pub fn radial_hovered_index(cursor: Vec2, anchors: &[Vec2]) -> Option<usize> {
    let half = RADIAL_ICON_SIZE_PX / 2.0;
    for (i, anchor) in anchors.iter().enumerate() {
        if (cursor.x - anchor.x).abs() <= half && (cursor.y - anchor.y).abs() <= half {
            return Some(i);
        }
    }
    None
}

/// Returns the legal destination piles for moving `card` from
/// `source_pile` in `game`.
///
/// Mirrors [`crate::card_plugin::handle_right_click`]'s decision logic
/// exactly — only foundations that legally accept the card and tableaus
/// that legally accept the card. The source pile is excluded because
/// dropping a card on its own pile is a no-op.
pub fn legal_destinations_for_card(
    card: &Card,
    source_pile: &PileType,
    game: &GameState,
) -> Vec<PileType> {
    let mut out = Vec::new();
    for slot in 0..4_u8 {
        let dest = PileType::Foundation(slot);
        if dest == *source_pile {
            continue;
        }
        if let Some(pile) = game.piles.get(&dest)
            && can_place_on_foundation(card, pile)
        {
            out.push(dest);
        }
    }
    for i in 0..7_usize {
        let dest = PileType::Tableau(i);
        if dest == *source_pile {
            continue;
        }
        if let Some(pile) = game.piles.get(&dest)
            && can_place_on_tableau(card, pile)
        {
            out.push(dest);
        }
    }
    out
}

/// Returns the topmost face-up draggable card under `cursor` (world
/// space) along with its source pile.
///
/// Reuses the same "topmost face-up card" semantics as
/// [`crate::card_plugin::handle_right_click`]: tableau columns offer
/// every face-up card, waste / foundations offer only their top card,
/// and stock is never draggable. Returns `None` for face-down cards,
/// empty piles, or clicks in dead space.
pub fn find_top_face_up_card_at(
    cursor: Vec2,
    game: &GameState,
    layout: &Layout,
) -> Option<(PileType, Card)> {
    let piles = [
        PileType::Waste,
        PileType::Foundation(0),
        PileType::Foundation(1),
        PileType::Foundation(2),
        PileType::Foundation(3),
        PileType::Tableau(0),
        PileType::Tableau(1),
        PileType::Tableau(2),
        PileType::Tableau(3),
        PileType::Tableau(4),
        PileType::Tableau(5),
        PileType::Tableau(6),
    ];
    for pile in piles {
        let Some(pile_cards) = game.piles.get(&pile) else {
            continue;
        };
        if pile_cards.cards.is_empty() {
            continue;
        }
        let is_tableau = matches!(pile, PileType::Tableau(_));
        for i in (0..pile_cards.cards.len()).rev() {
            let card = &pile_cards.cards[i];
            if !card.face_up {
                continue;
            }
            // Only the top card is draggable on non-tableau piles.
            if !is_tableau && i != pile_cards.cards.len() - 1 {
                continue;
            }
            let pos = card_position(game, layout, &pile, i);
            let half = layout.card_size / 2.0;
            if cursor.x < pos.x - half.x
                || cursor.x > pos.x + half.x
                || cursor.y < pos.y - half.y
                || cursor.y > pos.y + half.y
            {
                continue;
            }
            return Some((pile, card.clone()));
        }
    }
    None
}

/// Mirror of `input_plugin::card_position` — kept private to this
/// module so the radial's hit-test geometry tracks renderer geometry
/// without depending on `input_plugin` internals.
fn card_position(game: &GameState, layout: &Layout, pile: &PileType, stack_index: usize) -> Vec2 {
    let base = layout.pile_positions[pile];
    if matches!(pile, PileType::Tableau(_)) {
        let mut y_offset = 0.0_f32;
        if let Some(pile_cards) = game.piles.get(pile) {
            for card in pile_cards.cards.iter().take(stack_index) {
                let step = if card.face_up {
                    TABLEAU_FAN_FRAC
                } else {
                    TABLEAU_FACEDOWN_FAN_FRAC
                };
                y_offset -= layout.card_size.y * step;
            }
        }
        Vec2::new(base.x, base.y + y_offset)
    } else {
        base
    }
}

/// Builds the `(destination, anchor)` list for a fresh radial open.
fn build_radial_destinations(centre: Vec2, dests: Vec<PileType>) -> Vec<(PileType, Vec2)> {
    let count = dests.len();
    dests
        .into_iter()
        .enumerate()
        .map(|(i, d)| (d, radial_anchor_for_index(centre, count, i, RADIAL_RADIUS_PX)))
        .collect()
}

// ---------------------------------------------------------------------------
// Cursor lookup — uses an override resource under MinimalPlugins, falls
// back to the real Window/Camera otherwise.
// ---------------------------------------------------------------------------

/// Returns the world-space cursor position. Prefers
/// [`RadialCursorOverride`] when present (test injection); otherwise
/// reads the primary window's cursor position and projects it through
/// the camera.
fn cursor_world(
    override_res: Option<&Res<RadialCursorOverride>>,
    windows: &Query<&Window, With<PrimaryWindow>>,
    cameras: &Query<(&Camera, &GlobalTransform)>,
) -> Option<Vec2> {
    if let Some(ovr) = override_res
        && let Some(pos) = ovr.0
    {
        return Some(pos);
    }
    let window = windows.single().ok()?;
    let cursor = window.cursor_position()?;
    let (camera, camera_transform) = cameras.single().ok()?;
    camera.viewport_to_world_2d(camera_transform, cursor).ok()
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// On `MouseButton::Right` `just_pressed`, attempts to open the radial
/// menu over the card the cursor is on. Skips when a left-mouse drag is
/// in progress, when the game is paused, or when the clicked card has no
/// legal destinations.
#[allow(clippy::too_many_arguments)]
fn radial_open_on_right_click(
    buttons: Option<Res<ButtonInput<MouseButton>>>,
    paused: Option<Res<PausedResource>>,
    drag: Res<DragState>,
    cursor_override: Option<Res<RadialCursorOverride>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Camera, &GlobalTransform)>,
    layout: Option<Res<LayoutResource>>,
    game: Option<Res<GameStateResource>>,
    mut state: ResMut<RightClickRadialState>,
) {
    if paused.is_some_and(|p| p.0) {
        return;
    }
    if !drag.is_idle() {
        return;
    }
    let Some(buttons) = buttons else { return };
    if !buttons.just_pressed(MouseButton::Right) {
        return;
    }
    if state.is_active() {
        // Already active — ignore re-presses.
        return;
    }
    let Some(layout) = layout else { return };
    let Some(game) = game else { return };
    let Some(world) = cursor_world(cursor_override.as_ref(), &windows, &cameras) else {
        return;
    };
    let Some((source_pile, card)) = find_top_face_up_card_at(world, &game.0, &layout.0) else {
        return;
    };

    // Only single-card right-click for now: foundations require single
    // cards and the highlight tint shows the same set the radial offers.
    let dests = legal_destinations_for_card(&card, &source_pile, &game.0);
    if dests.is_empty() {
        return;
    }
    let legal_destinations = build_radial_destinations(world, dests);

    *state = RightClickRadialState::Active {
        source_pile,
        count: 1,
        cards: vec![card.id],
        legal_destinations,
        centre: world,
        hovered_index: None,
    };
}

/// Opens the radial menu after a sustained touch hold on a face-up card.
///
/// Counts up while the touch is down, the drag threshold has not been
/// crossed, and the radial is not yet active. Fires after
/// [`LONG_PRESS_SECS`] (0.5 s). The timer resets whenever these
/// conditions are not met, so lifting, committing a drag, or the radial
/// already being open all clear it cleanly.
#[allow(clippy::too_many_arguments)]
fn radial_open_on_long_press(
    time: Res<Time>,
    mut hold_timer: Local<f32>,
    drag: Res<DragState>,
    paused: Option<Res<PausedResource>>,
    touches: Option<Res<Touches>>,
    cameras: Query<(&Camera, &GlobalTransform)>,
    layout: Option<Res<LayoutResource>>,
    game: Option<Res<GameStateResource>>,
    mut state: ResMut<RightClickRadialState>,
) {
    // Guard: only count while a touch is down, uncommitted, and radial is idle.
    let Some(active_id) = drag.active_touch_id else {
        *hold_timer = 0.0;
        return;
    };
    if drag.committed || state.is_active() || paused.is_some_and(|p| p.0) {
        *hold_timer = 0.0;
        return;
    }

    *hold_timer += time.delta_secs();
    if *hold_timer < LONG_PRESS_SECS {
        return;
    }
    *hold_timer = 0.0;

    // Resolve current touch world position.
    let Some(touches) = touches else { return };
    let Some(touch) = touches.iter().find(|t| t.id() == active_id) else {
        return;
    };
    let Some((camera, cam_xf)) = cameras.single().ok() else { return };
    let Some(world) = camera.viewport_to_world_2d(cam_xf, touch.position()).ok() else {
        return;
    };
    let Some(layout) = layout else { return };
    let Some(game) = game else { return };

    let Some((source_pile, card)) = find_top_face_up_card_at(world, &game.0, &layout.0) else {
        return;
    };
    let dests = legal_destinations_for_card(&card, &source_pile, &game.0);
    if dests.is_empty() {
        return;
    }
    let legal_destinations = build_radial_destinations(world, dests);
    *state = RightClickRadialState::Active {
        source_pile,
        count: 1,
        cards: vec![card.id],
        legal_destinations,
        centre: world,
        hovered_index: None,
    };
}

/// Each frame while `Active`, updates `hovered_index` based on the
/// current cursor position. Cheap — just re-runs hit-testing against
/// the precomputed anchors. The overlay redraw system reads this index
/// to apply the focused tint and scale.
fn radial_track_cursor(
    cursor_override: Option<Res<RadialCursorOverride>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Camera, &GlobalTransform)>,
    touches: Option<Res<Touches>>,
    mut state: ResMut<RightClickRadialState>,
) {
    let RightClickRadialState::Active {
        legal_destinations,
        hovered_index,
        ..
    } = state.as_mut()
    else {
        return;
    };
    // Cursor first (mouse / test override); fall back to first active touch
    // so the player can slide their held finger over radial icons on Android.
    let world = cursor_world(cursor_override.as_ref(), &windows, &cameras).or_else(|| {
        let (camera, cam_xf) = cameras.single().ok()?;
        let touch_pos = touches.as_ref()?.iter().next()?.position();
        camera.viewport_to_world_2d(cam_xf, touch_pos).ok()
    });
    let Some(world) = world else { return };
    let anchors: Vec<Vec2> = legal_destinations.iter().map(|(_, a)| *a).collect();
    *hovered_index = radial_hovered_index(world, &anchors);
}

/// Handles exit conditions while `Active`:
/// 1. Right-mouse release → confirm if hovering, otherwise cancel.
/// 2. Touch lift (`Touches::iter_just_released`) → confirm if hovering, cancel otherwise.
/// 3. `Escape` → cancel.
/// 4. Left-mouse press → cancel (keeps the existing drag pipeline clean).
#[allow(clippy::too_many_arguments)]
fn radial_handle_release_or_cancel(
    buttons: Option<Res<ButtonInput<MouseButton>>>,
    keys: Option<Res<ButtonInput<KeyCode>>>,
    touches: Option<Res<Touches>>,
    mut state: ResMut<RightClickRadialState>,
    mut moves: MessageWriter<MoveRequestEvent>,
) {
    if !state.is_active() {
        return;
    }

    let escape_pressed = keys
        .as_ref()
        .is_some_and(|k| k.just_pressed(KeyCode::Escape));
    let right_released = buttons
        .as_ref()
        .is_some_and(|b| b.just_released(MouseButton::Right));
    let left_pressed = buttons
        .as_ref()
        .is_some_and(|b| b.just_pressed(MouseButton::Left));
    // Finger lift: any touch that ended or was cancelled this frame.
    let touch_ended = touches.as_ref().is_some_and(|t| {
        t.iter_just_released().next().is_some() || t.iter_just_canceled().next().is_some()
    });

    if !escape_pressed && !right_released && !left_pressed && !touch_ended {
        return;
    }

    // On confirm (right-release or touch-lift while hovering), fire a move.
    let confirm = right_released || touch_ended;
    if confirm
        && let RightClickRadialState::Active {
            source_pile,
            count,
            legal_destinations,
            hovered_index: Some(idx),
            ..
        } = state.as_ref()
        && let Some((dest, _)) = legal_destinations.get(*idx)
    {
        moves.write(MoveRequestEvent {
            from: source_pile.clone(),
            to: dest.clone(),
            count: *count,
        });
    }

    *state = RightClickRadialState::Idle;
}

// ---------------------------------------------------------------------------
// Visual overlay — spawns / despawns sprites in step with the state.
//
// Strategy: on every frame, despawn ALL prior overlay entities and
// respawn the current snapshot. Cheap (≤ 11 sprites + a centre dot) and
// keeps the overlay always perfectly in sync without component
// bookkeeping. Skipped in tests because `MinimalPlugins` does not
// register `Sprite` rendering anyway and the state-machine assertions
// don't rely on entity existence.
// ---------------------------------------------------------------------------

/// Despawns and respawns the radial overlay sprites every frame the
/// state is `Active`; despawns them when the state returns to `Idle`.
///
/// Reads [`SettingsResource`] so the focused-icon outline can boost to
/// [`BORDER_SUBTLE_HC`] under high-contrast mode. Per-frame respawn is
/// the simplest place to fold HC in: this is the only system that
/// owns the rim sprite, so there's no parallel paint path to fight.
/// ([`HighContrastBorder`](crate::ui_theme::HighContrastBorder) doesn't
/// apply because the rim is a `Sprite`, not a UI node with
/// `BorderColor`, and the entities don't persist across frames.)
fn radial_redraw_overlay(
    state: Res<RightClickRadialState>,
    settings: Option<Res<SettingsResource>>,
    mut commands: Commands,
    existing_icons: Query<Entity, With<RadialIcon>>,
    existing_centres: Query<Entity, With<RadialCentre>>,
) {
    // Always clear last-frame overlay entities first.
    for e in &existing_icons {
        commands.entity(e).despawn();
    }
    for e in &existing_centres {
        commands.entity(e).despawn();
    }

    let RightClickRadialState::Active {
        legal_destinations,
        hovered_index,
        centre,
        ..
    } = state.as_ref()
    else {
        return;
    };

    // Centre dot — small bright marker so the player can see where the
    // ring is anchored even when the cursor moves.
    commands.spawn((
        RadialCentre,
        Sprite {
            color: ACCENT_PRIMARY,
            custom_size: Some(Vec2::splat(8.0)),
            ..default()
        },
        Transform::from_xyz(centre.x, centre.y, Z_RADIAL_MENU + 0.01),
    ));

    let high_contrast = settings.as_ref().is_some_and(|s| s.0.high_contrast_mode);
    for (i, (_pile, anchor)) in legal_destinations.iter().enumerate() {
        let focused = *hovered_index == Some(i);
        let scale = if focused { RADIAL_HOVER_SCALE } else { 1.0 };
        let fill = if focused { STATE_SUCCESS } else { ACCENT_PRIMARY };
        let outline = radial_rim_outline(focused, high_contrast);

        commands
            .spawn((
                RadialIcon { index: i },
                Sprite {
                    color: fill,
                    custom_size: Some(Vec2::splat(RADIAL_ICON_SIZE_PX)),
                    ..default()
                },
                Transform {
                    translation: Vec3::new(anchor.x, anchor.y, Z_RADIAL_MENU),
                    scale: Vec3::splat(scale),
                    ..default()
                },
            ))
            .with_children(|p| {
                // Outline ring — drawn as a slightly larger sprite
                // behind the fill so it reads as a halo, not a stroke.
                p.spawn((
                    Sprite {
                        color: outline,
                        custom_size: Some(Vec2::splat(RADIAL_ICON_SIZE_PX + 4.0)),
                        ..default()
                    },
                    Transform::from_xyz(0.0, 0.0, -0.01),
                ));
            });
    }
}

/// Pure decision logic for the radial-icon rim outline colour.
///
/// Resting icons always carry [`BORDER_SUBTLE`] so the focused icon
/// reads as the obvious target. Under high-contrast mode the focused
/// rim boosts to [`BORDER_SUBTLE_HC`] (`#a0a0a0`) instead of
/// [`BORDER_STRONG`] (`#505050`) — naive marker substitution via
/// [`HighContrastBorder`](crate::ui_theme::HighContrastBorder) would
/// invert the hierarchy because the resting colour
/// (`#353535`) is darker than `BORDER_STRONG`. This shape keeps the
/// focused rim *more* visible under HC, not less.
///
/// Factored out as a pure function so the truth-table is unit-testable
/// without spinning up the per-frame respawn system.
fn radial_rim_outline(focused: bool, high_contrast: bool) -> Color {
    match (focused, high_contrast) {
        (true, true) => BORDER_SUBTLE_HC,
        (true, false) => BORDER_STRONG,
        (false, _) => BORDER_SUBTLE,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::compute_layout;
    use bevy::ecs::message::Messages;
    use solitaire_core::card::{Card as CoreCard, Rank, Suit};
    use solitaire_core::game_state::{DrawMode, GameState};

    /// Build a minimal Bevy app wired with `RadialMenuPlugin` and the
    /// resources / messages it depends on. No window, no camera — the
    /// `RadialCursorOverride` resource feeds the cursor position.
    fn radial_test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_message::<MoveRequestEvent>();
        app.init_resource::<DragState>();
        app.init_resource::<ButtonInput<MouseButton>>();
        app.init_resource::<ButtonInput<KeyCode>>();
        app.init_resource::<RadialCursorOverride>();
        app.add_plugins(RadialMenuPlugin);
        app
    }

    /// Deterministic single-card board: Ace of Clubs on Tableau(0),
    /// every other pile empty. The Ace has exactly one legal
    /// destination — Foundation(0) — under the standard rules
    /// (`can_place_on_foundation` accepts the Ace on an empty foundation).
    fn ace_only_state() -> GameState {
        let mut g = GameState::new(0, DrawMode::DrawOne);
        // Wipe everything.
        g.piles.get_mut(&PileType::Stock).unwrap().cards.clear();
        g.piles.get_mut(&PileType::Waste).unwrap().cards.clear();
        for slot in 0..4_u8 {
            g.piles.get_mut(&PileType::Foundation(slot)).unwrap().cards.clear();
        }
        for i in 0..7_usize {
            g.piles.get_mut(&PileType::Tableau(i)).unwrap().cards.clear();
        }
        // Ace of Clubs on Tableau(0).
        g.piles
            .get_mut(&PileType::Tableau(0))
            .unwrap()
            .cards
            .push(CoreCard {
                id: 100,
                suit: Suit::Clubs,
                rank: Rank::Ace,
                face_up: true,
            });
        g
    }

    /// Place a face-down King on Tableau(0). `find_top_face_up_card_at`
    /// must skip it.
    fn face_down_only_state() -> GameState {
        let mut g = GameState::new(0, DrawMode::DrawOne);
        g.piles.get_mut(&PileType::Stock).unwrap().cards.clear();
        g.piles.get_mut(&PileType::Waste).unwrap().cards.clear();
        for slot in 0..4_u8 {
            g.piles.get_mut(&PileType::Foundation(slot)).unwrap().cards.clear();
        }
        for i in 0..7_usize {
            g.piles.get_mut(&PileType::Tableau(i)).unwrap().cards.clear();
        }
        g.piles
            .get_mut(&PileType::Tableau(0))
            .unwrap()
            .cards
            .push(CoreCard {
                id: 100,
                suit: Suit::Spades,
                rank: Rank::King,
                face_up: false,
            });
        g
    }

    fn install_resources(app: &mut App, state: GameState, layout_window: Vec2, cursor: Vec2) {
        app.insert_resource(GameStateResource(state));
        app.insert_resource(LayoutResource(compute_layout(layout_window, 0.0, 0.0, true)));
        app.world_mut().resource_mut::<RadialCursorOverride>().0 = Some(cursor);
    }

    fn press(app: &mut App, button: MouseButton) {
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(button);
    }

    fn release(app: &mut App, button: MouseButton) {
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .release(button);
    }

    fn clear_buttons(app: &mut App) {
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .clear();
    }

    fn collect_move_events(app: &mut App) -> Vec<MoveRequestEvent> {
        let events = app.world().resource::<Messages<MoveRequestEvent>>();
        let mut cursor = events.get_cursor();
        cursor.read(events).cloned().collect()
    }

    // -----------------------------------------------------------------------
    // Pure-function tests
    // -----------------------------------------------------------------------

    #[test]
    fn radial_anchor_single_destination_above_centre() {
        let centre = Vec2::new(100.0, 200.0);
        let pos = radial_anchor_for_index(centre, 1, 0, 80.0);
        // Single destination → straight above (centre + (0, radius)).
        assert!((pos.x - 100.0).abs() < 1e-3);
        assert!((pos.y - 280.0).abs() < 1e-3);
    }

    #[test]
    fn radial_anchor_two_destinations_first_above_second_below() {
        let centre = Vec2::ZERO;
        let radius = 50.0;
        let p0 = radial_anchor_for_index(centre, 2, 0, radius);
        let p1 = radial_anchor_for_index(centre, 2, 1, radius);
        // index 0 is at 12 o'clock; index 1 is the opposite side.
        assert!(p0.y > p1.y);
        assert!(p0.x.abs() < 1e-3);
        assert!(p1.x.abs() < 1e-3);
    }

    #[test]
    fn radial_anchor_zero_count_returns_centre() {
        let centre = Vec2::new(7.0, -3.0);
        assert_eq!(radial_anchor_for_index(centre, 0, 0, 80.0), centre);
    }

    #[test]
    fn radial_hovered_index_inside_box_returns_index() {
        let anchors = vec![Vec2::new(100.0, 0.0), Vec2::new(0.0, 100.0)];
        // Cursor squarely inside icon 1's box.
        assert_eq!(radial_hovered_index(Vec2::new(0.0, 100.0), &anchors), Some(1));
    }

    #[test]
    fn radial_hovered_index_outside_returns_none() {
        let anchors = vec![Vec2::new(100.0, 0.0), Vec2::new(0.0, 100.0)];
        assert_eq!(radial_hovered_index(Vec2::new(500.0, 500.0), &anchors), None);
    }

    #[test]
    fn legal_destinations_for_ace_includes_only_first_empty_foundation() {
        let g = ace_only_state();
        let card = CoreCard {
            id: 100,
            suit: Suit::Clubs,
            rank: Rank::Ace,
            face_up: true,
        };
        let dests = legal_destinations_for_card(&card, &PileType::Tableau(0), &g);
        // Ace can be placed on every empty foundation. We only need
        // the count to be ≥ 1 and the source pile to be excluded.
        assert!(!dests.is_empty(), "Ace must have at least one legal destination");
        assert!(!dests.contains(&PileType::Tableau(0)));
    }

    #[test]
    fn legal_destinations_excludes_source_pile() {
        let g = ace_only_state();
        let card = CoreCard {
            id: 100,
            suit: Suit::Clubs,
            rank: Rank::Ace,
            face_up: true,
        };
        let dests = legal_destinations_for_card(&card, &PileType::Foundation(0), &g);
        assert!(!dests.contains(&PileType::Foundation(0)));
    }

    // -----------------------------------------------------------------------
    // System-level tests (state machine + event firing)
    // -----------------------------------------------------------------------

    /// Pressing right-click on a face-up card with at least one legal
    /// destination must transition the state to `Active` carrying the
    /// expected source / count / legal-destination set.
    #[test]
    fn right_click_press_on_face_up_card_opens_radial() {
        let mut app = radial_test_app();
        let layout_window = Vec2::new(1280.0, 800.0);
        let layout = compute_layout(layout_window, 0.0, 0.0, true);
        let ace_pos = layout.pile_positions[&PileType::Tableau(0)];

        install_resources(&mut app, ace_only_state(), layout_window, ace_pos);
        // Initial state — Idle.
        assert_eq!(*app.world().resource::<RightClickRadialState>(), RightClickRadialState::Idle);

        press(&mut app, MouseButton::Right);
        app.update();

        let state = app.world().resource::<RightClickRadialState>().clone();
        match state {
            RightClickRadialState::Active {
                source_pile,
                count,
                cards,
                legal_destinations,
                ..
            } => {
                assert_eq!(source_pile, PileType::Tableau(0));
                assert_eq!(count, 1);
                assert_eq!(cards, vec![100]);
                assert!(!legal_destinations.is_empty());
                assert!(legal_destinations
                    .iter()
                    .any(|(p, _)| matches!(p, PileType::Foundation(_))));
            }
            other => panic!("expected Active, got {other:?}"),
        }
    }

    /// Releasing the right button while the cursor is over a destination
    /// icon must fire a `MoveRequestEvent` and return the state to Idle.
    #[test]
    fn right_click_release_over_destination_fires_move_request() {
        let mut app = radial_test_app();
        let layout_window = Vec2::new(1280.0, 800.0);
        let layout = compute_layout(layout_window, 0.0, 0.0, true);
        let ace_pos = layout.pile_positions[&PileType::Tableau(0)];

        install_resources(&mut app, ace_only_state(), layout_window, ace_pos);
        press(&mut app, MouseButton::Right);
        app.update();

        // Capture the destination chosen — pull anchor[0] from the state.
        let (dest_pile, anchor) = match app.world().resource::<RightClickRadialState>() {
            RightClickRadialState::Active { legal_destinations, .. } => legal_destinations[0].clone(),
            _ => panic!("expected Active"),
        };

        // Move the cursor onto that anchor and release.
        app.world_mut().resource_mut::<RadialCursorOverride>().0 = Some(anchor);
        // Need a track-cursor pass first so hovered_index updates.
        app.update();
        // Then release.
        clear_buttons(&mut app);
        release(&mut app, MouseButton::Right);
        app.update();

        // Move event must have fired.
        let events = collect_move_events(&mut app);
        assert_eq!(events.len(), 1, "exactly one MoveRequestEvent expected");
        let evt = &events[0];
        assert_eq!(evt.from, PileType::Tableau(0));
        assert_eq!(evt.to, dest_pile);
        assert_eq!(evt.count, 1);
        // State must return to Idle.
        assert_eq!(*app.world().resource::<RightClickRadialState>(), RightClickRadialState::Idle);
    }

    /// Releasing the right button far from any icon must clear state
    /// without firing any MoveRequestEvent.
    #[test]
    fn right_click_release_outside_any_destination_cancels() {
        let mut app = radial_test_app();
        let layout_window = Vec2::new(1280.0, 800.0);
        let layout = compute_layout(layout_window, 0.0, 0.0, true);
        let ace_pos = layout.pile_positions[&PileType::Tableau(0)];

        install_resources(&mut app, ace_only_state(), layout_window, ace_pos);
        press(&mut app, MouseButton::Right);
        app.update();
        assert!(app.world().resource::<RightClickRadialState>().is_active());

        // Move cursor far away — well outside every icon's hit-box.
        app.world_mut().resource_mut::<RadialCursorOverride>().0 = Some(Vec2::new(10_000.0, 10_000.0));
        app.update();

        clear_buttons(&mut app);
        release(&mut app, MouseButton::Right);
        app.update();

        let events = collect_move_events(&mut app);
        assert!(events.is_empty(), "no MoveRequestEvent on outside-release");
        assert_eq!(*app.world().resource::<RightClickRadialState>(), RightClickRadialState::Idle);
    }

    /// Pressing Escape while the radial is active must cancel cleanly,
    /// without firing any MoveRequestEvent.
    #[test]
    fn escape_cancels_active_radial() {
        let mut app = radial_test_app();
        let layout_window = Vec2::new(1280.0, 800.0);
        let layout = compute_layout(layout_window, 0.0, 0.0, true);
        let ace_pos = layout.pile_positions[&PileType::Tableau(0)];

        install_resources(&mut app, ace_only_state(), layout_window, ace_pos);
        press(&mut app, MouseButton::Right);
        app.update();
        assert!(app.world().resource::<RightClickRadialState>().is_active());

        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Escape);
        app.update();

        let events = collect_move_events(&mut app);
        assert!(events.is_empty(), "no MoveRequestEvent on Escape cancel");
        assert_eq!(*app.world().resource::<RightClickRadialState>(), RightClickRadialState::Idle);
    }

    /// Right-clicking on a face-down card must NOT open the radial.
    #[test]
    fn right_click_on_face_down_card_does_not_open_radial() {
        let mut app = radial_test_app();
        let layout_window = Vec2::new(1280.0, 800.0);
        let layout = compute_layout(layout_window, 0.0, 0.0, true);
        let king_pos = layout.pile_positions[&PileType::Tableau(0)];

        install_resources(&mut app, face_down_only_state(), layout_window, king_pos);
        press(&mut app, MouseButton::Right);
        app.update();

        assert_eq!(
            *app.world().resource::<RightClickRadialState>(),
            RightClickRadialState::Idle,
            "face-down cards must not open the radial"
        );
    }

    // -----------------------------------------------------------------------
    // radial_rim_outline — accessibility / high-contrast truth table
    // -----------------------------------------------------------------------

    #[test]
    fn rim_resting_uses_subtle_outline_without_hc() {
        assert_eq!(radial_rim_outline(false, false), BORDER_SUBTLE);
    }

    #[test]
    fn rim_focused_uses_strong_outline_without_hc() {
        assert_eq!(radial_rim_outline(true, false), BORDER_STRONG);
    }

    #[test]
    fn rim_focused_boosts_to_subtle_hc_under_hc() {
        assert_eq!(radial_rim_outline(true, true), BORDER_SUBTLE_HC);
    }

    #[test]
    fn rim_resting_stays_subtle_under_hc_to_preserve_hierarchy() {
        // Naive marker substitution would also flip the resting outline
        // to BORDER_SUBTLE_HC, which is *lighter* than BORDER_STRONG —
        // that would invert the focused/resting hierarchy. Holding the
        // resting colour at BORDER_SUBTLE keeps the focused icon the
        // obvious target under HC.
        assert_eq!(radial_rim_outline(false, true), BORDER_SUBTLE);
    }
}
