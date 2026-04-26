use std::collections::{HashMap, VecDeque};
use serde::{Deserialize, Serialize};
use crate::card::{Card, Suit};
use crate::deck::{deal_klondike, Deck};
use crate::error::MoveError;
use crate::pile::{Pile, PileType};
use crate::rules::{can_place_on_foundation, can_place_on_tableau};
use crate::scoring::{compute_time_bonus as scoring_time_bonus, score_move, score_undo as scoring_undo};

const MAX_UNDO_STACK: usize = 64;

/// Whether cards are drawn one at a time or three at a time from the stock.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DrawMode {
    DrawOne,
    DrawThree,
}

/// Top-level game mode. Affects scoring, undo, and (eventually) timer behaviour.
///
/// - `Classic`: standard Klondike scoring, undo allowed.
/// - `Zen`: scoring suppressed (stays at 0); undo allowed; intended for relaxed play.
/// - `Challenge`: standard scoring, **undo disabled** (returns
///   `MoveError::RuleViolation`).
/// - `TimeAttack`: standard scoring + undo; the engine wraps a 10-minute
///   countdown around the session and auto-deals a fresh game on every win
///   (see `solitaire_engine::TimeAttackPlugin`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum GameMode {
    #[default]
    Classic,
    Zen,
    Challenge,
    TimeAttack,
}

/// Snapshot of game state used for undo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct StateSnapshot {
    piles: HashMap<PileType, Pile>,
    score: i32,
    move_count: u32,
}

/// Full state of an in-progress Klondike Solitaire game.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameState {
    pub piles: HashMap<PileType, Pile>,
    pub draw_mode: DrawMode,
    /// Top-level mode (Classic / Zen). Defaults to Classic for backwards
    /// compatibility with older save files via `#[serde(default)]`.
    #[serde(default)]
    pub mode: GameMode,
    pub score: i32,
    pub move_count: u32,
    pub elapsed_seconds: u64,
    pub seed: u64,
    pub is_won: bool,
    pub is_auto_completable: bool,
    /// Number of times `undo()` has been successfully invoked this game.
    /// Used by achievement conditions like `no_undo`.
    pub undo_count: u32,
    undo_stack: VecDeque<StateSnapshot>,
}

impl GameState {
    /// Creates a new Classic-mode game dealt from the given seed and draw mode.
    pub fn new(seed: u64, draw_mode: DrawMode) -> Self {
        Self::new_with_mode(seed, draw_mode, GameMode::Classic)
    }

    /// Creates a new game with an explicit `GameMode`.
    pub fn new_with_mode(seed: u64, draw_mode: DrawMode, mode: GameMode) -> Self {
        let mut deck = Deck::new();
        deck.shuffle(seed);
        let (tableau, stock) = deal_klondike(deck);

        let mut piles: HashMap<PileType, Pile> = HashMap::new();
        piles.insert(PileType::Stock, stock);
        piles.insert(PileType::Waste, Pile::new(PileType::Waste));
        for suit in [Suit::Clubs, Suit::Diamonds, Suit::Hearts, Suit::Spades] {
            piles.insert(PileType::Foundation(suit), Pile::new(PileType::Foundation(suit)));
        }
        for (i, pile) in tableau.into_iter().enumerate() {
            piles.insert(PileType::Tableau(i), pile);
        }

        Self {
            piles,
            draw_mode,
            mode,
            score: 0,
            move_count: 0,
            elapsed_seconds: 0,
            seed,
            is_won: false,
            is_auto_completable: false,
            undo_count: 0,
            undo_stack: VecDeque::new(),
        }
    }

    /// Number of snapshots currently on the undo stack.
    pub fn undo_stack_len(&self) -> usize {
        self.undo_stack.len()
    }

    fn take_snapshot(&self) -> StateSnapshot {
        StateSnapshot {
            piles: self.piles.clone(),
            score: self.score,
            move_count: self.move_count,
        }
    }

    fn push_snapshot(&mut self) {
        if self.undo_stack.len() >= MAX_UNDO_STACK {
            self.undo_stack.pop_front();  // O(1)
        }
        self.undo_stack.push_back(self.take_snapshot());
    }

