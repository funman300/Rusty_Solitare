//! Keyboard-driven card selection and full keyboard drag-and-drop.
//!
//! ## Two-mode flow
//!
//! Selection works as a small state machine across two resources:
//!
//! 1. [`SelectionState`] tracks the *source-pick* mode. `Tab` / `Shift+Tab`
//!    cycles a focus through piles that have a face-up draggable top card.
//!    The focused card is decorated with an accent-coloured [`SelectionHighlight`].
//!
//! 2. [`KeyboardDragState`] tracks the *destination-pick* mode. Pressing
//!    `Enter` while a pile is focused enters
//!    [`KeyboardDragState::Lifted`] — the cards are visually "lifted" by
//!    populating [`crate::resources::DragState`] (cards / origin_pile /
//!    cursor_offset / origin_z / `active_touch_id = Some(KEYBOARD_DRAG_TOUCH_ID)`
//!    sentinel so mouse handlers ignore the keyboard-driven drag), and the
//!    arrow keys (or `Tab` / `Shift+Tab`) cycle through *legal* destination
//!    piles only. A second `Enter` confirms the move; `Esc` cancels back to
//!    source-pick mode.
//!
//! ## Mutual exclusion with mouse drag
//!
//! While a mouse drag is in progress (`DragState` non-empty *and* not the
//! keyboard sentinel) all keyboard input is ignored. Conversely, while the
//! keyboard drag is active, mouse handlers in `input_plugin` short-circuit
//! because they check `DragState.is_idle()` before starting a new drag and
//! the mouse-up / drag-update systems explicitly skip `DragState` entries
//! whose `active_touch_id.is_some()`.
//!
//! ## Why a separate resource
//!
//! Keeping the lift state out of `SelectionState` lets `Esc` cancel the
//! lift without losing the source focus — a single Esc reverts to
//! source-pick, a second Esc clears the source focus. It also lets HUD
//! widgets that already read `SelectionState::selected_pile` keep working
//! unchanged whether the player is in source-pick or destination-pick mode.

use bevy::input::ButtonInput;
use bevy::prelude::*;
use solitaire_core::game_state::GameState;
use solitaire_core::pile::PileType;
use solitaire_core::rules::{can_place_on_foundation, can_place_on_tableau};

use crate::card_plugin::CardEntity;
use crate::events::{InfoToastEvent, MoveRequestEvent, StateChangedEvent};
use crate::game_plugin::GameMutation;
use crate::input_plugin::{best_destination, best_tableau_destination_for_stack};
use crate::layout::LayoutResource;
use crate::pause_plugin::PausedResource;
use crate::resources::{DragState, GameStateResource};
use crate::ui_theme::{ACCENT_PRIMARY, STATE_SUCCESS, STATE_WARNING};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Tracks which pile currently has keyboard focus.
///
/// `None` means no pile is selected.
#[derive(Resource, Debug, Default)]
pub struct SelectionState {
    /// The pile whose top face-up card is currently selected, or `None`.
    pub selected_pile: Option<PileType>,
}

/// Sentinel value used in [`crate::resources::DragState::active_touch_id`]
/// to mark a `DragState` populated by the keyboard-drag flow rather than a
/// real mouse or touch drag.
///
/// Mouse handlers in `input_plugin` already skip `DragState` entries whose
/// `active_touch_id.is_some()`, so this value provides clean mutual
/// exclusion without changing `DragState`'s shape.
pub const KEYBOARD_DRAG_TOUCH_ID: u64 = u64::MAX;

/// Two-state machine for the keyboard drag flow. `Idle` is the resting
/// state; while `Lifted`, the player is choosing a destination pile with
/// the arrow keys.
///
/// See the [module-level docs](self) for the full state machine.
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub enum KeyboardDragState {
    /// No keyboard drag in progress. `Tab` / `Enter` operate on
    /// [`SelectionState`].
    #[default]
    Idle,
    /// Source pile is lifted; arrow keys / `Tab` cycle through
    /// `legal_destinations` and `Enter` fires the move.
    Lifted {
        /// Pile the cards were lifted from.
        source_pile: PileType,
        /// Number of cards lifted (1 for waste / foundation, full face-up
        /// run length for a tableau column).
        count: usize,
        /// Card ids being lifted, in the same bottom-to-top order
        /// `DragState.cards` expects.
        cards: Vec<u32>,
        /// Pre-computed list of piles the lifted stack can legally be
        /// placed on. Always at least one entry while in this variant —
        /// if no legal destinations exist the state machine refuses to
        /// enter `Lifted` in the first place.
        legal_destinations: Vec<PileType>,
        /// Cursor into `legal_destinations`. Always `< legal_destinations.len()`.
        destination_index: usize,
    },
}

impl KeyboardDragState {
    /// Returns the currently focused destination pile while [`Lifted`], or
    /// `None` while [`Idle`].
    ///
    /// [`Lifted`]: KeyboardDragState::Lifted
    /// [`Idle`]: KeyboardDragState::Idle
    pub fn focused_destination(&self) -> Option<&PileType> {
        match self {
            Self::Idle => None,
            Self::Lifted {
                legal_destinations,
                destination_index,
                ..
            } => legal_destinations.get(*destination_index),
        }
    }

    /// Returns `true` when the keyboard drag is in the `Lifted` state.
    pub fn is_lifted(&self) -> bool {
        matches!(self, Self::Lifted { .. })
    }
}

/// System set label for the key-handling system.
///
/// `PausePlugin` registers `toggle_pause` before this set so it can read
/// [`SelectionState`] before `handle_selection_keys` clears it on Escape.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct SelectionKeySet;

