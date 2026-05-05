//! Win-game replay recording + storage.
//!
//! When a player wins, the engine freezes the in-memory recording into a
//! [`Replay`] and persists it to `<data_dir>/solitaire_quest/latest_replay.json`
//! via [`save_latest_replay_to`]. The Stats screen offers a "Watch replay"
//! action that loads it via [`load_latest_replay_from`] so the player can
//! revisit (or, in a future build, watch the engine re-execute) the path
//! they took to victory.
//!
//! Schema versioning: bump [`REPLAY_SCHEMA_VERSION`] whenever the on-disk
//! shape changes. [`load_latest_replay_from`] returns `None` when the file
//! carries any other version so older replays are silently dropped instead
//! of crashing the loader.
//!
//! The recording is intentionally minimal — only [`ReplayMove`] entries
//! that successfully advanced the game. `Undo` is **not** recorded: a
//! replay represents the canonical path the player ultimately took to win,
//! so backed-out missteps simply do not appear in the move list. The
//! starting deal is not stored either — the [`seed`](Replay::seed) +
//! [`draw_mode`](Replay::draw_mode) + [`mode`](Replay::mode) are sufficient
//! for `GameState::new_with_mode` to rebuild the identical layout.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use solitaire_core::game_state::{DrawMode, GameMode};
use solitaire_core::pile::PileType;

const APP_DIR_NAME: &str = "solitaire_quest";
const LATEST_REPLAY_FILE_NAME: &str = "latest_replay.json";

/// Save-file schema version for [`Replay`]. Increment when the on-disk
/// representation changes incompatibly so [`load_latest_replay_from`] can
/// reject older formats and the player simply has no replay rather than
/// seeing a broken one.
///
/// History:
/// - v1: initial release. `ReplayMove` had separate `Draw` and `Recycle`
///   variants which carried the *outcome* of a stock interaction rather
///   than the player's atomic input.
/// - v2 (current): `Draw` + `Recycle` collapsed into a single `StockClick`
///   variant. The engine resolves draw-vs-recycle deterministically from
///   the current stock state, so the input alone is sufficient and the
///   replay model now stores atomic player inputs end-to-end.
pub const REPLAY_SCHEMA_VERSION: u32 = 2;

/// Default value for [`Replay::schema_version`] when deserialising files
/// that pre-date the field. Any value other than [`REPLAY_SCHEMA_VERSION`]
/// causes [`load_latest_replay_from`] to return `None`.
fn schema_v0() -> u32 {
    0
}

/// One atomic player input recorded during a winning game, in the order
/// it was applied to the live `GameState`.
///
/// `Undo` is intentionally absent — see the module-level docs.
///
/// The variants represent *inputs*, not outcomes. `StockClick` covers
/// every player click on the stock pile; the engine then resolves
/// draw-vs-recycle deterministically from the current state during both
/// recording and playback, so the same input always produces the same
/// effect on the same starting deal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReplayMove {
    /// A successful `move_cards(from, to, count)` call.
    Move {
        /// Source pile.
        from: PileType,
        /// Destination pile.
        to: PileType,
        /// Number of cards moved.
        count: usize,
    },
    /// A click on the stock pile. Resolves to a draw when stock is
    /// non-empty and to a waste→stock recycle when stock is empty.
    StockClick,
}

/// A complete recording of a single winning game.
///
/// Replays are reconstructed by rebuilding a fresh
/// `GameState::new_with_mode(seed, draw_mode, mode)` and applying the
/// [`moves`](Self::moves) in order. The presentation fields
/// ([`time_seconds`](Self::time_seconds), [`final_score`](Self::final_score),
/// [`recorded_at`](Self::recorded_at)) drive the Stats UI caption such as
/// "Replay (2:14 win on 2026-05-02)".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Replay {
    /// Schema version. See [`REPLAY_SCHEMA_VERSION`].
    #[serde(default = "schema_v0")]
    pub schema_version: u32,
    /// Seed used for the deal — replay rasterises the deck via
    /// `GameState::new_with_mode(seed, draw_mode, mode)`.
    pub seed: u64,
    /// Draw mode the recorded game was played in.
    pub draw_mode: DrawMode,
    /// Game mode the recorded game was played in.
    pub mode: GameMode,
    /// Total wall-clock seconds the win took. Used for the Stats UI
    /// "Replay (2:14 win on 2026-05-02)" caption.
    pub time_seconds: u64,
    /// Final score at the moment of the win.
    pub final_score: i32,
    /// ISO-8601 date the win was recorded.
    pub recorded_at: NaiveDate,
    /// Ordered move list. Each entry is what the player did, replayable
    /// against a fresh `GameState` constructed from the seed.
    pub moves: Vec<ReplayMove>,
}

impl Replay {
    /// Construct a fresh replay with the current schema version. The
    /// caller fills in the recorded fields; this is the canonical
    /// constructor used by the engine on win.
    pub fn new(
        seed: u64,
        draw_mode: DrawMode,
        mode: GameMode,
        time_seconds: u64,
        final_score: i32,
        recorded_at: NaiveDate,
        moves: Vec<ReplayMove>,
    ) -> Self {
        Self {
            schema_version: REPLAY_SCHEMA_VERSION,
            seed,
            draw_mode,
            mode,
            time_seconds,
            final_score,
            recorded_at,
            moves,
        }
    }
}