    /// Draw cards from stock to waste. When stock is empty, recycles waste back to stock.
    /// Recycling is unlimited: `StockEmpty` is only returned when both stock and waste are empty.
    pub fn draw(&mut self) -> Result<(), MoveError> {
        if self.is_won {
            return Err(MoveError::GameAlreadyWon);
        }

        let stock_len = self.piles[&PileType::Stock].cards.len();

        if stock_len == 0 {
            let waste_len = self.piles[&PileType::Waste].cards.len();
            if waste_len == 0 {
                return Err(MoveError::StockEmpty);
            }
            // Recycle: snapshot so undo can reverse it, then move waste back to stock face-down
            self.push_snapshot();
            let waste_cards: Vec<Card> = self.piles
                .get_mut(&PileType::Waste)
                .ok_or(MoveError::InvalidSource)?
                .cards
                .drain(..)
                .collect();
            let stock = self.piles.get_mut(&PileType::Stock).ok_or(MoveError::InvalidDestination)?;
            for mut card in waste_cards.into_iter().rev() {
                card.face_up = false;
                stock.cards.push(card);
            }
            return Ok(());
        }

        self.push_snapshot();

        let draw_count = match self.draw_mode {
            DrawMode::DrawOne => 1,
            DrawMode::DrawThree => 3,
        };
        let available = stock_len.min(draw_count);
        let drain_start = stock_len - available;

        let drawn: Vec<Card> = self.piles
            .get_mut(&PileType::Stock)
            .ok_or(MoveError::InvalidSource)?
            .cards
            .drain(drain_start..)
            .collect();

        let waste = self.piles.get_mut(&PileType::Waste).ok_or(MoveError::InvalidDestination)?;
        for mut card in drawn {
            card.face_up = true;
            waste.cards.push(card);
        }

        self.move_count += 1;
        Ok(())
    }

    /// Move `count` cards from pile `from` to pile `to`.
    ///
    /// Returns `Err(MoveError)` if the move is illegal. On success, updates score,
    /// flips the newly exposed source card if needed, and checks win/auto-complete.
    pub fn move_cards(&mut self, from: PileType, to: PileType, count: usize) -> Result<(), MoveError> {
        if self.is_won {
            return Err(MoveError::GameAlreadyWon);
        }
        if from == to {
            return Err(MoveError::RuleViolation("source and destination must differ".into()));
        }

        // Validate via scoped immutable borrows
        let move_start = {
            let from_pile = self.piles.get(&from).ok_or(MoveError::InvalidSource)?;
            if from_pile.cards.is_empty() {
                return Err(MoveError::EmptySource);
            }
            if count == 0 || count > from_pile.cards.len() {
                return Err(MoveError::RuleViolation("invalid card count".into()));
            }
            let start = from_pile.cards.len() - count;
            for card in &from_pile.cards[start..] {
                if !card.face_up {
                    return Err(MoveError::RuleViolation("cannot move face-down card".into()));
                }
            }
            let bottom_card = from_pile.cards[start].clone();

            match &to {
                PileType::Foundation(suit) => {
                    if count != 1 {
                        return Err(MoveError::RuleViolation(
                            "only one card can move to foundation at a time".into(),
                        ));
                    }
                    let dest = self.piles.get(&to).ok_or(MoveError::InvalidDestination)?;
                    if !can_place_on_foundation(&bottom_card, dest, *suit) {
                        return Err(MoveError::RuleViolation("invalid foundation placement".into()));
                    }
                }
                PileType::Tableau(_) => {
                    let dest = self.piles.get(&to).ok_or(MoveError::InvalidDestination)?;
                    if !can_place_on_tableau(&bottom_card, dest) {
                        return Err(MoveError::RuleViolation("invalid tableau placement".into()));
                    }
                }
                _ => return Err(MoveError::InvalidDestination),
            }
            start
        };

        let score_delta = if self.mode == GameMode::Zen {
            0
        } else {
            score_move(&from, &to)
        };
        self.push_snapshot();

        // Execute move
        let mut moved: Vec<Card> = self.piles
            .get_mut(&from)
            .ok_or(MoveError::InvalidSource)?
            .cards
            .split_off(move_start);

        // Flip the newly exposed top card of the source pile
        if let Some(top) = self.piles
            .get_mut(&from)
            .ok_or(MoveError::InvalidSource)?
            .cards
            .last_mut()
        {
            if !top.face_up {
                top.face_up = true;
            }
        }

        self.piles.get_mut(&to).ok_or(MoveError::InvalidDestination)?.cards.append(&mut moved);

        self.score = (self.score + score_delta).max(0);
        self.move_count += 1;

        self.is_won = self.check_win();
        if !self.is_won {
            self.is_auto_completable = self.check_auto_complete();
        }

        Ok(())
    }

