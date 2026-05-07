# Changelog

All notable changes to Solitaire Quest are documented here. The format is
based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and this
project follows [Semantic Versioning](https://semver.org/).

## [Unreleased]

_Nothing yet._

## [0.18.0] — 2026-05-06

The launch-experience round. The engine used to drop the player on a
silent default Classic deal whether they had unfinished work or not;
v0.18.0 replaces that with two stacked decision points — a Restore
prompt for in-progress saves, then an MSSC-style Home / mode picker
that surfaces Daily / Zen / Challenge / Time Attack as picture tiles
with live stats. The same round closes the last solver-on-main-thread
hot path (winnable-only seed selection moves to
`AsyncComputeTaskPool`), wires "Copy share link" into Stats, lights a
"Won before" HUD chip on re-deals of beaten seeds, and tidies the
unified-3.0 rule set across CLAUDE.md / CLAUDE_SPEC.md /
CLAUDE_WORKFLOW.md / CLAUDE_PROMPT_PACK.md.

### Added

- **Restore prompt on launch** (`3c7a0eb`). When `game_state.json`
  holds an in-progress game (`move_count > 0`, not won), the engine
  now seeds `GameStateResource` with a fresh deal and holds the saved
  game in a new `PendingRestoredGame` resource. After the splash
  clears, a "Welcome back" modal offers **Continue** (Enter / C /
  click) or **New game** (N / click). Fresh-deal saves
  (`move_count == 0`) skip the prompt and load directly.
- **Save preservation while the prompt is unanswered** (`f863d85`).
  Both `save_game_state_on_exit` and `auto_save_game_state` consult
  `PendingRestoredGame` first: if it still holds a pending saved
  game, that's what gets persisted (or the auto-save is skipped),
  so exiting before answering the prompt no longer overwrites the
  meaningful save with the placeholder fresh deal.
- **Home / mode picker auto-shows on launch** (`dd63261`). The mode
  picker was only reachable via **M** during gameplay; players who
  hadn't discovered the hotkey never saw the Daily / Zen / Challenge
  / Time Attack entry points after the splash cleared. `HomePlugin`
  gains an `auto_show_on_launch` flag (default true) and a
  one-shot `LaunchHomeShown` gate. Skips when the Restore prompt is
  on screen so Welcome-back still takes precedence.
- **MSSC-style Home picker — header / chips / score chips / draw
  mode** (`ae40a1d`). Player-stats header strip (Level / XP /
  Lifetime Score, compact-formatted as `1.2M` / `12.3K` / `1,234`)
  acts as a clickable shortcut to Profile. Draw-mode chip row above
  the mode cards lets the player flip Draw 1 / Draw 3 from the
  picker itself; persists `settings.json` and respawns the modal so
  the active state repaints cleanly. Per-mode best-score / streak
  chips on each card; hidden on a 0 best so a fresh profile doesn't
  read "Best 0" everywhere.
- **Today's Event callout on the Daily card** (`b73d246`). "Today,
  May 6" date line plus the server-fetched goal (when SyncPlugin is
  wired). Once today's daily is recorded as completed, the date
  flips to `Today, May 6 • Done` in `ACCENT_PRIMARY` so the picker
  reads as a reward state rather than a TODO.
- **Picture-tile mode cards** (`9fe650f` + glyph-picking follow-ups
  `40d6e0a`, `c30b04e`, `d065d49`). Mode cards become a wrapping
  2-up grid (`FlexWrap::Wrap`, tiles 48 % wide, `min_height: 180px`)
  with a centred Unicode-glyph centrepiece per tile. Final glyph set
  picked from FiraMono-Medium's actual coverage: ♣ Classic, ◆ Daily,
  ○ Zen, ▲ Challenge, → TimeAttack. `ACCENT_PRIMARY` when the mode is
  unlocked, `TEXT_DISABLED` when locked. Centrepiece is a `Text` node
  for now — when real per-mode artwork lands, swap to `Image` without
  touching tile layout, focus order, or chip rendering.
- **Solver-vetted seed selection on `AsyncComputeTaskPool`**
  (`d489e7a`). Closes the worst-case 6 s UI stall on a New Game
  click with "Winnable deals only" enabled. New `PendingNewGameSeed`
  resource holds the in-flight `Task<u64>` plus the original
  request's `mode` / `confirmed` flags. `poll_pending_new_game_seed`
  runs `.before(GameMutation)` and replays a synthetic
  `NewGameRequestEvent` once the task resolves — the player sees no
  extra-frame visual lag. Cancel-on-replace: a fresh
  `NewGameRequestEvent` while a task is in flight drops the old
  task, letting Bevy's `Task` Drop cancel cooperatively at the next
  await point.
- **"Won before" HUD indicator** (`bdac754`). When the current
  deal's `(seed, draw_mode, mode)` triple matches an entry in the
  rolling `ReplayHistory`, the HUD's tier-2 context row shows
  **✓ Won before** in `STATE_SUCCESS`. Cleared on win (the on-screen
  victory cue is enough) and on first-time deals. New
  `HudWonPreviously` marker driven by a separate
  `update_won_previously` system; gracefully no-ops in headless
  tests that don't load `StatsPlugin`.
