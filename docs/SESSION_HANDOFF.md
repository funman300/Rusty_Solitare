# Solitaire Quest — Session Handoff

> Last updated: 2026-04-25
> Branch: `master` — pushed to https://git.aleshym.co/funman300/Rusty_Solitare.git
> Test count: **228 passing** (83 core + 54 data + 91 engine), `cargo clippy --workspace -- -D warnings` clean

---

## What Has Been Built

### Phase 1 — Workspace Setup ✅ COMPLETE

All seven Cargo crates created and compiling cleanly:

| Crate | Status | Purpose |
|---|---|---|
| `solitaire_core` | Fully implemented | Pure Rust game logic — NO Bevy, NO network |
| `solitaire_sync` | Stub | Shared API types (`SyncPayload`, `SyncResponse`) |
| `solitaire_data` | Stub | `SyncError` enum + `SyncProvider` trait |
| `solitaire_engine` | Stub | Bevy ECS systems — all plugins added in Phase 3 |
| `solitaire_server` | Stub | Axum sync server — implemented in Phase 8C |
| `solitaire_gpgs` | Compile-time stub | Google Play Games bridge — Android only, JNI in Phase: Android |
| `solitaire_app` | Working | Opens blank Bevy window titled "Solitaire Quest" at 1280×800 |

Fast compile profiles, `assets/` directory structure, and `.env.example` are all in place.

### Phase 2 — Core Game Engine ✅ COMPLETE

`solitaire_core` is fully implemented with 68 passing tests and zero clippy warnings.

**Modules:**
- `card.rs` — `Suit` (Clubs/Diamonds/Hearts/Spades, `is_red()`/`is_black()`), `Rank` (Ace–King, `value() -> u8`), `Card` (id, suit, rank, face_up)
- `pile.rs` — `PileType` (Stock, Waste, Foundation(Suit), Tableau(usize)), `Pile` (new, top)
- `error.rs` — `MoveError`: InvalidSource, InvalidDestination, EmptySource, RuleViolation(String), UndoStackEmpty, GameAlreadyWon, StockEmpty
- `deck.rs` — `Deck::new()`, `Deck::shuffle(seed: u64)` using seeded `StdRng` (cross-platform deterministic), `deal_klondike(deck) -> ([Pile; 7], Pile)`
- `rules.rs` — `can_place_on_foundation(card, pile, suit)`, `can_place_on_tableau(card, pile)`
- `scoring.rs` — `score_move(from, to)`, `score_undo()` (-15), `compute_time_bonus(elapsed_seconds)` (700_000/s)
- `game_state.rs` — `DrawMode`, `GameState` with full game loop

**GameState public API:**
```rust
GameState::new(seed: u64, draw_mode: DrawMode) -> Self
GameState::draw(&mut self) -> Result<(), MoveError>
GameState::move_cards(&mut self, from: PileType, to: PileType, count: usize) -> Result<(), MoveError>
GameState::undo(&mut self) -> Result<(), MoveError>
GameState::check_win(&self) -> bool
GameState::check_auto_complete(&self) -> bool
GameState::compute_time_bonus(&self) -> i32
GameState::undo_stack_len(&self) -> usize
```

**Key GameState rules:**
- Undo stack capped at 64 entries (oldest evicted)
- Score never goes below 0
- Waste recycling is unlimited — `StockEmpty` only when both stock AND waste are simultaneously empty
- Recycle (waste → stock) pushes a snapshot so it can be undone
- Newly exposed top card of source pile is flipped face-up automatically on `move_cards`
- Win: all 4 foundations at 13 cards
- Auto-complete: stock empty + waste empty + all tableau cards face-up

---

## Commit History

```
b8dc7cb fix(core): remove stock_recycled limit, replace unwrap, snapshot on recycle, fix derives
58f1465 feat(core): add GameState with draw, move_cards, undo, win/auto-complete detection
43194b0 fix(core): use StdRng doc comment, replace expect() with debug_assert in deal_klondike
17bbec0 feat(core): add pile, error, deck, rules, scoring modules with tests
fcf878b feat(core): add Card, Suit, Rank types with tests
f84d7c5 fix(workspace): add derives/docs per code review, remove unused thiserror from solitaire_sync
684f077 feat(workspace): initialize all seven crates with stubs and blank Bevy window
```

