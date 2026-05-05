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
const REPLAY_HISTORY_FILE_NAME: &str = "replays.json";

/// Maximum number of recent winning replays the rolling history retains.
///
/// When [`append_replay_to_history`] pushes a fresh entry past this cap,
/// the oldest entry is dropped so the file never grows unbounded. The
/// player can revisit any of the last [`REPLAY_HISTORY_CAP`] wins from
/// the Stats overlay's replay selector — older wins age out silently.
pub const REPLAY_HISTORY_CAP: usize = 8;

/// Save-file schema version for [`ReplayHistory`]. Bump when the on-disk
/// shape of the wrapper changes incompatibly so [`load_replay_history_from`]
/// returns `None` for older files (the player simply sees an empty
/// history rather than a half-loaded broken one). Bumping
/// [`REPLAY_SCHEMA_VERSION`] independently invalidates individual
/// [`Replay`] payloads inside an otherwise-current history.
///
/// History:
/// - v1 (current): initial release of the rolling history wrapper.
pub const REPLAY_HISTORY_SCHEMA_VERSION: u32 = 1;

/// Default value for [`ReplayHistory::schema_version`] when deserialising
/// files that pre-date the field. Any value other than
/// [`REPLAY_HISTORY_SCHEMA_VERSION`] causes [`load_replay_history_from`]
/// to return `None`.
fn history_schema_v0() -> u32 {
    0
}

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

/// Rolling history of the player's most recent winning replays.
///
/// Stored as a single JSON file at
/// `<data_dir>/solitaire_quest/replays.json` (see
/// [`replay_history_path`]). Capped at [`REPLAY_HISTORY_CAP`] entries —
/// when [`append_replay_to_history`] pushes past the cap, the oldest
/// entry is dropped so the file never grows unbounded.
///
/// `replays[0]` is always the most recent win; the Stats overlay's
/// replay selector defaults to that entry and surfaces the older
/// entries behind a small chooser so the player can revisit a memorable
/// game even after a more recent win.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplayHistory {
    /// Schema version. See [`REPLAY_HISTORY_SCHEMA_VERSION`].
    #[serde(default = "history_schema_v0")]
    pub schema_version: u32,
    /// Most recent first. Capped at [`REPLAY_HISTORY_CAP`] entries —
    /// older entries drop off when the cap is hit.
    pub replays: Vec<Replay>,
}

impl Default for ReplayHistory {
    /// An empty history at the current schema version. Used by callers
    /// that need a starting point before the first winning replay has
    /// ever been recorded.
    fn default() -> Self {
        Self {
            schema_version: REPLAY_HISTORY_SCHEMA_VERSION,
            replays: Vec::new(),
        }
    }
}

impl ReplayHistory {
    /// Returns the most recent replay (`replays[0]`), or `None` when the
    /// history is empty. Convenience used by the Stats overlay's default
    /// selector position.
    pub fn most_recent(&self) -> Option<&Replay> {
        self.replays.first()
    }

    /// Returns the number of replays currently retained.
    pub fn len(&self) -> usize {
        self.replays.len()
    }

    /// Returns `true` when no replays have been recorded yet.
    pub fn is_empty(&self) -> bool {
        self.replays.is_empty()
    }
}

/// Returns the platform-specific path to `latest_replay.json`, or `None`
/// if `dirs::data_dir()` is unavailable (e.g. minimal Linux containers).
#[deprecated(
    note = "single-slot replay storage replaced by the rolling history at \
            replay_history_path(); kept for the one-shot legacy migration \
            in migrate_legacy_latest_replay"
)]
pub fn latest_replay_path() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join(APP_DIR_NAME).join(LATEST_REPLAY_FILE_NAME))
}

/// Returns the platform-specific path to `replays.json`, the rolling
/// history file, or `None` if `dirs::data_dir()` is unavailable (e.g.
/// minimal Linux containers).
pub fn replay_history_path() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join(APP_DIR_NAME).join(REPLAY_HISTORY_FILE_NAME))
}