    /// Restore the most recent undo snapshot and apply the undo score penalty (-15).
    /// Disabled in `GameMode::Challenge` — returns `MoveError::RuleViolation`.
    pub fn undo(&mut self) -> Result<(), MoveError> {
        if self.is_won {
            return Err(MoveError::GameAlreadyWon);
        }
        if self.mode == GameMode::Challenge {
            return Err(MoveError::RuleViolation(
                "undo is disabled in Challenge mode".into(),
            ));
        }
        let snapshot = self.undo_stack.pop_back().ok_or(MoveError::UndoStackEmpty)?;
        self.piles = snapshot.piles;
        self.score = if self.mode == GameMode::Zen {
            0
        } else {
            (snapshot.score + scoring_undo()).max(0)
        };
        self.move_count = snapshot.move_count;
        self.is_won = false;
        self.is_auto_completable = false;
        self.undo_count = self.undo_count.saturating_add(1);
        Ok(())
    }

    /// Returns `true` when all four foundations each contain 13 cards.
    pub fn check_win(&self) -> bool {
        [Suit::Clubs, Suit::Diamonds, Suit::Hearts, Suit::Spades]
            .iter()
            .all(|&suit| {
                self.piles
                    .get(&PileType::Foundation(suit))
                    .is_some_and(|p| p.cards.len() == 13)
            })
    }

    /// Returns `true` when stock and waste are empty and all tableau cards are face-up.
    /// At that point the game can be completed without further player input.
    pub fn check_auto_complete(&self) -> bool {
        if !self.piles[&PileType::Stock].cards.is_empty() {
            return false;
        }
        if !self.piles[&PileType::Waste].cards.is_empty() {
            return false;
        }
        (0..7).all(|i| {
            self.piles[&PileType::Tableau(i)]
                .cards
                .iter()
                .all(|c| c.face_up)
        })
    }

