//! Keyboard-driven card selection (Task #68).
//!
//! Pressing `Tab` cycles through piles that have a face-up draggable top card.
//! Pressing `Enter` or `Space` fires a [`MoveRequestEvent`] to the best
//! available destination using the following priority order, then clears the
//! selection:
//!
//! 1. Move the top card to its best foundation (count = 1).
//! 2. Move the full face-up run from the selected tableau pile to the best
//!    tableau destination (count = run length).  Single-card stacks from
//!    non-tableau piles fall back to [`best_destination`] for tableau targets.
//!
//! Pressing `Escape` clears the selection without moving.
//!
//! The selected card is highlighted by a cyan [`SelectionHighlight`] outline
//! sprite parented to the selected card entity. The highlight is despawned when
//! the selection is cleared.

use bevy::input::ButtonInput;
use bevy::prelude::*;
use solitaire_core::card::Suit;
use solitaire_core::pile::PileType;

use crate::card_plugin::CardEntity;
use crate::events::{InfoToastEvent, MoveRequestEvent};
use crate::game_plugin::GameMutation;
use crate::input_plugin::{best_destination, best_tableau_destination_for_stack};
use crate::layout::LayoutResource;
use crate::pause_plugin::PausedResource;
use crate::resources::GameStateResource;

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
            .add_systems(
                Update,
                (
                    handle_selection_keys.before(GameMutation),
                    update_selection_highlight.after(GameMutation),
                ),
            );
    }
}

// ---------------------------------------------------------------------------
// Pile cycle order
// ---------------------------------------------------------------------------

/// The ordered list of piles that are considered for keyboard cycling.
///
/// Order: Waste → Foundation×4 → Tableau 0–6.
fn cycled_piles() -> Vec<PileType> {
    let mut piles = vec![
        PileType::Waste,
        PileType::Foundation(Suit::Clubs),
        PileType::Foundation(Suit::Diamonds),
        PileType::Foundation(Suit::Hearts),
        PileType::Foundation(Suit::Spades),
    ];
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

/// Handles Tab / Enter / Space / Escape for keyboard card selection.
#[allow(clippy::too_many_arguments)]
fn handle_selection_keys(
    keys: Res<ButtonInput<KeyCode>>,
    paused: Option<Res<PausedResource>>,
    game: Res<GameStateResource>,
    mut selection: ResMut<SelectionState>,
    mut moves: MessageWriter<MoveRequestEvent>,
    mut info_toast: MessageWriter<InfoToastEvent>,
) {
    if paused.is_some_and(|p| p.0) {
        return;
    }

    // Build the list of piles that currently have a face-up draggable top card.
    let available: Vec<PileType> = {
        let all = [
            PileType::Waste,
            PileType::Foundation(Suit::Clubs),
            PileType::Foundation(Suit::Diamonds),
            PileType::Foundation(Suit::Hearts),
            PileType::Foundation(Suit::Spades),
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

    // Enter / Space — execute move for the selected pile's top card (or full
    // face-up run when the source is a tableau column).
    //
    // Priority:
    //   1. Foundation move — always count = 1.
    //   2. Tableau stack move — count = full face-up run length from the source.
    let activate =
        keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::Space);
    if activate {
        if let Some(ref pile) = selection.selected_pile.clone() {
            if let Some(card) = game
                .0
                .piles
                .get(pile)
                .and_then(|p| p.cards.last())
                .filter(|c| c.face_up)
            {
                // --- Priority 1: foundation move (single card) ---
                let foundation_dest = try_foundation_dest(card, &game.0);
                if let Some(dest) = foundation_dest {
                    moves.write(MoveRequestEvent {
                        from: pile.clone(),
                        to: dest,
                        count: 1,
                    });
                    selection.selected_pile = None;
                    return;
                }

                // --- Priority 2: tableau stack move ---
                // Count the full contiguous face-up run in the source pile.
                let run_len = face_up_run_len(game.0.piles.get(pile).map(|p| p.cards.as_slice()).unwrap_or(&[]));
                let bottom_card = game
                    .0
                    .piles
                    .get(pile)
                    .and_then(|p| {
                        let start = p.cards.len().saturating_sub(run_len);
                        p.cards.get(start)
                    });
                if let Some(bottom) = bottom_card {
                    if let Some((dest, count)) =
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
                }

                // --- Fallback: single-card move to any destination ---
                // Covers non-tableau sources (Waste, Foundation) that have no
                // stack-move logic.
                if let Some(dest) = best_destination(card, &game.0) {
                    moves.write(MoveRequestEvent {
                        from: pile.clone(),
                        to: dest,
                        count: 1,
                    });
                    selection.selected_pile = None;
                }
            }
        }
    }
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
    for suit in [Suit::Clubs, Suit::Diamonds, Suit::Hearts, Suit::Spades] {
        let dest = PileType::Foundation(suit);
        if let Some(pile) = game.piles.get(&dest) {
            if can_place_on_foundation(card, pile, suit) {
                return Some(dest);
            }
        }
    }
    None
}

/// Maintains the `SelectionHighlight` outline sprite.
///
/// When a pile is selected, a cyan sprite is placed at the selected card's
/// position. When the selection is cleared the highlight entity is despawned.
fn update_selection_highlight(
    mut commands: Commands,
    selection: Res<SelectionState>,
    game: Res<GameStateResource>,
    layout: Option<Res<LayoutResource>>,
    card_entities: Query<(Entity, &CardEntity)>,
    highlights: Query<Entity, With<SelectionHighlight>>,
) {
    // Always despawn any existing highlight first.
    for entity in &highlights {
        commands.entity(entity).despawn();
    }

    let Some(ref pile) = selection.selected_pile else {
        return;
    };
    let Some(layout) = layout else {
        return;
    };
    let Some(card) = game
        .0
        .piles
        .get(pile)
        .and_then(|p| p.cards.last())
        .filter(|c| c.face_up)
    else {
        return;
    };

    let card_id = card.id;
    let card_size = layout.0.card_size;

    // Find the entity for the selected card so we can read its position.
    for (entity, card_entity) in &card_entities {
        if card_entity.card_id == card_id {
            // Spawn the highlight as a child of the card entity so it moves
            // with it automatically.
            commands.entity(entity).with_children(|b| {
                b.spawn((
                    SelectionHighlight,
                    Sprite {
                        color: Color::srgba(0.0, 1.0, 1.0, 0.5),
                        custom_size: Some(card_size + Vec2::splat(4.0)),
                        ..default()
                    },
                    // Slightly behind the card face so text labels are still visible.
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
}
