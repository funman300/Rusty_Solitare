# Solitaire Quest — Session Handoff

**Last updated:** 2026-05-02 (session 9, post-v0.14.0 release prep) — v0.14.0 cut. The Quat bug fixes, the rest of the v0.13.0 candidate list, and the entire replay → upload → web-viewer pipeline are all bundled in this release. Direction now opens for the next round.

## Status at pause

- **HEAD on origin:** v0.14.0's tag commit (CHANGELOG + handoff refresh).
- **Working tree:** clean apart from untracked `CARD_PLAN.md` (intentional).
- **Build:** `cargo clippy --workspace --all-targets -- -D warnings` clean.
- **Tests:** **1134 passed / 0 failed** across the workspace.
- **Tags on origin:** `v0.9.0`, `v0.10.0`, `v0.11.0`, `v0.12.0`, `v0.13.0`, `v0.14.0`.

## Where we are

v0.14.0 is the largest release since the card-theme system. Three threads land together:

1. **The remaining v0.13.0-era UX candidates** — theme thumbnails, daily-challenge calendar, Time Attack auto-save, per-mode bests, time-bonus multiplier slider.
2. **Quat smoke-test bug fixes** — multi-card move validation, softlock detection, deal-tween information leak.
3. **The replay pipeline** — record on win, persist to disk, upload to server, view in browser via a new `solitaire_wasm` crate. The biggest single feature since the card-theme system.

The card-flight web animations and replay E2E test coverage close out the pipeline.

### Design direction (unchanged)

- **Tone:** Balatro — chunky readable type, theatrical hierarchy, satisfying micro-interactions.
- **Palette:** Midnight Purple base + Balatro yellow primary + warm magenta secondary.
- See `~/.claude/projects/-home-manage-Rusty-Solitare/memory/project_ux_overhaul_2026-04.md` (machine-local).

### Canonical remote

`github.com/funman300/Rusty_Solitaire` is the canonical repo. Always push there.

## Session 8 + 9 (shipped 2026-05-02) — v0.14.0

### v0.13.0-era UX candidates (had landed but missed v0.13.0's tag)

| Area | Commit | What landed |
|---|---|---|
| Theme thumbnails | `ba527de` | Each Settings → Cosmetic theme chip renders an Ace + back preview pair via `rasterize_svg`. Cached per theme. Missing-SVG themes show a transparent placeholder rather than crashing. |
| Daily-challenge calendar | `1a10476` | 14-dot horizontal calendar in the Profile modal. Today is ringed, completed days fill `STATE_SUCCESS`, missed days fill `BG_ELEVATED`. Caption: "Current streak: N · Longest: M". `PlayerProgress` gains `daily_challenge_history` (capped at 365) and `daily_challenge_longest_streak`. |
| Time Attack auto-save | `0001432` | New sibling `time_attack_session.json` next to `game_state.json`. Atomic .tmp + rename. 30 s auto-save while active + on `AppExit`. Sessions whose 10-min window expired in real time while the app was closed are discarded on load. |
| Per-mode bests | `3984231` | StatsSnapshot gains six `#[serde(default)]` fields (Classic / Zen / Challenge × best_score + fastest_win_seconds). Stats screen renders a "Per-mode bests" section. Lifetime totals continue to roll all modes together. |
| Time-bonus slider | `89c51ab` | Settings → Gameplay slider 0.0–2.0, default 1.0, "Off" at zero. Multiplies the time-bonus shown in the win modal. Cosmetic only — does NOT affect achievement unlock thresholds. |

### Quat smoke-test bug fixes

