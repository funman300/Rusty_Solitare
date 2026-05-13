//! WebAssembly bindings for browser-side replay playback and interactive gameplay.
//!
//! The web replay player at `<server>/replays/<id>` fetches a [`Replay`]
//! JSON via `GET /api/replays/:id`, hands it to [`ReplayPlayer::new`],
//! and then advances frame-by-frame with [`ReplayPlayer::step`]. Each
//! step applies one [`ReplayMove`] to the underlying `GameState` and
//! returns the resulting pile snapshot as JSON for the JS layer to
//! render.
//!
//! The state machine is the same Rust [`solitaire_core::GameState`]
//! the desktop client uses, so the two implementations cannot drift —
//! same seed + same input list = same pile state at every step,
//! regardless of which platform replays the game.
//!
//! The crate intentionally does **not** depend on `solitaire_data`
//! (which pulls `dirs`, `keyring`, `reqwest`, and other non-wasm
//! crates) — instead it defines a minimal `Replay` mirror with the
//! same serde shape as `solitaire_data::Replay`. The JSON wire format
//! is the contract.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use solitaire_core::card::Suit;
use solitaire_core::game_state::{DrawMode, GameMode, GameState};
use solitaire_core::pile::PileType;
use wasm_bindgen::prelude::*;

/// Mirrors the variants of `solitaire_data::ReplayMove` v2 (atomic
/// player inputs, post-StockClick refinement). Only the JSON shape
/// matters for cross-crate compatibility.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReplayMove {
    Move {
        from: PileType,
        to: PileType,
        count: usize,
    },
    StockClick,
}

/// Mirrors `solitaire_data::Replay` v2.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Replay {
    #[serde(default)]
    pub schema_version: u32,
    pub seed: u64,
    pub draw_mode: DrawMode,
    pub mode: GameMode,
    pub time_seconds: u64,
    pub final_score: i32,
    pub recorded_at: NaiveDate,
    pub moves: Vec<ReplayMove>,
}

/// JS-friendly snapshot of a `GameState` at a particular replay step.
#[derive(Debug, Clone, Serialize)]
pub struct StateSnapshot {
    pub step_idx: usize,
    pub total_steps: usize,
    pub score: i32,
    pub move_count: u32,
    pub is_won: bool,
    pub stock: Vec<CardSnapshot>,
    pub waste: Vec<CardSnapshot>,
    /// Length 4 — one per foundation slot, in slot order (0..=3). The
    /// claimed suit (if any) is the bottom card's suit.
    pub foundations: [Vec<CardSnapshot>; 4],
    /// Length 7 — one per tableau column (0..=6).
    pub tableaus: [Vec<CardSnapshot>; 7],
}

/// One card, projected for the JS card renderer. `face_up = false`
/// means the card back is drawn; in that case `suit` and `rank` are
/// still set (so the renderer doesn't need separate "unknown" data),
/// just hidden visually.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct CardSnapshot {
    pub id: u32,
    /// `"clubs" | "diamonds" | "hearts" | "spades"`.
    pub suit: &'static str,
    /// 1-13, where 1 is Ace and 13 is King.
    pub rank: u8,
    pub face_up: bool,
}

impl From<&solitaire_core::card::Card> for CardSnapshot {
    fn from(c: &solitaire_core::card::Card) -> Self {
        Self {
            id: c.id,
            suit: match c.suit {
                Suit::Clubs => "clubs",
                Suit::Diamonds => "diamonds",
                Suit::Hearts => "hearts",
                Suit::Spades => "spades",
            },
            rank: c.rank.value(),
            face_up: c.face_up,
        }
    }
}

/// Browser-side replay state machine. Owns a live `GameState` and the
/// replay's move list; each `step()` applies the next move.
#[wasm_bindgen]
pub struct ReplayPlayer {
    game: GameState,
    moves: Vec<ReplayMove>,
    step_idx: usize,
}

// Native-callable methods. Used by both the wasm-bindgen interface
// below and by unit tests, which can't go through `serde_wasm_bindgen`
// (it panics on non-wasm targets).
impl ReplayPlayer {
    /// Construct from a raw replay JSON string. Returns the parsing
    /// error as a `String` so the wasm-bindgen wrapper can convert
    /// it to a `JsValue` and tests can assert on it directly.
    pub fn from_json(replay_json: &str) -> Result<Self, String> {
        let replay: Replay =
            serde_json::from_str(replay_json).map_err(|e| format!("invalid replay JSON: {e}"))?;
        let game =
            GameState::new_with_mode(replay.seed, replay.draw_mode.clone(), replay.mode);
        Ok(Self {
            game,
            moves: replay.moves,
            step_idx: 0,
        })
    }

