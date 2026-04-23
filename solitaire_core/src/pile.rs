use serde::{Deserialize, Serialize};
use crate::card::{Card, Suit};

/// Identifies which pile on the board a set of cards belongs to.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PileType {
    /// The face-down draw pile.
    Stock,
    /// The face-up discard pile drawn to.
    Waste,
    /// One of the four suit-ordered foundation piles.
    Foundation(Suit),
    /// One of the seven tableau columns (0–6).
    Tableau(usize),
}

/// A named collection of cards in a specific board position.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pile {
    pub pile_type: PileType,
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
    fn pile_type_foundation_uses_suit() {
        assert_ne!(PileType::Foundation(Suit::Hearts), PileType::Foundation(Suit::Spades));
    }

    #[test]
    fn pile_type_tableau_uses_index() {
        assert_ne!(PileType::Tableau(0), PileType::Tableau(6));
    }
}