/// Save a [`Replay`] atomically to `path` using the standard `.tmp` →
/// rename contract that the rest of `storage.rs` uses.
///
/// Overwrites any existing replay — only the most recent winning replay
/// is retained on disk.
#[deprecated(
    note = "single-slot replay storage replaced by the rolling history; \
            use append_replay_to_history instead. Kept for the one-shot \
            legacy migration."
)]
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
#[deprecated(
    note = "single-slot replay storage replaced by the rolling history; \
            use load_replay_history_from instead. Kept for the one-shot \
            legacy migration."
)]
pub fn load_latest_replay_from(path: &Path) -> Option<Replay> {
    let data = fs::read(path).ok()?;
    let replay: Replay = serde_json::from_slice(&data).ok()?;
    if replay.schema_version != REPLAY_SCHEMA_VERSION {
        return None;
    }
    Some(replay)
}

/// Save a [`ReplayHistory`] atomically to `path` using the standard
/// `.tmp` → rename contract.
///
/// The on-disk encoding is pretty-printed JSON; the file is intended to
/// be small (≤ [`REPLAY_HISTORY_CAP`] entries, each carrying a few
/// hundred move records at most) so the readability tradeoff is fine.
pub fn save_replay_history_to(path: &Path, history: &ReplayHistory) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(history).map_err(io::Error::other)?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, json.as_bytes())?;
    fs::rename(&tmp, path)?;
    Ok(())
}

/// Load a [`ReplayHistory`] from `path`, returning `None` when the file
/// is missing, corrupt, or carries a [`schema_version`](ReplayHistory::schema_version)
/// other than [`REPLAY_HISTORY_SCHEMA_VERSION`].
///
/// Individual [`Replay`] entries inside an otherwise-current history are
/// filtered to only those carrying [`REPLAY_SCHEMA_VERSION`] — older
/// entries are silently dropped so a future bump of the inner replay
/// schema does not corrupt the wrapper.
pub fn load_replay_history_from(path: &Path) -> Option<ReplayHistory> {
    let data = fs::read(path).ok()?;
    let history: ReplayHistory = serde_json::from_slice(&data).ok()?;
    if history.schema_version != REPLAY_HISTORY_SCHEMA_VERSION {
        return None;
    }
    let filtered: Vec<Replay> = history
        .replays
        .into_iter()
        .filter(|r| r.schema_version == REPLAY_SCHEMA_VERSION)
        .collect();
    Some(ReplayHistory {
        schema_version: REPLAY_HISTORY_SCHEMA_VERSION,
        replays: filtered,
    })
}

/// Append `replay` to the front of the rolling history at `path`,
/// dropping the oldest entry once [`REPLAY_HISTORY_CAP`] is exceeded,
/// and persist the updated history atomically.
///
/// If `path` has no existing history (missing file, corrupt, or
/// schema-mismatched) a fresh [`ReplayHistory::default`] is used as the
/// starting point so the new replay is always saved. The returned
/// [`ReplayHistory`] is the exact value written to disk so callers can
/// update an in-memory mirror (e.g. the Stats overlay's
/// `ReplayHistoryResource`) without a follow-up `load`.
pub fn append_replay_to_history(
    path: &Path,
    replay: Replay,
) -> io::Result<ReplayHistory> {
    let mut history = load_replay_history_from(path).unwrap_or_default();
    // Most recent first. Reserve the front slot; pop the oldest if we
    // exceed the cap so the file never grows unbounded.
    history.replays.insert(0, replay);
    if history.replays.len() > REPLAY_HISTORY_CAP {
        history.replays.truncate(REPLAY_HISTORY_CAP);
    }
    save_replay_history_to(path, &history)?;
    Ok(history)
}

