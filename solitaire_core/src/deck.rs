use rand::{seq::SliceRandom, SeedableRng};
use rand::rngs::StdRng;
use crate::card::{Card, Rank, Suit};
use crate::pile::{Pile, PileType};

const ALL_SUITS: [Suit; 4] = [Suit::Clubs, Suit::Diamonds, Suit::Hearts, Suit::Spades];
const ALL_RANKS: [Rank; 13] = [
    Rank::Ace, Rank::Two, Rank::Three, Rank::Four, Rank::Five,
    Rank::Six, Rank::Seven, Rank::Eight, Rank::Nine, Rank::Ten,
    Rank::Jack, Rank::Queen, Rank::King,
];

/// A standard 52-card deck.
pub struct Deck {
    pub cards: Vec<Card>,
}

impl Deck {
    /// Creates an unshuffled deck with all 52 unique cards (id 0–51).
    pub fn new() -> Self {
        let mut cards = Vec::with_capacity(52);
        let mut id = 0u32;
        for &suit in &ALL_SUITS {
            for &rank in &ALL_RANKS {
                cards.push(Card { id, suit, rank, face_up: false });
                id += 1;
            }
        }
        Self { cards }
    }

    /// Shuffles the deck in-place using Fisher-Yates with a seeded `SmallRng`.
    /// The same seed always produces the same order on any platform.
    pub fn shuffle(&mut self, seed: u64) {
        let mut rng = StdRng::seed_from_u64(seed);
        self.cards.shuffle(&mut rng);
    }
}

impl Default for Deck {
    fn default() -> Self {
        Self::new()
    }
}

/// Deals a standard Klondike layout from a pre-shuffled deck.
///
/// Returns 7 tableau piles and the remaining stock pile.
/// Column `i` contains `i + 1` cards; only the top card is face-up.
/// Stock receives the remaining 24 cards, all face-down.
pub fn deal_klondike(deck: Deck) -> ([Pile; 7], Pile) {
    let mut tableau: [Pile; 7] = core::array::from_fn(|i| Pile::new(PileType::Tableau(i)));
    let mut cards = deck.cards.into_iter();

    for (col, pile) in tableau.iter_mut().enumerate() {
        for row in 0..=col {
            let mut card = cards.next().expect("deck has 52 cards");
            card.face_up = row == col;
            pile.cards.push(card);
        }
    }

    let mut stock = Pile::new(PileType::Stock);
    stock.cards.extend(cards);
    (tableau, stock)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deck_new_has_52_cards() {
        assert_eq!(Deck::new().cards.len(), 52);
    }

    #[test]
    fn deck_new_has_unique_ids() {
        let deck = Deck::new();
        let mut ids: Vec<u32> = deck.cards.iter().map(|c| c.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 52);
    }

    #[test]
    fn deck_new_has_all_suits_and_ranks() {
        let deck = Deck::new();
        for suit in ALL_SUITS {
            for rank in ALL_RANKS {
                assert!(
                    deck.cards.iter().any(|c| c.suit == suit && c.rank == rank),
                    "missing {rank:?} {suit:?}"
                );
            }
        }
    }

    #[test]
    fn same_seed_produces_same_order() {
        let mut d1 = Deck::new(); d1.shuffle(42);
        let mut d2 = Deck::new(); d2.shuffle(42);
        assert_eq!(d1.cards, d2.cards);
    }

    #[test]
    fn different_seeds_produce_different_orders() {
        let mut d1 = Deck::new(); d1.shuffle(1);
        let mut d2 = Deck::new(); d2.shuffle(2);
        assert_ne!(d1.cards, d2.cards);
    }

    #[test]
    fn deal_klondike_correct_tableau_sizes() {
        let mut deck = Deck::new(); deck.shuffle(0);
        let (tableau, stock) = deal_klondike(deck);
        for (i, pile) in tableau.iter().enumerate() {
            assert_eq!(pile.cards.len(), i + 1, "col {i} wrong size");
        }
        assert_eq!(stock.cards.len(), 24);
    }

    #[test]
    fn deal_klondike_top_cards_are_face_up() {
        let mut deck = Deck::new(); deck.shuffle(0);
        let (tableau, _) = deal_klondike(deck);
        for pile in &tableau {
            assert!(pile.cards.last().unwrap().face_up);
        }
    }

    #[test]
    fn deal_klondike_non_top_cards_are_face_down() {
        let mut deck = Deck::new(); deck.shuffle(0);
        let (tableau, _) = deal_klondike(deck);
        for pile in &tableau {
            for card in &pile.cards[..pile.cards.len().saturating_sub(1)] {
                assert!(!card.face_up);
            }
        }
    }

    #[test]
    fn deal_klondike_stock_is_face_down() {
        let mut deck = Deck::new(); deck.shuffle(0);
        let (_, stock) = deal_klondike(deck);
        assert!(stock.cards.iter().all(|c| !c.face_up));
    }

    #[test]
    fn deal_klondike_all_52_cards_present() {
        let mut deck = Deck::new(); deck.shuffle(99);
        let (tableau, stock) = deal_klondike(deck);
        let mut ids: Vec<u32> = stock.cards.iter().map(|c| c.id).collect();
        for pile in &tableau { ids.extend(pile.cards.iter().map(|c| c.id)); }
        ids.sort_unstable();
        assert_eq!(ids, (0u32..52).collect::<Vec<_>>());
    }
}
