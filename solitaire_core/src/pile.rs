use serde::{Deserialize, Serialize};
use crate::card::{Card, Suit};

/// Identifies which pile on the board a set of cards belongs to.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum PileType {
    /// The face-down draw pile.
    Stock,
    /// The face-up discard pile drawn to.
    Waste,
    /// One of the four foundation slots (0..=3). The claimed suit, if any,
    /// is derived from the bottom card of the pile (always an Ace by
    /// construction).
    Foundation(u8),
    /// One of the seven tableau columns (0–6).
    Tableau(usize),
}

/// A named collection of cards in a specific board position.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pile {
    /// Which pile this is (Stock, Waste, Foundation slot, or Tableau column).
    pub pile_type: PileType,
    /// Cards in the pile, bottom-to-top stacking order. Last element is the top card.
    pub cards: Vec<Card>,
}

impl Pile {
    /// Creates a new empty pile of the given type.
    pub fn new(pile_type: PileType) -> Self {
        Self { pile_type, cards: Vec::new() }
    }

    /// Returns a reference to the top (last) card, or `None` if empty.
    pub fn top(&self) -> Option<&Card> {
        self.cards.last()
    }

    /// For foundation piles: returns `Some(suit)` once at least one card has
    /// landed (the bottom card is always an Ace of the claimed suit).
    /// Returns `None` for empty foundations or non-foundation piles.
    pub fn claimed_suit(&self) -> Option<Suit> {
        match self.pile_type {
            PileType::Foundation(_) => self.cards.first().map(|c| c.suit),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::{Card, Rank, Suit};

    #[test]
    fn new_pile_is_empty() {
        let pile = Pile::new(PileType::Stock);
        assert!(pile.cards.is_empty());
    }

    #[test]
    fn pile_top_returns_last_card() {
        let mut pile = Pile::new(PileType::Waste);
        pile.cards.push(Card { id: 0, suit: Suit::Hearts, rank: Rank::Ace, face_up: true });
        pile.cards.push(Card { id: 1, suit: Suit::Clubs, rank: Rank::Two, face_up: true });
        assert_eq!(pile.top().unwrap().id, 1);
    }

    #[test]
    fn pile_top_on_empty_is_none() {
        let pile = Pile::new(PileType::Waste);
        assert!(pile.top().is_none());
    }

    #[test]
    fn pile_type_foundation_uses_slot_index() {
        assert_ne!(PileType::Foundation(0), PileType::Foundation(3));
    }

    #[test]
    fn pile_type_tableau_uses_index() {
        assert_ne!(PileType::Tableau(0), PileType::Tableau(6));
    }

    #[test]
    fn claimed_suit_is_none_for_empty_foundation() {
        let pile = Pile::new(PileType::Foundation(0));
        assert!(pile.claimed_suit().is_none());
    }

    #[test]
    fn claimed_suit_is_none_for_non_foundation() {
        let mut pile = Pile::new(PileType::Tableau(0));
        pile.cards.push(Card { id: 0, suit: Suit::Hearts, rank: Rank::Ace, face_up: true });
        assert!(pile.claimed_suit().is_none());
    }

    #[test]
    fn claimed_suit_returns_bottom_card_suit() {
        let mut pile = Pile::new(PileType::Foundation(2));
        pile.cards.push(Card { id: 0, suit: Suit::Hearts, rank: Rank::Ace, face_up: true });
        pile.cards.push(Card { id: 1, suit: Suit::Hearts, rank: Rank::Two, face_up: true });
        assert_eq!(pile.claimed_suit(), Some(Suit::Hearts));
    }
}