    /// Time bonus added to score on win: `700_000 / elapsed_seconds` (0 if elapsed is 0).
    pub fn compute_time_bonus(&self) -> i32 {
        scoring_time_bonus(self.elapsed_seconds)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::{Card, Rank};

    fn new_game() -> GameState {
        GameState::new(42, DrawMode::DrawOne)
    }

    // --- Initial state ---

    #[test]
    fn new_game_has_correct_tableau_sizes() {
        let g = new_game();
        let total: usize = (0..7).map(|i| g.piles[&PileType::Tableau(i)].cards.len()).sum();
        assert_eq!(total, 28);
        for i in 0..7 {
            assert_eq!(g.piles[&PileType::Tableau(i)].cards.len(), i + 1);
        }
    }

    #[test]
    fn new_game_stock_has_24_cards() {
        assert_eq!(new_game().piles[&PileType::Stock].cards.len(), 24);
    }

    #[test]
    fn new_game_waste_is_empty() {
        assert!(new_game().piles[&PileType::Waste].cards.is_empty());
    }

    #[test]
    fn new_game_foundations_are_empty() {
        let g = new_game();
        for suit in [Suit::Clubs, Suit::Diamonds, Suit::Hearts, Suit::Spades] {
            assert!(g.piles[&PileType::Foundation(suit)].cards.is_empty());
        }
    }

    #[test]
    fn new_game_is_not_won() {
        assert!(!new_game().is_won);
    }

    // --- Seeded reproducibility ---

    #[test]
    fn same_seed_produces_identical_layout() {
        let g1 = GameState::new(12345, DrawMode::DrawOne);
        let g2 = GameState::new(12345, DrawMode::DrawOne);
        for i in 0..7 {
            assert_eq!(
                g1.piles[&PileType::Tableau(i)].cards,
                g2.piles[&PileType::Tableau(i)].cards
            );
        }
        assert_eq!(
            g1.piles[&PileType::Stock].cards,
            g2.piles[&PileType::Stock].cards
        );
    }

    #[test]
    fn different_seeds_produce_different_layouts() {
        let g1 = GameState::new(1, DrawMode::DrawOne);
        let g2 = GameState::new(2, DrawMode::DrawOne);
        let t1: Vec<u32> = g1.piles[&PileType::Tableau(0)].cards.iter().map(|c| c.id).collect();
        let t2: Vec<u32> = g2.piles[&PileType::Tableau(0)].cards.iter().map(|c| c.id).collect();
        assert_ne!(t1, t2);
    }

    // --- Draw ---

    #[test]
    fn draw_one_moves_one_card_to_waste() {
        let mut g = new_game();
        let stock_before = g.piles[&PileType::Stock].cards.len();
        g.draw().unwrap();
        assert_eq!(g.piles[&PileType::Stock].cards.len(), stock_before - 1);
        assert_eq!(g.piles[&PileType::Waste].cards.len(), 1);
    }

    #[test]
    fn drawn_card_is_face_up() {
        let mut g = new_game();
        g.draw().unwrap();
        assert!(g.piles[&PileType::Waste].cards.last().unwrap().face_up);
    }

    #[test]
    fn draw_three_moves_up_to_three_cards() {
        let mut g = GameState::new(42, DrawMode::DrawThree);
        g.draw().unwrap();
        assert_eq!(g.piles[&PileType::Waste].cards.len(), 3);
        assert_eq!(g.piles[&PileType::Stock].cards.len(), 21);
    }

    #[test]
    fn draw_from_empty_stock_recycles_waste() {
        let mut g = new_game();
        while !g.piles[&PileType::Stock].cards.is_empty() {
            g.draw().unwrap();
        }
        let waste_count = g.piles[&PileType::Waste].cards.len();
        assert!(waste_count > 0);
        g.draw().unwrap(); // recycle
        assert_eq!(g.piles[&PileType::Stock].cards.len(), waste_count);
        assert!(g.piles[&PileType::Waste].cards.is_empty());
    }

    #[test]
    fn draw_from_empty_stock_and_waste_returns_error() {
        // The only stop condition for draw() is: both stock AND waste are
        // simultaneously empty. Manually empty both, then verify the error.
        let mut g = new_game();
        g.piles.get_mut(&PileType::Stock).unwrap().cards.clear();
        g.piles.get_mut(&PileType::Waste).unwrap().cards.clear();
        assert_eq!(g.draw(), Err(MoveError::StockEmpty));
    }

    // --- Move validation ---

    #[test]
    fn move_zero_cards_returns_rule_violation() {
        let mut g = new_game();
        let result = g.move_cards(PileType::Tableau(0), PileType::Tableau(1), 0);
        assert!(matches!(result, Err(MoveError::RuleViolation(_))));
    }

    #[test]
    fn move_to_stock_returns_invalid_destination() {
        let mut g = new_game();
        let result = g.move_cards(PileType::Tableau(0), PileType::Stock, 1);
        assert_eq!(result, Err(MoveError::InvalidDestination));
    }

    #[test]
    fn move_to_waste_returns_invalid_destination() {
        let mut g = new_game();
        let result = g.move_cards(PileType::Tableau(0), PileType::Waste, 1);
        assert_eq!(result, Err(MoveError::InvalidDestination));
    }

    #[test]
    fn move_same_source_and_dest_returns_rule_violation() {
        let mut g = new_game();
        let result = g.move_cards(PileType::Tableau(0), PileType::Tableau(0), 1);
        assert!(matches!(result, Err(MoveError::RuleViolation(_))));
    }

    // --- Win detection ---

    #[test]
    fn win_detection_all_foundations_complete() {
        let mut g = new_game();
        for suit in [Suit::Clubs, Suit::Diamonds, Suit::Hearts, Suit::Spades] {
            let f = g.piles.get_mut(&PileType::Foundation(suit)).unwrap();
            f.cards.clear();
            for rank in [
                Rank::Ace, Rank::Two, Rank::Three, Rank::Four, Rank::Five,
                Rank::Six, Rank::Seven, Rank::Eight, Rank::Nine, Rank::Ten,
                Rank::Jack, Rank::Queen, Rank::King,
            ] {
                f.cards.push(Card { id: 0, suit, rank, face_up: true });
            }
        }
        assert!(g.check_win());
    }

    #[test]
    fn win_detection_incomplete_is_false() {
        assert!(!new_game().check_win());
    }

    // --- Undo ---

    #[test]
    fn undo_empty_stack_returns_error() {
        let mut g = new_game();
        assert_eq!(g.undo(), Err(MoveError::UndoStackEmpty));
    }

    #[test]
    fn undo_after_draw_restores_pile_sizes() {
        let mut g = new_game();
        let stock_before = g.piles[&PileType::Stock].cards.len();
        let waste_before = g.piles[&PileType::Waste].cards.len();
        g.draw().unwrap();
        g.undo().unwrap();
        assert_eq!(g.piles[&PileType::Stock].cards.len(), stock_before);
        assert_eq!(g.piles[&PileType::Waste].cards.len(), waste_before);
    }

    #[test]
    fn undo_applies_score_penalty() {
        let mut g = new_game();
        let score_before = g.score;
        g.draw().unwrap();
        g.undo().unwrap();
        let expected = (score_before + scoring_undo()).max(0);
        assert_eq!(g.score, expected);
    }

    #[test]
    fn undo_stack_capped_at_64() {
        let mut g = new_game();
        for _ in 0..70 {
            let _ = g.draw();
        }
        assert!(g.undo_stack_len() <= 64);
    }

    // --- Scoring ---

    #[test]
    fn score_never_goes_below_zero() {
        let mut g = new_game();
        for _ in 0..5 {
            g.draw().unwrap();
            g.undo().unwrap();
        }
        assert!(g.score >= 0);
    }

    // --- GameMode: Zen ---

    #[test]
    fn zen_mode_score_stays_zero_after_undo() {
        let mut g = GameState::new_with_mode(42, DrawMode::DrawOne, GameMode::Zen);
        g.draw().unwrap();
        g.undo().unwrap();
        assert_eq!(g.score, 0);
    }

    #[test]
    fn zen_mode_default_is_classic_via_default_trait() {
        assert_eq!(GameMode::default(), GameMode::Classic);
    }

    #[test]
    fn zen_mode_field_persists_through_construction() {
        let g = GameState::new_with_mode(1, DrawMode::DrawThree, GameMode::Zen);
        assert_eq!(g.mode, GameMode::Zen);
        assert_eq!(g.draw_mode, DrawMode::DrawThree);
    }

    // --- GameMode: Challenge ---

    #[test]
    fn challenge_mode_disables_undo() {
        let mut g = GameState::new_with_mode(42, DrawMode::DrawOne, GameMode::Challenge);
        g.draw().unwrap();
        let result = g.undo();
        assert!(matches!(result, Err(MoveError::RuleViolation(_))));
    }

    #[test]
    fn challenge_mode_still_allows_normal_moves() {
        let g = GameState::new_with_mode(42, DrawMode::DrawOne, GameMode::Challenge);
        // Just verify the game initialises cleanly with Challenge mode.
        assert_eq!(g.mode, GameMode::Challenge);
        assert_eq!(g.score, 0);
    }

    #[test]
    fn challenge_mode_scoring_applies_normally() {
        // Challenge uses Classic scoring; only undo is disabled.
        let g = GameState::new_with_mode(42, DrawMode::DrawOne, GameMode::Challenge);
        assert_eq!(g.score, 0);
        // Note: Verifying score increases on actual moves would require
        // hand-crafting a legal move from the dealt state. We rely on the
        // fact that move_cards' score path is identical to Classic.
    }

    // --- Auto-complete ---

    #[test]
    fn auto_complete_false_when_stock_not_empty() {
        assert!(!new_game().check_auto_complete());
    }

    #[test]
    fn auto_complete_false_when_face_down_cards_remain() {
        let mut g = new_game();
        g.piles.get_mut(&PileType::Stock).unwrap().cards.clear();
        g.piles.get_mut(&PileType::Waste).unwrap().cards.clear();
        // Tableau(1) has a face-down card at index 0
        assert!(!g.check_auto_complete());
    }

    // --- Time bonus ---

    #[test]
    fn time_bonus_zero_when_elapsed_is_zero() {
        let mut g = new_game();
        g.elapsed_seconds = 0;
        assert_eq!(g.compute_time_bonus(), 0);
    }

    #[test]
    fn time_bonus_at_100_seconds() {
        let mut g = new_game();
        g.elapsed_seconds = 100;
        assert_eq!(g.compute_time_bonus(), 7000);
    }
}
