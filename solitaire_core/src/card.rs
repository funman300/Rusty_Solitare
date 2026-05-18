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
    /// All four suits in declaration order.
    pub const SUITS: [Self; 4] = [Self::Clubs, Self::Diamonds, Self::Hearts, Self::Spades];

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
    Ace   = 1,
    Two   = 2,
    Three = 3,
    Four  = 4,
    Five  = 5,
    Six   = 6,
    Seven = 7,
    Eight = 8,
    Nine  = 9,
    Ten   = 10,
    Jack  = 11,
    Queen = 12,
    King  = 13,
}

impl Rank {
    /// All thirteen ranks in ascending order.
    pub const RANKS: [Self; 13] = [
        Self::Ace, Self::Two, Self::Three, Self::Four, Self::Five,
        Self::Six, Self::Seven, Self::Eight, Self::Nine, Self::Ten,
        Self::Jack, Self::Queen, Self::King,
    ];

    /// Numeric value: Ace = 1, King = 13.
    pub fn value(self) -> u8 {
        self as u8
    }

    const fn new(n: u8) -> Option<Self> {
        match n {
            1  => Some(Self::Ace),
            2  => Some(Self::Two),
            3  => Some(Self::Three),
            4  => Some(Self::Four),
            5  => Some(Self::Five),
            6  => Some(Self::Six),
            7  => Some(Self::Seven),
            8  => Some(Self::Eight),
            9  => Some(Self::Nine),
            10 => Some(Self::Ten),
            11 => Some(Self::Jack),
            12 => Some(Self::Queen),
            13 => Some(Self::King),
            _  => None,
        }
    }

    /// Returns the rank `n` steps above `self`, or `None` if it would exceed King.
    pub const fn checked_add(self, n: u8) -> Option<Self> {
        Self::new((self as u8).saturating_add(n))
    }

    /// Returns the rank `n` steps below `self`, or `None` if it would go below Ace.
    pub const fn checked_sub(self, n: u8) -> Option<Self> {
        match (self as u8).checked_sub(n) {
            Some(v) => Self::new(v),
            None => None,
        }
    }
}

/// A single playing card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Card {
    /// Unique identifier for this card within the deal. Stable across moves and undo.
    pub id: u32,
    /// The card's suit (Clubs, Diamonds, Hearts, Spades).
    pub suit: Suit,
    /// The card's rank (Ace through King).
    pub rank: Rank,
    /// Whether the card is visible to the player. Face-down cards may not be moved.
    pub face_up: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rank_values_are_sequential() {
        for (i, r) in Rank::RANKS.iter().enumerate() {
            assert_eq!(r.value(), (i + 1) as u8);
        }
    }

    #[test]
    fn rank_as_u8_matches_value() {
        for r in Rank::RANKS {
            assert_eq!(r as u8, r.value());
        }
    }

    #[test]
    fn rank_checked_add_boundary() {
        assert_eq!(Rank::King.checked_add(1), None);
        assert_eq!(Rank::Queen.checked_add(1), Some(Rank::King));
        assert_eq!(Rank::Ace.checked_add(1), Some(Rank::Two));
        assert_eq!(Rank::Five.checked_add(3), Some(Rank::Eight));
    }

    #[test]
    fn rank_checked_sub_boundary() {
        assert_eq!(Rank::Ace.checked_sub(1), None);
        assert_eq!(Rank::Two.checked_sub(1), Some(Rank::Ace));
        assert_eq!(Rank::King.checked_sub(1), Some(Rank::Queen));
        assert_eq!(Rank::Five.checked_sub(3), Some(Rank::Two));
    }

    #[test]
    fn suit_suits_contains_all_four() {
        assert_eq!(Suit::SUITS.len(), 4);
        assert!(Suit::SUITS.contains(&Suit::Clubs));
        assert!(Suit::SUITS.contains(&Suit::Diamonds));
        assert!(Suit::SUITS.contains(&Suit::Hearts));
        assert!(Suit::SUITS.contains(&Suit::Spades));
    }

    #[test]
    fn suit_red_and_black_are_complementary() {
        for suit in [Suit::Clubs, Suit::Diamonds, Suit::Hearts, Suit::Spades] {
            assert_ne!(suit.is_red(), suit.is_black(), "{suit:?} must be exactly one of red/black");
        }
        assert!(Suit::Diamonds.is_red() && Suit::Hearts.is_red());
        assert!(Suit::Clubs.is_black() && Suit::Spades.is_black());
    }
}