---

### Phase 3 — Bevy Rendering & Interaction ✅ COMPLETE

All sub-phases (3A–3F) done. Plugins: `GamePlugin`, `TablePlugin`, `CardPlugin`, `InputPlugin`, `AnimationPlugin`. Full game playable — drag/drop with rule validation, keyboard shortcuts (U/N/D/Esc), animated slides, win cascade. UI via `bevy::ui`, no egui.

### Phase 4 — Statistics Persistence ✅ COMPLETE

- `solitaire_data::StatsSnapshot` with `update_on_win` / `record_abandoned` / `win_rate`
- Atomic file I/O via `save_stats_to` (`.tmp` → rename)
- `StatsPlugin` in `solitaire_engine` — loads on startup, persists on `GameWonEvent` (win) and `NewGameRequestEvent` (abandoned if move_count>0 and not won)
- Full-window overlay toggled with `S` — games played/won, win rate, streak, best score, fastest, avg
- `StatsPlugin::default()` for production, `StatsPlugin::headless()` for tests (no disk I/O)

### Phase 5 — Achievements ✅ COMPLETE (14 of ~19)

- `solitaire_core::achievement` — `AchievementContext` + `AchievementDef` + `ALL_ACHIEVEMENTS` + `check_achievements`
- `solitaire_core::GameState.undo_count` — tracks whether undo was used (for `no_undo` / `speed_and_skill`)
- `solitaire_data::AchievementRecord` + atomic `achievements.json` persistence
- `AchievementPlugin` — on `GameWonEvent`, build context from `StatsResource` + `GameState` + `chrono::Local` hour, evaluate all conditions, persist newly-unlocked records, emit `AchievementUnlockedEvent(id)`
- `AnimationPlugin`'s toast resolves the event's ID to the achievement's name via `achievement_plugin::display_name_for`
- New `StatsUpdate` system set lets `AchievementPlugin` order itself after stats are incremented
- Deferred: `daily_devotee` (needs `PlayerProgress`), `comeback` (needs recycle counter), `zen_winner` (needs modes), `perfectionist` (needs max-score calc). Stubs can be added in later phases.

### Phase 6 (part 1) — XP, Levels, ProgressPlugin ✅ COMPLETE

- `solitaire_data::PlayerProgress` with `total_xp`, `level`, daily/weekly/unlock fields
- `level_for_xp(xp)` and `xp_for_win(time, used_undo)` helpers (per ARCHITECTURE.md §13)
- `add_xp(amount) -> prev_level` with `leveled_up_from(prev)` for level-up detection
- Atomic `progress.json` persistence via `save_progress_to` / `load_progress_from`
- `ProgressPlugin` — on `GameWonEvent`, awards XP (base 50 + speed bonus 10–50 + no-undo 25), persists, emits `LevelUpEvent`
- `ProgressUpdate` system set for ordering downstream systems
- `ProgressPlugin::default()` for production, `::headless()` for tests

### Phase 6 (part 2a) — Daily Challenge + Level-Up Toast ✅ COMPLETE

- `daily_seed_for(date)` deterministic per-date seed
- `PlayerProgress::record_daily_completion(date)` with streak / reset / idempotency rules
- `DailyChallengePlugin`: today's seed in a resource; pressing **C** starts a daily-seed new game; on winning a daily-seed game, awards **+100 XP**, updates streak, persists, fires `DailyChallengeCompletedEvent`
- `LevelUpEvent` now spawns a toast through `AnimationPlugin`
- `daily_devotee` achievement wired (streak ≥ 7); `AchievementContext` gains `daily_challenge_streak` and reads from `ProgressResource`

### Phase 6 (part 2b) — Weekly Goals ✅ COMPLETE

- `solitaire_data::weekly` — `WeeklyGoalKind`, `WeeklyGoalDef`, `WeeklyGoalContext`, `current_iso_week_key`, three starter goals (5 wins / 3 no-undo / 3 fast)
- `PlayerProgress` — `weekly_goal_week_iso`, `roll_weekly_goals_if_new_week`, `record_weekly_progress`
- `WeeklyGoalsPlugin` — on `GameWonEvent`, rolls week if needed, increments matching goals, awards `WEEKLY_GOAL_XP` (75) per completion, fires `WeeklyGoalCompletedEvent`