/// Marker component placed on the outline sprite used as the keyboard-selection
/// highlight.
///
/// Exactly one entity with this marker should exist at any time. It is
/// despawned when the selection is cleared.
#[derive(Component, Debug)]
pub struct SelectionHighlight;

/// Registers the keyboard selection resources and systems.
pub struct SelectionPlugin;

impl Plugin for SelectionPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SelectionState>()
            .init_resource::<KeyboardDragState>()
            .add_systems(
                Update,
                (
                    handle_selection_keys
                        .in_set(SelectionKeySet)
                        .before(GameMutation),
                    clear_selection_on_state_change.after(GameMutation),
                    update_selection_highlight
                        .after(GameMutation)
                        .run_if(
                            resource_changed::<SelectionState>
                                .or(resource_changed::<KeyboardDragState>)
                                .or(resource_changed::<crate::GameStateResource>),
                        ),
                ),
            );
    }
}

// ---------------------------------------------------------------------------
// Pile cycle order
// ---------------------------------------------------------------------------

/// The ordered list of piles that are considered for keyboard cycling.
///
/// Order: Waste → Foundation slots 0–3 → Tableau 0–6.
fn cycled_piles() -> Vec<PileType> {
    let mut piles = vec![PileType::Waste];
    for slot in 0..4_u8 {
        piles.push(PileType::Foundation(slot));
    }
    for i in 0..7_usize {
        piles.push(PileType::Tableau(i));
    }
    piles
}

/// Given a list of *available* piles and the currently selected pile, return
/// the next pile in cycling order, wrapping around.
///
/// If `current` is `None` the first available pile is returned.
/// If `available` is empty, `None` is returned.
pub fn cycle_next_pile(
    available: &[PileType],
    current: Option<&PileType>,
) -> Option<PileType> {
    if available.is_empty() {
        return None;
    }
    let order = cycled_piles();

    let Some(cur) = current else {
        // No current selection: return the first available pile in cycle order.
        return order.iter().find(|p| available.contains(p)).cloned();
    };

    // Find the position of `cur` inside the ordered list, then scan forward
    // for the next available pile (wrapping).
    let cur_pos = order.iter().position(|p| p == cur);
    let start = cur_pos.map_or(0, |pos| pos + 1);

    // Search from `start` forward, wrapping around, skipping `cur`.
    let n = order.len();
    for offset in 0..n {
        let candidate = &order[(start + offset) % n];
        if available.contains(candidate) {
            return Some(candidate.clone());
        }
    }
    None
}