    /// Apply the next move. Returns `None` once the list is exhausted.
    pub fn step_native(&mut self) -> Option<StateSnapshot> {
        if self.step_idx >= self.moves.len() {
            return None;
        }
        let mv = self.moves[self.step_idx].clone();
        let _ = match mv {
            ReplayMove::Move { from, to, count } => self.game.move_cards(from, to, count),
            ReplayMove::StockClick => self.game.draw(),
        };
        self.step_idx += 1;
        Some(self.snapshot())
    }

    fn snapshot(&self) -> StateSnapshot {
        let pile_cards = |t: PileType| -> Vec<CardSnapshot> {
            self.game
                .piles
                .get(&t)
                .map(|p| p.cards.iter().map(CardSnapshot::from).collect())
                .unwrap_or_default()
        };
        let foundations: [Vec<CardSnapshot>; 4] = [
            pile_cards(PileType::Foundation(0)),
            pile_cards(PileType::Foundation(1)),
            pile_cards(PileType::Foundation(2)),
            pile_cards(PileType::Foundation(3)),
        ];
        let tableaus: [Vec<CardSnapshot>; 7] = [
            pile_cards(PileType::Tableau(0)),
            pile_cards(PileType::Tableau(1)),
            pile_cards(PileType::Tableau(2)),
            pile_cards(PileType::Tableau(3)),
            pile_cards(PileType::Tableau(4)),
            pile_cards(PileType::Tableau(5)),
            pile_cards(PileType::Tableau(6)),
        ];
        StateSnapshot {
            step_idx: self.step_idx,
            total_steps: self.moves.len(),
            score: self.game.score,
            move_count: self.game.move_count,
            is_won: self.game.is_won,
            stock: pile_cards(PileType::Stock),
            waste: pile_cards(PileType::Waste),
            foundations,
            tableaus,
        }
    }
}

// JS-facing surface. Thin wrapper around the native API: serialises
// `StateSnapshot` to `JsValue` via `serde_wasm_bindgen` and converts
// `String` errors to `JsValue` strings. Native unit tests bypass this
// layer because `serde_wasm_bindgen::to_value` panics off-target.
#[wasm_bindgen]
impl ReplayPlayer {
    /// Construct from a raw replay JSON string.
    #[wasm_bindgen(constructor)]
    pub fn new(replay_json: &str) -> Result<ReplayPlayer, JsValue> {
        #[cfg(feature = "console_error_panic_hook")]
        console_error_panic_hook::set_once();
        Self::from_json(replay_json).map_err(|e| JsValue::from_str(&e))
    }

    /// Snapshot the current `GameState` as a JS object (see `StateSnapshot`).
    pub fn state(&self) -> JsValue {
        serde_wasm_bindgen::to_value(&self.snapshot()).unwrap_or(JsValue::NULL)
    }

    /// Apply the next move; returns the post-step snapshot, or `null`
    /// once the move list is exhausted.
    pub fn step(&mut self) -> JsValue {
        match self.step_native() {
            Some(snap) => serde_wasm_bindgen::to_value(&snap).unwrap_or(JsValue::NULL),
            None => JsValue::NULL,
        }
    }

    /// Total number of moves the replay contains.
    pub fn total_steps(&self) -> usize {
        self.moves.len()
    }

    /// 0-indexed position of the next move to apply.
    pub fn step_idx(&self) -> usize {
        self.step_idx
    }

    /// Returns `true` once every move has been applied.
    pub fn is_finished(&self) -> bool {
        self.step_idx >= self.moves.len()
    }
}

// ---------------------------------------------------------------------------
// Interactive game surface
// ---------------------------------------------------------------------------

/// Full snapshot of a live `SolitaireGame` for the JS renderer.
#[derive(Debug, Clone, Serialize)]
pub struct GameSnapshot {
    pub score: i32,
    pub move_count: u32,
    pub is_won: bool,
    pub is_auto_completable: bool,
    pub undo_count: u32,
    /// Number of snapshots currently on the undo stack; 0 means undo is unavailable.
    pub undo_stack_len: usize,
    pub stock: Vec<CardSnapshot>,
    pub waste: Vec<CardSnapshot>,
    pub foundations: [Vec<CardSnapshot>; 4],
    pub tableaus: [Vec<CardSnapshot>; 7],
}

/// Result returned to JS from every mutating game action.
#[derive(Debug, Clone, Serialize)]
pub struct ActionResult {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<GameSnapshot>,
}

