# Changelog

All notable changes to Ferrous Solitaire are documented here. The format is
based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and this
project follows [Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.28.0] — 2026-05-14

### Changed

- **Rename: Solitaire Quest → Ferrous Solitaire.** Android package id changed
  from `com.solitairequest.app` to `com.ferrousapp.solitaire`; existing installs
  must be uninstalled first (Android treats the new id as a new app).
  Data directory renamed from `solitaire_quest/` to `ferrous_solitaire/`.

### Fixed

- **BUG-3: Multi-modal stacking** (`hud_plugin.rs`). `handle_menu_button`
  now checks `scrims.is_empty()` — a `Query<(), With<ModalScrim>>` guard —
  before calling `spawn_menu_popover`. Tapping ≡ while any modal (Stats,
  Settings, Profile, Help) is open is now a no-op. Previously Stats + Profile
  could be open simultaneously.
- **UX-7: Help text single-line overflow** (`help_plugin.rs`). The HUD menu
  button description "Menu: Stats, Settings, Profile, Achievements" wrapped to
  two lines on Android. Shortened to "Open menu (Stats, Settings, Profile...)"
  which fits on one line. Verified on device.
- **UX-5b: Home mode glyph corruption** (`home_plugin.rs`). Mode selector icons
  were using Geometric Shapes block (U+25xx) absent from the bundled FiraMono
  font — rendered as missing-glyph rectangles on Android. Replaced with card
  suits (U+2660–2666) which FiraMono covers: ♦ Daily, ♥ Zen, ♠ Challenge.
- **UX-1: Modal Done button in gesture zone** (`safe_area.rs`). New
  `apply_safe_area_to_modal_scrims` Bevy system pads every `ModalScrim` bottom
  by `SafeAreaInsets.bottom / scale_factor`. Modal cards are now centred over
  the safe area, not the full physical screen. The Settings / Help / Stats Done
  buttons are reachable on gesture-nav Android devices. Verified on device.

---

## [0.23.0] — 2026-05-12

Phase 8 sync UI: the self-hosted-server connection flow is now fully
playable end-to-end. Players can open a Connect modal from Settings,
enter a server URL + credentials, log in or register, and see the
sync-status section update live. Token expiry auto-reopens the modal.
Account deletion ships a two-click destroy flow. Server deployment
artifacts (Dockerfile + docker-compose) let self-hosters spin up in one
command.

### Added

- **Sync setup modal — Connect / Disconnect flow** (`432061c`).
  New `SyncSetupPlugin` (`solitaire_engine/src/sync_setup_plugin.rs`)
  provides the full server-connection UI. Three tab-stopped text fields
  (URL, Username, Password) handle keyboard input via `MessageReader<KeyboardInput>`
  with focus cycling on Tab. "Log In" and "Register" buttons each spawn an
  async `AsyncComputeTaskPool` task that calls the new
  `SolitaireServerClient::login()` / `::register()` methods; `poll_auth_task`
  harvests the result, stores tokens via `store_tokens()`, hot-swaps
  `SyncProviderResource` to the new server backend, fires
  `ManualSyncRequestEvent` to pull immediately, and closes the modal.
  An inline `SyncAuthError` label displays credential errors without a
  toast. The modal is idempotent (`existing.is_empty()` guard) — safe
  to open programmatically.
- **`SyncConfigureRequestEvent`, `SyncLogoutRequestEvent`,
  `DeleteAccountRequestEvent`** (`432061c`). Three new engine events
  wire the Settings buttons → plugin handlers. `SyncConfigureRequestEvent`
  opens the setup modal; `SyncLogoutRequestEvent` disconnects and resets
  `SyncProviderResource` to `LocalOnlyProvider`; `DeleteAccountRequestEvent`
  opens the deletion confirmation modal.
- **Settings sync section — dynamic backend UI** (`432061c`).
  `sync_row()` in `SettingsPlugin` now takes `backend: &SyncBackend` and
  renders conditionally: `Local` → "Connect" button; `SolitaireServer` →
  username label + "Sync Now" + "Disconnect" + "Delete Account". Three new
  `SettingsButton` discriminants (`ConnectSync` tab 91, `DisconnectSync`
  tab 92, `DeleteAccount` tab 93) feed into a new `handle_sync_buttons`
  system extracted from `handle_settings_buttons` to stay within Bevy's
  16-parameter system limit.
- **`SolitaireServerClient::login()` and `::register()`** (`432061c`).
  Both POST to `/api/auth/login` and `/api/auth/register` respectively.
  Private helper `extract_auth_tokens` parses `{ access_token, refresh_token }`.
  409 CONFLICT → "username already taken"; 401/403 → "invalid credentials";
  400 → server message echoed to the player.
- **Re-auth prompt on token expiry** (`6ce5564`).
  `poll_pull_result` in `SyncPlugin` now fires `InfoToastEvent("Session
  expired — please reconnect")` + `SyncConfigureRequestEvent` when the
  pull task resolves to `SyncError::Auth(_)`. Because the modal is
  idempotent the re-open is safe to trigger from any system path.
- **Server deployment artifacts** (`6ce5564`).
  `solitaire_server/Dockerfile`: multi-stage build (`rust:1.95-slim` →
  `debian:bookworm-slim`); copies `.sqlx` offline cache so `SQLX_OFFLINE=true`
  succeeds without a live database at build time; exposes port 8080.
  `solitaire_server/docker-compose.yml`: single-service compose file;
  `db-data` volume at `/app/data`; `DATABASE_URL` and `JWT_SECRET` from
  environment; HTTP health-check via `wget`. `solitaire_server/.env.example`:
  documents all required variables with generation hint (`openssl rand -hex 32`).
- **Account deletion flow** (`272d31f`).
  "Delete Account" in Settings fires `DeleteAccountRequestEvent` →
  `SyncSetupPlugin::open_delete_confirm_modal` spawns a danger-red
  confirmation modal with "Cancel" and "Delete Forever" buttons.
  "Delete Forever" submits an async `PendingDeleteTask` that calls
  `SyncProvider::delete_account()`; `poll_delete_task` on Ok fires
  `SyncLogoutRequestEvent` + a success toast; on Err shows an error toast
  and leaves the modal open. Two-click destroy pattern — no accidental
  account deletion possible.

### Removed

- **`SyncAuthResultEvent`** (`432061c`). Defined but never emitted or
  consumed; removed as dead code.

### Stats

- Tests: **1300+ passing** / 0 failing
- Clippy: clean
- Crates touched: `solitaire_data` (sync_client), `solitaire_engine`
  (events, settings_plugin, sync_plugin, sync_setup_plugin [new], lib),
  `solitaire_app` (lib.rs), `solitaire_server` (Dockerfile,
  docker-compose.yml, .env.example [new])

## [0.22.0] — 2026-05-08

Adds difficulty-tier game selection, Android JNI bridges for keystore and
clipboard, Play-by-Seed dialog, and double-tap auto-move on touch screens.
Also closes the Prev/Next replay-selector spawn-site item carried since v0.19.0.

### Added

- **Difficulty-tier game mode** (this release).
  `DifficultyLevel` enum (`Easy / Medium / Hard / Expert / Grandmaster /
  Random`) added to `solitaire_core::game_state` alongside a new
  `GameMode::Difficulty(DifficultyLevel)` variant. Five pre-verified seed
  catalogs (40 seeds each, 200 total) are generated by the new
  `gen_difficulty_seeds` binary in `solitaire_assetgen`; each catalog
  contains seeds proven winnable at progressively larger solver budgets
  (1 K → 200 K moves). `DifficultyPlugin` resolves `StartDifficultyRequestEvent`
  → catalog seed → `NewGameRequestEvent`; the `Random` tier uses a
  system-time seed and intentionally bypasses the winnable-only filter.
  The home overlay gains an expandable `▶ Difficulty` section between the
  Draw Mode row and the mode-card grid; the last-played tier is persisted
  in `Settings::last_difficulty` and pre-expands/highlights on re-open.
  Difficulty wins pool into Classic stats (no separate buckets).
- **Prev/Next replay selector in the Stats overlay** (`a449f60`).
  `ReplayPrevButton`, `ReplayNextButton`, `ReplaySelectorCaption`, and
  `ReplaySelectorDetail` nodes now spawn inside `spawn_stats_screen`
  as a flex row of two bordered chips flanking a `"Replay N / M"`
  caption, with a detail line below showing the selected replay's
  duration + date and an optional `"· Shareable"` badge. Both chips
  carry `ModalButton(Secondary)` so the existing `repaint_modal_buttons`
  paint loop gives them hover/press feedback at zero extra cost.
  `repaint_replay_selector_detail` is wired into the existing
  `.chain()` alongside `handle_replay_selector_buttons` and
  `repaint_replay_selector_caption`. The click handler and repaint
  systems have been registered (and dormant) since v0.19.0; this
  commit is purely the missing spawn site.
- **6 new selector unit tests** (`a449f60`). Covers: spawn-site
  presence (Prev, Next, Caption, Detail all spawn with the screen),
  caption initial text ("Replay 1 / 1"), detail initial text
  ("{dur} win on {date}"), Shareable badge when `share_url` is set,
  empty-history "No replays" caption, and ordinal wrapping.
  `make_test_replay(time_seconds, share_url)` helper encapsulates
  `Replay::new(...)` + `chrono::NaiveDate`.

### Fixed

- **`const { assert!() }` for dim-layer z-order test** (`a449f60`).
  Converted `assert!(Z_REPLAY_DIM < Z_REPLAY_OVERLAY, …)` in
  `replay_overlay` tests to `const { assert!(…) }` to satisfy
  `clippy::assertions_on_constants` (constant-fold at compile time
  rather than a runtime no-op).

### Added (post-cut, same pending release)

- **Double-tap auto-move on touch screens** (`395a322`).
  `handle_double_tap` fires `MoveRequestEvent` (single card to
  foundation/tableau, or a whole face-up stack via
  `best_tableau_destination_for_stack`) when two `TouchPhase::Ended`
  events on the same card arrive within `DOUBLE_TAP_WINDOW` (0.5 s,
  slightly wider than the mouse `DOUBLE_CLICK_WINDOW` to account for
  touch latency). If no legal destination exists, fires
  `MoveRejectedEvent` (audio + visual rejection feedback).  The system
  is inserted into the touch drag chain immediately before
  `touch_end_drag` so `DragState.active_touch_id` and `committed` are
  still readable; the tap timestamp is tracked in a `Local<HashMap<u32,
  f32>>` keyed by card ID.
- **Play-by-Seed dialog** (`0cb1587`).
  `PlayBySeedPlugin` adds a numeric-input modal that accepts a decimal
  seed, runs a solver preview in the background (debounced 500 ms via
  `AsyncComputeTaskPool`), and shows a win/no-win verdict before
  dealing.  A new `HomeMode::PlayBySeed` card in the home overlay fires
  `StartPlayBySeedRequestEvent`; the handler in `PlayBySeedPlugin`
  spawns the dialog.  Digit, Backspace, Enter (confirm), and Escape
  (cancel) are handled via `ButtonInput<KeyCode>`.  Five unit tests
  cover spawn, digit append, buffer read, confirm, and cancel paths.