/// Returns the platform-specific path to `latest_replay.json`, or `None`
/// if `dirs::data_dir()` is unavailable (e.g. minimal Linux containers).
pub fn latest_replay_path() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join(APP_DIR_NAME).join(LATEST_REPLAY_FILE_NAME))
}

/// Save a [`Replay`] atomically to `path` using the standard `.tmp` →
/// rename contract that the rest of `storage.rs` uses.
///
/// Overwrites any existing replay — only the most recent winning replay
/// is retained on disk.
pub fn save_latest_replay_to(path: &Path, replay: &Replay) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(replay).map_err(io::Error::other)?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, json.as_bytes())?;
    fs::rename(&tmp, path)?;
    Ok(())
}

/// Load a [`Replay`] from `path`, returning `None` when the file is
/// missing, corrupt, or carries a [`schema_version`](Replay::schema_version)
/// other than [`REPLAY_SCHEMA_VERSION`].
///
/// Schema-mismatch is treated as "no replay" so the player just sees the
/// "No replay recorded yet" caption rather than a half-loaded broken
/// replay. Bumping [`REPLAY_SCHEMA_VERSION`] therefore invalidates every
/// older save without further migration code.
pub fn load_latest_replay_from(path: &Path) -> Option<Replay> {
    let data = fs::read(path).ok()?;
    let replay: Replay = serde_json::from_slice(&data).ok()?;
    if replay.schema_version != REPLAY_SCHEMA_VERSION {
        return None;
    }
    Some(replay)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    fn tmp_path(name: &str) -> PathBuf {
        env::temp_dir().join(format!("solitaire_test_replay_{name}.json"))
    }

    fn sample_replay() -> Replay {
        let date = NaiveDate::from_ymd_opt(2026, 5, 2).expect("valid date");
        Replay::new(
            12345,
            DrawMode::DrawThree,
            GameMode::Classic,
            134,
            5_120,
            date,
            vec![
                ReplayMove::StockClick,
                ReplayMove::Move {
                    from: PileType::Waste,
                    to: PileType::Tableau(3),
                    count: 1,
                },
                ReplayMove::StockClick,
                ReplayMove::Move {
                    from: PileType::Tableau(3),
                    to: PileType::Foundation(0),
                    count: 1,
                },
            ],
        )
    }

    /// A non-trivial replay with mixed move kinds must round-trip
    /// byte-identically through `save_latest_replay_to` /
    /// `load_latest_replay_from`. Catches any future field that forgets
    /// `Serialize`/`Deserialize` or breaks the on-disk format.
    #[test]
    fn replay_round_trips_through_save_and_load() {
        let path = tmp_path("round_trip");
        let _ = fs::remove_file(&path);

        let replay = sample_replay();
        save_latest_replay_to(&path, &replay).expect("save");

        let loaded = load_latest_replay_from(&path).expect("load must succeed");
        assert_eq!(loaded, replay, "round-trip must preserve every field");

        let _ = fs::remove_file(&path);
    }

    /// A file written by an older schema (or a pre-`schema_version`
    /// build) must be rejected. We write a minimal v0 fixture and assert
    /// that `load_latest_replay_from` returns `None` so the player gets
    /// a clean "no replay" state instead of a broken one.
    #[test]
    fn replay_legacy_schema_version_falls_through_to_none() {
        let path = tmp_path("legacy_schema");
        let _ = fs::remove_file(&path);

        // No `schema_version` key — defaults to 0 via `schema_v0()`. Even
        // if the rest of the JSON parses cleanly, the version gate must
        // reject it.
        let v0_json = r#"{
            "seed": 1,
            "draw_mode": "DrawOne",
            "mode": "Classic",
            "time_seconds": 60,
            "final_score": 100,
            "recorded_at": "2025-01-01",
            "moves": []
        }"#;
        fs::write(&path, v0_json).expect("write v0 fixture");

        assert!(
            load_latest_replay_from(&path).is_none(),
            "v0 replay must be rejected (schema gate)",
        );

        let _ = fs::remove_file(&path);
    }

    /// Atomic-write contract — `.tmp` must not be left behind after
    /// `save_latest_replay_to` returns. Mirrors the same check that
    /// guards `save_game_state_to` in `storage.rs`.
    #[test]
    fn replay_save_is_atomic() {
        let path = tmp_path("atomic");
        let _ = fs::remove_file(&path);

        save_latest_replay_to(&path, &sample_replay()).expect("save");
        let tmp = path.with_extension("json.tmp");
        assert!(!tmp.exists(), ".tmp must be cleaned up after rename");

        let _ = fs::remove_file(&path);
    }

    /// Loading from a path that does not exist must return `None`, not
    /// panic or surface an `Err`.
    #[test]
    fn replay_missing_file_returns_none() {
        let path = tmp_path("missing_xyz");
        let _ = fs::remove_file(&path);
        assert!(load_latest_replay_from(&path).is_none());
    }

    /// Loading from a corrupt / partially-written file must return
    /// `None`, not surface a deserialiser error to the engine.
    #[test]
    fn replay_corrupt_file_returns_none() {
        let path = tmp_path("corrupt");
        fs::write(&path, b"not valid json!!!").expect("write");
        assert!(load_latest_replay_from(&path).is_none());
        let _ = fs::remove_file(&path);
    }
}
