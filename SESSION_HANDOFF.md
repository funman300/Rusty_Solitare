# Solitaire Quest — Session Handoff

**Last updated:** 2026-05-12 — Leaderboard display name shipped (`03be4fc`). All commits pushed to origin.

Phase 8 closes the self-hosted-server connection arc end-to-end: login/register
modal, re-auth on token expiry, account deletion flow, server deployment
artifacts (Dockerfile + docker-compose), replay upload on win, web replay
player (WASM + HTML/CSS/JS served by the server), leaderboard opt-in/out,
and full server integration tests.

---

## Current state

- **HEAD locally:** `03be4fc` (feat: leaderboard custom display name).
- **HEAD on origin:** `03be4fc` (fully pushed).
- **Working tree:** clean (only `solitaire-release.jks.bak2` untracked — intentional).
- **Build:** `cargo clippy --workspace --all-targets -- -D warnings` clean.
- **Tests:** **1300+ passing / 0 failing** across the workspace.
- **Tags on origin:** `v0.9.0` through `v0.22.0`.

---

## What shipped in Phase 8 (432061c – bd388fe)

| Commit | Summary |
|--------|---------|
| `432061c` | Sync setup modal (login/register/connect/disconnect) |
| `6ce5564` | Re-auth on expired session + server deployment artifacts |
| `272d31f` | Account deletion flow + `handle_sync_buttons` refactor |
| `bd388fe` | CHANGELOG v0.23.0 documentation |

Also shipped (pre-Phase 8 but post-v0.22.0, already in CHANGELOG):
- `solitaire_wasm` crate: WASM ReplayPlayer bindings for browser-side replay playback
- Server replay API: `POST /api/replays`, `GET /api/replays/recent`, `GET /api/replays/:id`
- Server web UI: `/replays/:id` HTML route + `ServeDir /web` static assets
- DB migration 002: `replays` table + two indexes
- Full server integration tests for replay endpoints
- `push_replay` in `sync_plugin` (uploads on win, writes share URL into replay history)
- Stats panel "Copy Share Link" button reads `share_url` from replay history

---

## Open punch list (ordered by priority)

### 1. Documentation debt (no code)
- [x] CHANGELOG [Unreleased] → v0.23.0 — done this session
- [x] ARCHITECTURE.md update — all 8 gaps closed, bumped to v1.3
- [x] SESSION_HANDOFF.md update — this file

### 2. Leaderboard wiring gaps
- [x] **Best-score auto-post.** Done (`303c78a`): `update_leaderboard_if_opted_in`
  called from both first-push and merge paths in `sync.rs`; uses SQLite `MIN`/`MAX`
  in the UPDATE so scores never regress on stale data.
- [x] **Display name = username.** Done (`03be4fc`): `leaderboard_display_name:
  Option<String>` added to `Settings`; editor modal in leaderboard panel; persists
  to `settings.json`; `handle_opt_in_button` prefers custom name over username.

### 3. Security hardening
- [x] **Refresh token rotation.** Done (`b129664`): `refresh_tokens` table
  (migration 003); jti embedded in JWT; rotate-on-use pattern; 3 integration
  tests.
- [x] **Sync endpoint rate limiting.** Done (`6e6f3ef`): `UserIdKeyExtractor`
  decodes JWT for per-user identity; falls back to IP; burst 10 / 6 min
  steady-state; integration test passes.

### 4. Android validation
- [x] **Android Keystore functional test.** Done (2026-05-11, Pixel 7 AVD,
  Android 14): `load_access_token()` exercised via `start_pull`; logcat confirmed
  `NotFound` returned cleanly — no JNI panic. See `docs/android/PLAYABILITY_TODO.md` P4.
- [x] **JNI clipboard functional test.** Done (2026-05-11): temporary `KEYCODE_C`
  hook confirmed `ClipboardManager.setPrimaryClip()` succeeds on Android 14.
  Hook reverted. Production path requires Interaction::Pressed + non-null `share_url`.
  Note: `adb shell input tap` doesn't deliver touch events on headless AVD (documented).
- [x] **`cargo apk build --lib` noisy stderr** — upstream cargo-apk bug; `--lib`
  is the canonical command (CLAUDE.md §15.1, docs/ANDROID.md). No in-repo fix possible.

### 5. Feature completeness
- [x] **Theme importer UI.** Done (`613bbf8`): "Scan for new themes" button in
  Settings Appearance section. Shows import path label, scans user_theme_dir()
  for .zip archives, fires InfoToastEvent per file, refreshes ThemeRegistry.
- [x] **`mirror_achievement` removed.** Done (`549a817`): method was a no-op
  default never overridden and never called; achievements already sync via
  `SyncPayload` push. Deleted from trait and blanket impl.
