use crate::card::{Card, Suit};
use crate::pile::Pile;

/// Returns `true` if `card` can be placed on `pile` as the next card in the foundation for `suit`.
///
/// Foundation rules: same suit, Ace starts, each subsequent card is one rank higher.
pub fn can_place_on_foundation(card: &Card, pile: &Pile, suit: Suit) -> bool {
    if card.suit != suit {
        return false;
    }
    match pile.cards.last() {
        None => card.rank.value() == 1,
        Some(top) => card.rank.value() == top.rank.value() + 1,
    }
}

/// Returns `true` if `card` (or the bottom card of a sequence) can be placed on `pile` in the tableau.
///
/// Tableau rules: Kings go on empty piles; otherwise alternating colour, one rank lower.
pub fn can_place_on_tableau(card: &Card, pile: &Pile) -> bool {
    match pile.cards.last() {
        None => card.rank.value() == 13,
        Some(top) => {
            card.rank.value() + 1 == top.rank.value()
                && card.suit.is_red() != top.suit.is_red()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::{Card, Rank, Suit};
    use crate::pile::{Pile, PileType};

    fn card(suit: Suit, rank: Rank) -> Card {
        Card { id: 0, suit, rank, face_up: true }
    }

    fn pile_with(pile_type: PileType, cards: Vec<Card>) -> Pile {
        Pile { pile_type, cards }
    }

    // Foundation tests
    #[test]
    fn foundation_ace_on_empty_is_valid() {
        let c = card(Suit::Hearts, Rank::Ace);
        let p = Pile::new(PileType::Foundation(Suit::Hearts));
        assert!(can_place_on_foundation(&c, &p, Suit::Hearts));
    }

    #[test]
    fn foundation_non_ace_on_empty_is_invalid() {
        let c = card(Suit::Hearts, Rank::Two);
        let p = Pile::new(PileType::Foundation(Suit::Hearts));
        assert!(!can_place_on_foundation(&c, &p, Suit::Hearts));
    }

    #[test]
    fn foundation_two_on_ace_same_suit_is_valid() {
        let c = card(Suit::Clubs, Rank::Two);
        let p = pile_with(PileType::Foundation(Suit::Clubs), vec![card(Suit::Clubs, Rank::Ace)]);
        assert!(can_place_on_foundation(&c, &p, Suit::Clubs));
    }

    #[test]
    fn foundation_wrong_suit_is_invalid() {
        let c = card(Suit::Hearts, Rank::Ace);
        let p = Pile::new(PileType::Foundation(Suit::Spades));
        assert!(!can_place_on_foundation(&c, &p, Suit::Spades));
    }

    #[test]
    fn foundation_skipping_rank_is_invalid() {
        let c = card(Suit::Diamonds, Rank::Three);
        let p = pile_with(PileType::Foundation(Suit::Diamonds), vec![card(Suit::Diamonds, Rank::Ace)]);
        assert!(!can_place_on_foundation(&c, &p, Suit::Diamonds));
    }

    // Tableau tests
    #[test]
    fn tableau_king_on_empty_is_valid() {
        let c = card(Suit::Hearts, Rank::King);
        let p = Pile::new(PileType::Tableau(0));
        assert!(can_place_on_tableau(&c, &p));
    }

    #[test]
    fn tableau_non_king_on_empty_is_invalid() {
        let c = card(Suit::Hearts, Rank::Queen);
        let p = Pile::new(PileType::Tableau(0));
        assert!(!can_place_on_tableau(&c, &p));
    }

    #[test]
    fn tableau_red_on_black_one_lower_is_valid() {
        let c = card(Suit::Hearts, Rank::Nine);
        let p = pile_with(PileType::Tableau(0), vec![card(Suit::Spades, Rank::Ten)]);
        assert!(can_place_on_tableau(&c, &p));
    }

    #[test]
    fn tableau_same_color_is_invalid() {
        let c = card(Suit::Clubs, Rank::Nine);
        let p = pile_with(PileType::Tableau(0), vec![card(Suit::Spades, Rank::Ten)]);
        assert!(!can_place_on_tableau(&c, &p));
    }

    #[test]
    fn tableau_wrong_rank_difference_is_invalid() {
        let c = card(Suit::Hearts, Rank::Eight);
        let p = pile_with(PileType::Tableau(0), vec![card(Suit::Spades, Rank::Ten)]);
        assert!(!can_place_on_tableau(&c, &p));
    }

    #[test]
    fn tableau_black_on_red_one_lower_is_valid() {
        let c = card(Suit::Clubs, Rank::Six);
        let p = pile_with(PileType::Tableau(0), vec![card(Suit::Hearts, Rank::Seven)]);
        assert!(can_place_on_tableau(&c, &p));
    }
}
