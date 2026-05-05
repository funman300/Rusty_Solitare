use crate::card::Card;
use crate::pile::Pile;

/// Returns `true` if `card` can be placed on the foundation `pile`.
///
/// Foundation rules:
/// - When the pile is empty, any Ace is accepted; the placed Ace's suit
///   becomes the pile's claimed suit (derived from the bottom card via
///   [`Pile::claimed_suit`](crate::pile::Pile::claimed_suit)).
/// - When the pile is non-empty, the next card must match the top card's
///   suit and be exactly one rank higher.
pub fn can_place_on_foundation(card: &Card, pile: &Pile) -> bool {
    match pile.cards.last() {
        None => card.rank.value() == 1,
        Some(top) => card.suit == top.suit && card.rank.value() == top.rank.value() + 1,
    }
}

/// Returns `true` if `card` (or the bottom card of a sequence) can be placed on `pile` in the tableau.
///
/// Tableau rules: Kings go on empty piles; otherwise alternating colour, one rank lower.
pub fn can_place_on_tableau(card: &Card, pile: &Pile) -> bool {
    match pile.cards.last() {
        None => card.rank.value() == 13,
        Some(top) => {
            top.face_up
                && card.rank.value() + 1 == top.rank.value()
                && card.suit.is_red() != top.suit.is_red()
        }
    }
}

/// Returns `true` if `cards` is a legal tableau run on its own — every
/// adjacent pair descends by one rank and alternates colour. A single
/// card is trivially valid. The destination check is separate; this
/// only validates the sequence's *internal* structure, which the tableau
/// move path must enforce so a player can't smuggle an arbitrary stack
/// onto another column when the bottom card happens to land legally.
pub fn is_valid_tableau_sequence(cards: &[Card]) -> bool {
    cards.windows(2).all(|w| {
        w[0].rank.value() == w[1].rank.value() + 1 && w[0].suit.is_red() != w[1].suit.is_red()
    })
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
        // Every suit's Ace must land on an empty foundation slot regardless of
        // its slot index; the slot claims the suit only after the Ace lands.
        for suit in [Suit::Clubs, Suit::Diamonds, Suit::Hearts, Suit::Spades] {
            let c = card(suit, Rank::Ace);
            let p = Pile::new(PileType::Foundation(0));
            assert!(
                can_place_on_foundation(&c, &p),
                "Ace of {suit:?} must land on empty slot 0",
            );
        }
    }

    #[test]
    fn foundation_non_ace_on_empty_is_invalid() {
        let c = card(Suit::Hearts, Rank::Two);
        let p = Pile::new(PileType::Foundation(0));
        assert!(!can_place_on_foundation(&c, &p));
    }

    #[test]
    fn foundation_two_on_ace_same_suit_is_valid() {
        let c = card(Suit::Clubs, Rank::Two);
        let p = pile_with(PileType::Foundation(0), vec![card(Suit::Clubs, Rank::Ace)]);
        assert!(can_place_on_foundation(&c, &p));
    }

    #[test]
    fn foundation_second_card_must_match_claimed_suit() {
        // Place Ace of Hearts on slot 0, then attempt 2 of Spades — rejected
        // because the slot's claimed suit is Hearts after the Ace lands.
        let p = pile_with(PileType::Foundation(0), vec![card(Suit::Hearts, Rank::Ace)]);
        let c = card(Suit::Spades, Rank::Two);
        assert!(!can_place_on_foundation(&c, &p));
    }

    #[test]
    fn foundation_skipping_rank_is_invalid() {
        let c = card(Suit::Diamonds, Rank::Three);
        let p = pile_with(PileType::Foundation(0), vec![card(Suit::Diamonds, Rank::Ace)]);
        assert!(!can_place_on_foundation(&c, &p));
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

    #[test]
    fn foundation_king_on_queen_completes_suit() {
        // The last card placed to complete a foundation is always King on Queen.
        let c = card(Suit::Spades, Rank::King);
        let p = pile_with(PileType::Foundation(0), vec![card(Suit::Spades, Rank::Queen)]);
        assert!(can_place_on_foundation(&c, &p));
    }

    #[test]
    fn foundation_king_wrong_suit_is_invalid() {
        // King of Hearts cannot go on a Spades-claimed foundation even if rank matches.
        let c = card(Suit::Hearts, Rank::King);
        let p = pile_with(PileType::Foundation(0), vec![card(Suit::Spades, Rank::Queen)]);
        assert!(!can_place_on_foundation(&c, &p));
    }

    #[test]
    fn tableau_ace_on_two_different_color_is_valid() {
        // Ace (rank 1) can be placed on a Two of the opposite colour in the tableau.
        // rank check: Ace.value() + 1 = 2 == Two.value() — passes.
        let c = card(Suit::Hearts, Rank::Ace);
        let p = pile_with(PileType::Tableau(0), vec![card(Suit::Spades, Rank::Two)]);
        assert!(can_place_on_tableau(&c, &p));
    }

    #[test]
    fn tableau_same_rank_different_color_is_invalid() {
        // Two cards of the same rank cannot be stacked regardless of colour.
        let c = card(Suit::Hearts, Rank::Nine);
        let p = pile_with(PileType::Tableau(0), vec![card(Suit::Spades, Rank::Nine)]);
        assert!(!can_place_on_tableau(&c, &p));
    }

    #[test]
    fn tableau_face_down_destination_top_is_invalid() {
        // A face-down top card must never be a valid placement target.
        let c = card(Suit::Hearts, Rank::Nine);
        let mut top = card(Suit::Spades, Rank::Ten);
        top.face_up = false;
        let p = pile_with(PileType::Tableau(0), vec![top]);
        assert!(!can_place_on_tableau(&c, &p));
    }

    #[test]
    fn tableau_sequence_validation() {
        // Single card is trivially a valid sequence.
        assert!(is_valid_tableau_sequence(&[card(Suit::Hearts, Rank::Five)]));
        // Valid descending alternating-colour run K♠ Q♥ J♣.
        assert!(is_valid_tableau_sequence(&[
            card(Suit::Spades, Rank::King),
            card(Suit::Hearts, Rank::Queen),
            card(Suit::Clubs, Rank::Jack),
        ]));
        // Same colour twice (Q♠ on K♠) — invalid.
        assert!(!is_valid_tableau_sequence(&[
            card(Suit::Spades, Rank::King),
            card(Suit::Spades, Rank::Queen),
        ]));
        // Rank gap (K♠ → J♥) — invalid.
        assert!(!is_valid_tableau_sequence(&[
            card(Suit::Spades, Rank::King),
            card(Suit::Hearts, Rank::Jack),
        ]));
    }
}