/// Interactive Klondike game backed by the real `solitaire_core` rules engine.
///
/// Construct with `new(seed, draw_three)`, then call `draw()`, `move_cards()`,
/// `undo()`, `auto_complete_step()` to advance the game. `state()` returns the
/// full pile snapshot at any time without mutating state.
#[wasm_bindgen]
pub struct SolitaireGame {
    game: GameState,
}

impl SolitaireGame {
    fn snap(&self) -> GameSnapshot {
        let cards = |t: PileType| -> Vec<CardSnapshot> {
            self.game
                .piles
                .get(&t)
                .map(|p| p.cards.iter().map(CardSnapshot::from).collect())
                .unwrap_or_default()
        };
        GameSnapshot {
            score: self.game.score,
            move_count: self.game.move_count,
            is_won: self.game.is_won,
            is_auto_completable: self.game.is_auto_completable,
            undo_count: self.game.undo_count,
            undo_stack_len: self.game.undo_stack_len(),
            stock: cards(PileType::Stock),
            waste: cards(PileType::Waste),
            foundations: [
                cards(PileType::Foundation(0)),
                cards(PileType::Foundation(1)),
                cards(PileType::Foundation(2)),
                cards(PileType::Foundation(3)),
            ],
            tableaus: [
                cards(PileType::Tableau(0)),
                cards(PileType::Tableau(1)),
                cards(PileType::Tableau(2)),
                cards(PileType::Tableau(3)),
                cards(PileType::Tableau(4)),
                cards(PileType::Tableau(5)),
                cards(PileType::Tableau(6)),
            ],
        }
    }

    fn pile_from_str(s: &str) -> Result<PileType, String> {
        match s {
            "stock" => Ok(PileType::Stock),
            "waste" => Ok(PileType::Waste),
            _ if s.starts_with("foundation-") => {
                let slot: u8 = s["foundation-".len()..]
                    .parse()
                    .map_err(|_| format!("bad pile: {s}"))?;
                if slot >= 4 {
                    return Err(format!("foundation slot out of range: {slot}"));
                }
                Ok(PileType::Foundation(slot))
            }
            _ if s.starts_with("tableau-") => {
                let col: usize = s["tableau-".len()..]
                    .parse()
                    .map_err(|_| format!("bad pile: {s}"))?;
                if col >= 7 {
                    return Err(format!("tableau col out of range: {col}"));
                }
                Ok(PileType::Tableau(col))
            }
            _ => Err(format!("unknown pile: {s}")),
        }
    }

    fn ok_js(&self) -> JsValue {
        serde_wasm_bindgen::to_value(&ActionResult {
            ok: true,
            error: None,
            snapshot: Some(self.snap()),
        })
        .unwrap_or(JsValue::NULL)
    }

    fn err_js(msg: impl std::fmt::Display) -> JsValue {
        serde_wasm_bindgen::to_value(&ActionResult {
            ok: false,
            error: Some(msg.to_string()),
            snapshot: None,
        })
        .unwrap_or(JsValue::NULL)
    }
}

#[wasm_bindgen]
impl SolitaireGame {
    /// Create a new DrawOne or DrawThree Classic game from the given seed.
    ///
    /// `seed` is a JS `number` (f64); values up to 2^53 are represented exactly.
    /// Pass `Date.now()` or a random integer from JS for variety.
    #[wasm_bindgen(constructor)]
    pub fn new(seed: f64, draw_three: bool) -> SolitaireGame {
        #[cfg(feature = "console_error_panic_hook")]
        console_error_panic_hook::set_once();
        let dm = if draw_three {
            DrawMode::DrawThree
        } else {
            DrawMode::DrawOne
        };
        SolitaireGame {
            game: GameState::new_with_mode(seed as u64, dm, GameMode::Classic),
        }
    }

    /// Full pile snapshot as a JS object.
    pub fn state(&self) -> JsValue {
        serde_wasm_bindgen::to_value(&self.snap()).unwrap_or(JsValue::NULL)
    }

    /// The seed used to deal this game.
    pub fn seed(&self) -> f64 {
        self.game.seed as f64
    }

    /// Draw from stock to waste (or recycle waste → stock when stock is empty).
    /// Returns `{ok, error?, snapshot?}`.
    pub fn draw(&mut self) -> JsValue {
        match self.game.draw() {
            Ok(()) => self.ok_js(),
            Err(e) => Self::err_js(e),
        }
    }