/// Returns `true` when cycling from `current` to `next` wraps around the
/// available list — i.e., `next` appears at or before `current` in the global
/// cycle order defined by [`cycled_piles`].
///
/// Both `current` and `next` must be `Some`; if either is `None` this returns
/// `false`.
fn did_wrap(
    available: &[PileType],
    current: Option<&PileType>,
    next: Option<&PileType>,
) -> bool {
    let (Some(cur), Some(nxt)) = (current, next) else {
        return false;
    };
    let order = cycled_piles();
    // Position of each pile within the *available* subset, ordered by the
    // global cycle order.
    let pos_in_available = |target: &PileType| -> Option<usize> {
        order
            .iter()
            .filter(|p| available.contains(p))
            .position(|p| p == target)
    };
    match (pos_in_available(cur), pos_in_available(nxt)) {
        (Some(cur_pos), Some(nxt_pos)) => nxt_pos <= cur_pos,
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// Handles `Tab` / `Enter` / `Space` / arrow keys / `Escape` for keyboard
/// card selection and keyboard drag-and-drop.
///
/// Source-pick mode (`KeyboardDragState::Idle`):
/// - `Tab` / `Shift+Tab` cycles `SelectionState` through draggable piles.
/// - `Enter` lifts the focused pile into `KeyboardDragState::Lifted`.
/// - `Space` is the legacy auto-move accelerator (foundation-first, then
///   best tableau target). Preserved so power users keep their muscle
///   memory; the new lift-and-pick flow is what `Enter` does.
/// - `Esc` clears `SelectionState`.
///
/// Destination-pick mode (`KeyboardDragState::Lifted`):
/// - `ArrowRight` / `ArrowDown` / `Tab` advance to the next legal
///   destination, wrapping at the end.
/// - `ArrowLeft` / `ArrowUp` / `Shift+Tab` move to the previous legal
///   destination.
/// - `Enter` confirms — fires `MoveRequestEvent` and returns to `Idle`.
/// - `Esc` cancels — clears the `DragState` and returns to source-pick
///   mode with `SelectionState` intact.
#[allow(clippy::too_many_arguments)]
fn handle_selection_keys(
    keys: Res<ButtonInput<KeyCode>>,
    paused: Option<Res<PausedResource>>,
    game: Res<GameStateResource>,
    mut selection: ResMut<SelectionState>,
    mut kbd_drag: ResMut<KeyboardDragState>,
    mut drag: ResMut<DragState>,
    mut moves: MessageWriter<MoveRequestEvent>,
    mut info_toast: MessageWriter<InfoToastEvent>,
) {
    if paused.is_some_and(|p| p.0) {
        return;
    }

    // Mutual exclusion with mouse drag — if a real mouse / touch drag is
    // running, swallow keyboard input. The keyboard-driven lift uses the
    // sentinel `active_touch_id`, so only that case may proceed.
    if !drag.is_idle() && drag.active_touch_id != Some(KEYBOARD_DRAG_TOUCH_ID) {
        return;
    }

    // ---------------------------------------------------------------------
    // Lifted (destination-pick) mode.
    // ---------------------------------------------------------------------
    if let KeyboardDragState::Lifted {
        source_pile,
        count,
        cards: _,
        legal_destinations,
        destination_index,
    } = &mut *kbd_drag
    {
        let shift_held =
            keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);

        // Cycle destinations forward / backward.
        let advance = keys.just_pressed(KeyCode::ArrowRight)
            || keys.just_pressed(KeyCode::ArrowDown)
            || (keys.just_pressed(KeyCode::Tab) && !shift_held);
        let retreat = keys.just_pressed(KeyCode::ArrowLeft)
            || keys.just_pressed(KeyCode::ArrowUp)
            || (keys.just_pressed(KeyCode::Tab) && shift_held);

        if advance {
            let n = legal_destinations.len();
            if n > 0 {
                *destination_index = (*destination_index + 1) % n;
            }
            return;
        }
        if retreat {
            let n = legal_destinations.len();
            if n > 0 {
                *destination_index = (*destination_index + n - 1) % n;
            }
            return;
        }

        // Confirm — fire MoveRequestEvent.
        if keys.just_pressed(KeyCode::Enter) {
            if let Some(dest) = legal_destinations.get(*destination_index).cloned() {
                moves.write(MoveRequestEvent {
                    from: source_pile.clone(),
                    to: dest,
                    count: *count,
                });
            }
            // Whether or not we fired, leave Lifted: a subsequent
            // `StateChangedEvent` will also reset us via
            // `clear_selection_on_state_change`, but explicit reset is
            // cleaner and lets the state-change clear handle the
            // SelectionState side.
            *kbd_drag = KeyboardDragState::Idle;
            drag.clear();
            return;
        }

        // Cancel back to source-pick mode — keep SelectionState focused.
        if keys.just_pressed(KeyCode::Escape) {
            *kbd_drag = KeyboardDragState::Idle;
            drag.clear();
            return;
        }

        // No other keys do anything while lifted.
        return;
    }

    // ---------------------------------------------------------------------
    // Idle (source-pick) mode.
    // ---------------------------------------------------------------------

    // Build the list of piles that currently have a face-up draggable top card.
    let available: Vec<PileType> = {
        let all = [
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
        all.into_iter()
            .filter(|p| {
                game.0
                    .piles
                    .get(p)
                    .and_then(|pile| pile.cards.last())
                    .is_some_and(|c| c.face_up)
            })
            .collect()
    };

    // Tab — cycle selection.
    if keys.just_pressed(KeyCode::Tab) {
        let next = cycle_next_pile(&available, selection.selected_pile.as_ref());
        if next.is_none() {
            info_toast.write(InfoToastEvent("No cards to select".to_string()));
        } else if selection.selected_pile.is_some()
            && did_wrap(&available, selection.selected_pile.as_ref(), next.as_ref())
        {
            info_toast.write(InfoToastEvent("Back to first card".to_string()));
        }
        selection.selected_pile = next;
        return;
    }

    // Escape — clear selection.
    if keys.just_pressed(KeyCode::Escape) {
        selection.selected_pile = None;
        return;
    }

    // Space — legacy auto-move accelerator. Foundation-first, then best
    // tableau stack target. Preserved so the muscle memory built around
    // `Tab` → `Space` keeps working; `Enter` is now the lift trigger.
    if keys.just_pressed(KeyCode::Space)
        && let Some(ref pile) = selection.selected_pile.clone()
        && let Some(card) = game
            .0
            .piles
            .get(pile)
            .and_then(|p| p.cards.last())
            .filter(|c| c.face_up)
    {
        // Priority 1: foundation move (single card).
        if let Some(dest) = try_foundation_dest(card, &game.0) {
            moves.write(MoveRequestEvent {
                from: pile.clone(),
                to: dest,
                count: 1,
            });
            selection.selected_pile = None;
            return;
        }
        // Priority 2: tableau stack move.
        let run_len = face_up_run_len(
            game.0.piles.get(pile).map_or(&[], |p| p.cards.as_slice()),
        );
        let bottom_card = game.0.piles.get(pile).and_then(|p| {
            let start = p.cards.len().saturating_sub(run_len);
            p.cards.get(start)
        });
        if let Some(bottom) = bottom_card
            && let Some((dest, count)) =
                best_tableau_destination_for_stack(bottom, pile, &game.0, run_len)
        {
            moves.write(MoveRequestEvent {
                from: pile.clone(),
                to: dest,
                count,
            });
            selection.selected_pile = None;
            return;
        }
        // Fallback for non-tableau sources.
        if let Some(dest) = best_destination(card, &game.0) {
            moves.write(MoveRequestEvent {
                from: pile.clone(),
                to: dest,
                count: 1,
            });
            selection.selected_pile = None;
        }
        return;
    }

    // Enter — lift the focused pile into destination-pick mode.
    if keys.just_pressed(KeyCode::Enter)
        && let Some(ref source) = selection.selected_pile.clone()
    {
        let Some(pile_cards) = game.0.piles.get(source) else {
            return;
        };
        // Determine the lift range: tableau lifts the full face-up run, all
        // other sources lift only the top card.
        let run_len = face_up_run_len(pile_cards.cards.as_slice());
        let count = if matches!(source, PileType::Tableau(_)) {
            run_len.max(1)
        } else {
            1
        };
        if pile_cards.cards.is_empty() {
            return;
        }
        let start = pile_cards.cards.len().saturating_sub(count);
        let lifted_cards: Vec<u32> =
            pile_cards.cards[start..].iter().map(|c| c.id).collect();
        let Some(bottom) = pile_cards.cards.get(start) else {
            return;
        };
        let legal = legal_destinations_for(bottom, source, &game.0, count);
        if legal.is_empty() {
            info_toast.write(InfoToastEvent(
                "No legal moves for this card".to_string(),
            ));
            return;
        }

        // Populate `DragState` with the keyboard sentinel so the existing
        // mouse-drag systems treat this as "not their drag".
        drag.cards = lifted_cards.clone();
        drag.origin_pile = Some(source.clone());
        drag.cursor_offset = Vec2::ZERO;
        drag.origin_z = 1.0;
        drag.press_pos = Vec2::ZERO;
        drag.committed = false;
        drag.active_touch_id = Some(KEYBOARD_DRAG_TOUCH_ID);

        *kbd_drag = KeyboardDragState::Lifted {
            source_pile: source.clone(),
            count,
            cards: lifted_cards,
            legal_destinations: legal,
            destination_index: 0,
        };
    }
}

// ---------------------------------------------------------------------------
// Legal-destination enumeration
// ---------------------------------------------------------------------------

/// Enumerate every pile that the lifted stack rooted at `bottom` can be
/// legally placed on, excluding the source pile itself.
///
/// Foundations are returned first (in slot order 0..4), then tableau
/// columns (in column order 0..7). Foundations only accept single-card
/// stacks, matching the existing rules engine.
///
/// The order is deliberate: the first entry is the most "obvious" target
/// (the lowest foundation or column number) which becomes the default
/// destination after a lift. Players who want a different column simply
/// press the right-arrow key once or twice.
pub(crate) fn legal_destinations_for(
    bottom: &solitaire_core::card::Card,
    source: &PileType,
    game: &GameState,
    stack_count: usize,
) -> Vec<PileType> {
    let mut out = Vec::new();
    if stack_count == 1 {
        for slot in 0..4_u8 {
            let dest = PileType::Foundation(slot);
            if &dest == source {
                continue;
            }
            if let Some(pile) = game.piles.get(&dest)
                && can_place_on_foundation(bottom, pile)
            {
                out.push(dest);
            }
        }
    }
    for i in 0..7_usize {
        let dest = PileType::Tableau(i);
        if &dest == source {
            continue;
        }
        if let Some(pile) = game.piles.get(&dest)
            && can_place_on_tableau(bottom, pile)
        {
            out.push(dest);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Count the contiguous face-up cards at the top of `cards`.
///
/// Walks backwards from the last element and stops at the first face-down card
/// (or when the slice is exhausted). Returns at least `1` when the top card is
/// face-up; returns `0` for an empty slice or when the top card is face-down.
fn face_up_run_len(cards: &[solitaire_core::card::Card]) -> usize {
    let mut count = 0;
    for card in cards.iter().rev() {
        if card.face_up {
            count += 1;
        } else {
            break;
        }
    }
    count
}

/// Find the best foundation destination for `card` — returns the first
/// foundation pile that legally accepts the card, or `None`.
///
/// This is intentionally separated from [`best_destination`] so the Enter
/// handler can attempt a foundation move first and fall through to a
/// multi-card stack move rather than accepting a single-card tableau move.
fn try_foundation_dest(
    card: &solitaire_core::card::Card,
    game: &solitaire_core::game_state::GameState,
) -> Option<PileType> {
    use solitaire_core::rules::can_place_on_foundation;
    for slot in 0..4_u8 {
        let dest = PileType::Foundation(slot);
        if let Some(pile) = game.piles.get(&dest)
            && can_place_on_foundation(card, pile) {
                return Some(dest);
            }
    }
    None
}

/// Clears the selection whenever the game state changes.
///
/// Without this, an undo or a rejected move could leave `selected_pile`
/// pointing at a pile whose top card changed, causing the highlight to
/// trail a different card than the player expects.
///
/// Also resets [`KeyboardDragState`] back to `Idle` and clears any
/// keyboard-driven [`DragState`] population — the lifted cards have just
/// moved (or been undone) so the cached `legal_destinations` are stale.
fn clear_selection_on_state_change(
    mut state_events: MessageReader<StateChangedEvent>,
    mut selection: ResMut<SelectionState>,
    mut kbd_drag: ResMut<KeyboardDragState>,
    mut drag: ResMut<DragState>,
) {
    if state_events.read().next().is_some() {
        selection.selected_pile = None;
        if matches!(*kbd_drag, KeyboardDragState::Lifted { .. }) {
            *kbd_drag = KeyboardDragState::Idle;
            // Only clear DragState if it's the keyboard sentinel — never
            // stomp a real mouse / touch drag.
            if drag.active_touch_id == Some(KEYBOARD_DRAG_TOUCH_ID) {
                drag.clear();
            }
        }
    }
}

/// Maintains the `SelectionHighlight` outline sprite.
///
/// When a pile is selected (source-pick mode), an accent-coloured sprite is placed
/// at the selected card's position. While
/// [`KeyboardDragState::Lifted`] the source highlight tints gold and a
/// second highlight follows the focused destination's top card — visually
/// telling the player "these cards will move to that pile when you press
/// Enter".
///
/// All highlights are despawned and respawned every frame so an undo /
/// rejected move can never leave a stale outline behind.
fn update_selection_highlight(
    mut commands: Commands,
    selection: Res<SelectionState>,
    kbd_drag: Res<KeyboardDragState>,
    game: Res<GameStateResource>,
    layout: Option<Res<LayoutResource>>,
    card_entities: Query<(Entity, &CardEntity)>,
    highlights: Query<Entity, With<SelectionHighlight>>,
) {
    // Always despawn any existing highlight first.
    for entity in &highlights {
        commands.entity(entity).despawn();
    }
    let Some(layout) = layout else {
        return;
    };
    let card_size = layout.0.card_size;

    // Highlight tints follow the Terminal palette's semantic state
    // tokens: ACCENT_PRIMARY focus/selection while picking the source, gold
    // attention/commitment once the cards are lifted, lime valid-move
    // tint on the destination. Alphas are kept non-zero so the card
    // face beneath remains readable through the wash.
    let lifted = kbd_drag.is_lifted();
    let source_color = if lifted {
        STATE_WARNING.with_alpha(0.6)
    } else {
        ACCENT_PRIMARY.with_alpha(0.5)
    };
    let dest_color = STATE_SUCCESS.with_alpha(0.6);

    // Resolve the source pile from KeyboardDragState (when lifted) or
    // SelectionState (otherwise). Lifted takes precedence so the gold
    // outline follows the actual lifted cards.
    let source_pile: Option<PileType> = match &*kbd_drag {
        KeyboardDragState::Lifted { source_pile, .. } => Some(source_pile.clone()),
        KeyboardDragState::Idle => selection.selected_pile.clone(),
    };

    if let Some(ref pile) = source_pile
        && let Some(card) = top_face_up_card(pile, &game.0)
    {
        spawn_highlight_on_card(
            &mut commands,
            &card_entities,
            card.id,
            card_size,
            source_color,
        );
    }

    // Destination highlight while lifted.
    if let Some(dest) = kbd_drag.focused_destination() {
        // For non-empty piles, anchor on the top card. For empty piles
        // (e.g. an empty tableau column), no card exists to anchor to;
        // skip — the source highlight already conveys that the player is
        // in destination-pick mode and the focused index is observable
        // via the resource.
        if let Some(card) = top_face_up_card(dest, &game.0) {
            spawn_highlight_on_card(
                &mut commands,
                &card_entities,
                card.id,
                card_size,
                dest_color,
            );
        }
    }
}

/// Returns the top face-up card on `pile`, or `None` if the pile is
/// empty or its top card is face-down.
fn top_face_up_card<'a>(
    pile: &PileType,
    game: &'a GameState,
) -> Option<&'a solitaire_core::card::Card> {
    game.piles
        .get(pile)
        .and_then(|p| p.cards.last())
        .filter(|c| c.face_up)
}

/// Spawn a `SelectionHighlight` sprite as a child of the entity carrying
/// the matching `CardEntity::card_id`. No-op if no entity matches.
fn spawn_highlight_on_card(
    commands: &mut Commands,
    card_entities: &Query<(Entity, &CardEntity)>,
    card_id: u32,
    card_size: Vec2,
    color: Color,
) {
    for (entity, card_entity) in card_entities {
        if card_entity.card_id == card_id {
            commands.entity(entity).with_children(|b| {
                b.spawn((
                    SelectionHighlight,
                    Sprite {
                        color,
                        custom_size: Some(card_size + Vec2::splat(4.0)),
                        ..default()
                    },
                    Transform::from_xyz(0.0, 0.0, -0.01),
                    Visibility::default(),
                ));
            });
            break;
        }
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn piles_from(names: &[&str]) -> Vec<PileType> {
        names
            .iter()
            .map(|&n| match n {
                "Waste" => PileType::Waste,
                "T0" => PileType::Tableau(0),
                "T1" => PileType::Tableau(1),
                "T2" => PileType::Tableau(2),
                _ => PileType::Waste,
            })
            .collect()
    }

    // -----------------------------------------------------------------------
    // Task #68 — cycle_next_pile pure-function tests
    // -----------------------------------------------------------------------

    #[test]
    fn cycle_next_pile_from_none() {
        // With [Waste, Tableau(0), Tableau(1)] available, starting from None → Waste.
        let available = piles_from(&["Waste", "T0", "T1"]);
        let result = cycle_next_pile(&available, None);
        assert_eq!(result, Some(PileType::Waste));
    }

    #[test]
    fn cycle_next_pile_from_waste() {
        // Starting from Waste → Tableau(0).
        let available = piles_from(&["Waste", "T0", "T1"]);
        let result = cycle_next_pile(&available, Some(&PileType::Waste));
        assert_eq!(result, Some(PileType::Tableau(0)));
    }

    #[test]
    fn cycle_next_pile_wraps() {
        // Starting from Tableau(1) → Waste (wraps back to start).
        let available = piles_from(&["Waste", "T0", "T1"]);
        let result = cycle_next_pile(&available, Some(&PileType::Tableau(1)));
        assert_eq!(result, Some(PileType::Waste));
    }

    #[test]
    fn cycle_next_pile_empty_returns_none() {
        let result = cycle_next_pile(&[], None);
        assert!(result.is_none());
    }

    // -----------------------------------------------------------------------
    // Task #59 — wrap detection: 3 piles, Tab ×3 fires wrap on third press
    // -----------------------------------------------------------------------

    /// Simulate three Tab presses over [Waste, Tableau(0), Tableau(1)].
    ///
    /// Press 1: None  → Waste       — no wrap (started from nothing)
    /// Press 2: Waste → Tableau(0)  — no wrap (advancing forward)
    /// Press 3: T(0)  → Tableau(1)  — no wrap (still advancing forward)
    /// (A fourth press would wrap T(1) → Waste.)
    #[test]
    fn wrap_detected_on_third_tab_with_three_piles() {
        let available = piles_from(&["Waste", "T0", "T1"]);

        // Press 1: no current selection → first pile, no wrap.
        let sel1 = cycle_next_pile(&available, None);
        assert_eq!(sel1, Some(PileType::Waste));
        assert!(!did_wrap(&available, None, sel1.as_ref()), "first Tab should not wrap");

        // Press 2: Waste → Tableau(0), no wrap.
        let sel2 = cycle_next_pile(&available, sel1.as_ref());
        assert_eq!(sel2, Some(PileType::Tableau(0)));
        assert!(!did_wrap(&available, sel1.as_ref(), sel2.as_ref()), "second Tab should not wrap");

        // Press 3: Tableau(0) → Tableau(1), still no wrap.
        let sel3 = cycle_next_pile(&available, sel2.as_ref());
        assert_eq!(sel3, Some(PileType::Tableau(1)));
        assert!(!did_wrap(&available, sel2.as_ref(), sel3.as_ref()), "third Tab (T0→T1) should not wrap");

        // Press 4: Tableau(1) → Waste, this IS the wrap.
        let sel4 = cycle_next_pile(&available, sel3.as_ref());
        assert_eq!(sel4, Some(PileType::Waste));
        assert!(did_wrap(&available, sel3.as_ref(), sel4.as_ref()), "fourth Tab should wrap back to Waste");
    }

    #[test]
    fn cycle_next_pile_single_element_wraps_to_itself() {
        let available = vec![PileType::Waste];
        let result = cycle_next_pile(&available, Some(&PileType::Waste));
        assert_eq!(result, Some(PileType::Waste));
    }

    // -----------------------------------------------------------------------
    // Task #8 — face_up_run_len pure-function tests
    // -----------------------------------------------------------------------

    #[test]
    fn face_up_run_len_empty_slice_is_zero() {
        assert_eq!(face_up_run_len(&[]), 0);
    }

    #[test]
    fn face_up_run_len_all_face_up() {
        use solitaire_core::card::{Card, Rank, Suit};
        let cards = vec![
            Card { id: 0, suit: Suit::Clubs, rank: Rank::King, face_up: true },
            Card { id: 1, suit: Suit::Hearts, rank: Rank::Queen, face_up: true },
            Card { id: 2, suit: Suit::Spades, rank: Rank::Jack, face_up: true },
        ];
        assert_eq!(face_up_run_len(&cards), 3);
    }

    #[test]
    fn face_up_run_len_mixed_stops_at_face_down() {
        use solitaire_core::card::{Card, Rank, Suit};
        let cards = vec![
            Card { id: 0, suit: Suit::Clubs, rank: Rank::King, face_up: false },
            Card { id: 1, suit: Suit::Hearts, rank: Rank::Queen, face_up: false },
            Card { id: 2, suit: Suit::Spades, rank: Rank::Jack, face_up: true },
            Card { id: 3, suit: Suit::Diamonds, rank: Rank::Ten, face_up: true },
        ];
        // Only the top two cards are face-up.
        assert_eq!(face_up_run_len(&cards), 2);
    }

    #[test]
    fn face_up_run_len_top_card_face_down_is_zero() {
        use solitaire_core::card::{Card, Rank, Suit};
        let cards = vec![
            Card { id: 0, suit: Suit::Clubs, rank: Rank::King, face_up: true },
            Card { id: 1, suit: Suit::Hearts, rank: Rank::Queen, face_up: false },
        ];
        assert_eq!(face_up_run_len(&cards), 0);
    }

    #[test]
    fn face_up_run_len_single_face_up_card() {
        use solitaire_core::card::{Card, Rank, Suit};
        let cards = vec![
            Card { id: 0, suit: Suit::Hearts, rank: Rank::Ace, face_up: true },
        ];
        assert_eq!(face_up_run_len(&cards), 1);
    }

    // -----------------------------------------------------------------------
    // Keyboard drag-and-drop — full integration tests
    //
    // Each test runs a `MinimalPlugins` Bevy app with `SelectionPlugin` and
    // builds a deterministic `GameState` so the legal-destination ordering
    // is predictable without depending on the deal RNG.
    // -----------------------------------------------------------------------

    use bevy::ecs::message::Messages;
    use solitaire_core::card::{Card, Rank, Suit};
    use solitaire_core::game_state::{DrawMode, GameState};

    /// Build a minimal app with `SelectionPlugin` only — no GamePlugin, no
    /// AssetServer. The `MoveRequestEvent` / `StateChangedEvent` /
    /// `InfoToastEvent` channels are registered manually so the plugin's
    /// systems compile and run.
    fn drag_test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_message::<MoveRequestEvent>();
        app.add_message::<StateChangedEvent>();
        app.add_message::<InfoToastEvent>();
        app.init_resource::<DragState>();
        app.init_resource::<ButtonInput<KeyCode>>();
        app.add_plugins(SelectionPlugin);
        app
    }

    /// Build a tableau-only board with deterministic top cards so the
    /// keyboard-cycle order is predictable.
    ///
    /// Layout:
    /// - Tableau(0): 5♣ face-up   (red destinations: 4♥ on T1 face-up below)
    /// - Tableau(1): 6♥ face-up
    /// - Tableau(2): 6♦ face-up
    /// - Tableau(3..7): empty
    /// - Stock / Waste / Foundations: empty
    ///
    /// 5♣ on T0 can legally go to either 6♥ on T1 or 6♦ on T2 (both red,
    /// rank one higher). It cannot go to a foundation (Foundation needs
    /// Ace first). It cannot go to an empty tableau (only Kings).
    /// Empty tableaus T3..T6 only accept Kings, so they are filtered out.
    fn deterministic_state() -> GameState {
        let mut g = GameState::new(0, DrawMode::DrawOne);
        // Clear stock, waste, all tableaus.
        g.piles.get_mut(&PileType::Stock).unwrap().cards.clear();
        g.piles.get_mut(&PileType::Waste).unwrap().cards.clear();
        for i in 0..7 {
            g.piles.get_mut(&PileType::Tableau(i)).unwrap().cards.clear();
        }
        // Place test cards.
        g.piles.get_mut(&PileType::Tableau(0)).unwrap().cards.push(Card {
            id: 100,
            suit: Suit::Clubs,
            rank: Rank::Five,
            face_up: true,
        });
        g.piles.get_mut(&PileType::Tableau(1)).unwrap().cards.push(Card {
            id: 101,
            suit: Suit::Hearts,
            rank: Rank::Six,
            face_up: true,
        });
        g.piles.get_mut(&PileType::Tableau(2)).unwrap().cards.push(Card {
            id: 102,
            suit: Suit::Diamonds,
            rank: Rank::Six,
            face_up: true,
        });
        g
    }

    fn install_state(app: &mut App, state: GameState) {
        app.insert_resource(GameStateResource(state));
    }

    fn press_key(app: &mut App, key: KeyCode) {
        let mut input = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        input.release(key);
        input.clear();
        input.press(key);
    }

    fn clear_input(app: &mut App) {
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .clear();
    }

    fn collect_move_events(app: &mut App) -> Vec<MoveRequestEvent> {
        let events = app.world().resource::<Messages<MoveRequestEvent>>();
        let mut cursor = events.get_cursor();
        cursor.read(events).cloned().collect()
    }

    /// Test 1 — Tab in idle state cycles to the first draggable pile.
    ///
    /// On the deterministic board, the first draggable pile in cycle order
    /// is `Tableau(0)` (the 5♣).
    #[test]
    fn tab_in_idle_cycles_to_first_draggable_pile() {
        let mut app = drag_test_app();
        install_state(&mut app, deterministic_state());
        app.update();

        // Initial state: nothing selected, KeyboardDragState::Idle.
        assert!(app.world().resource::<SelectionState>().selected_pile.is_none());
        assert_eq!(*app.world().resource::<KeyboardDragState>(), KeyboardDragState::Idle);

        press_key(&mut app, KeyCode::Tab);
        app.update();

        let selected = app.world().resource::<SelectionState>().selected_pile.clone();
        // The cycle order starts at Waste, but Waste is empty so the next
        // available pile (Tableau(0)) is selected.
        assert_eq!(selected, Some(PileType::Tableau(0)));
        assert_eq!(*app.world().resource::<KeyboardDragState>(), KeyboardDragState::Idle);
    }

    /// Test 2 — Enter while a source is selected lifts the stack.
    ///
    /// `DragState.cards` must be populated with the lifted card ids and the
    /// keyboard sentinel must be set.
    #[test]
    fn enter_in_source_selected_lifts_the_stack() {
        let mut app = drag_test_app();
        install_state(&mut app, deterministic_state());
        app.update();

        // Manually focus Tableau(0) so we don't depend on Tab.
        app.world_mut().resource_mut::<SelectionState>().selected_pile =
            Some(PileType::Tableau(0));

        press_key(&mut app, KeyCode::Enter);
        app.update();

        // Assert KeyboardDragState is Lifted with the right metadata.
        let kbd = app.world().resource::<KeyboardDragState>().clone();
        match kbd {
            KeyboardDragState::Lifted {
                source_pile,
                count,
                cards,
                legal_destinations,
                destination_index,
            } => {
                assert_eq!(source_pile, PileType::Tableau(0));
                assert_eq!(count, 1);
                assert_eq!(cards, vec![100]);
                assert!(
                    !legal_destinations.is_empty(),
                    "lifted stack must have at least one legal destination"
                );
                assert_eq!(destination_index, 0);
            }
            other => panic!("expected Lifted, got {other:?}"),
        }

        // DragState must mirror the lifted cards and carry the keyboard sentinel.
        let drag = app.world().resource::<DragState>();
        assert_eq!(drag.cards, vec![100]);
        assert_eq!(drag.origin_pile, Some(PileType::Tableau(0)));
        assert_eq!(drag.active_touch_id, Some(KEYBOARD_DRAG_TOUCH_ID));
    }

    /// Test 3 — Arrow keys in `Lifted` cycle through *legal* destinations
    /// only (foundations and tableaus that pass `can_place_on_*`), and
    /// wrap at the end of the list.
    #[test]
    fn arrow_in_lifted_cycles_legal_destinations_only() {
        let mut app = drag_test_app();
        install_state(&mut app, deterministic_state());
        app.update();
        app.world_mut().resource_mut::<SelectionState>().selected_pile =
            Some(PileType::Tableau(0));
        press_key(&mut app, KeyCode::Enter);
        app.update();

        // Capture the destination list. For the deterministic state the 5♣
        // (black) can land on 6♥ (T1) or 6♦ (T2) — both red, rank one
        // higher. Verify that the destinations are exactly those tableaus
        // (in cycle order T1 then T2).
        let initial_dests: Vec<PileType> = match app.world().resource::<KeyboardDragState>() {
            KeyboardDragState::Lifted { legal_destinations, .. } => legal_destinations.clone(),
            _ => panic!("expected Lifted"),
        };
        assert_eq!(
            initial_dests,
            vec![PileType::Tableau(1), PileType::Tableau(2)],
            "5♣ must legally accept exactly T1 (6♥) and T2 (6♦) as destinations",
        );

        // Verify all are legal (defensive — equivalent to the assertion
        // above but documented as a per-destination check).
        for dest in &initial_dests {
            let bottom_card = Card {
                id: 100,
                suit: Suit::Clubs,
                rank: Rank::Five,
                face_up: true,
            };
            let pile = app.world().resource::<GameStateResource>().0.piles.get(dest).unwrap().clone();
            assert!(
                can_place_on_tableau(&bottom_card, &pile),
                "destination {dest:?} must be legal for the lifted stack",
            );
        }

        // Initial focused destination = first entry.
        assert_eq!(
            app.world().resource::<KeyboardDragState>().focused_destination(),
            Some(&PileType::Tableau(1)),
        );

        // ArrowRight → next.
        clear_input(&mut app);
        press_key(&mut app, KeyCode::ArrowRight);
        app.update();
        assert_eq!(
            app.world().resource::<KeyboardDragState>().focused_destination(),
            Some(&PileType::Tableau(2)),
        );

        // ArrowRight again → wraps to first.
        clear_input(&mut app);
        press_key(&mut app, KeyCode::ArrowRight);
        app.update();
        assert_eq!(
            app.world().resource::<KeyboardDragState>().focused_destination(),
            Some(&PileType::Tableau(1)),
            "destination index must wrap back to 0 after exhausting the list",
        );
    }

    /// Test 4 — Enter while `Lifted` with a destination focused fires
    /// exactly one `MoveRequestEvent` and resets the state machine to
    /// `Idle` with `DragState` cleared.
    #[test]
    fn enter_in_lifted_with_destination_fires_move_request_event() {
        let mut app = drag_test_app();
        install_state(&mut app, deterministic_state());
        app.update();
        app.world_mut().resource_mut::<SelectionState>().selected_pile =
            Some(PileType::Tableau(0));
        press_key(&mut app, KeyCode::Enter);
        app.update();

        // Sanity: lifted with a focused destination.
        assert!(app.world().resource::<KeyboardDragState>().is_lifted());
        let expected_dest = app
            .world()
            .resource::<KeyboardDragState>()
            .focused_destination()
            .cloned()
            .expect("must have a focused destination after lift");

        // Confirm with Enter.
        clear_input(&mut app);
        press_key(&mut app, KeyCode::Enter);
        app.update();

        let events = collect_move_events(&mut app);
        assert_eq!(events.len(), 1, "exactly one MoveRequestEvent must fire");
        assert_eq!(events[0].from, PileType::Tableau(0));
        assert_eq!(events[0].to, expected_dest);
        assert_eq!(events[0].count, 1);

        // State machine resets.
        assert_eq!(
            *app.world().resource::<KeyboardDragState>(),
            KeyboardDragState::Idle,
            "Enter on lifted must return state machine to Idle",
        );
        assert!(
            app.world().resource::<DragState>().is_idle(),
            "DragState must be cleared after confirming the move",
        );
    }

    /// Test 5 — Esc while `Lifted` cancels back to source-selected with
    /// `SelectionState` intact and `DragState` cleared.
    #[test]
    fn escape_in_lifted_returns_to_source_selected() {
        let mut app = drag_test_app();
        install_state(&mut app, deterministic_state());
        app.update();
        app.world_mut().resource_mut::<SelectionState>().selected_pile =
            Some(PileType::Tableau(0));
        press_key(&mut app, KeyCode::Enter);
        app.update();
        assert!(app.world().resource::<KeyboardDragState>().is_lifted());

        // Esc cancels.
        clear_input(&mut app);
        press_key(&mut app, KeyCode::Escape);
        app.update();

        assert_eq!(
            *app.world().resource::<KeyboardDragState>(),
            KeyboardDragState::Idle,
            "Esc on lifted must return state machine to Idle",
        );
        assert_eq!(
            app.world().resource::<SelectionState>().selected_pile,
            Some(PileType::Tableau(0)),
            "Esc on lifted must keep SelectionState intact (source-pick mode)",
        );
        assert!(
            app.world().resource::<DragState>().is_idle(),
            "DragState must be cleared after cancelling the lift",
        );
    }

    /// Mouse drag in progress (non-keyboard `active_touch_id`) must
    /// suppress keyboard input — pressing Tab while a real mouse drag is
    /// running must not change `SelectionState`.
    #[test]
    fn keyboard_input_ignored_while_mouse_drag_active() {
        let mut app = drag_test_app();
        install_state(&mut app, deterministic_state());
        app.update();

        // Simulate a real mouse drag by populating DragState without the
        // keyboard sentinel.
        {
            let mut drag = app.world_mut().resource_mut::<DragState>();
            drag.cards = vec![100];
            drag.origin_pile = Some(PileType::Tableau(0));
            drag.committed = true;
            drag.active_touch_id = None;
        }

        let before = app.world().resource::<SelectionState>().selected_pile.clone();
        press_key(&mut app, KeyCode::Tab);
        app.update();
        let after = app.world().resource::<SelectionState>().selected_pile.clone();

        assert_eq!(
            before, after,
            "Tab must not change SelectionState while a mouse drag is in progress",
        );
    }

    /// Esc on a lifted state with no prior state-change does NOT clear
    /// `SelectionState`. A second Esc (now that the state is Idle) does.
    #[test]
    fn double_escape_clears_source_selection() {
        let mut app = drag_test_app();
        install_state(&mut app, deterministic_state());
        app.update();
        app.world_mut().resource_mut::<SelectionState>().selected_pile =
            Some(PileType::Tableau(0));
        press_key(&mut app, KeyCode::Enter);
        app.update();

        clear_input(&mut app);
        press_key(&mut app, KeyCode::Escape);
        app.update();
        assert_eq!(
            app.world().resource::<SelectionState>().selected_pile,
            Some(PileType::Tableau(0)),
            "first Esc only cancels the lift",
        );

        clear_input(&mut app);
        press_key(&mut app, KeyCode::Escape);
        app.update();
        assert!(
            app.world().resource::<SelectionState>().selected_pile.is_none(),
            "second Esc clears the source selection",
        );
    }
}
