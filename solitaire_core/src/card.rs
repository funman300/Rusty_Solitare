use serde::{Deserialize, Serialize};

/// Card suit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Suit {
    Clubs,
    Diamonds,
    Hearts,
    Spades,
}

impl Suit {
    /// Returns `true` for red suits (Diamonds, Hearts).
    pub fn is_red(self) -> bool {
        matches!(self, Suit::Diamonds | Suit::Hearts)
    }

    /// Returns `true` for black suits (Clubs, Spades).
    pub fn is_black(self) -> bool {
        !self.is_red()
    }
}

/// Card rank, Ace through King.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Rank {
    Ace,
    Two,
    Three,
    Four,
    Five,
    Six,
    Seven,
    Eight,
    Nine,
    Ten,
    Jack,
    Queen,
    King,
}

impl Rank {
    /// Numeric value: Ace = 1, King = 13.
    pub fn value(self) -> u8 {
        match self {
            Rank::Ace   => 1,
            Rank::Two   => 2,
            Rank::Three => 3,
            Rank::Four  => 4,
            Rank::Five  => 5,
            Rank::Six   => 6,
            Rank::Seven => 7,
            Rank::Eight => 8,
            Rank::Nine  => 9,
            Rank::Ten   => 10,
            Rank::Jack  => 11,
            Rank::Queen => 12,
            Rank::King  => 13,
        }
    }
}

/// A single playing card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Card {
    pub id: u32,
    pub suit: Suit,
    pub rank: Rank,
    pub face_up: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rank_value_ace_is_one() {
        assert_eq!(Rank::Ace.value(), 1);
    }

    #[test]
    fn rank_value_king_is_thirteen() {
        assert_eq!(Rank::King.value(), 13);
    }

    #[test]
    fn rank_values_are_sequential() {
        let ranks = [
            Rank::Ace, Rank::Two, Rank::Three, Rank::Four, Rank::Five,
            Rank::Six, Rank::Seven, Rank::Eight, Rank::Nine, Rank::Ten,
            Rank::Jack, Rank::Queen, Rank::King,
        ];
        for (i, r) in ranks.iter().enumerate() {
            assert_eq!(r.value(), (i + 1) as u8);
        }
    }

    #[test]
    fn suit_red_is_diamonds_and_hearts() {
        assert!(Suit::Diamonds.is_red());
        assert!(Suit::Hearts.is_red());
        assert!(!Suit::Clubs.is_red());
        assert!(!Suit::Spades.is_red());
    }

    #[test]
    fn suit_black_is_clubs_and_spades() {
        assert!(Suit::Clubs.is_black());
        assert!(Suit::Spades.is_black());
        assert!(!Suit::Diamonds.is_black());
        assert!(!Suit::Hearts.is_black());
    }

    #[test]
    fn card_face_up_field_reflects_construction() {
        let card = Card { id: 0, suit: Suit::Hearts, rank: Rank::Ace, face_up: false };
        assert!(!card.face_up);
        let card2 = Card { id: 1, suit: Suit::Spades, rank: Rank::King, face_up: true };
        assert!(card2.face_up);
    }
}