- [x] **WASM build script.** Done (`40d0712`): `build_wasm.sh` at repo root
  documents `wasm-pack build --target web`, cleans up pkg metadata files,
  includes dependency guard + install instructions.
- [x] **Server password reset.** Done (`7514684`): `--reset-password <username>`
  subcommand reads new password from stdin, bcrypt-hashes it, invalidates all
  active sessions for the user.

### 5b. Android UX polish (2026-05-12)

- [x] **UX-1 — Modal Done button in gesture zone.** `apply_safe_area_to_modal_scrims` system
  added to `SafeAreaInsetsPlugin` (`safe_area.rs`). Pads every `ModalScrim` bottom by
  `insets.bottom / scale`. Fires on resource change + `Added<ModalScrim>`. Verified on device.
- [x] **UX-5b — Home mode glyph corruption.** Geometric Shapes (U+25xx, absent from FiraMono)
  replaced with card suits U+2660–2666 in `home_plugin.rs`. Affects Zen/Challenge/Daily mode
  selector buttons at level 5+.
- [x] **UX-7 — Help text wrap.** Android HUD entry shortened to
  `"Open menu (Stats, Settings, Profile...)"` in `help_plugin.rs` — fits one line.
- [x] **BUG-3 — Multi-modal stacking.** `handle_menu_button` now checks
  `scrims: Query<(), With<ModalScrim>>` and guards `spawn_menu_popover` with `scrims.is_empty()`.
  Verified on device: ≡ tap while Stats open does nothing.

  **Note:** These 4 fixes are implemented and verified but not yet committed.

### 6. Testing gaps
- [x] **Server 401 → refresh → retry path.** Done (`198df75`): both
  `jwt_refresh_on_401_succeeds` (pull) and
  `push_retries_after_401_on_expired_access_token` (push) in
  `solitaire_data/tests/sync_round_trip.rs`.
- [x] **WASM winning-replay step-through.** Done (`b4ada2a`): greedy solver
  searches seeds 1–200 at test time; steps every move through `ReplayPlayer`;
  asserts `is_won = true` on the final `StateSnapshot`.

---

## ARCHITECTURE.md gaps (for the update pass)

Items missing from the doc:
1. `solitaire_wasm` crate (§2 workspace + §3 responsibilities)
2. Replay API endpoints (§9 API Reference — 3 new routes)
3. Web replay player route (`/replays/:id` + `ServeDir /web`)
4. `SyncProvider` trait: 6 added methods
5. Theme system in Bevy plugin table (§5)
6. `Settings` new fields: `color_blind_mode`, `high_contrast_mode`,
   `reduce_motion_mode`, `window_geometry`, `selected_card_back`,
   `selected_background`
7. DB migration 002 (§7)
8. Update "Last Updated" date

---

## Process notes

- **Commit attribution:** use `funman300` as git user. Co-author line:
  `Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>`.
- **Commit format:** `type(scope): description` per CLAUDE.md §7.
- **Never commit without:** `cargo test --workspace` passing + clippy clean.
- **Sub-agents** stage/verify only; orchestrator commits.
- **`CARD_PLAN.md`** referenced in `theme/` module comments but not present in
  repo. Clean up references or commit the file.
- **Token-port pattern** (v0.20.0): when migrating tokens, walk every concrete
  artifact downstream — PNGs, SVGs, literals, comments. Three "walked past this"
  follow-ups in v0.21.0 all had this shape.

---

## Resume prompt

```
You are a senior Rust + Bevy developer working on Solitaire Quest.
Working directory: <Rusty_Solitaire clone path>.
Branch: master. v0.23.0 is the current version (HEAD: 03be4fc). Fully pushed.

READ FIRST (in order):
  1. SESSION_HANDOFF.md  — this file
  2. CHANGELOG.md        — [0.23.0] section has full Phase 8 detail
  3. CLAUDE.md           — unified-4.0 rule set
  4. ARCHITECTURE.md     — v1.3, fully up to date
  5. docs/ui-mockups/    — design system + mockup library
  6. docs/android/       — Android setup + build runbook
  7. ~/.claude/projects/<this-project>/memory/MEMORY.md

OPEN WORK:
  Phase 8 punch list is fully closed. All items verified complete.
  Remaining nuisance: `cargo apk build --lib` noisy stderr (cosmetic, non-blocking).

  4 Android UX fixes are implemented and verified but NOT YET COMMITTED:
    - BUG-3 (hud_plugin.rs): multi-modal stacking guard
    - UX-7 (help_plugin.rs): help text wrap on Android
    - UX-5b (home_plugin.rs): FiraMono glyph corruption in mode selector
    - UX-1 (safe_area.rs): modal Done button in gesture zone

  Commit those first, then suggest Phase 9 planning.
```