/// One-shot migration from the legacy single-slot
/// `latest_replay.json` file to the rolling [`ReplayHistory`] stored at
/// `history_path`.
///
/// Behaviour matrix:
/// - `history_path` already exists  → no-op (the rolling history wins).
/// - `history_path` is absent and `latest_path` is absent → no-op.
/// - `history_path` is absent and `latest_path` exists with a valid
///   replay → seed a fresh history with that one replay and write it.
/// - `history_path` is absent and `latest_path` exists but is corrupt /
///   schema-mismatched → write an empty history (we know the player is
///   on the new build and shouldn't keep being prompted to migrate).
///
/// The legacy `latest_replay.json` file is intentionally NOT deleted by
/// this helper — keep it for one release as a safety net so a player
/// rolling back to the previous build doesn't lose their last winning
/// replay. The deletion is planned for the release after this one.
pub fn migrate_legacy_latest_replay(latest_path: &Path, history_path: &Path) {
    if history_path.exists() {
        // Rolling history is authoritative once it exists.
        return;
    }
    if !latest_path.exists() {
        return;
    }
    // Use the deprecated loader directly — the migration is the one
    // place we still consult the legacy file shape on purpose.
    #[allow(deprecated)]
    let legacy = load_latest_replay_from(latest_path);
    let history = match legacy {
        Some(replay) => ReplayHistory {
            schema_version: REPLAY_HISTORY_SCHEMA_VERSION,
            replays: vec![replay],
        },
        None => ReplayHistory::default(),
    };
    if let Err(e) = save_replay_history_to(history_path, &history) {
        // Migration failure is non-fatal: on the next launch we'll just
        // try again. We log to stderr rather than panic so headless
        // tests stay quiet.
        eprintln!(
            "replay: failed to migrate legacy latest_replay.json into rolling history: {e}",
        );
    }
}