- **75 new challenge seeds** (`2062bd0`).
  New `gen_seeds` binary in `solitaire_assetgen` brute-searches seeds
  in the `0xCAFEBABE…` namespace and filters for hands solvable in
  ≤250 moves via the core solver.  The 75 confirmed-win seeds are
  appended to `CHALLENGE_SEEDS` in `solitaire_data::challenge`.

### Fixed (post-cut, same pending release)

- **Gate `handle_fullscreen` to non-Android** (`45436d0`).
  F11 fullscreen toggle makes no sense on Android (the OS owns window
  sizing); the fn and its `MonitorSelection`/`WindowMode` imports are
  now `#[cfg(not(target_os = "android"))]`-gated.  The `add_systems`
  call is extracted as a separate statement so `#[cfg]` can annotate it
  (attributes cannot appear mid-chain in Rust).
- **Android APK launch: export `android_main`** (`202a64d`).
  `NativeActivity` dlopen-s `libsolitaire_app.so` and calls
  `android_main` as its entry point. Without the symbol the app
  crashed immediately with `UnsatisfiedLinkError`. The new function
  sets `bevy::android::ANDROID_APP` (required by `WinitPlugin`) then
  delegates to `run()` — equivalent to what `#[bevy_main]` would
  generate, but usable on an arbitrary entry point name.
- **Android APK launch: gate `resize_constraints` to non-Android**
  (`202a64d`). On Android `max_width/max_height` default to `0.0`;
  Bevy's clamp panicked with `min=800 > max=0`.
- **Android APK launch: gate `apply_smart_default_window_size` to
  non-Android** (`202a64d`). The system calls `.clamp(800.0,
  logical_w)` which panics when the emulator reports zero window
  dimensions during early Android lifecycle events. The OS controls
  window size on Android; the system is irrelevant there.
- **Ignore `.idea/` IDE project files** (`16242e6`). Android Studio
  created `.idea/` when the project was opened during APK
  verification; added to `.gitignore` and removed the accidentally-
  committed files.

### Android verification result

APK boots on `x86_64-linux-android` in a Pixel_7 AVD (Android 14 /
API 34, SwiftShader Vulkan). App runs for 2+ minutes without crashing.
Bevy renderer initialises, splash screen loads. This is the first
confirmed end-to-end device run.

### Stats

- Tests: **1300+ passing** / 0 failing
- Clippy: clean
- Crates touched: `solitaire_core` (game_state), `solitaire_data`
  (settings, stats, difficulty_seeds, challenge), `solitaire_engine`
  (events, difficulty_plugin, home_plugin, hud_plugin, win_summary_plugin,
  input_plugin, play_by_seed_plugin, lib), `solitaire_app` (lib.rs),
  `solitaire_assetgen` (gen_difficulty_seeds + gen_seeds binaries)

## [0.21.8] — 2026-05-08

Patch release for replay-overlay polish. Through-line:
**notch-label centering + WIN MOVE HC legibility + HC system extension**.
All three items were "optional polish" flagged in the v0.21.7 handoff;
all three ship in two commits.

### Added

- **`STATE_SUCCESS_HC` constant** (`c50eaf8`). Brighter lime
  (`#c8e862`, L≈0.73) in `ui_theme` for use wherever the
  standard `STATE_SUCCESS` (`#acc267`, L≈0.51) needs extra
  luminance under HC mode. Sits above the bumped notch ticks
  (`BORDER_SUBTLE_HC` gray, L≈0.60) so a WIN MOVE marker at
  this colour is unambiguous.
- **`HighContrastBackground::with_hc(default, hc)` constructor**
  (`c50eaf8`). Extends `HighContrastBackground` with an
  `hc_color: Color` field (default = `BORDER_SUBTLE_HC` via
  `with_default()`). `update_high_contrast_backgrounds` now
  reads `marker.hc_color` instead of the hardcoded constant —
  backwards-compatible; all existing `with_default()` usages
  continue to bump to gray.
- **WIN MOVE scrub-bar marker HC bump** (`c50eaf8`). Marker
  now carries `HighContrastBackground::with_hc(STATE_SUCCESS,
  STATE_SUCCESS_HC)` so the lime stays lime under HC (brighter
  lime rather than gray). Pin test locks both the default and
  HC colour fields on the spawned entity.

### Fixed

- **Scrub-bar notch-label centering** (`b44d277`). Middle
  three labels ("25%", "50%", "75%") previously had their
  left edge at the notch; now their text centre coincides
  with the notch tick. Implemented using the CSS
  `translateX(-50%)` pattern for Bevy 0.18 UI: a fixed
  `SCRUB_LABEL_CENTER_WIDTH = 36 px` container with
  `margin.left = -18 px` is placed at `left: Percent(pct)`,
  and `Justify::Center` centres the text within it. Endpoint
  labels ("0%", "100%") keep their flush-left / flush-right
  anchoring. `with_default()` remains one-argument.

### Stats

- Tests: 1276 passing / 0 failing (engine: 831)
- Clippy: clean
- Crates touched: `solitaire_engine` (replay_overlay.rs,
  ui_theme.rs, settings_plugin.rs)

## [0.21.7] — 2026-05-08

Patch release closing the last major B-2 sub-piece. Through-line:
**mini-tableau preview dim layer**. The mockup's "Game Peek Band at
50 % opacity" is now implemented as a full-screen UI scrim that darkens
the card world during replay so the chrome (banner + move-log panel)
reads clearly against the scene.

### Added

- **Full-screen tableau dim layer** (`da3e542`). Spawns a
  `ReplayTableauDimLayer` UI node (100 % × 100 %, 50 % opacity
  black) at `Z_REPLAY_DIM = Z_REPLAY_OVERLAY − 1 = 54` whenever
  a replay starts; despawned alongside the banner and move-log
  panel when the replay ends. Bevy's UI/world compositor means
  no changes to `card_plugin` are needed — UI nodes always
  render above world-space sprites regardless of `Transform.z`.
  The dim layer carries no `Interaction` component (purely
  visual; pointer events pass through). Adds `Z_REPLAY_DIM`
  and `TABLEAU_DIM_ALPHA` constants plus two new tests:
  lifecycle (spawn/despawn mirrors the floating-chip pattern)
  and z-ordering invariant (`Z_REPLAY_DIM < Z_REPLAY_OVERLAY`
  pinned). 1275 tests pass / 0 failing.

### Stats

- Tests: 1275 passing / 0 failing
- Clippy: clean
- Crates touched: `solitaire_engine` (replay_overlay.rs)

## [0.21.6] — 2026-05-08

Patch release for the post-v0.21.5 work. Through-line:
**Move Log panel + scrub-UX polish**. v0.21.5 closed out the
keyboard-accelerator surface (Space / Esc / ← / →) and the
keybind footer; v0.21.6 builds on that with two parallel
threads — accessibility + scrub-on-hold polish for the v0.21.5
surfaces, plus a brand-new Move Log panel anchored to the
viewport's bottom edge that gives players a 5-row recent-and-
upcoming move history alongside the existing top-edge banner.

The Move Log panel is the first replay-overlay surface that
*isn't* attached to the banner — it lives at a separate screen
anchor (bottom: 0) with its own spawn/despawn lifecycle.
Establishes the pattern for "multi-anchor replay UI" that the
remaining B-2 sub-piece (mini-tableau preview) will inherit.

### Added

- **HC-mode coverage for the scrub track + quarter-mark notch
  ticks** (`d3cb1a5`). Adds parallel primitive
  `HighContrastBackground` to `ui_theme` and a paint system
  `update_high_contrast_backgrounds` in `settings_plugin` that
  mirrors the existing border-marker pattern but targets
  `BackgroundColor` instead of `BorderColor`. Tags the 1 px
  scrub track Node and all five quarter-mark notch ticks so
  they bump from `BORDER_SUBTLE` (`#505050`) →
  `BORDER_SUBTLE_HC` (`#a0a0a0`) under HC mode. Scrub fill
  (`ACCENT_PRIMARY`) and WIN MOVE marker (`STATE_SUCCESS`)
  don't get the marker — accent and state colours are already
  saturated and don't need an HC luminance variant.
- **Continuous scrub on key-held arrow keys** (`2e25476`).
  Holding ← or → triggers continuous step at 100 ms cadence
  (10 steps/sec) — matches the mockup's `[← →] scrub`
  terminology while keeping single-press = single-step
  semantics. Per-key accumulators in a new
  `ReplayScrubKeyHold` resource; `just_pressed` events bypass
  the accumulator and fire immediately. Release resets to 0
  so the next fresh press fires immediately rather than at
  half-interval.