- **"Copy share link" Stats button** (`540869c`). End-to-end replay
  sharing on a server-backed sync backend:
  `sync_plugin::push_replay_on_win` spawns the upload on
  `AsyncComputeTaskPool` and stores the handle in
  `PendingReplayUpload` (drops any in-flight predecessor — the most
  recent win is what the player wants the link for);
  `poll_replay_upload_result` writes `<server>/replays/<id>` to
  `LastSharedReplayUrl` on success; the Stats overlay's action bar
  gains a button that writes the URL to the OS clipboard via
  `arboard` and surfaces a "Copied: \<url\>" toast. URL is in-memory
  only — sharing must happen within the session of the win.
- **Empty-state copy + onboarding hints** (`56e2e6f`). Leaderboard
  empty state: two-tier "Be the first on the leaderboard." headline
  + body invite. Achievements panel: first-launch hint above the
  grid until the first unlock. Volume hotkeys (`[` / `]`) now emit
  an `InfoToastEvent` with the new percentage so off-panel
  adjustments give visible feedback (previously silent).
- **Enter dismisses the Win Summary and starts a fresh deal**
  (`17e0737`). The post-win modal's "Play Again" was click-only;
  keyboard-only players had to reach for the mouse to leave the
  celebration screen. The button label gains a trailing return-key
  glyph so the keyboard path is discoverable on first sight.
- **`N` opens the real Confirm/Cancel modal** (`93660c2`). The old
  "Press N again" double-tap pattern was a UI-first violation (only
  continuation was another keystroke). `N` now fires
  `NewGameRequestEvent::default()` directly; `handle_new_game`'s
  active-game check spawns the existing `ConfirmNewGameScreen`. The
  HUD button already routed through the same modal — keyboard and
  mouse paths are unified. `Shift+N` keeps the keyboard power-user
  bypass (`confirmed: true`).

### Changed

- **Settings row layout** (`a4bc063`). All five
  slider/toggle row helpers (volume × 2, tooltip delay, time-bonus
  multiplier, replay-move interval, generic toggle) restructured to
  a label-spacer-cluster layout (`width: 100%`, label gets
  `flex-grow: 1`, controls cluster sits flush right). Stable across
  varying value-text widths ("0.80" → "1.00", "Instant" vs "1.5 s")
  and narrow windows.
- **Docs adopt the unified-3.0 rule set** (`f2f30c8`). `CLAUDE.md`
  grows from a 114-line pointer doc to a 571-line rulebook (hard
  global constraints §2, engine rules §3, asset rules §4, code
  standards §5, build + verification §6, git workflow §7, the ASK
  BEFORE list §8, Context Injection System §14). New companions:
  `CLAUDE_SPEC.md` (formal architecture spec — crate dependency
  graph, data ownership, state-machine invariants, sync merge /
  server contracts, validation checklist),
  `CLAUDE_WORKFLOW.md` (two-agent Builder/Guardian pipeline with
  hard-fail patterns), `CLAUDE_PROMPT_PACK.md` (task-type
  templates). Three duplicate rule passages removed across
  `CLAUDE_SPEC.md` and `ARCHITECTURE.md`.
- **Test discipline pruning** (`a49a340`). Removed 43 low-value
  tests across `solitaire_data` and `solitaire_core` (default-value
  tests, serde-derive round-trips on plain structs, single-field
  clamp tests, near-duplicates, constant-equals-itself tests). None
  pinned a behaviour contract or a regression on a real bug. Future
  agent briefs request tests for behaviour contracts or real-bug
  regressions, not a count of N.

### Fixed

- **Esc on a modal no longer opens Pause underneath** (`08b006f`).
  A single Esc press on Confirm New Game / Restore / Home /
  Onboarding / Settings used to both close the modal and spawn the
  Pause overlay on top in the same frame. `toggle_pause` now skips
  when any non-Pause `ModalScrim` is in the world; the HUD-button
  path is gated too. The four modal queries are bundled into a
  `PauseModalQueries` `SystemParam` to stay under Bevy's
  16-parameter cap.
- **Esc dismisses Home / accepts the Restore-prompt default**
  (`d48b948`). Both screens previously ignored Esc, leaving the
  player no keyboard-only escape after the previous fix. Home: Esc
  behaves like Cancel (despawns the modal, keeps the underlying
  default deal). Restore: Esc maps to Continue (preserves the saved
  game, matching how the primary action already advertises Enter).
- **Esc dismisses the topmost modal when Profile stacks on Home**
  (`9aa0dd2`). Clicking the Home header chip opens Profile on top
  of Home; Esc used to close Home (because
  `handle_home_cancel_button` fired with no awareness of layered
  modals) and leave Profile orphaned over the game.
  `profile_plugin` now splits P/button (toggle) from Esc
  (close-only); `handle_home_cancel_button` skips its Esc branch
  when any other `ModalScrim` exists.