#[cfg(test)]
// The legacy single-slot tests still exercise `save_latest_replay_to` /
// `load_latest_replay_from` on purpose — they're the round-trip
// guardrails for the migration source format.
#[allow(deprecated)]
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

    // -----------------------------------------------------------------------
    // ReplayHistory — rolling list of recent wins
    // -----------------------------------------------------------------------

    /// Build a [`Replay`] whose `final_score` carries `id` so tests can
    /// assert ordering / identity without writing a deep equality match.
    fn replay_with_id(id: i32) -> Replay {
        let date = NaiveDate::from_ymd_opt(2026, 5, 2).expect("valid date");
        Replay::new(
            id as u64,
            DrawMode::DrawOne,
            GameMode::Classic,
            60,
            id,
            date,
            vec![ReplayMove::StockClick],
        )
    }

    /// Pushing past [`REPLAY_HISTORY_CAP`] must drop the oldest entries —
    /// the on-disk file (and the in-memory mirror returned by the helper)
    /// stays bounded so the user's data dir never grows unbounded.
    #[test]
    fn append_replay_to_history_caps_at_eight() {
        let path = tmp_path("history_cap");
        let _ = fs::remove_file(&path);

        let mut last_returned = ReplayHistory::default();
        for i in 0..10 {
            last_returned = append_replay_to_history(&path, replay_with_id(i))
                .expect("append must succeed");
        }

        assert_eq!(
            last_returned.replays.len(),
            REPLAY_HISTORY_CAP,
            "history must be capped at REPLAY_HISTORY_CAP entries",
        );
        // The most recent ten pushes were ids 0..=9; ids 9, 8, ..., 2
        // survive (newest first), ids 0 and 1 aged out.
        let ids: Vec<i32> = last_returned.replays.iter().map(|r| r.final_score).collect();
        assert_eq!(
            ids,
            vec![9, 8, 7, 6, 5, 4, 3, 2],
            "newest entries must survive, oldest must age out",
        );

        // The on-disk file must agree with the returned in-memory copy.
        let loaded = load_replay_history_from(&path).expect("load must succeed");
        assert_eq!(loaded, last_returned, "disk must mirror returned history");

        let _ = fs::remove_file(&path);
    }

    /// `append_replay_to_history` must place new entries at index 0 so
    /// the Stats overlay's default selector (most recent) lands on the
    /// just-saved replay.
    #[test]
    fn append_replay_inserts_at_front() {
        let path = tmp_path("history_front");
        let _ = fs::remove_file(&path);

        append_replay_to_history(&path, replay_with_id(1)).expect("append 1");
        append_replay_to_history(&path, replay_with_id(2)).expect("append 2");
        let history = append_replay_to_history(&path, replay_with_id(3)).expect("append 3");

        let ids: Vec<i32> = history.replays.iter().map(|r| r.final_score).collect();
        assert_eq!(
            ids,
            vec![3, 2, 1],
            "history must be reverse-chronological (newest first)",
        );

        let _ = fs::remove_file(&path);
    }

    /// On first launch with the new code, a pre-existing
    /// `latest_replay.json` must seed the new rolling history so the
    /// player doesn't lose their last winning replay across the upgrade.
    #[test]
    fn legacy_latest_replay_migrates_to_history_on_first_launch() {
        let latest = tmp_path("legacy_migrate_latest");
        let history = tmp_path("legacy_migrate_history");
        let _ = fs::remove_file(&latest);
        let _ = fs::remove_file(&history);

        // Seed the legacy file with a real replay.
        let legacy_replay = sample_replay();
        save_latest_replay_to(&latest, &legacy_replay).expect("seed legacy");
        assert!(!history.exists(), "history file must not exist pre-migration");

        migrate_legacy_latest_replay(&latest, &history);

        assert!(history.exists(), "migration must create the history file");
        let loaded = load_replay_history_from(&history)
            .expect("post-migration history must load");
        assert_eq!(loaded.replays.len(), 1, "history must hold exactly the legacy entry");
        assert_eq!(loaded.replays[0], legacy_replay, "entry must equal the legacy replay");
        // Legacy file is intentionally retained for one release as a
        // safety net — see `migrate_legacy_latest_replay` doc comment.
        assert!(latest.exists(), "legacy file must NOT be deleted by migration");

        let _ = fs::remove_file(&latest);
        let _ = fs::remove_file(&history);
    }

    /// When the rolling history file already exists, the migration must
    /// be a no-op — we never want to overwrite the player's accumulated
    /// history with a stale single-slot legacy entry.
    #[test]
    fn migrate_is_noop_when_history_already_exists() {
        let latest = tmp_path("legacy_noop_latest");
        let history = tmp_path("legacy_noop_history");
        let _ = fs::remove_file(&latest);
        let _ = fs::remove_file(&history);

        save_latest_replay_to(&latest, &sample_replay()).expect("seed legacy");
        let pre_existing = ReplayHistory {
            schema_version: REPLAY_HISTORY_SCHEMA_VERSION,
            replays: vec![replay_with_id(42)],
        };
        save_replay_history_to(&history, &pre_existing).expect("seed history");

        migrate_legacy_latest_replay(&latest, &history);

        let loaded = load_replay_history_from(&history).expect("load");
        assert_eq!(loaded, pre_existing, "existing history must not be overwritten");

        let _ = fs::remove_file(&latest);
        let _ = fs::remove_file(&history);
    }

    /// A populated [`ReplayHistory`] must round-trip byte-identically
    /// through `save_replay_history_to` / `load_replay_history_from`.
    #[test]
    fn replay_history_round_trips_through_save_and_load() {
        let path = tmp_path("history_round_trip");
        let _ = fs::remove_file(&path);

        let history = ReplayHistory {
            schema_version: REPLAY_HISTORY_SCHEMA_VERSION,
            replays: vec![replay_with_id(7), replay_with_id(3), sample_replay()],
        };
        save_replay_history_to(&path, &history).expect("save");
        let loaded = load_replay_history_from(&path).expect("load");
        assert_eq!(loaded, history, "round-trip must preserve every field");

        let _ = fs::remove_file(&path);
    }

    /// A file written by an older history schema must be rejected so the
    /// player sees a clean empty history rather than a half-loaded one.
    #[test]
    fn replay_history_legacy_schema_version_falls_through_to_none() {
        let path = tmp_path("history_legacy_schema");
        let _ = fs::remove_file(&path);

        // No `schema_version` key → defaults to 0 via `history_schema_v0()`.
        let v0_json = r#"{
            "replays": []
        }"#;
        fs::write(&path, v0_json).expect("write v0 fixture");

        assert!(
            load_replay_history_from(&path).is_none(),
            "v0 history must be rejected (schema gate)",
        );

        let _ = fs::remove_file(&path);
    }

    /// Atomic-write contract for the rolling history — `.tmp` must not be
    /// left behind after `save_replay_history_to` returns.
    #[test]
    fn replay_history_save_is_atomic() {
        let path = tmp_path("history_atomic");
        let _ = fs::remove_file(&path);

        save_replay_history_to(&path, &ReplayHistory::default()).expect("save");
        let tmp = path.with_extension("json.tmp");
        assert!(!tmp.exists(), ".tmp must be cleaned up after rename");

        let _ = fs::remove_file(&path);
    }
}