### Phase 6 (part 3) — Completion Toasts + Progression Panel ✅ COMPLETE

- `AnimationPlugin` now surfaces `DailyChallengeCompletedEvent` (shows streak) and `WeeklyGoalCompletedEvent` (shows goal description) as 3-second toasts.
- Stats overlay (**S** key) appends a Progression section: level, total XP, daily streak, and a Weekly Goals list iterating `WEEKLY_GOALS` with `progress/target` for each.

### Phase 6 (part 4a) — Elapsed Time + Zen Mode ✅ COMPLETE

- `tick_elapsed_time` in `GamePlugin` ticks `GameState.elapsed_seconds` once per real-world second while not won; `advance_elapsed` is a pure helper for direct unit testing.
- `GameMode` enum (`Classic` / `Zen`) added to `solitaire_core::game_state`. `GameState.mode` field; `GameState::new_with_mode` ctor. Zen suppresses scoring in `move_cards` and `undo`. Field is `#[serde(default)]` for backwards-compatible saved games.
- `NewGameRequestEvent` carries an optional `mode`; `handle_new_game` falls back to the current game's mode when `None`.
- `Z` key starts a fresh Zen game.

### Phase 6 (part 4b) — Challenge Mode + Level-5 Gate ✅ COMPLETE

- `GameMode::Challenge` variant in core; `undo()` returns `RuleViolation` in Challenge.
- `solitaire_data::challenge` — `CHALLENGE_SEEDS` static list, `challenge_seed_for(index)` wrapping modulo length, `challenge_count()`.
- `PlayerProgress.challenge_index` (serde-default) tracks progression.
- `ChallengePlugin` advances the cursor on Challenge-mode wins, persists, fires `ChallengeAdvancedEvent`. **X** key starts a Challenge-mode game with the current seed.
- Both **Z** (Zen) and **X** (Challenge) are gated to `level >= CHALLENGE_UNLOCK_LEVEL` (5).

### Phase 6 (part 4c) — Time Attack + Unlock UI ✅ COMPLETE

- `GameMode::TimeAttack` variant added to core (no scoring/undo changes — just a session marker).
- `TimeAttackPlugin` (engine) — `TimeAttackResource { active, remaining_secs, wins }` (session-only, not persisted), `TimeAttackEndedEvent { wins }`. **T** starts a session (gated to level ≥ 5) and deals a TimeAttack-mode game; the timer (`TIME_ATTACK_DURATION_SECS = 600.0`) decrements each frame; wins during the active session bump the counter and auto-deal a fresh game.
- `AnimationPlugin` surfaces `TimeAttackEndedEvent` as a 5-second summary toast.
- `StatsPlugin` overlay (**S**) appends an "Unlocks" subsection (card backs / backgrounds, sorted/deduped, "None" when empty) and a live "Time Attack" panel showing remaining minutes/seconds + wins while a session is active.
- Helper `format_id_list` factored out + tested.

### Phase 7 (part 1) — Help Overlay + Challenge Toast ✅ COMPLETE

- `HelpPlugin`: **H** or `?` toggles a full-window cheat sheet listing all keybindings (gameplay, mode hotkeys, overlays). 3 unit tests.
- `AnimationPlugin` now surfaces `ChallengeAdvancedEvent` as a 3-second toast ("Challenge N cleared!").

### Phase 7 (part 2) — Synthesized SFX + AudioPlugin ✅ COMPLETE

- New workspace crate `solitaire_assetgen` with bin `gen_sfx`. Synthesizes five 44.1kHz mono 16-bit PCM WAVs from a deterministic LCG noise source + sine/square synths into `assets/audio/`. Run with `cargo run -p solitaire_assetgen --bin gen_sfx`. Output is committed; end users never run the generator.
- `AudioPlugin` (`solitaire_engine`): embeds the WAVs via `include_bytes!()`, decodes once via `kira::StaticSoundData::from_cursor`, plays on `DrawRequestEvent` (flip), `MoveRequestEvent` (place), `NewGameRequestEvent` (deal), `GameWonEvent` (fanfare).
- Backend handle stored as `NonSend` (cpal stream is `!Send` on some platforms). Plugin degrades gracefully if no audio device is available — logs a warning, gameplay continues silently.
- Single decode unit test (`embedded_wavs_decode_successfully`) keeps the loader and generator in sync.