- **Restore-prompt resolution suppresses Home auto-show**
  (`b7c3a49`). Resolving the Welcome-back prompt cleared
  `PendingRestoredGame` and despawned the modal, but the
  launch-time Home auto-show then fired the next frame and stacked
  itself over the player's chosen path. `LaunchHomeShown` becomes
  `pub` so `handle_restore_prompt` flips it to `true` after either
  resolution; **M** still re-opens the picker on demand.
- **Game timers freeze while the Home picker is up** (`c497c31`).
  The HUD's elapsed-time counter ticked from the moment the default
  Classic deal landed at startup, even though the auto-show Home
  picker was still up — the player saw "0:11" before they had
  chosen a mode. `tick_elapsed_time` and `advance_time_attack` now
  also gate on the absence of `HomeScreen`, mirroring their
  existing `PausedResource` check.
- **Popover rows stay visible regardless of action-bar fade**
  (`cc63532`). Opening Modes / Menu showed a solid dark-purple
  block in the top-right with no readable content — the action-bar
  auto-fade was matching the popover rows by their shared
  `ActionButton` marker and dropping their alpha to the
  cursor-position-based fade value (typically 0). New `PopoverRow`
  marker on rows in `spawn_modes_popover` / `spawn_menu_popover`;
  `apply_action_fade` excludes them via `Without<PopoverRow>`.

### Stats

- 1166 passing tests (was 1208 at v0.17.0 close — 43 net removals
  from the test-discipline prune plus 1 net-new test from the
  async-seed work, no behaviour regressions).
- Zero clippy warnings under `--workspace --all-targets -- -D warnings`.

## [0.17.0] — 2026-05-06

A short follow-up round on top of v0.16.0: the H-key hint is no
longer a heuristic guess but the actual best first move suggested by
the v0.15.0 solver, and the in-engine replay player now has a
player-tunable playback rate.

### Added

- **Replay-rate slider** in Settings → Gameplay. Tunes
  `replay_move_interval_secs` from 0.10 s to 1.00 s in 0.05 s steps;
  default 0.45 s. `tick_replay_playback` reads the value from
  `SettingsResource` per frame so the slider takes effect on the
  next playback tick — no restart required.

### Changed

- **Solver-driven hints.** Pressing **H** used to surface a
  heuristic-best move (foundation moves preferred, then
  tableau-to-tableau by depth-of-flip-revealed). It now asks the
  v0.15.0 solver for the actual provably-best first move via the
  new `solitaire_core::solver::try_solve_with_first_move` /
  `try_solve_from_state` APIs. When the solver returns inconclusive
  (rare deals where the bound runs out before a result), the old
  heuristic remains the fallback. Median 2 ms per H press.

### Stats

- 1208 passing tests (was 1196 at v0.16.0 close).
- Zero clippy warnings under `--workspace --all-targets -- -D warnings`.

## [0.16.0] — 2026-05-06

A modal-feel polish round. Every overlay screen now scrolls when its
content overflows the 800×600 minimum window, every clickable button
shows a hand cursor on hover, keyboard focus lands on the primary
button on the same frame the modal opens, and read-only modals
dismiss when the player clicks the scrim outside the card.

### Added

- **Pointer cursor on hover** for every interactive `Button` entity
  (modal buttons, HUD action bar, mode-launcher cards, settings
  toggles, Stats selectors). `update_cursor_icon` gains a fourth
  branch sitting between Grabbing (active drag) and Grab
  (draggable card hover): when no drag is active and any
  `Interaction::Hovered`/`Pressed` button is detected, the window
  cursor swaps to `SystemCursorIcon::Pointer`. A pure
  `pick_cursor_icon` helper makes the priority logic
  unit-testable.
- **Click-outside-to-dismiss** for the six read-only modals: Stats,
  Achievements, Help, Profile, Leaderboard, Home. New
  `ScrimDismissible` marker on `ModalScrim` opts a modal in;
  `dismiss_modal_on_scrim_click` runs in `Update`, despawns the
  topmost dismissible scrim on a left-mouse press whose cursor
  lands on the scrim and outside every `ModalCard`. Bevy's
  hierarchy despawn cascades to the card and children.
  Settings, Onboarding, Pause, Forfeit confirm, and Confirm New
  Game intentionally don't opt in — they carry unsaved or
  destructive state.

### Fixed