- **Move Log panel** (`d6f32d3` + `140251b` + `e7345ae` +
  `4437a1a`). New bottom-edge UI panel showing a 5-row window
  onto recent + upcoming moves: 2 prev rows above the active
  row + active row highlighted in `ACCENT_PRIMARY` + 2 next
  rows below. Header reads `▌ MOVE LOG · N/M` (or
  `▌ MOVE LOG · COMPLETE` when finished). Active row carries
  a `▶` focus prefix and `TEXT_PRIMARY_HC` text colour for
  legible contrast against the brick-red highlight. Prev /
  next rows render in `TEXT_SECONDARY` so the active row
  stays the focal point.
  - Sibling-of-banner pattern (separate root entity anchored
    at viewport bottom, not a banner child) — same
    spawn/despawn lifecycle as `ReplayFloatingProgressChip`,
    different screen anchor.
  - Five pure helpers handle the formatting:
    `format_pile`, `format_move_body`,
    `format_move_log_header`, `format_kth_recent_row` (active
    + prev), `format_kth_next_row` (next). 1-indexed display
    numbers throughout (`Foundation(2)` reads as "foundation
    3" rather than the enum's 0-index).
  - Panel grows from 56 → 84 → 112 px across the four
    move-log commits. `MOVE_LOG_PREV_ROWS` and
    `MOVE_LOG_NEXT_ROWS` constants (both = 2) parameterise
    the row count; `format_kth_recent_row` and
    `format_kth_next_row` return empty for out-of-range k so
    panels gracefully under-fill at the start (cursor=1) and
    end (cursor=N-1) of a replay.
  - HC marker on the panel's top border so the 1 px edge
    bumps under HC mode (same pattern as the keybind footer).

### Changed

- **`react_to_state_change` despawns the Move Log panel** on
  `Playing → Inactive` alongside the banner root and floating
  progress chip. Third query in the same defer-and-despawn
  cycle.
- **Move Log panel height grew 56 → 84 → 112 px** across the
  prev-rows and next-rows commits. The panel is sized to fit
  the chosen row count + header + padding; tunable via the
  `MOVE_LOG_PANEL_HEIGHT` const.
- **`format_active_move_row` now prefixes the `▶` focus
  marker** (`e7345ae`). Wraps `format_kth_recent_row(state, 1)`
  and prepends the prefix when the body is non-empty. Empty
  case still returns empty — cursor=0 doesn't paint a stray
  `▶` on an otherwise-empty row.

### Documentation

- `SESSION_HANDOFF.md` refreshed twice this cycle — once
  recording the HC paint + continuous-scrub polish, then
  again as the Move Log arc shipped commit-by-commit. The
  Resume menu's B option now traces the full arc:
  notches → labels → footer → ESC → HC → arrow keys →
  HC paint → continuous scrub → move log.

### Stats

- **1273 passing tests / 0 failing** across the workspace
  (net +23 from v0.21.5's 1250 baseline):
  - 2 from `d3cb1a5` (HC marker on track + notches).
  - 2 from `2e25476` (continuous-scrub repeat-while-held +
    release-resets-accumulator).
  - 8 from `d6f32d3` (move-log panel init + 5 helpers + 3
    spawn / lifecycle scenarios).
  - 4 from `140251b` (prev rows: helper k coverage + spawn
    cardinality + spawn texts + repaint on cursor advance).
  - 3 from `e7345ae` (active row highlight: wrapper bg +
    text colour + focus prefix + cursor=0 stays empty).
  - 4 from `4437a1a` (next rows: helper k coverage + spawn
    cardinality + spawn texts + under-fill at replay end).
- Clippy clean across the workspace.

## [0.21.5] — 2026-05-08

Patch release for the post-v0.21.4 work. One through-line:
**replay-overlay scrubbing affordances + accessibility**. v0.21.4
shipped pause / resume / step + the WIN MOVE marker as the first
*scrubbing-shaped* additions to the replay overlay; v0.21.5
fills out the rest of the scrubbing UX so the player has both
visual anchor points (notches + labels) and a complete keyboard
control surface (Space / Esc / ← / →) for navigating a paused
replay.

Two of the six commits in this cycle are layout-changing — they
grow the banner height from 60 px → 76 px → 92 px to make room
for the notch labels and keybind footer. Banner geometry was
fixed for every prior B-2 commit; this release establishes the
"grow the container, add a flex-column child" pattern that the
remaining B-2 sub-pieces (move-log scroller, mini-tableau
preview) will inherit when they land.

### Added

- **Quarter-mark scrub-bar notches** (`fe68861`). Five 1 px
  vertical ticks at 0 / 25 / 50 / 75 / 100 % give the player
  visual anchor points without needing to mentally bisect the
  bar. Pure helper `scrub_notch_positions()` returns the fixed
  array; spawn loop sits next to the WIN MOVE marker spawn so
  the lifecycles match. Notches paint in `BORDER_SUBTLE` (same
  as the unfilled track) and rely on extending past the 1 px
  track (5 px tall, anchored 2 px above the track top) for
  visibility — same trick the WIN MOVE marker uses. Spawned
  *after* the WIN MOVE marker so a notch and the marker
  landing on the same percentage paint the marker on top.
- **Percentage labels under each notch** (`d322abf`). Five
  `0%` / `25%` / `50%` / `75%` / `100%` labels in a new 16 px
  row beneath the 1 px scrub track give the player explicit
  quarter-mark readouts. Banner grew from 60 → 76 px to
  accommodate the row — first **layout-changing** commit in
  the B-2 arc. Pure helper `scrub_notch_labels()` returns the
  fixed array, paired index-for-index with
  `scrub_notch_positions()`. Spawn loop applies an "endpoints
  flush, middle three percent-anchored" positioning pattern:
  leftmost label gets `left: 0`, rightmost gets `right: 0`,
  middle three anchor at `left: Val::Percent(p)` since Bevy
  0.18 UI lacks a clean CSS-style `translate-x: -50%`
  centering primitive. Label colour is `TEXT_SECONDARY`
  rather than the mockup's `BORDER_SUBTLE` (the latter would
  match the notches but is too low-contrast against
  `BG_ELEVATED_HI` to read at 12 px).
- **Keybind-hint footer** (`1873b3f`). Vim-style mode line on
  the left (`▌ NORMAL │ replay`) plus a keybind hint on the
  right at the bottom edge of the banner. Banner grew from
  76 → 92 px to fit the 16 px footer row. Surfaces every
  wired keyboard accelerator visually so CLAUDE.md §3.3's
  UI-first contract holds for keyboard accelerators too. The
  footer lists *only* keybinds that are actually wired —
  the only-wired-keybinds discipline means each release
  cycle's hint string is a precise honest contract with the
  player. Two pure helpers (`keybind_footer_mode_text`,
  `keybind_footer_hint_text`) keep the static text testable.
  1 px top border in `BORDER_SUBTLE` separates the footer
  from the labels row.
- **ESC keyboard accelerator for replay-stop** (`90e24d9`).
  New `handle_stop_keyboard` system parallels
  `handle_pause_keyboard` in shape — fires only when state
  is `Playing`, calls `stop_replay_playback`. Cross-plugin
  coordination via `pause_plugin::toggle_pause`: added a
  fourth defer-if check
  (`replay_state.is_some_and(|s| s.is_playing())`) right
  after the existing `other_modal_scrims` check so ESC
  during active replay belongs to the replay overlay, not
  the pause modal.
- **HC-mode coverage for the keybind-footer top border**
  (`23902cd`).
  `HighContrastBorder::with_default(BORDER_SUBTLE)` marker
  on the footer's border-carrying Node so the existing
  `apply_high_contrast_borders` system bumps the 1 px top
  border from `#505050` → `#a0a0a0` when
  `Settings::high_contrast_mode` is on. Without the marker
  the footer reads as floating loose under HC because the
  border that anchors it to the labels row is
  near-invisible.
- **← / → keyboard accelerators for paused stepping**
  (`e5c4f51`). New `step_backwards_replay_playback` in
  `replay_playback.rs` decrements the cursor and dispatches
  `UndoRequestEvent`; the game's `handle_undo` reads it
  next frame to reverse its most-recent move. Hooks the
  existing undo system rather than replaying-forward-from-
  zero — every replay-applied move pushes to the undo stack
  the same way a player move would, so undo is the right
  reversal primitive. Both arrow keys are paused-only via
  the same destructure-gate pattern the forward step uses.
  The mockup labels these `[← →] scrub`; single-move step
  is the closest behaviour shippable today, so the footer
  hint reads `[← →] step` — only-wired-keybinds discipline.

### Changed

- **Banner height grew 60 → 76 → 92 px** across two
  layout-changing commits (`d322abf` then `1873b3f`). Top
  row's `flex_grow: 1.0` still consumes 59 px so the
  existing content (label / progress chip / buttons) has
  the same vertical space; the new rows (16 px labels +
  16 px footer) extend the banner downward into the
  gameplay area. Banner geometry is now mutable — every
  prior B-2 commit fit inside fixed 60 px space.
- **Keybind-footer hint text grew alongside the wirings**:
  `[SPACE] pause/resume` →
  `[SPACE] pause/resume · [ESC] stop` →
  `[SPACE] pause/resume · [ESC] stop · [← →] step`.
- **`pause_plugin::toggle_pause` now defers when a replay
  is active** (`90e24d9`). Adds a fourth defer-if check to
  the existing modal-stack pattern.
- **`ReplayOverlayPlugin` registers
  `add_message::<UndoRequestEvent>()`** (`e5c4f51`).
  Defensive registration so the plugin runs cleanly under
  `MinimalPlugins` without `GamePlugin` attached.

### Documentation

- `SESSION_HANDOFF.md` refreshed five times this cycle.
  The B option in the Resume menu now traces the full arc:
  notches → labels → footer → ESC → HC → arrow keys.
- The pre-existing `daily_challenge` warning test that
  fails when wall-clock UTC is within 30 minutes of
  midnight is documented in this cycle's handoff. Same
  shape as the earlier `winnable_seed_search` flake —
  time-dependent, deterministically passes outside the
  trigger window.

### Stats

- **1250 total tests / 1249 passing / 1 pre-existing
  time-dependent flake** across the workspace (net +22 from
  v0.21.4's 1228 baseline):
  - 4 from `fe68861` (scrub-notch coverage)
  - 4 from `d322abf` (notch-label coverage)
  - 4 from `1873b3f` (keybind-footer coverage)
  - 3 from `90e24d9` (ESC-accelerator coverage)
  - 1 from `23902cd` (HC-marker coverage)
  - 6 from `e5c4f51` (arrow-keyboard coverage)
- **Pre-existing flake**:
  `daily_challenge_plugin::tests::check_system_fires_warning_event_only_once_per_day`
  fails when wall-clock UTC is within 30 minutes of
  midnight. Verified pre-existing by stash-and-retest
  before each commit. Will pass deterministically outside
  the trigger window. Not introduced by this release.
- Clippy clean across the workspace.

## [0.21.4] — 2026-05-08

Patch release for the post-v0.21.3 work. One through-line:
**replay-scrubbing accessibility**. The replay overlay used to be
pure-passive — the player started a replay, watched it execute,
and waited for it to end. v0.21.4 adds the scaffolding for
*navigating within* a replay: a WIN MOVE marker on the scrub bar
so the player can see at a glance where the winning move sits,
and pause / resume / step controls so they can stop on any move
and inspect the board.

The work is also the first three commits on the B-2 replay
screen-takeover redesign arc. The remaining pieces (screen-
takeover layout, move-log scroller, mini-tableau preview) are
deferred to a future cycle because they need a layout reflow
that the existing banner-only overlay can't carry.

### Added

- **`Replay::win_move_index: Option<usize>` data field**
  (`ab857bb`). Additive optional field on the persisted
  `Replay` shape. `#[serde(default)]` keeps older
  `latest_replay.json` / `replays.json` files loadable without
  bumping `REPLAY_SCHEMA_VERSION` — this is purely additive.
  Populated at the live recording site
  (`game_plugin::handle_game_won`) via a new builder-style
  setter `Replay::with_win_move_index`. For fresh recordings
  the value is always `Some(moves.len() - 1)` because recording
  freezes on win, but storing it explicitly lets the playback
  UI read the WIN MOVE position directly without re-deriving
  on every render.
- **WIN MOVE scrub-bar marker** (`52befa6`). New
  `ReplayOverlayWinMoveMarker` component spawned as a sibling
  to `ReplayOverlayScrubFill` under the 1px scrub track,
  absolute-positioned at `replay.win_move_index / total %` of
  the bar. Painted in `STATE_SUCCESS` (green) so the marker
  reads as "this is where the win lives." Pure helper
  `win_move_marker_pct` returns `None` for any state where the
  marker shouldn't draw (Inactive, Completed, replay missing
  the field, empty move list); percentage clamps to `[0, 100]`
  defensively. Spawn-time only — the position never changes
  during a single playback because the underlying `Replay` is
  immutable while `Playing`.
- **Pause / Resume / Step playback controls** (`fbe48ac`). New
  `paused: bool` field on `ReplayPlaybackState::Playing`.
  `tick_replay_playback` skips the `secs_to_next` decrement
  entirely while paused so cursor and timer freeze together;
  resuming starts the next move from a full interval. New
  public API: `toggle_pause_replay_playback` and
  `step_replay_playback` (the latter hard-gated to `Playing {
  paused: true }` via the destructure pattern itself, so
  manual stepping can't race the tick loop). On-screen Pause
  and Step buttons sit alongside the existing Stop button;
  `Space` keyboard accelerator toggles pause / resume.
- **`Replay::with_win_move_index` builder** (`ab857bb`).
  Chainable setter so the recording site can write
  `Replay::new(...).with_win_move_index(idx)`. Keeps
  `Replay::new`'s signature stable across the 13+ existing
  test-fixture call sites that don't care about the field.

### Changed

- **`Replay::new` writes `win_move_index: None`** (`ab857bb`).
  Existing canonical constructor stays signature-compatible
  with all existing callers. The field is opt-in via the
  builder.
- **`game_plugin::handle_game_won` populates the new field**
  (`ab857bb`). The recording site computes
  `recording.moves.len().checked_sub(1)` as the win-move
  index. `checked_sub` rather than direct subtraction guards
  the unreachable empty-recording branch (which is also
  guarded earlier in the function).
- **`tick_replay_playback` honors the new `paused` flag**
  (`fbe48ac`). Skipping the timer decrement is the only
  behavior change; the loop body and Completed-detection are
  unchanged. Stepping fires moves directly via
  `step_replay_playback`, bypassing the tick path entirely.
- **Pause / Resume button label is reactive** (`fbe48ac`).
  `update_pause_button_label` walks `Children` from the
  marked button to its inner `Text` and repaints the label
  whenever `ReplayPlaybackState` changes. Pure helper
  `pause_button_label` covers all four state arms (running,
  paused, inactive, completed).
- **25 existing `Playing { ... }` construction sites gained
  `paused: false`** (`fbe48ac`). Mechanical edit across
  `replay_overlay`, `achievement_plugin`, and
  `replay_playback` tests to satisfy the new field
  requirement. No behavioral change.

### Documentation

- `SESSION_HANDOFF.md` refreshed three times this cycle —
  once after each post-cut feature commit. The B-2 entry in
  the Visual-identity follow-ups list now points at the
  remaining sub-pieces (screen-takeover layout, move-log
  scroller, mini-tableau preview) as a single multi-session
  arc rather than three independent ones, since they share a
  layout-reflow prerequisite.

### Stats

- **1228 passing tests / 0 failing** across the workspace
  (net +21 from v0.21.3's 1207 baseline):
  - 5 from `ab857bb`'s `win_move_index` coverage: default
    constructor, builder set / set-None, on-disk round-trip,
    legacy-JSON-loads-with-None backward-compat. The last
    test pins the no-schema-bump claim — if a future refactor
    drops the `#[serde(default)]`, that test catches it.
  - 8 from `52befa6`'s WIN MOVE marker: pure-helper truth
    table (Inactive / Completed / no-field / correct-position
    / clamp) + spawn-presence-with-field /
    spawn-absence-without / despawn-with-overlay observables.
  - 8 from `fbe48ac`'s playback controls: label truth table,
    label repaint on state change, click-toggles-paused,
    step advances cursor by exactly one with paused
    preserved, step-while-running no-op, Space toggles
    paused.
- Zero clippy warnings under `cargo clippy --workspace
  --all-targets -- -D warnings`.
- `cargo test --workspace` clean.

## [0.21.3] — 2026-05-08

Patch release for the post-v0.21.2 work. One through-line:
**accessibility arc closure**. v0.21.2 explicitly carved out
"dynamic-paint sites" (HUD action buttons, modal buttons, radial
menu rim) on the assumption that their existing paint cycles would
race the central `update_high_contrast_borders` system. v0.21.3
walks the actual code, finds the carve-out was over-cautious, and
closes it. Bonus: the first real consumer of `ToastVariant::Warning`
also lands here, making the `ToastVariant` enum fully load-bearing
(every variant has at least one driver).

### Added

- **`WarningToastEvent(String)` — first `ToastVariant::Warning`
  consumer** (`279e23d`). Generic carrier message that any system
  can fire to spawn a 4 s amber-bordered fire-and-forget toast.
  Mirrors the v0.21.2 `MoveRejectedEvent` → `Error` toast wiring:
  domain message crosses the plugin boundary, the animation
  plugin's `handle_warning_toast` system reads it and spawns. Not
  queued (Warning is alert-shaped, not info-shaped — should never
  block on a queue).
- **Daily-challenge-expiry warning** (`279e23d`). First in-engine
  driver of `WarningToastEvent`. New
  `daily_challenge_plugin::check_daily_expiry_warning` system
  fires at most once per `DailyChallengeResource::date` when the
  player is within 30 min of UTC midnight reset and today's
  challenge isn't yet complete. Suppression decided by a pure
  helper (`compute_expiry_warning_minutes`) covering: already-
  completed-today, already-shown-for-this-date, outside the
  threshold window, post-midnight rollover. Pure-helper-plus-
  thin-system shape because `Utc::now()` can't be pinned without
  injecting a clock resource — overkill for one consumer.
- **`radial_rim_outline` pure helper** (`c153363`). Decision
  logic for the radial-menu rim outline colour. Resting outlines
  always carry `BORDER_SUBTLE`; focused outlines carry
  `BORDER_STRONG` normally and `BORDER_SUBTLE_HC` under HC. Naive
  marker substitution would invert the focused-vs-resting
  hierarchy because `BORDER_SUBTLE_HC` (`#a0a0a0`) is *lighter*
  than `BORDER_STRONG` (`#505050`); folding the choice in here
  keeps the focused rim more visible under HC, not less.

### Changed

- **HC marker pattern extended to HUD action buttons + modal
  buttons** (`c153363`). Re-reading the code revealed both sites'
  paint systems (`paint_action_buttons`, `paint_modal_buttons`)
  only mutate `BackgroundColor` — `BorderColor` is set once at
  spawn and never touched. So the existing
  `HighContrastBorder::with_default(BORDER_SUBTLE)` marker
  pattern works cleanly for both, no race. v0.21.2's carve-out
  comment was based on assumed-but-not-actual race risk; this
  cycle treats it as the doc-vs-implementation drift pattern in
  the wild and verifies before trusting.
- **Radial menu rim folds HC into per-frame respawn**
  (`c153363`). The rim is the only true dynamic-painter of the
  three carved-out sites — `radial_redraw_overlay` despawns and
  respawns all rim sprites every frame the radial is `Active`.
  The `HighContrastBorder` marker can't apply (entities don't
  persist across frames) so HC is read directly in the system
  via `Option<Res<SettingsResource>>` and routed through
  `radial_rim_outline`. The `Option<Res<...>>` shape preserves
  test compatibility under `MinimalPlugins`.
- **Animation plugin registers `WarningToastEvent`** (`279e23d`).
  Joins `InfoToastEvent`, `MoveRejectedEvent` etc. in
  `AnimationPlugin::build`. Daily-challenge plugin also
  registers it (idempotent) so the message exists when running
  the daily plugin under `MinimalPlugins` without the animation
  plugin attached.

### Documentation

- `SESSION_HANDOFF.md` refreshed twice this cycle — once after
  the Toast Warning wiring (menu trimmed 5 → 4 options), and
  again after the HC dynamic-paint rollout (menu trimmed 4 → 3,
  with all remaining options now flagged as multi-session). The
  `High-contrast accessibility mode` entry in the Visual-identity
  follow-ups list is updated to reflect that no "un-tagged
  because race-risk" surfaces remain.

### Stats

- **1207 passing tests / 0 failing** across the workspace
  (net +12 from v0.21.2's 1195 baseline):
  - 7 tests for `compute_expiry_warning_minutes` (`279e23d`)
    covering each suppression rule + the inclusive boundary at
    exactly 30 min remaining.
  - 1 in-Bevy test (`check_system_fires_warning_event_only_once_per_day`)
    pinning `DailyExpiryWarningShown`'s once-per-date
    suppression and the symmetric "already-completed-today"
    suppression.
  - 4 truth-table tests for `radial_rim_outline` (`c153363`):
    focused × HC. The "resting stays subtle under HC" test
    explicitly documents *why* — it's the hierarchy-preservation
    invariant a future refactor might be tempted to break.
- Zero clippy warnings under `cargo clippy --workspace
  --all-targets -- -D warnings`.
- `cargo test --workspace` clean.

## [0.21.2] — 2026-05-08

Patch release for the post-v0.21.1 polish work. Three through-
lines: **accessibility extensions** (reduce-motion gating for
splash animations, full HC chrome rollout across 8 surfaces),
**replay polish** (floating MOVE chip above the focused card
during playback), and the **first real consumer of
`ToastVariant::Error`** (invalid-move feedback as the third leg
of the existing audio + visual rejection-feedback stool).

The accessibility extensions close two threads v0.21.1 left
explicitly open: reduce-motion was previously gated only on card
slide_secs, and HC borders had `BORDER_SUBTLE_HC` defined but no
consumers. v0.21.2 finishes both — non-essential motion in the
splash boot screen now respects reduce-motion, and every static-
border chrome surface (modal scaffold, tooltip, help / stats /
home / settings panels) boosts to the HC variant under high-
contrast mode. Dynamic-paint sites (HUD action buttons, modal
buttons, radial menu rim) intentionally stay un-tagged because
their existing paint cycles would race the HC system; they
remain open for a future iteration that needs a different shape.

### Added

- **`sync_pile_marker_visibility` system precursor was v0.21.1's;
  this cycle adds**: `update_high_contrast_borders` system in
  `settings_plugin` (`c9af1ea`). Walks all entities tagged with
  `HighContrastBorder` each Update tick, swaps `BorderColor` to
  `BORDER_SUBTLE_HC` when high-contrast mode is on. Compares
  current colour and only mutates when different so Bevy's
  change-detection doesn't trigger repaints every frame. New
  `HighContrastBorder { default_color: Color }` component carries
  the off-state colour at each tagged site so the system can
  revert correctly.
- **HC chrome rollout — 8 tagged surfaces** (`c9af1ea` modal
  scaffold; `d87761d` tooltip + onboarding key chips + help
  panel key chips + stats panel cells; `ec804d5` home Level/XP/
  Score row + home mode-selector buttons + home mode-hotkey
  chips + 4 settings panel surfaces). Each tagging is one line
  on the spawn tuple. The marker-component architecture pays
  back proportionally to the number of consumers — the per-
  commit cost dropped from ~75 lines (foundation + first
  surface) to ~13 lines (4 surfaces) to ~9 lines (7 surfaces).
- **Floating MOVE chip during replay** (`2fb2d63`). New
  `ReplayFloatingProgressChip` marker on a `Text2d` entity
  rendered in 2D world space above the destination pile of the
  most-recently-applied move. Sibling of the banner overlay (not
  a child) because it lives in world-space coordinates, not the
  UI tree. Lifecycle matches the banner: `spawn_overlay` spawns
  the chip alongside the banner when a replay starts;
  `react_to_state_change` despawns it when the replay ends.
  World-space placement (rather than UI-space + camera projection)
  uses the same `LayoutResource` pile coordinates that drive
  every other piece of pile geometry — stays correctly positioned
  through window resizes for free. Hidden when cursor=0 (no
  moves applied yet) or when the last applied move was a
  `StockClick` (no destination pile to follow).
- **`handle_move_rejected_toast` system + first real
  `ToastVariant::Error` consumer** (`68d50b5`). When
  `MoveRejectedEvent` fires (illegal placement attempt), spawns
  a 2-second pink-bordered "Invalid move" toast. Joins the
  existing `card_invalid.wav` (audio cue) and destination-pile
  shake (visual cue) as the accessibility-focused readable text
  channel — covers deaf players (no audio reliance) and
  reduce-motion players (no shake reliance) with a persistent
  ~2 s text cue. Drops the `#[allow(dead_code)]` from
  `ToastVariant::Error` and updates its doc to point at the new
  consumer.

### Changed

- **Splash scanline overlay skipped under reduce-motion**
  (`ed152e2`). `spawn_splash` reads `Settings::reduce_motion_mode`
  and skips the scanline texture / overlay node entirely when
  on. Without the scanlines the boot screen still reads as
  terminal-themed (foreground content, borders, palette swatches
  unchanged); the scanlines are decorative.
- **Splash cursor pulse held under reduce-motion** (`ed152e2`).
  `pulse_splash_cursor` reads `Settings::reduce_motion_mode` and
  skips the per-frame sine-pulse multiplier when on — the cursor
  still fades in / out with the global splash alpha (essential
  timing) but doesn't blink. Spec calls out non-essential motion
  as the reduce-motion target; the global fade is essential
  (otherwise the splash would hard-cut on/off, which is
  jarring), and the cursor blink is decorative.
- **`AnimationPlugin::build` registers
  `MoveRejectedEvent`** (`68d50b5`). Bevy's `add_message` is
  idempotent, so the duplicate registration with
  `feedback_anim_plugin` (which already registered the message)
  coexists cleanly. Required for the new
  `handle_move_rejected_toast` system to run under
  MinimalPlugins (tests).

### Documentation

- `docs/ui-mockups/design-system.md` and `SESSION_HANDOFF.md`
  refreshed in lockstep with the rollouts. The handoff's
  Resume-prompt menu trimmed twice this cycle as Options A and F
  closed in v0.21.1, then this commit cycle's accessibility
  extensions implicitly closed the "future scope" footnotes
  v0.21.1 left on F's documentation.

### Stats

- **1195 passing tests / 0 failing** across the workspace
  (net +3 from v0.21.1's 1192 baseline). New tests added by
  this cycle:
  - `splash_skips_scanline_overlay_under_reduce_motion`
    (`ed152e2`) pins the reduce-motion gate on the splash
    scanline overlay. Discovered an asset-fixture bootstrapping
    detail along the way: under `MinimalPlugins`,
    `Assets<Image>` isn't auto-inserted; the test had to add
    `bevy::asset::AssetPlugin::default()` and
    `init_asset::<bevy::image::Image>()`. Pattern flagged for
    future asset-using tests.
  - `floating_chip_spawns_and_despawns_with_overlay`
    (`2fb2d63`) pins the floating MOVE chip's lifecycle:
    absent on Inactive, exactly one on Playing, absent again
    on return to Inactive.
  - `move_rejected_event_spawns_error_toast` (`68d50b5`) pins
    the new toast wiring: firing a `MoveRejectedEvent` spawns
    exactly one `ToastOverlay` on the next tick.
- Zero clippy warnings under `cargo clippy --workspace
  --all-targets -- -D warnings`.
- `cargo test --workspace` clean.

## [0.21.1] — 2026-05-08

Patch release for the post-v0.21.0 work — closes Resume-prompt
Options A (app icon) and F (high-contrast + reduce-motion
accessibility modes), plus a card-visual iteration cycle that
moved through three states: the v0.21.0 Terminal pink/gray, a
brief 4-colour-deck experiment (hearts pink, diamonds gold,
clubs lime, spades gray), and a reversion to traditional 2-colour
"Microsoft Solitaire on dark mode" pairing (saturated red +
near-white). Two visible bugs surfaced and were fixed during
the iteration: the suit-coloured border produced anti-aliasing
artifacts at rounded card corners (border dropped entirely),
and the pile-marker sprite bleed-through created visible "gray
L" shapes where cards sat on markers (markers now hide when
occupied — the documented but previously-not-enforced "remain
visible only where a pile is empty" invariant).

### Added

- **Desktop window icon** (`3eb3a26`). Runtime `Window::icon`
  wired via `WinitWindows`; embedded 256 px PNG decoded on
  startup via `tiny_skia` and handed to winit. Plus a 9-size
  PNG hierarchy at `assets/icon/icon_<size>.png` covering
  Linux hicolor (16/24/32/48/64/128/256/512), Windows `.ico`
  targets (16/32/48/256), and macOS `.icns` targets
  (16/32/64/128/256/512/1024). All sizes generated from a
  shared `icon_svg` builder (Terminal `▌RS` mark on dark
  `#151515` with brick-red accent) by a new
  `icon_generator` example. Pin test `icon_svg_pin` guards
  rasterised RGBA bytes against `usvg`/`resvg` drift. Two
  new `solitaire_app` deps target-gated to non-Android:
  direct `winit = "0.30"` (for `Icon` construction —
  `bevy_winit` 0.18 doesn't re-export it) and direct
  `tiny-skia` (for PNG → RGBA decode). Android draws its
  launcher icon from the APK manifest, so neither dep is
  needed there.
- **`Settings::high_contrast_mode` flag** (`c5787c6`). Boosts
  card text colours: hearts/diamonds → `RED_SUIT_COLOUR_HC`
  (`#ff6868`), clubs/spades → `TEXT_PRIMARY_HC` (`#f5f5f5`).
  Composes with `color_blind_mode`: CBM lime wins over HC red
  on red suits when both are on; HC still applies to dark
  suits independent of CBM. Six new tests pin the truth
  table.
- **`Settings::reduce_motion_mode` flag** (`c5787c6`). Forces
  `effective_slide_secs` to `0.0` regardless of the
  `AnimSpeed` selection, making cards snap instantly to their
  target. Two new tests pin the gate behaviour and the
  fall-through to `anim_speed_to_secs` when off. Future
  scope: gate splash scanline / cursor pulse / warning-chip
  pulse on the same flag.
- **Settings UI toggle rows** (`07e0357`). Two new rows in
  the Settings panel under Cosmetic (alongside Color-blind):
  "High Contrast" and "Reduce Motion". `tab-walk` order
  visits all three accessibility flags in one vertical run.
  Same shape as the existing `ColorBlindText` toggle scaffold
  with marker components, label updaters, click handlers,
  and disambiguator chains.
- **`sync_pile_marker_visibility` system** (`4d48cad`).
  Implements the module-level doc invariant in `table_plugin`
  ("pile markers ... remain visible only where a pile is
  empty") that was previously declared but not enforced.
  Hides the pile-marker sprite for any pile that has a card
  on top, shows it for empty piles. Closes the "gray L
  corners" artifact where the marker's translucent fill bled
  through the rounded card corners.

### Changed

- **Card-face suit colours** (`62b61cc` → `ddb6540`). Started
  the cycle at v0.21.0's Terminal pink (`#fb9fb1`) / gray
  (`#d0d0d0`), briefly experimented with a 4-colour deck
  (`62b61cc` — hearts pink, diamonds gold, clubs lime, spades
  gray) for faster suit recognition by hue alone, then
  reverted to traditional 2-colour pairing at the player's
  request (`ddb6540`). Final state: `RED_SUIT_COLOUR =
  #e35353` (saturated red, replacing the v0.21.0 pink) and
  `BLACK_SUIT_COLOUR = #e8e8e8` (near-white, brighter than
  the v0.21.0 `#d0d0d0` foreground gray so the dark suits
  read as a chromatic-neutral counterpart to the saturated
  red rather than as "the same gray as body text"). Reads
  like Microsoft Solitaire on dark mode. `RED_SUIT_COLOUR_HC`
  rebumped to `#ff6868` (brighter saturated red) so HC stays
  more chromatic than the new default red rather than the
  previous pinker boost. The 4-colour experiment's commit
  history is preserved in the log; net delta vs. v0.21.0 is
  the new red + new near-white.
- **Card-face border dropped** (`dd97021`). The earlier 1 px
  suit-coloured stroke on the card body produced
  anti-aliasing artifacts at the rounded corners (the colored
  stroke faded through gray pixels into the play surface).
  Cards now have no border — body fill alone defines the
  shape against the play surface; the 5-unit brightness gap
  between `#1a1a1a` body and `#151515` surface is enough to
  read as a card edge without an explicit stroke.
  `design-system.md` § Game Cards line 225 updated in
  lockstep.
- **Settings UI accessibility row count** (`07e0357`). Three
  toggles in Cosmetic now: Color-blind, High Contrast,
  Reduce Motion. Existing query-disambiguator chains in
  `handle_settings_buttons` extended with `Without<HighContrastText>`
  and `Without<ReduceMotionText>` so the new components
  don't ambiguate the existing mutations.

### Fixed

- **Bevy 0.18 system-param validation panic on icon startup**
  (`716a025`). `NonSend<WinitWindows>` failed validation on
  the first few frames before winit's `Resumed` event populated
  the resource. Bevy 0.18's stricter validation panics rather
  than skips when a non-send resource is absent; the error
  message itself spelled out the fix ("wrap the parameter in
  `Option<T>` and handle `None` when it happens"). Wraps
  `winit_windows` as `Option<NonSend<WinitWindows>>` and
  early-returns on `None`.
- **"Gray L corners" on cards** (`4d48cad`). Two artifacts
  were producing similar-looking grey at card corners: the
  SVG stroke fading through gray pixels (closed by `dd97021`)
  and the pile-marker sprite bleeding through the rounded
  cutouts (closed by `4d48cad`). Right test target, wrong
  visible-artifact target on the first attempt — the pin
  test correctly drifted 52 face hashes, but the visible
  gray came from a different layer. Two layers, two fixes;
  the second closed the player-visible complaint.

### Documentation

- `docs/ui-mockups/design-system.md` § Suit Colors retitled
  through three states (Terminal 2-color → "Four-color
  deck" → final "Two-color traditional pairing"). Final
  table records the saturated red + near-white. § Game Cards
  border spec changed from "1px solid in suit color" to
  "Border: none" with the artifact-rationale audit trail.
  CBM section text updated through each colour-scheme
  iteration.
- `SESSION_HANDOFF.md` refreshed twice this cycle (`0c1cc40`
  + `31139ae`) — the first reset the post-v0.21.0 narrative
  ("no threads in flight"), the second recorded Options A +
  F closures and trimmed the Resume-prompt menu.
- New module-level doc strings on the new constants
  (`RED_SUIT_COLOUR_HC`, `TEXT_PRIMARY_HC`, `BORDER_SUBTLE_HC`,
  `RED_SUIT_COLOUR_CBM` semantic shift) record the
  composability rules between CBM and HC and the "what to
  use this for" rationale.

### Stats

- **1192 passing tests / 0 failing** across the workspace
  (net +8 from v0.21.0's 1184 baseline). New tests added by
  this release:
  - `card_face_svg_pin` integration test rebaselined three
    times during the suit-colour iteration; final hashes
    pin the saturated-red + near-white + no-border state.
  - 4 high-contrast text_colour tests + 2 reduce-motion
    `effective_slide_secs` tests in `card_plugin` /
    `animation_plugin` (from `c5787c6`).
  - 1 `icon_svg_pin` integration test guarding the icon
    rasterisation pipeline (from `48b28d2` — actually
    landed in v0.21.0's accounting but worth noting for the
    cycle).
  - 1 `pile_markers_hide_when_pile_is_occupied` test pinning
    the new visibility-by-occupancy invariant (from
    `4d48cad`).
- Zero clippy warnings under `cargo clippy --workspace
  --all-targets -- -D warnings`.
- `cargo test --workspace` clean.

## [0.21.0] — 2026-05-08

Closes the visual-identity arc opened in v0.20.0. Three through-lines
landed: the **card-face / suit / card-back artwork migration** that
v0.20.0 deliberately deferred, the **splash boot-screen + replay-
overlay polish** that closes Resume-prompt Options B and C, and a
late-cycle **`ACCENT_PRIMARY` palette swap** from cyan `#6fc2ef` to
brick red `#a54242` after a quick stakeholder review on the
shipped art.

The card-face arc is the largest piece by commit count (10 of the
25 post-tag commits) and shape: it ports both rendering paths
production traverses — the PNG fallback at `assets/cards/*.png`
and the bundled-default theme SVGs at
`solitaire_engine/assets/themes/default/*.svg` that
`include_bytes!()`-embed into the binary and override the PNGs at
runtime — to identical Terminal-aesthetic art generated by the
same `face_svg` / `back_svg` builders. A new
`card_face_svg_pin` integration test pins rasteriser output via
FNV-1a on raw RGBA bytes, so future `usvg`/`resvg` upgrades or
intentional builder edits surface as test failures rather than
silent visual drift. The pin test fired three times during the
arc (text→path glyph fix, glyph orientation tweak, palette swap)
and rebaselined cleanly each time via the empty-then-paste
bootstrap pattern baked into the test.

Three sign-off follow-ups surfaced once a human booted the
running game and they all matched the same shape — "fallback
path the chrome migration walked past": the embedded default
theme overrode the new PNGs at runtime, the table backgrounds
were a separate PNG path that the v0.20.0 chrome migration
didn't touch, and the action-button row's `font_size: 16.0`
literal slipped through the typography migration audit. All
three are recorded under "Fixed" below.

Phase 8 (sync) and the Phase Android runtime gaps (JNI bridges,
APK launch verification on device) remain open and roll forward.

### Added

- **Card-face SVG generator pipeline** (`5623368` plan doc,
  `3a4bb63` PoC, `babe5cc` full generator, `48b28d2` pin test).
  `solitaire_engine/examples/card_face_generator.rs` writes 52
  face PNGs + 5 back PNGs into `assets/cards/` and 53 theme SVGs
  into `solitaire_engine/assets/themes/default/`, all from the
  shared `face_svg` / `back_svg` builders in the new
  `solitaire_engine::assets::card_face_svg` module. Run with
  `cargo run --example card_face_generator --release`. The PoC
  (`card_face_poc.rs`) stays alongside as historical record of
  the per-card grain proof. Pin test `card_face_svg_pin`
  guards rasterised output via inline FNV-1a so the arc has
  test-time coverage of both intentional builder edits (rebase
  via empty-then-paste) and unintentional dependency-upgrade
  drift.
- **Background generator example** (in `8719f77`).
  `solitaire_engine/examples/background_generator.rs` emits 5
  flat Terminal-palette play-surface PNGs at 120 × 168, the
  same tile size the legacy felt textures used (the runtime
  stretches to `window_size * 2.0` so source resolution is
  immaterial). All 5 slots stay in the near-black family —
  `#151515` canonical, `#0a0a0a` deeper, `#1a1a1a` elevated,
  `#121820` cool tint, `#201812` warm tint.
- **Splash boot-screen port** (`cacb19c`). Full mockup-spec
  splash: header, boot log, progress bar, palette swatches,
  version footer, plus the `SplashFadable` scaffold that lets
  any future overlay fade `N >> 3` elements via one marker +
  one global lerp query (replaces the `Without<X>, Without<Y>`
  exclusion pattern that the legacy splash hit at three
  siblings).
- **Splash trailing cursor pulse** (`29136d8`). Trailing
  6×12 px Node, sine-pulsed, multiplied with the global splash
  fade — the "multiply, don't override" pattern that resolves
  the original `cacb19c` skip-rationale. Closes Option B half 1
  from the SESSION_HANDOFF Resume prompt.
- **Splash tiled scanline overlay** (`a27cf5a`). Runtime-
  generated 2×2 RGBA8 texture tiled via `NodeImageMode::Tiled`;
  per-pixel alpha × tint alpha gives multiplicative fade
  integration without new abstractions. Closes Option B
  half 2.
- **Replay overlay scrub bar** (`c84d9f4`). 1px accent fill at
  the bottom of the banner, mirroring `cursor / total`. Per-
  frame updater + scrub-pct unit tests.
- **Replay overlay banner label port** (`6204db8`). The
  "▌ replay" headline picks up the cursor-block treatment that
  aligns it with the splash boot-screen idiom.
- **Replay overlay GAME caption** (`54005d5`). `GAME #YYYY-DDD`
  game-identifier caption beneath the headline. Mirrors the
  mockup's right-anchored ID but stays grouped with the headline
  so the two pieces of "this is a replay of game X" read as one
  unit.
- **Replay overlay MOVE chip** (`e080b49`). `MOVE N/M` progress
  readout wrapped in a 1px accent-bordered chip — discrete
  callout rather than free-floating text. Closes Option C from
  the SESSION_HANDOFF Resume prompt (paired with `54005d5`).
- **Terminal desktop-adaptation spec** (`39b8496`).
  `docs/ui-mockups/desktop-adaptation.md` — the rules-based
  companion to the 24-mockup library. Closes the spec gap
  exposed when 23 of 24 mockups turned out to be mobile-only;
  any future plugin port should read this first and apply the
  universal rules before consulting the per-screen table.
- **`solitaire_engine::assets::card_face_svg` module**
  (`48b28d2`). Public SVG builders (`face_svg`, `back_svg`,
  `suit_path_d`) extracted from the example so the pin test
  could call them — examples can't be referenced from
  `tests/`. The generator and the test now share the same
  source-of-truth, so the pin guards both rendering paths
  the engine consults.

### Changed

- **`ACCENT_PRIMARY` swapped from cyan `#6fc2ef` to brick red
  `#a54242`** (`a292a7e`). Project-wide palette decision after
  initial rollout. Affects every cyan-accented surface — splash
  boot screen, home menu glyphs, action chevrons, replay
  overlay banner + scrub fill + chip border, achievement
  checkmarks, leaderboard #1 indicator, radial menu fill, focus
  ring, card-back canonical badge. `RED_SUIT_COLOUR_CBM`
  swapped in lockstep from cyan to lime `#acc267` so the
  colour-blind alternative stays hue-distinct from the new
  red-family primary. Comment doc strings throughout the
  engine retuned from "cyan" to "accent" / "primary-accent" so
  future palette changes don't require comment churn. Spec doc
  `design-system.md` updated in lockstep with historical
  references preserved as audit trail.
- **Card-face / suit / card-back constants migrated to Terminal
  palette in lockstep with new artwork** (`e8bf9d7`). Five
  constants flipped: `CARD_FACE_COLOUR` → `#1a1a1a` (was
  off-white `#fafaf2`), `RED_SUIT_COLOUR` → `#fb9fb1` (was deep
  red `#c71f26`), `BLACK_SUIT_COLOUR` → `#d0d0d0` (was near-
  black `#141414`), `CARD_FACE_COLOUR_RED_CBM` renamed to
  `RED_SUIT_COLOUR_CBM` and repurposed from a face-background
  tint to a suit-glyph swap (the Terminal face is uniformly
  `CARD_FACE_COLOUR` regardless of CBM; CBM only swaps red
  suits to a hue-distinct alternative in the glyph itself).
  `card_back_colour()` retuned to the 5 base16-eighties accent
  colours matching `BACK_ACCENTS`. `face_colour()` deleted —
  the function collapsed to a constant once the Terminal face
  became uniform. `text_colour()` gained a `color_blind: bool`
  parameter to surface the CBM swap on the constant-fallback
  path (the production path bakes glyphs into the PNG, but
  tests under `MinimalPlugins` still need the CBM-aware
  fallback). Four `face_colour` CBM tests collapsed into two
  `text_colour` CBM tests in the same commit.
- **Default-theme SVG art regenerated to Terminal aesthetic**
  (`a14200a`). `solitaire_engine/assets/themes/default/*.svg`
  — the bundled-default theme that
  `include_bytes!()`-embeds into the binary — was still the
  legacy vector-playing-cards art post-`e8bf9d7`. The PNG
  migration alone didn't change what production rendered
  because `apply_theme_to_card_image_set` overrides
  `CardImageSet.faces[..]` at startup with the theme's
  rasterised SVG handles. Both rendering paths now agree:
  same `face_svg` / `back_svg` builders feed both paths, and
  the pin test guards both.
- **Card glyphs render upright in both corners** (`dd101b3`).
  The traditional 180° inverted-corner-indicator rotation on
  the bottom-right glyph was dropped at user preference —
  single-orientation digital play doesn't benefit from the
  flipped-readback convention. Both glyphs now render in the
  same upright orientation. `design-system.md` § Game Cards
  line 220 updated in lockstep — the deviation from
  traditional playing-card layout is documented in the spec,
  not just the code.
- **Action-button row typography aligned to `TYPE_BODY`**
  (`ae84dc1`). Was a hardcoded `font_size: 16.0` literal that
  the v0.20.0 typography-migration audit walked past. Brings
  it in line with the `TYPE_*` token system every other text
  element in `hud_plugin` already routes through, and trims
  ~12% off label widths so the action-button row no longer
  collides with the left-anchored HUD column at portrait /
  narrow window widths. Pairs with a horizontal-padding step-
  down from `VAL_SPACE_3` to `VAL_SPACE_2`: ~96 px reclaimed
  across the 6-button row.
- **Table backgrounds flattened to solid Terminal colours**
  (`8719f77`). Replaces the legacy felt-texture PNGs at
  `assets/backgrounds/bg_*.png` with 5 flat near-black
  variants per design-system.md (Terminal play surface is
  flat; no felt, no gradient). On-disk tile weight drops
  from ~16 KB average to ~100 bytes per tile; runtime
  appearance flips from green felt to flat `#151515`.

### Fixed

- **Card suit glyphs rendered as near-invisible "tofu" marks**
  (`af414b6`). The bundled `FiraMono` in
  `svg_loader::shared_fontdb` doesn't carry usable U+2660-2666
  glyphs at the requested size — usvg silently substituted a
  default-size fallback regardless of `font-size="20"` /
  `font-size="64"`. Switched suit-glyph rendering from `<text>`
  elements to inline SVG `<path>` elements via a new
  `suit_path_d` helper authoring each suit as a single closed
  perimeter in a 32×32 logical box. Path-based rendering
  bypasses the font system entirely — same bytes on every
  machine, no fontdb dependency, no substitution risk. Same
  path data renders correctly whether filled (♥ ♠) or outlined
  (♦ ♣ — the always-on color-blind glyph differentiation).
- **Default-theme SVGs were overriding new PNG artwork at
  runtime** (`a14200a`). The PNG migration in `e8bf9d7` looked
  correct under `cargo test` (the constant-fallback path
  matched) but a real `cargo run` showed legacy white cards
  because `theme::plugin::apply_theme_to_card_image_set`
  overlays the bundled-default theme's rasterised SVGs onto
  `CardImageSet.faces[..]` at startup, and those SVGs were
  still legacy. Fixed by regenerating both rendering paths
  from the same `face_svg` / `back_svg` builders. The
  migration plan flagged "Theme system — out of scope here";
  that was a planning miss documented in the SESSION_HANDOFF.
- **Top-bar HUD column collided with action-button row at
  portrait window widths** (`ae84dc1`). Both nodes were
  absolute-positioned siblings at `top: VAL_SPACE_2` without a
  shared flex parent, so they could overlap horizontally when
  the window narrowed past their combined natural widths.
  Fixed via the typography + padding tightening described
  under "Changed" — minimal-blast-radius fix; the structural
  fix (shared `JustifyContent::SpaceBetween` parent) stays
  open as a follow-up if narrower windows surface.
- **Table-surface fill was still legacy green felt despite
  v0.20.0's chrome-migration claim** (`8719f77`). Commit
  `651f406` retuned in-engine constants but the runtime path
  loads from `assets/backgrounds/bg_0.png`, an on-disk PNG that
  the migration didn't touch. Same shape as the default-theme
  override above — token migration walked past a fallback
  rendering path. Fixed by regenerating the 5 background PNGs.

### Stats

- **1184 passing tests / 0 failing** across the workspace
  (net +8 from v0.20.0's 1176 baseline). New tests this cycle:
  the scrub-bar pair (`scrub_pct_covers_state_corners`,
  `overlay_scrub_fill_tracks_cursor`); the splash boot-screen
  pair (`splash_renders_terminal_boot_screen_content`,
  `fadables_start_transparent_and_reach_full_alpha`); the
  splash-polish pair (`build_scanline_image_has_expected_2x2_rgba_bytes`,
  `scanline_overlay_spawns_and_fades_with_splash`); the
  card-face pin (one integration test in
  `card_face_svg_pin.rs` that exercises 57 rasteriser outputs
  through 57 hash comparisons in a single
  `#[test]`-marked function); and the CBM consolidation that
  rewrote four `face_colour` tests as two `text_colour` CBM
  tests in the same commit (net 0 to count, clean rewrite).
- Zero clippy warnings under `cargo clippy --workspace
  --all-targets -- -D warnings`.
- `cargo test --workspace` clean.

### Documentation

- `docs/ui-mockups/card-face-migration.md` (`5623368`) — the
  multi-session lockstep migration plan that the card-face arc
  followed step-by-step. Now reads as historical record of
  closed work; lessons documented under "Process notes" in
  SESSION_HANDOFF.md.
- `docs/ui-mockups/desktop-adaptation.md` (`39b8496`) — rules-
  based companion to the 24-mockup library. Required reading
  before any future plugin port.
- `docs/ui-mockups/design-system.md` updates: § Game Cards
  line 220 (glyph orientation), CTA / suit-red-cb / Card-back
  badge / Primary button / Bottom-bar active-icon palette
  retunes for the cyan→red swap. Historical references
  preserved as audit trail.
- Multiple `SESSION_HANDOFF.md` refreshes (`a65e5b8`,
  `13ae160`, `44f5972`, `73ac67d`, `ef54cde`, `d109c32`)
  recording Options B / C / D closures and process notes.

## [0.20.0] — 2026-05-07

Two through-lines closed: a full **Android port** (build target,
first 54 MB APK, JNI-free per-app persistence shim) and the
**Terminal visual-identity port** that replaces the prior
Premium-Solitaire palette across every UI surface. The Android
arc opened in `fb8b2ac` (compile + APK), continued in `4b51e50`
(`solitaire_data::data_dir` shim closing the CLAUDE.md §10
`dirs::data_dir() = None` pitfall), and is functional end-to-end
on a real device — though the runtime artwork is still the legacy
white-card palette, and JNI ClipboardManager / keyring bridges
remain stubbed (matching v0.19.0's documented fallback behaviour).
The Terminal port lands as a top-down stack: the `ui_theme` token
API in `0d477ac` is load-bearing, and the rest of the cycle is
downstream applications (modal scaffold, gameplay-feedback,
toasts, table / card chrome, splash cursor, hint-highlight
pairing). The card faces and suit-pip palette are deliberately
NOT migrated — those track PNG artwork that hasn't been
regenerated yet, and swapping the fallback constants ahead of the
artwork would mix two visual systems on any code path where
image loading fails.

The 24 Stitch-rendered mockups in `docs/ui-mockups/` are now
in-tree (`fa7f98a`); future plugin work should diff against the
matching mockup before touching pixels.

Two threads from v0.19.0's punch list also closed in this cycle:
the pull-failure test flake (`67c150b`), the Settings opt-out for
the smart-default window sizer (`e1b8766`), and the share-link
discoverability surfacing (`9b065e5`). The remaining v0.19.0
candidate — the app-icon round — stays open.

### Added

- **`ui_theme` Terminal design-token system** (`0d477ac`). Single
  source of truth for the engine's visual identity:
  base16-eighties palette (cyan primary CTA, lime/lavender/gold/
  teal/pink semantic accents), 5-rung type scale, 7-rung 4-multiple
  spacing scale, 3-step radius, 14-rung z-index hierarchy, full
  motion budget, and four invariant-pinning unit tests. Every
  downstream port commit in this cycle reads from this module —
  swapping the palette is now a one-file edit, not a hunt across
  ~50 plugin files. Card-shadow alphas pinned to 0 (Terminal
  achieves depth via 1px borders + tonal layering, no
  `box-shadow`); the rendering path is left intact so a future
  palette can re-enable shadows without touching consumers.
- **`ToastVariant` enum + Terminal toast styling** (`a137607`).
  Toasts now follow `docs/ui-mockups/design-system.md`: opaque
  `BG_ELEVATED` fill, 1px accent border keyed off
  `Info` / `Warning` / `Error` / `Celebration` variants, 18px
  monospaced caption (`TYPE_BODY_LG`), bottom-anchored. All ten
  call sites pass their semantic variant: achievement / level-up
  / XP / daily / weekly / challenge → Celebration (lavender);
  goal-announcement / time-attack / settings volume / auto-complete
  → Info (teal). Two regression tests pin variant→border mapping
  to the design tokens and require all four borders to be visually
  distinct. Queued and immediate toasts use slightly different
  bottom anchors (6 % vs. 14 %) so a celebration toast spawned
  alongside a queued info banner layers above it.
- **Terminal cursor block on the splash overlay** (`cdcadda`).
  The launch splash now renders the design system's signature
  `▌` cyan (`ACCENT_PRIMARY`) glyph (96 px, hand-tuned literal)
  above the wordmark, matching `docs/ui-mockups/splash-mobile.html`.
  Cursor fades on the same per-frame alpha schedule as the title
  and subtitle so the brand beat still dissolves as a single
  layer. Did *not* pull in the mockup's full boot-loader treatment
  (scanline overlay, ✓ check log, progress bar, ROOT@SOLITAIRE
  prompt) — those are aesthetic features warranting their own
  commit.
- **Terminal design-system spec + 24-mockup library** (`fa7f98a`).
  `docs/ui-mockups/design-system.md` (palette, type scale, spacing
  scale, motion budget, component library, accessibility notes —
  color-blind toggle, high-contrast mode, glyph differentiation,
  canonical `"Terminal"` card-back theme) and 24 Stitch-rendered
  mockups (HTML + PNG): 12 redesigned existing screens, 1 desktop
  home variant, 2 onboarding steps, and 9 missing-plugin screens
  (splash, challenge, time-attack, weekly-goals, leaderboard,
  sync, level-up, replay, radial-menu). The spec the rest of this
  cycle ports against; future plugin work diffs here before
  touching pixels.
- **Android build target — first working APK** (`fb8b2ac`).
  `cargo apk build -p solitaire_app --target x86_64-linux-android`
  now produces a 54 MB debug-signed APK at
  `target/debug/apk/ferrous-solitaire.apk`. Five gating points
  resolved end-to-end:
  - **`solitaire_app` split into bin + lib.** cargo-apk needs a
    `cdylib` to bundle as `libmain.so`; pure-bin crates panic
    with "Bin is not compatible with Cdylib". `src/lib.rs`
    carries the ECS bootstrap as `pub fn run`; `src/main.rs` is
    a 3-line shim that delegates for the desktop path.
  - **`[package.metadata.android]`** pins target SDK 34 / min
    SDK 26 and points `assets = "../assets"` at the workspace
    asset directory so desktop and APK share one set.
  - **Workspace `bevy` features** add `android-native-activity`
    (target-gated inside bevy_internal — desktop builds compile
    it out). Pairs with cargo-apk's NativeActivity wrapper.
  - **`arboard` target-gated** to `cfg(not(target_os =
    "android"))`. The crate has no Android backend; cargo apk
    fails with E0433 on `platform::Clipboard` if left
    unconditional. Stats's "Copy share link" surfaces an
    informational toast on Android until JNI ClipboardManager
    lands in the Phase-Android round.
  - **`keyring` + `keyring-core` target-gated.** Bionic doesn't
    expose `libc::__errno_location` so the transitive
    `rpassword` won't compile. `auth_tokens` ships an Android
    stub returning `KeychainUnavailable` for every call —
    matches the existing fallback for a Linux box without
    Secret Service.
  - Cosmetic: cargo-apk panics post-sign when it tries to also
    wrap the bin target. The APK on disk is unaffected;
    `cargo apk build --lib` is the small workaround.
- **Android developer setup + build runbook** (`59424a3`).
  Captures Debian 13 toolchain install (JDK 21, unzip, SDK
  licence prompts), the `cargo apk build` invocation, the
  cosmetic post-sign panic workaround, and a what-is-wired-vs-
  stubbed table for the android target. Runnable on a fresh
  clone — no machine-local context required.
- **F3-toggleable FPS / frame-time overlay** (`690e1d2`).
  `DiagnosticsHudPlugin` wraps Bevy's `FrameTimeDiagnosticsPlugin`
  and renders a corner readout the developer toggles with F3.
  Hidden by default; F3 is not gated by pause / modal state.
  Reads `smoothed()` so the cell isn't a per-frame jittery
  scoreboard. Format: `FPS NN \u{2022} M.MM ms`. Anchored
  top-right at `z = Z_SPLASH + 100` above every modal / toast /
  splash. Update system bails when hidden so the
  diagnostic-store lookup is free when nobody's looking.
- **"Smart window size" Settings toggle** (`e1b8766`). Gameplay
  section gains an opt-out toggle for v0.19.0's
  `apply_smart_default_window_size` system. New
  `Settings::disable_smart_default_size: bool` with
  `#[serde(default)]` so legacy `settings.json` files load to
  the shipped behaviour (smart sizer enabled). `solitaire_app::main`
  reads the flag once at startup and skips the system's
  registration when set. Saved window geometry still wins over
  both branches; tooltip on the row makes that explicit.
- **"Shareable" badge on the Latest-win caption** (`9b065e5`).
  The Stats overlay's Latest-win caption now appends
  `\u{2022} Shareable` when the displayed replay carries a
  populated `share_url`. Players can see at a glance whether the
  Copy share link button will produce a URL or surface the
  upload-prerequisite toast.
- **Help overlay covers M / P / Win-Summary-Enter** (`35516d3`).
  Three new rows in the Overlays section: M (Home / Mode
  launcher), P (Profile), and the Enter accelerator that
  dismisses the Win Summary modal. Three post-v0.18 entries
  that had drifted out of the cheat sheet are now listed.

### Changed

- **Gameplay-feedback colours route through Terminal state
  tokens** (`ceec4fc`). Selection-highlight tints in
  `selection_plugin` and the valid-drop marker tint in
  `cursor_plugin` were hand-tuned RGB literals. Migrated to
  semantic state tokens: keyboard-drag picking source →
  `ACCENT_PRIMARY` (cyan focus); keyboard-drag lifted source →
  `STATE_WARNING` (gold attention); destination → `STATE_SUCCESS`
  (lime valid-move); `cursor_plugin::MARKER_VALID` →
  `STATE_SUCCESS` at 0.55 α with a tracking test pinning its RGB
  to the token. Three stale doc comments in `ui_modal` corrected
  ("loud yellow CTA" / "magenta secondary accent" → cyan /
  lavender to match the actual token values).
- **`table_plugin` chrome migration to Terminal tokens** (`651f406`).
  `marker_colour` promoted to module-level `pub const
  PILE_MARKER_DEFAULT_COLOUR` so `cursor_plugin::MARKER_DEFAULT`
  imports the const directly — replaces the prior
  duplicated literal kept in sync only by doc comment with a
  compile-enforced invariant. The empty-tableau "K" placeholder
  text now uses `TEXT_PRIMARY` at 0.35 α; `HINT_PILE_HIGHLIGHT_COLOUR`
  retuned from bright `srgb(1.0, 0.85, 0.1)` to the `STATE_WARNING`
  token (`#ddb26f`) with a tracking test, and the existing "is
  gold" character test loosened to fit the muted Terminal gold
  while still rejecting non-warm colours.
- **`card_plugin` chrome migration to Terminal tokens** (`d752870`).
  Drag-elevation shadow now sources its colour from
  `CARD_SHADOW_COLOR` + `CARD_SHADOW_ALPHA_DRAG` so the Terminal
  "no box-shadow" policy disables the stack shadow in lockstep
  with the per-card shadows. `RIGHT_CLICK_HIGHLIGHT_COLOUR`
  retuned from raw green to `STATE_SUCCESS` at 0.6 α with a
  tracking test. The duplicated `PILE_MARKER_DEFAULT_COLOUR`
  const dropped — this plugin now imports the promoted const
  from `table_plugin`. Stock recycle "↺" text moved from raw
  white-at-0.7-α to `TEXT_PRIMARY.with_alpha(0.7)`. Card-face /
  suit / card-back palette constants were intentionally NOT
  migrated (the runtime path renders PNG artwork that's still on
  the previous "white card" palette).
- **Hint-source card tint matches the destination pile**
  (`9891ae4`). `input_plugin`'s hint-source card tint moved from
  raw bright-yellow `srgba(1.0, 1.0, 0.4, 1.0)` to `STATE_WARNING`,
  so the source card and the destination pile (which already uses
  `STATE_WARNING` via `HINT_PILE_HIGHLIGHT_COLOUR`) wear the same
  attention colour as a coherent pair.

### Fixed

- **`solitaire_data::data_dir` shim closes the Android persistence
  gap** (`4b51e50`). `dirs::data_dir()` returns `None` on Android,
  which silently disabled every persistence path (settings, stats,
  achievements, replays, game-state, time-attack sessions, user
  themes). New `solitaire_data::platform::data_dir()` shim falls
  through to `dirs::data_dir()` on desktop and returns the per-app
  sandbox at `/data/data/com.ferrousapp.solitaire/files` on Android
  — no JNI needed, since the package id is pinned in
  `[package.metadata.android]`. Six call sites across
  `solitaire_data` plus `solitaire_engine/assets/user_dir.rs`
  migrated. CLAUDE.md §10 already flagged this as a known
  pitfall; the shim pays it down at the one chokepoint instead
  of per feature.
- **`card_shadow_params` test aligned with Terminal "no shadow"
  intent** (`1d1543e`). The Terminal token system pinned both
  `CARD_SHADOW_ALPHA_IDLE` and `CARD_SHADOW_ALPHA_DRAG` to 0.0,
  which made the prior `drag_alpha > idle_alpha` assertion fail
  (`0 > 0` is false). Loosened to `drag_alpha >= idle_alpha`
  with a comment naming the new invariant: under Terminal both
  are 0; under any future palette that re-enables shadows, drag
  still must not be weaker than idle. The useful regression-guard
  (catching an accidental swap of the two constants) is preserved.
- **`pull_failure_sets_error_status` test flake** (`67c150b`).
  The fixed 5-update budget was the last test still subject to
  the AsyncComputeTaskPool starvation mode that v0.19.0's
  auto-save fix already cleared. Replaced with a wall-clock-
  bounded loop (5-second deadline, `std::thread::yield_now`
  between iterations) that exits as soon as the status flips.
  Mirrors the auto-save flake fix shape.

### Stats

- **1176 passing tests / 0 failing** across the workspace
  (six new tests this cycle: four `ui_theme` invariant guards
  for the type / spacing / z-index scales + `scaled_duration`,
  one toast-variant-border-mapping pair, and four palette-
  tracking guards on `MARKER_VALID` / `HINT_PILE_HIGHLIGHT_COLOUR`
  / `RIGHT_CLICK_HIGHLIGHT_COLOUR` / toast-border distinctness).
- Zero clippy warnings under `--workspace --all-targets -- -D warnings`.

## [0.19.0] — 2026-05-06

Closes the v0.18.0 punch list (items B and D — async hint and
persistent replay share URLs), expands desktop platform fit
(Wayland session support + monitor-aware default window size for
HiDPI / 4K displays), polishes the win-celebration and
double-click animation paths, and clears two test-flake
contributors. A short-lived "Rusty Pixel" pixel-art card theme
was prototyped and reverted in the same window — the engine
plumbing it touched (`pixel_art` field on `ThemeMeta`, PNG
manifest face support, second `embedded://` theme channel) was
fully reverted and is not part of this release.

### Changed

- **H-key hint runs on `AsyncComputeTaskPool`** (`3e11e9e`). The
  synchronous `try_solve_from_state` call on every H press is gone;
  `handle_keyboard_hint` now spawns a task whose result the new
  `pending_hint::poll_pending_hint_task` system surfaces one frame
  later. New `PendingHintTask` resource carries the in-flight handle
  plus `move_count_at_spawn` for staleness detection;
  `drop_pending_hint_on_state_change` cancels the task whenever the
  game state shifts; `PendingHintTask::spawn` implements
  cancel-on-replace so two quick H presses keep at most one task in
  flight. Mirrors the v0.18.0 `PendingNewGameSeed` template.
  `emit_hint_visuals` and `find_heuristic_hint` are extracted as
  `pub` helpers so the polling system can call them.
- **Persistent replay share URLs** (`42d90b1`). v0.18.0's
  `LastSharedReplayUrl` was an in-memory resource wiped on quit —
  the player had to share within the session of the win.
  `solitaire_data::Replay` now carries a `share_url: Option<String>`
  field with `#[serde(default)]` (no `REPLAY_SCHEMA_VERSION` bump
  needed; older `replays.json` files load unchanged with `share_url
  == None` on every entry). `poll_replay_upload_result` writes the
  resolved URL into `replays[0].share_url` and persists the updated
  history via `save_replay_history_to`. The Stats overlay's
  "Copy share link" button reads from
  `history.0.replays[selected.0].share_url`, so the Prev/Next
  selector's currently-displayed replay drives the clipboard
  contents — each historical win keeps its own URL.
  `LastSharedReplayUrl` removed (its role is now subsumed by the
  `share_url` field on the replay record).

### Added

- **Wayland session support** (`b57db01`). The workspace
  `Cargo.toml` Bevy feature list now enables `wayland` alongside
  `x11`. winit prefers Wayland when `WAYLAND_DISPLAY` is set on the
  session, falling back to X11 when it isn't. Pre-fix, a Wayland
  desktop environment fell through to XWayland, rendering the
  game inside an X11 frame stitched into the Wayland compositor.
  Post-fix, the game opens as a native Wayland surface. Costs a
  few hundred KB of binary for the libwayland-client bindings;
  cross-distro friendly because winit dlopen-probes the libraries
  rather than hard-linking them.
- **Monitor-relative default window size** (`b57db01`). On launches
  with no saved geometry, the new
  `apply_smart_default_window_size` Update system queries
  `Monitor` (with the `PrimaryMonitor` marker) and resizes the
  primary window to ~70 % of the monitor's *logical* size on the
  first frame. Before, every fresh launch opened at 1280×800
  regardless of monitor; on a 4K monitor that's a comparatively
  tiny window in one corner. Logical size already accounts for
  the OS's HiDPI scale factor, so a Retina display reporting
  scale_factor 2.0 yields the same physical inches as a 1080p
  display reporting 1.0. Skipped entirely when saved geometry was
  applied — the player's chosen size always wins.

### Fixed

- **Duplicate "You Win" toast on game-won** (`55c235b`). The
  post-win UI was firing two celebration surfaces: a 4-second
  toast banner ("You Win! Score: X Time: Y") on top of the
  `win_summary_plugin`'s "You Won!" modal. In screenshots the
  toast banner was partially clipped behind the modal card,
  peeking out on either side. The toast predated the modal and is
  strictly subsumed by it; removed. The cards-fly-off cascade
  animation (`MotionCurve::Expressive` per-card rotation drift)
  is unchanged — that's the visual celebration, distinct from
  the textual celebration the modal owns. `WIN_TOAST_SECS` const
  removed.
- **Double-click on a single card with no destination now plays
  the reject animation** (`d7ffb16`). `handle_double_click` only
  fired `MoveRejectedEvent` for multi-card stacks with no
  destination; a double-click on a single card whose top didn't
  fit any foundation or tableau slot produced zero feedback —
  no `card_invalid.wav`, no source-pile shake. Both priorities'
  failure paths now converge on a single rejection at the end of
  the double-click branch, so single-card and stack misses get
  the same feedback shape as drag-and-drop rejections.
- **Double-click move animation no longer plays twice**
  (`6037596`). On a successful double-click, the slide-to-
  destination animation rendered twice — once from the move's
  `StateChangedEvent` landing, then again from the release's
  `end_drag` firing a redundant `StateChangedEvent` mid-slide.
  `sync_cards_on_change` saw the card mid-CardAnim (`cur ≠
  target`) and replaced the in-flight tween with a fresh one
  starting at the mid-position, visibly restarting the slide. The
  defensive `StateChangedEvent` write in `end_drag`'s
  uncommitted-drag branch is removed; `start_drag` only mutates
  `DragState` (never card transforms), so an uncommitted drag
  has no visual side effect to undo. The committed-drag branch
  keeps its `StateChangedEvent` since real drag snap-backs do
  need a resync.
- **`auto_save_writes_after_30_seconds` test flake** (`91b7605`).
  The test's single-frame `app.update()` was sensitive to
  first-frame `Time::delta_secs()` variance under heavy parallel
  cargo-test load, and to production-disk
  `~/.local/share/ferrous_solitaire/game_state.json` state leaking
  into the test world via `GamePlugin::build`'s load path.
  `test_app` now resets `PendingRestoredGame(None)` after plugin
  build (preventing the dev machine's saved-game state from
  tripping the auto-save guard) and the test re-arms the timer in
  a small bounded loop until the file appears (robust against
  first-frame Time variance). No production-code change.

### Stats

- 1170 passing tests (was 1166 at v0.18.0 close — net +4 from
  the persistent share URL backwards-compat test, the three
  async-hint tests, minus the dropped synchronous hint tests).
- Zero clippy warnings under `--workspace --all-targets -- -D warnings`.

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
  the separate `ferrous-solitaire-pkgbuild` directory).
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