    /// Move `count` cards from pile `from` to pile `to`.
    ///
    /// Pile names: `"stock"`, `"waste"`, `"foundation-0"` .. `"foundation-3"`,
    /// `"tableau-0"` .. `"tableau-6"`.
    ///
    /// Returns `{ok, error?, snapshot?}`.
    pub fn move_cards(&mut self, from: &str, to: &str, count: usize) -> JsValue {
        let from_pile = match Self::pile_from_str(from) {
            Ok(p) => p,
            Err(e) => return Self::err_js(e),
        };
        let to_pile = match Self::pile_from_str(to) {
            Ok(p) => p,
            Err(e) => return Self::err_js(e),
        };
        match self.game.move_cards(from_pile, to_pile, count) {
            Ok(()) => self.ok_js(),
            Err(e) => Self::err_js(e),
        }
    }

    /// Undo the last move. Returns `{ok, error?, snapshot?}`.
    pub fn undo(&mut self) -> JsValue {
        match self.game.undo() {
            Ok(()) => self.ok_js(),
            Err(e) => Self::err_js(e),
        }
    }

    /// Apply one auto-complete move (only valid when `is_auto_completable`).
    /// Returns the post-move snapshot or `null` when auto-complete is unavailable.
    pub fn auto_complete_step(&mut self) -> JsValue {
        if !self.game.is_auto_completable {
            return JsValue::NULL;
        }
        match self.game.next_auto_complete_move() {
            Some((from, to)) => {
                let _ = self.game.move_cards(from, to, 1);
                self.ok_js()
            }
            None => JsValue::NULL,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_replay_json() -> String {
        // Minimal v2 replay: seed 42, two stock clicks. Real winning
        // replays will have many more moves; for the test we just
        // verify deserialization + step() advances correctly.
        r#"{
            "schema_version": 2,
            "seed": 42,
            "draw_mode": "DrawOne",
            "mode": "Classic",
            "time_seconds": 60,
            "final_score": 100,
            "recorded_at": "2026-05-02",
            "moves": ["StockClick", "StockClick"]
        }"#
        .to_string()
    }

    /// Constructing from a valid v2 replay JSON must succeed and
    /// initialise step_idx to 0.
    #[test]
    fn new_initialises_step_idx_zero() {
        let player = ReplayPlayer::from_json(&sample_replay_json()).expect("valid JSON");
        assert_eq!(player.step_idx, 0);
        assert_eq!(player.moves.len(), 2);
    }

    /// Each step advances the index; once exhausted, step_native returns None.
    #[test]
    fn steps_advance_then_terminate() {
        let mut player = ReplayPlayer::from_json(&sample_replay_json()).expect("valid JSON");
        assert!(player.step_native().is_some());
        assert_eq!(player.step_idx, 1);
        assert!(player.step_native().is_some());
        assert_eq!(player.step_idx, 2);
        assert!(player.step_native().is_none(), "no further steps");
    }

    /// Malformed JSON returns an error rather than panicking.
    #[test]
    fn invalid_json_returns_error() {
        let result = ReplayPlayer::from_json("not valid json");
        assert!(result.is_err());
    }

    // -------------------------------------------------------------------------
    // Winning-sequence step-through
    // -------------------------------------------------------------------------

    /// Greedy Klondike solver for DrawOne Classic.
    ///
    /// Returns a `ReplayMove` list that wins the game from `seed`, or `None`
    /// when the greedy heuristic gets stuck within the move budget.
    ///
    /// Priority order (highest first):
    ///   1. Waste → Foundation
    ///   2. Tableau top → Foundation
    ///   3. Tableau stack → Tableau, only if the move uncovers a face-down card
    ///   4. Waste → Tableau
    ///   5. Draw from stock (recycle is automatic inside `GameState::draw`)
    fn greedy_solve(seed: u64) -> Option<Vec<ReplayMove>> {
        use solitaire_core::game_state::{DrawMode, GameMode, GameState};
        use solitaire_core::pile::PileType;

        let mut game = GameState::new_with_mode(seed, DrawMode::DrawOne, GameMode::Classic);
        let mut moves: Vec<ReplayMove> = Vec::new();
        const MAX_MOVES: usize = 10_000;

        'outer: loop {
            if game.is_won {
                return Some(moves);
            }
            if moves.len() >= MAX_MOVES {
                return None;
            }

            // Auto-complete: drive to win without further player input.
            if game.is_auto_completable {
                while let Some((from, to)) = game.next_auto_complete_move() {
                    if game.move_cards(from.clone(), to.clone(), 1).is_err() {
                        return None;
                    }
                    moves.push(ReplayMove::Move { from, to, count: 1 });
                }
                return if game.is_won { Some(moves) } else { None };
            }

            // P1: Waste → Foundation.
            for slot in 0..4_u8 {
                if game
                    .move_cards(PileType::Waste, PileType::Foundation(slot), 1)
                    .is_ok()
                {
                    moves.push(ReplayMove::Move {
                        from: PileType::Waste,
                        to: PileType::Foundation(slot),
                        count: 1,
                    });
                    continue 'outer;
                }
            }

            // P2: Tableau top → Foundation.
            for i in 0..7_usize {
                for slot in 0..4_u8 {
                    if game
                        .move_cards(PileType::Tableau(i), PileType::Foundation(slot), 1)
                        .is_ok()
                    {
                        moves.push(ReplayMove::Move {
                            from: PileType::Tableau(i),
                            to: PileType::Foundation(slot),
                            count: 1,
                        });
                        continue 'outer;
                    }
                }
            }

            // P3: Tableau stack → Tableau only when it uncovers a face-down card.
            let mut made_move = false;
            'p3: for i in 0..7_usize {
                let pile_len = game.piles[&PileType::Tableau(i)].cards.len();
                for count in 1..=pile_len {
                    let start = pile_len - count;
                    // Only worth moving if a face-down card sits just below.
                    let would_uncover =
                        start > 0 && !game.piles[&PileType::Tableau(i)].cards[start - 1].face_up;
                    if !would_uncover {
                        continue;
                    }
                    for j in 0..7_usize {
                        if i == j {
                            continue;
                        }
                        if game
                            .move_cards(PileType::Tableau(i), PileType::Tableau(j), count)
                            .is_ok()
                        {
                            moves.push(ReplayMove::Move {
                                from: PileType::Tableau(i),
                                to: PileType::Tableau(j),
                                count,
                            });
                            made_move = true;
                            break 'p3;
                        }
                    }
                }
            }
            if made_move {
                continue 'outer;
            }

            // P4: Waste → Tableau.
            for j in 0..7_usize {
                if game
                    .move_cards(PileType::Waste, PileType::Tableau(j), 1)
                    .is_ok()
                {
                    moves.push(ReplayMove::Move {
                        from: PileType::Waste,
                        to: PileType::Tableau(j),
                        count: 1,
                    });
                    continue 'outer;
                }
            }

            // P5: Draw from stock (handles recycle automatically).
            if game.draw().is_ok() {
                moves.push(ReplayMove::StockClick);
                continue 'outer;
            }

            // No moves available — greedy solver is stuck on this seed.
            return None;
        }
    }

    /// Full end-to-end winning-sequence regression test.
    ///
    /// 1. Runs the greedy solver on seeds 1–200 to find the first
    ///    deterministically winnable game.
    /// 2. Serialises the winning move list as a `Replay` JSON string.
    /// 3. Feeds the JSON to `ReplayPlayer::from_json`.
    /// 4. Steps through every move via `step_native` and asserts `is_won`
    ///    on the final snapshot.
    ///
    /// Regression target: a `GameState` or `ReplayMove` change that breaks
    /// an historically valid move sequence will cause `is_won` to be `false`
    /// at the end of the replay, failing this test before any release.
    #[test]
    fn replay_player_completes_full_winning_sequence() {
        use chrono::NaiveDate;
        use solitaire_core::game_state::{DrawMode, GameMode};

        let (seed, winning_moves) = (1_u64..=200)
            .find_map(|s| greedy_solve(s).map(|m| (s, m)))
            .expect("at least one seed in 1..=200 must be solvable by the greedy strategy");

        let replay = Replay {
            schema_version: 2,
            seed,
            draw_mode: DrawMode::DrawOne,
            mode: GameMode::Classic,
            time_seconds: 300,
            final_score: 0,
            recorded_at: NaiveDate::from_ymd_opt(2026, 5, 12)
                .expect("2026-05-12 is a valid date"),
            moves: winning_moves.clone(),
        };
        let json = serde_json::to_string(&replay).expect("replay serialises to JSON cleanly");

        let mut player =
            ReplayPlayer::from_json(&json).expect("solver-generated replay JSON must be valid");
        assert_eq!(player.step_idx, 0, "player must start at step 0");
        assert_eq!(
            player.moves.len(),
            winning_moves.len(),
            "player must hold the complete move list"
        );

        let mut last_snap: Option<StateSnapshot> = None;
        while let Some(snap) = player.step_native() {
            last_snap = Some(snap);
        }

        let snap = last_snap.expect("winning sequence must contain at least one move");
        assert!(
            snap.is_won,
            "seed {seed}: final snapshot after full replay must have is_won = true \
             ({} moves applied)",
            winning_moves.len()
        );
        assert_eq!(
            snap.step_idx,
            winning_moves.len(),
            "step_idx after the last move must equal the total move count"
        );
        assert!(
            player.step_native().is_none(),
            "step_native must return None once all moves are exhausted"
        );
    }
}