| Area | Commit | What landed |
|---|---|---|
| Move validation (#1) | `f1aeb24` | `solitaire_core::rules::is_valid_tableau_sequence(&[Card]) -> bool` checks every adjacent pair in a moved stack descends one rank with alternating colour. Wired into `move_cards`. Closes the bug where any multi-card lift could be dropped as long as the bottom landed legally. |
| Deal-tween leak (#4) | `3eabc14` | New-game snaps every card sprite to the stock pile position before writing `StateChangedEvent`, so all 52 cards animate from a single deck point during the deal. Previously sprites started from previous-game positions, briefly revealing the prior deal. |
| Softlock detection (#2) | `2716472` | `has_legal_moves` rewritten: walks every potential move source (every stock card, every waste card, the face-up top of every tableau column) against every foundation and every tableau. Previous heuristic returned `true` whenever stock had cards, hiding genuine softlocks. `GameOverScreen` now actually fires for true softlocks. |
| End-game screen (#3) | — | Resolved as downstream of #2. The pre-existing `GameOverScreen` and `WinSummaryOverlay` already cover the close-out paths; the softlock screen just never spawned because the old `has_legal_moves` lied. |

### Replay pipeline (the major feature)

| Area | Commit | What landed |
|---|---|---|
| Replay storage | `42535f5` | `solitaire_data::replay::Replay` (seed + draw_mode + mode + score + time + recorded date + ordered move list) and atomic save/load helpers under `<data_dir>/latest_replay.json`. Schema v1; `load` returns None for any other version. |
| Engine recording | `57d1c58` | `RecordingReplay` resource + `ReplayPath` settings. Every successful `MoveRequestEvent` / `DrawRequestEvent` appends to recording; `GameWonEvent` freezes the recording into a `Replay` and persists. Undo intentionally not recorded. New game clears the recording. |
| Stats button | `d9f36bf` | Stats overlay surfaces a "Latest win:" caption + "Watch replay" button. Loads from disk via `LatestReplayResource`. (Full in-engine playback deferred — button currently fires an `InfoToastEvent` describing the replay.) |
| Server upload + fetch | `93182fa` | `POST /api/replays` accepts a `Replay` JSON; `GET /api/replays/:id` returns it. JWT-gated. SQL migration for the new `replays` table. |
| Engine sync | `23c9704` | Engine uploads winning replays automatically when the player has cloud sync configured. Re-uses the existing JWT/refresh-token flow. |
| WASM crate | `5bed43e` | New workspace member `solitaire_wasm` compiles replay-relevant `solitaire_core` types to WebAssembly so a browser can re-execute a replay client-side. `wasm-bindgen` glue. |
| Web viewer | `07b8ecd` | `GET /replays/:id` returns HTML + CSS + the wasm bundle. Browser fetches the replay JSON, rasterises a deal from the seed, and animates the recorded moves. |
| E2E coverage | `3081505` | Server tests covering the full upload → fetch round-trip via `axum::test`. |
| Web flight anim | `1fcd032` | Card-flight tweens on the web side so the browser viewer reads as a real game replay rather than a static dump. |

## Open punch list

### Release prep
1. **Smoke-test on the alex machine** after pulling — confirm Quat's three bug fixes hold up in real gameplay, and try the new replay button + web viewer end-to-end.
2. **Desktop packaging** per `ARCHITECTURE.md §17`. The Arch PKGBUILD exists in `/home/manage/solitaire-quest-pkgbuild/` (separate repo). Pending: app icon, macOS `.icns` + notarisation cert, Windows `.ico` + Authenticode cert, AppImage recipe.

### UX iteration (next-round candidates)

- **Solver-at-deal toggle** (Quat investigation #1, still deferred): add a Settings → Gameplay toggle "Winnable deals only" rather than baking solver-only into every deal. Lightest middle ground.
- **Disable Bevy's default audio feature** (Quat investigation #2, still deferred): one-line `default-features = false` swap on the workspace `bevy =` line, re-enable explicitly the features the engine uses (`render`, `bevy_winit`, `2d`, `bevy_window`, `png`, `bevy_text`, `bevy_ui`, `bevy_log`, `bevy_asset`, `default_font`, `bevy_state`). Drops ~50 transitive crates including the rodio + symphonia stack the project doesn't use (kira handles audio).
- **In-engine replay playback** — promote the "Watch replay" button from a stub toast to a real playback overlay that re-runs the recorded moves with `CardAnimation` tweens. The wasm crate already proves the playback math; the in-engine version reuses the same execute logic against the live game state.
- **Per-replay history** — currently single-slot at `latest_replay.json`. A "best replay per mode" bucket or a recent-N rolling list would let players revisit notable wins.
- **Solver-driven hint system** — extend the existing hint toggle so a deal-time solver provides higher-quality hints (currently a heuristic). Requires the solver from the toggle work above.
- **Achievement: "won via replay path"** — track when a player wins a deal whose previously-saved replay also won the same deal. Mostly fun; trivial scope.

## Card-theme system (CARD_PLAN.md, fully shipped)

Seven phases landed across `b8fb3fb` → `924a1e2` in v0.11.0; v0.13.0's `7ed4f2c` consumes the per-theme `back.svg`; v0.14.0's `ba527de` adds preview thumbnails. End-to-end:

- **Bundled default theme** ships in the binary via `embedded://` — 52 hayeah/playing-cards-assets SVGs + a midnight-purple `back.svg`.
- **User themes** under `themes://`. Drop a directory containing `theme.ron` + 53 SVGs.
- **Importer** at `solitaire_engine::theme::import_theme(zip)` validates archives and atomically unpacks.
- **Picker UI** in Settings → Cosmetic; thumbnails + the active theme's `back` override the legacy `back_N.png` picker when present.

## Resume prompt

```
You are a senior Rust + Bevy developer working on Solitaire Quest.
Working directory: <Rusty_Solitaire clone path on this machine — local
directory may still be named Rusty_Solitare from earlier; that's fine>.
Branch: master. Direction is OPEN — v0.14.0 just shipped covering the
Quat bug fixes, the v0.13.0 candidate tail, and the entire
replay-pipeline feature.

State: HEAD at v0.14.0. Working tree clean apart from untracked
CARD_PLAN.md (intentional).
Build: cargo clippy --workspace --all-targets -- -D warnings clean.
Tests: 1134 passed / 0 failed.

READ FIRST (in order, before doing anything):
  1. SESSION_HANDOFF.md  — v0.14.0 changelog + open punch list
  2. CHANGELOG.md        — release-by-release record
  3. CLAUDE.md           — hard rules (UI-first, no panics, etc.)
  4. ARCHITECTURE.md     — crate responsibilities + data flow
  5. ~/.claude/projects/<this-project>/memory/MEMORY.md
                         — saved feedback / project context (machine-local;
                           may be missing on a fresh machine)

DECISION TO ASK THE PLAYER FIRST:
  A. Smoke-test v0.14.0 on the alex machine first to confirm the
     three Quat bug fixes hold up in real gameplay and the replay
     pipeline works end-to-end (record → upload → web viewer).
  B. Take the deferred Bevy-audio-feature trim (Quat investigation
     #2) — one-line workspace edit, ~50 fewer transitive crates.
  C. Take the deferred solver toggle (Quat investigation #1): add
     "Winnable deals only" Settings toggle. Larger.
  D. Promote the in-engine "Watch replay" button to real playback.
  E. Pick from the remaining "next-round candidates" in this doc.
  F. Take the deferred desktop-packaging item (needs artwork +
     signing certs from the user).

WORKFLOW NOTES:
  - Commits use:
      git -c user.name=funman300 -c user.email=root@vscode.infinity \
          commit -m "..."
  - When attributing playtester feedback in commits/docs, use "Quat"
    not "Rhys" (saved feedback memory).
  - Sub-agents stage + verify only; orchestrator commits.
  - Every commit must pass build / clippy / test before pushing.
  - Push to GitHub (origin) — that is the canonical remote.

OPEN AT THE START: ask which of A–F. Don't pick unilaterally.
```