- **Modal content scrolls when it overflows** (Achievements, Help,
  Stats, Profile, Leaderboard). Each modal's body Node now
  carries `Overflow::scroll_y()` plus a `max_height` constraint
  (`Val::Vh(70.0)` for most, `Val::Vh(50.0)` for the
  leaderboard's variable-length ranking section) and a marker
  component (`AchievementsScrollable`, `HelpScrollable`,
  `StatsScrollable`, `ProfileScrollable`,
  `LeaderboardScrollable`). A sibling `scroll_*_panel` system
  per modal routes `MouseWheel` events into the body's
  `ScrollPosition`. Mirrors the existing `SettingsPanelScrollable`
  pattern. Home modal intentionally not scrolled — its five
  mode cards + Cancel are sized to fit at 800×600 by design.
- **Modal focus arrives on the same frame the modal opens.**
  Previously `attach_focusable_to_modal_buttons` and
  `auto_focus_on_modal_open` ran in `Update` alongside arbitrary
  click-handlers that spawn modals; with no ordering edge,
  Bevy's deferred `Commands` queued the new entities but the
  attach system couldn't see them on the same tick. Both systems
  moved to `PostUpdate` so the schedule boundary itself supplies
  the sync point — `FocusedButton` is always populated before
  `app.update()` returns. The very next Tab/Enter press lands on
  a populated resource instead of wasting itself moving focus
  from None to the primary.

### Stats

- 1196 passing tests (was 1178 at v0.15.0 close).
- Zero clippy warnings under `--workspace --all-targets -- -D warnings`.

## [0.15.0] — 2026-05-02

In-engine replay playback, the Klondike solver + "Winnable deals
only" toggle, a 19th achievement, rolling replay history, and a
significant build-time / binary-size win from disabling Bevy's
default audio stack.

### Added

- **In-engine replay playback** for the Stats overlay's Watch Replay
  button. New `ReplayPlaybackPlugin` runs a state machine
  (Inactive / Playing / Completed) that resets the live game to the
  recorded deal and ticks through `replay.moves` at
  `REPLAY_MOVE_INTERVAL_SECS` (0.45 s) firing the canonical
  `MoveRequestEvent` / `DrawRequestEvent` per recorded move.
  Recording is suppressed during playback so replays don't re-record
  themselves.
- **Replay overlay banner** (`ReplayOverlayPlugin`) anchored to the
  top of the window during playback. Shows "Replay" label, "Move N
  of M" progress, and a Stop button. Z-order leaves modals
  (Settings, Pause, Help) free to render on top so the player can
  adjust audio mid-replay.
- **Rolling replay history** at `<data_dir>/replays.json` capped at
  8 entries. Replaces the single-slot `latest_replay.json` (legacy
  file is migrated forward on first launch via
  `migrate_legacy_latest_replay`). Stats overlay gains a Prev / Next
  selector and a "Replay N / M" caption so the player can revisit
  older wins.
- **"Cinephile" achievement** (#19). Unlocks the first time
  `ReplayPlaybackState` transitions Playing → Completed (i.e. the
  replay played out to its end without the player pressing Stop).
  Stop transitions Playing → Inactive directly so it doesn't count.
- **Klondike solver** in `solitaire_core::solver`. Iterative-DFS
  with memoisation on a 64-bit canonical state hash, two budget
  knobs (move_budget + state_budget) for pathological cases, and a
  three-state `SolverResult` (Winnable / Unwinnable / Inconclusive).
  Median solve time 2 ms; pathological inconclusives cap near
  120 ms. Pure logic — `solitaire_core` keeps no Bevy or I/O.
- **"Winnable deals only" toggle** in Settings → Gameplay (default
  off). When on, `handle_new_game` walks seed N, N+1, N+2, …
  through `try_solve` until it finds Winnable or Inconclusive,
  capped at `SOLVER_DEAL_RETRY_CAP` (50) attempts. Daily
  challenges, replays, and explicit-seed requests bypass the
  solver — only random Classic deals are gated.

### Changed

- **Bevy default-feature trim** (`bevy = { default-features = false,
  features = [...] }` in workspace Cargo.toml) drops 51 transitive
  crates including the `bevy_audio` → rodio → cpal 0.15 + symphonia
  chain that the project doesn't use (kira handles audio directly).
  The retained feature list is curated to exactly what the engine
  uses; `solitaire_wasm` is unaffected because it doesn't depend on
  bevy.

### Stats

- 1178 passing tests (was 1134 at v0.14.0 close).
- Zero clippy warnings under `--workspace --all-targets -- -D warnings`.

## [0.14.0] — 2026-05-02

Two threads land in v0.14.0: the second half of the post-v0.12.0 UX
candidate list (theme thumbnails, daily-challenge calendar, Time Attack
auto-save, per-mode bests, time-bonus multiplier) plus a **major new
feature** — the replay pipeline (record → upload → web viewer). Three
Quat-reported bugs from a smoke-test round shipped alongside.

### Added

- **Theme-picker thumbnails** in Settings → Cosmetic. Each theme chip
  renders a small Ace-of-Spades + back preview pair via the existing
  `rasterize_svg` path. Cached per theme in a new
  `ThemeThumbnailCache`. Themes that lack a preview SVG fall back to
  a transparent placeholder rather than crashing.
- **14-day daily-challenge calendar** in the Profile modal. Horizontal
  row of dots showing the trailing two weeks; today's dot is ringed
  in `ACCENT_PRIMARY`, completed days fill `STATE_SUCCESS`, missed
  days fill `BG_ELEVATED`. Caption above the row reads "Current
  streak: N · Longest: M".
- **Time Attack session auto-save** to `<data_dir>/time_attack_session.json`,
  atomic .tmp + rename. 30-second auto-save while a session is active,
  plus on `AppExit`. Sessions whose 10-minute window expired in real
  time while the app was closed are discarded on load. Classic, Zen,
  and Challenge already auto-saved correctly via `game_state.json` —
  Time Attack was the only mode missing session-level persistence.
- **Per-mode best-score and fastest-win readouts** in the Stats screen.
  `StatsSnapshot` gains six `#[serde(default)]` fields (Classic / Zen
  / Challenge × best_score + fastest_win_seconds). Stats screen renders
  a "Per-mode bests" section between the primary cell grid and
  progression. Lifetime totals continue to roll all modes together.
- **Time-bonus multiplier slider** in Settings → Gameplay (0.0–2.0,
  0.1 steps, default 1.0, "Off" label at zero). Cosmetic only —
  multiplies the time-bonus shown in the win modal but does NOT
  affect achievement unlock thresholds (those still use the raw
  unmultiplied score).
- **Win-replay recording + storage.** Every move during a successful
  game appends to a `RecordingReplay` resource; on `GameWonEvent`
  the recording freezes into a `Replay` (seed + draw_mode + mode +
  score + time + ordered move list) and persists to
  `<data_dir>/latest_replay.json` atomically. Single-slot — overwrites
  on every win.
- **"Watch replay" button** in the Stats overlay. Shows the latest
  win's caption and surfaces a button that loads the replay (button
  fires an `InfoToastEvent` describing the replay; full in-engine
  playback is deferred to a future build).
- **Replay upload + fetch endpoints** on the server. `POST /api/replays`
  accepts a `Replay` JSON; `GET /api/replays/:id` returns it. JWT-gated
  with the existing auth middleware. Engine uploads winning replays
  automatically when the player has cloud sync configured.
- **`solitaire_wasm` crate** — new workspace member compiling
  replay-relevant `solitaire_core` types to WebAssembly so a
  browser can re-execute a replay client-side. No-std-friendly
  surface; `wasm-bindgen` glue.
- **Web replay viewer** served from the Solitaire server.
  `GET /replays/:id` returns HTML + CSS + the wasm bundle that
  fetches the replay JSON, rasterises a deal from the seed, and
  animates the recorded moves.
- **Card flight animations on the web side** so the browser viewer
  reads as a real game replay rather than a static dump.

### Fixed

- **Multi-card lift validation.** `solitaire_core::rules::is_valid_tableau_sequence`
  rejects a moved stack whose adjacent cards don't form a descending
  alternating-colour run. Previously a player could lift any
  multi-card selection and drop it as long as the bottom landed
  legally. Wired into `move_cards`'s tableau-destination branch.
- **Softlock detection.** `has_legal_moves` rewritten to walk every
  potential move source (every stock card, every waste card, the
  face-up top of every tableau column) and check it against every
  foundation and every tableau. Previously the heuristic
  early-returned `true` whenever stock had cards — players got
  stuck in unwinnable end-states with no end-game screen.
  `GameOverScreen` now actually fires for true softlocks. Quat's
  exact reproduction case is pinned by a new test.
- **Deal-tween information leak.** New-game now snaps every card
  sprite to the stock pile position before writing
  `StateChangedEvent`, so all 52 cards animate from a single point
  during the deal. Previously the sprites started from their
  previous-game positions, briefly revealing the prior deal.

### Documentation

- `SESSION_HANDOFF.md` refreshed for the Quat smoke-test round
  including investigation findings on solver decisions and
  dependency duplicates.

### Stats

- 1134 passing tests (was 1053 at v0.13.0 close).
- Zero clippy warnings under `--workspace --all-targets -- -D warnings`.

## [0.13.0] — 2026-05-02

Third UX iteration round on top of v0.12.0. Six handoff candidates
shipped — three small polish items, three larger interaction
features (theme-aware backs, full keyboard play, right-click power
shortcut). Plus two code-review fixes (font handling unified,
sccache wiring removed).

### Added

- **Tooltip-delay slider** in Settings → Gameplay. `tooltip_delay_secs`
  ranges [0.0, 1.5] in 0.1 s steps; "Instant" label when zero.
  `Settings.tooltip_delay_secs` round-trips through serialise/deserialise
  with `#[serde(default)]`. The hover-delay comparison in
  `ui_tooltip` reads from `SettingsResource` with the existing
  `MOTION_TOOLTIP_DELAY_SECS` as the test-fixture fallback.
- **Win-streak fire animation.** New `WinStreakMilestoneEvent` fires
  from `stats_plugin` when `win_streak_current` crosses any of
  [3, 5, 10] (only the threshold crossing — not every subsequent
  win). The HUD streak readout scale-pulses 1.0 → 1.20 → 1.0 over
  `MOTION_STREAK_FLOURISH_SECS` (0.6 s).
- **Score-breakdown reveal on the win modal.** Replaces the single
  "Score: N" line with a per-component reveal (Base / Time bonus /
  No-undo bonus / Mode multiplier / Total). Rows fade in over
  `MOTION_SCORE_BREAKDOWN_FADE_SECS` (0.12 s) staggered by
  `MOTION_SCORE_BREAKDOWN_STAGGER_SECS` (0.15 s). Honours
  `AnimSpeed::Instant` by spawning all rows fully visible.
- **Card backs follow the active theme.** `theme.ron`'s `back` slot
  now actually drives the face-down sprite. Active-theme back
  rasterises alongside the faces and supersedes the legacy
  `back_N.png` picker. The picker remains as a fallback for themes
  that don't ship a back, and the Settings UI surfaces a caption
  ("Active theme provides its own back") + dimmed swatches when
  the override is in effect.
- **Keyboard-only drag-and-drop.** Tab cycles draggable card stacks,
  Enter "lifts" the focused stack, arrow keys (or Tab) cycle the
  legal-destination targets only, Enter confirms, Esc cancels. A
  new `KeyboardDragState` resource models the two-mode flow without
  changing the existing `SelectionState` contract. Mutual exclusion
  with mouse drag uses a sentinel `DragState.active_touch_id =
  KEYBOARD_DRAG_TOUCH_ID` (u64::MAX) so neither pipeline can
  trample the other.
- **Right-click radial menu.** Hold right-click on a face-up card →
  a small ring of icons appears at the cursor with one entry per
  legal destination. Release over an icon → fires
  `MoveRequestEvent`; release in dead space, Esc, or left-click
  cancels. Skips the drag motion entirely. New `RadialMenuPlugin`
  owns the flow; co-exists with the existing `RightClickHighlight`
  pile-marker tint.

### Fixed

- **Font handling consolidated to bundled-only.** Code-review
  feedback: the SVG rasteriser previously mixed
  `load_system_fonts` + bundled FiraMono + a lenient resolver,
  which made card text rendering depend on host fontconfig. Picked
  option (a) and applied it across both layers — `font_plugin` now
  embeds `assets/fonts/main.ttf` via `include_bytes!()` and
  registers it with `Assets<Font>`; `svg_loader::shared_fontdb`
  loads only the bundled bytes; the new `bundled_font_resolver`
  ignores the SVG's `font-family` request and always returns the
  single bundled face. A parse failure aborts with a clear error
  ("bundled FiraMono failed to parse — binary is corrupt").

### Removed

- **Project-level sccache wiring.** Code-review feedback: sccache
  shouldn't be a per-project build dependency. Cargo's incremental
  cache already covers the single-project case, and forcing
  `rustc-wrapper = "sccache"` workspace-wide meant every contributor
  had to install it. `.cargo/config.toml` deleted entirely; plain
  `cargo build` now works without setup.

### Documentation

- `help_plugin` controls reference gains a "Mouse" section covering
  double-click auto-move, right-click highlight, and the new
  hold-RMB radial.
- `help_plugin` also gains a "Keyboard drag" section for the new
  Tab/Enter/Arrows/Esc flow.
- Onboarding slide 3 picks up a `Tab → Enter` row referencing the
  full keyboard drag path.

### Stats

- 1053 passing tests (was 1031 at v0.12.0 close).
- Zero clippy warnings under `--workspace --all-targets -- -D warnings`.

## [0.12.0] — 2026-05-02

UX feel polish round on top of v0.11.0. Six small-but-tangible
improvements that make the play surface feel more responsive,
forgiving, and discoverable, plus the doc refresh that should have
ridden along with v0.11.0.

### Added

- **Foundation completion flourish.** When a King lands on a
  foundation (Ace-through-King for that suit), a brief celebration
  fires: King card scale-pulses 1.0 → 1.15 → 1.0 over 0.4 s, the
  foundation marker tints `STATE_SUCCESS` for the first half then
  fades, and a synthesised C6→E6→G6 bell ping plays (~240 ms,
  octave above `win_fanfare`'s root so the fourth completion + win
  cascade layer cleanly). New `FoundationCompletedEvent { slot,
  suit }` carries the trigger so future systems can hook in.
- **Drag-cancel return tween.** Illegal drops glide each dragged
  card back to its origin slot over 150 ms with a quintic ease-out
  curve (`MotionCurve::Responsive`, zero overshoot — reads forgiving
  rather than jittery). The audio cue (`card_invalid.wav`) still
  fires for negative feedback. Right-click and double-click invalid
  paths still use `ShakeAnim` since there's no motion to interpolate.
- **Focus ring breathing.** The keyboard focus ring's alpha modulates
  with a 1.4 s sin curve over [0.65, 1.0] of its native value so the
  indicator catches the eye on focus changes without competing with
  gameplay. Honours `AnimSpeed::Instant` by reverting to the static
  outline for reduced-motion users.
- **First-win achievement onboarding toast.** After the player's
  very first win, a one-shot info toast surfaces "First win! Press
  A to see your achievements." `Settings.shown_achievement_onboarding`
  persists the seen state so the cue never re-fires (legacy
  `settings.json` files load to `false` via `#[serde(default)]`).
- **Mode Launcher digit shortcuts.** Pressing M opens the Home modal
  (the Mode Launcher); inside it, pressing 1–5 launches each mode
  directly without needing Tab + Enter. Locked modes (Zen, Challenge,
  Time Attack at level < 5) are silent no-ops. Modal-scoped — digit
  keys outside the launcher fire nothing.

### Fixed

- **Card aspect ratio matches hayeah SVGs.** `CARD_ASPECT` 1.4 →
  1.4523 to match the bundled artwork's natural 167.087 × 242.667
  dimensions. Cards previously rendered ~3.6 % vertically squashed.
  The vertical-budget math in `compute_layout` uses `CARD_ASPECT`
  algebraically so the worst-case-tableau-fits-on-screen guarantee
  adapts automatically.

### Documentation

- **README refresh** with v0.11.0+ features (card themes, HUD
  overhaul, drag feel, unlocked foundations) and a corrected controls
  table — the previous table inverted Z/U for undo and listed H for
  help when F1 is the binding.
- **CHANGELOG.md** added (this file), covering v0.9.0–v0.12.0 with
  Keep a Changelog 1.1.0 conventions.

### Stats

- 1007 passing tests (was 982 at v0.11.0).
- Zero clippy warnings under `--workspace --all-targets -- -D warnings`.

## [0.11.0] — 2026-05-02

The biggest release since 0.10.0. Headline threads: a runtime card-theme
system, an HUD restructure that reclaims the play surface, and a round of
UX feel polish surfaced by smoke testing.

### Added

- **Runtime card-theme system** (CARD_PLAN phases 1–7).
  - Bundled default theme ships in the binary via `embedded://` — 52
    [hayeah/playing-cards-assets](https://github.com/hayeah/playing-cards-assets)
    SVGs (MIT) plus a midnight-purple `back.svg` as original work.
  - User themes live under `themes://` rooted at `user_theme_dir()`. Drop
    a directory containing `theme.ron` + 53 SVGs and the registry picks
    it up on next launch.
  - Importer at `solitaire_engine::theme::import_theme(zip)` validates
    archives (20 MB cap, zip-slip rejection, manifest validation, every
    SVG round-tripped through the rasteriser) and atomically unpacks.
  - Picker UI in **Settings → Cosmetic**; selection persists as
    `selected_theme_id` and propagates to live sprites.
- **Reserved HUD top band** (64 px) so cards no longer crowd the score
  readout or action buttons; layout's `top_y` shifts down accordingly.
- **Action-bar auto-fade** — buttons fade out when the cursor leaves the
  band, fade back in when it returns. Lerp at ~167 ms.
- **Visible drop-target overlay during drag** — a soft fill plus 3 px
  outline drawn ABOVE stacked cards for every legal target (full fanned
  column for tableaux, card-sized for foundations and empty tableaux).
  Replaces the previously invisible pile-marker tint.
- **Card drop shadows** — every card casts a neutral 25 % black shadow
  with a 4 px halo; cards in the active drag set switch to a lifted
  shadow (40 % alpha, larger offset, bigger halo).
- **Stock remaining-count badge** — small `·N` chip at the top-right of
  the stock pile so the player can see how close they are to a recycle.
  Hides when the stock empties.

### Changed

- **Foundations are unlocked.** `PileType::Foundation(Suit)` →
  `Foundation(u8)` (slot 0..3). The claimed suit is derived from the
  bottom card via `Pile::claimed_suit()` — no separate field, no
  claim-stuck-after-undo bugs. Any Ace lands in any empty slot, and the
  slot then claims that suit. `next_auto_complete_move` prefers a
  claim-matched slot before falling back to the first empty slot for
  Aces. Empty foundation markers render as plain placeholders (no
  "C/D/H/S").
- **HUD selection label** and **hint toast** read `claimed_suit()` and
  fall through to "Foundation N" / "move to foundation" only when the
  slot is empty.

### Fixed

- **`shared_fontdb` now bundles FiraMono.** The hayeah SVGs reference
  `Bitstream Vera Sans` and `Arial` by name. On minimal Linux installs
  / fresh Wayland sessions / chroots where neither is installed AND the
  CSS-generic aliases don't resolve, card rank/suit text vanished. The
  bundled font is loaded into fontdb and pinned as every CSS generic's
  target so the resolver always lands on something real. Surfaced when
  a second-machine pull rendered cards without glyphs.
- **Theme asset path resolution** — `AssetPath::resolve` (concatenates)
  → `resolve_embed` (RFC 1808 sibling resolution). Was producing paths
  like `…/theme.ron/hearts_4.svg` and failing to load every face SVG.
- **Sync exit log spam** — `push_on_exit` silently no-ops on
  `LocalOnlyProvider`'s `UnsupportedPlatform` instead of warn-spamming
  every shutdown.
- **usvg font-substitution warn spam** — custom `FontResolver.select_font`
  appends `Family::SansSerif` and `Family::Serif` to every query so
  unmatched named families silently fall through.

### Migration

- **In-progress saves invalidated.** `GameState.schema_version` bumped
  1 → 2; pre-v2 `game_state.json` files silently fall through to "fresh
  game on launch." Stats, progress, achievements, and settings live in
  separate files and are unaffected.

### Stats

- 982 passing tests (was 819 at v0.10.0).
- Zero clippy warnings under `--workspace --all-targets -- -D warnings`.

## [0.10.0] — 2026-04-29

PNG art pipeline plus a major dependency pass. The first release where
the binary shipped with bundled artwork.

### Added

- **52 individual card face PNGs** generated via `solitaire_assetgen`.
- **Custom font** (FiraMono-Medium) loaded via `AssetServer` at startup
  through the new `FontPlugin`.
- **Card backs and backgrounds** upgraded to 120×168 with richer
  patterns.
- **Ambient audio loop** wired through the kira mixer.
- **Arch Linux PKGBUILDs** for the game client and sync server (under
  the separate `solitaire-quest-pkgbuild` directory).
- **Workspace README, CI workflow, migration guide.**

### Changed

- **Bevy 0.15 → 0.18** workspace migration.
- **kira 0.9 → 0.12** audio backend migration.
- **Edition 2024**, MSRV pinned to **Rust 1.95**.
- **rand 0.9** upgrade.
- **Card rendering** moved from `Text2d` overlay to PNG-backed
  `Sprite` with face/back atlases; `Text2d` retained as a headless
  fallback when `CardImageSet` is absent (tests under MinimalPlugins).
- **Asset pipeline** switched from `include_bytes!()` for PNGs/TTFs to
  runtime `AssetServer::load()` so artwork can be swapped without a
  recompile. Audio remains embedded.
- **Removed Google Play Games Services sync backend** — redundant with
  the self-hosted server.

### Fixed

- **Server JWT secret** loaded at startup (was lazy, surfaced as
  intermittent 500s).
- **Daily-challenge race** in the server's seed-generation path.
- **Rate limiter** switched to `SmartIpKeyExtractor` so the limit
  applies per real client IP rather than per upstream proxy.
- **Touch input** uses `MessageReader<TouchInput>` (Bevy 0.18 rename).
- **Sync push/pull races** in async task scheduling.
- **Hot-path allocations** reduced in card-rendering systems.
- **Conflict report coverage** added for sync merge edge cases.

### Stats

- 819 passing tests at tag time.

## [0.9.0] — 2026-04-28

Initial public-tagged release. Established the workspace structure
(`solitaire_core` / `_sync` / `_data` / `_engine` / `_server` / `_app` /
`_assetgen`), the modal scaffold via `ui_modal`, the design-token system
in `ui_theme`, and the four-tier HUD layout. Foundations were
suit-locked at this point; cards rendered as `Text2d` rank/suit overlays
with no PNG artwork yet.

### Added

- Klondike core (Draw One / Draw Three modes).
- Progression system (XP, levels, 18 achievements, daily challenge,
  weekly goals, special modes at level 5).
- Self-hosted sync server (Axum + SQLite + JWT auth).
- All 12 overlay screens migrated to the `ui_modal` scaffold with real
  Primary/Secondary/Tertiary buttons.
- Animation upgrades: `SmoothSnap` slide curves, scoped settle bounce,
  deal jitter, win-cascade rotation.
- Splash screen, focus rings (Phases 1–3), tooltips infrastructure +
  HUD/Settings/popover applications, achievement integration tests,
  destructive-confirm verb unification, leaderboard error/idle states,
  first-launch empty-state polish, hit-target accessibility fix,
  CREDITS.md, persistent window geometry, mode-launcher Home repurpose,
  client-side sync round-trip integration tests.

[Unreleased]: https://github.com/funman300/Rusty_Solitaire/compare/v0.16.0...HEAD
[0.16.0]: https://github.com/funman300/Rusty_Solitaire/compare/v0.15.0...v0.16.0
[0.15.0]: https://github.com/funman300/Rusty_Solitaire/compare/v0.14.0...v0.15.0
[0.14.0]: https://github.com/funman300/Rusty_Solitaire/compare/v0.13.0...v0.14.0
[0.13.0]: https://github.com/funman300/Rusty_Solitaire/compare/v0.12.0...v0.13.0
[0.12.0]: https://github.com/funman300/Rusty_Solitaire/compare/v0.11.0...v0.12.0
[0.11.0]: https://github.com/funman300/Rusty_Solitaire/compare/v0.10.0...v0.11.0
[0.10.0]: https://github.com/funman300/Rusty_Solitaire/compare/v0.9.0...v0.10.0
[0.9.0]: https://github.com/funman300/Rusty_Solitaire/releases/tag/v0.9.0