### Phase 7 (part 3) — MoveRejectedEvent + Pause Menu ✅ COMPLETE

- New `MoveRejectedEvent { from, to, count }`. `end_drag` fires it when the cursor is over a real pile but `can_place_*` rejects the placement. `AudioPlugin` plays `card_invalid.wav` on it.
- New `PausePlugin` + `PausedResource(bool)`. **Esc** toggles a full-window pause overlay (ZIndex 220) and flips the resource. `tick_elapsed_time` and `advance_time_attack` skip work while paused. Input is deliberately not blocked — pause is a "stop the clock" screen, nothing more.
- `HelpPlugin` cheat sheet updated to reflect the new Esc behaviour.

## What Is Next

### Phase 7 (part 4+) — Polish

- **Volume controls**: Settings overlay with `sfx_volume` slider; persist via `solitaire_data::Settings`. Apply to kira's main-track gain.
- **Ambient loop**: optional sixth WAV — needs taste, deferred.
- **Onboarding**: first-run banner pointing at the **H**/`?` cheat sheet (single-shot via `Settings.first_run_complete`).
- **Optional**: block input while paused (drag, hotkeys) for stricter pause semantics.

### Phase 8 — Sync

| Phase | Scope |
|---|---|
| Phase 8A–C | Local storage + `SyncProvider` + self-hosted Axum server + client |
| Phase 8D | GPGS stub fully wired into settings UI |

---

## Important Implementation Notes

### Versions (Cargo.toml workspace deps)

- `bevy = "0.15"` (resolved to 0.15.3) — UI via built-in `bevy::ui`, no bevy_egui
- `kira = "0.9"` — audio via `kira` crate directly, no bevy_kira_audio or AssetServer
- `rand = "0.8"` — note: `small_rng` feature is NOT enabled; use `StdRng`, not `SmallRng`

### Asset strategy

- No `AssetServer` — assets embedded at compile time using `include_bytes!()`
- Fonts: `Font::try_from_bytes(include_bytes!("../assets/fonts/main.ttf"))`
- Audio: load from `&[u8]` via `kira` `StaticSoundData::from_cursor()`
- Card rendering: procedural (`bevy::prelude::Sprite` + `Text2d`) — no sprite sheets required

### Hard rules (from CLAUDE.md)
- `solitaire_core` and `solitaire_sync` must NEVER gain Bevy or network dependencies
- No `unwrap()` or `panic!()` in game logic — use `Result<_, MoveError>` everywhere
- All state transitions return `Result` — `debug_assert!` is acceptable for structural invariants
- `SyncPlugin` must NEVER match on `SyncBackend` enum inside a Bevy system — always call through the `SyncProvider` trait
- Atomic file writes only: write to `.tmp` then `rename()`
- `cargo clippy --workspace -- -D warnings` must pass clean
- `cargo test --workspace` must pass clean

### Lessons from this session
- `rand = "0.8"` without `features = ["small_rng"]` means `SmallRng` is unavailable — use `StdRng`
- `tower-governor` uses underscores in the crate name (not hyphens in Cargo.toml)
- When implementing `draw()` in `GameState`: recycle is unlimited, stop condition is BOTH piles empty simultaneously
- Recycle must push a snapshot (so it can be undone) even though it doesn't count as a "move"

---

## Implementation Plan Document

The detailed task-by-task plan for Phases 1 and 2 is at:
`docs/superpowers/plans/2026-04-20-phase1-2-workspace-core.md`

For Phase 3 onwards, write a new plan using the `superpowers:writing-plans` skill before starting implementation.

---

## Running the Project

```bash
# Check everything compiles
cargo check --workspace

# Run all tests (214 tests, all should pass)
cargo test --workspace

# Lint (must be zero warnings)
cargo clippy --workspace -- -D warnings

# Run the game
cargo run -p solitaire_app --features bevy/dynamic_linking
```
