# Solitaire Quest — Session Handoff

**Last updated:** 2026-05-12 — WASM build script + push-retry test shipped (`198df75`). HEAD locally: `198df75`. Push pending.

Phase 8 closes the self-hosted-server connection arc end-to-end: login/register
modal, re-auth on token expiry, account deletion flow, server deployment
artifacts (Dockerfile + docker-compose), replay upload on win, web replay
player (WASM + HTML/CSS/JS served by the server), leaderboard opt-in/out,
and full server integration tests.

---

## Current state

- **HEAD locally:** `198df75` (test: push retry + build_test_pool).
- **HEAD on origin:** `08f74d1` (pushed — 3 commits ahead).
- **Working tree:** `SESSION_HANDOFF.md` modified, uncommitted.
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
- **Best-score auto-post missing.** `POST /api/sync/push` merges stats/achievements/
  progress but never touches the `leaderboard` table. Players who opt in never
  have their `best_time_secs` / `best_score` updated automatically. Fix: update
  the leaderboard row inside the server's sync push handler (or on `GameWonEvent`
  via a new async task in `sync_plugin`).
- **Display name = username.** `handle_opt_in_button` uses the `SyncBackend`
  username as the leaderboard display name. Consider adding
  `leaderboard_display_name: Option<String>` to `Settings` for players who
  want a different public identity.

### 3. Security hardening
- [x] **Refresh token rotation.** Done (`b129664`): `refresh_tokens` table
  (migration 003); jti embedded in JWT; rotate-on-use pattern; 3 integration
  tests.
- [x] **Sync endpoint rate limiting.** Done (`6e6f3ef`): `UserIdKeyExtractor`
  decodes JWT for per-user identity; falls back to IP; burst 10 / 6 min
  steady-state; integration test passes.

### 4. Android validation
- **Android Keystore functional test** — JNI AES-GCM code ships (`f281425`) but
  no AVD round-trip test has been run. Required before Phase 8 sync goes live on
  Android.
- **JNI clipboard functional test** — same status (`2c822ba`). Note: `adb tap`
  doesn't work in headless AVD (see memory); requires a touch-gesture path.
- **`cargo apk build --lib` noisy stderr** — post-sign panic doesn't affect the
  APK but pollutes CI output. Document `--lib` as canonical or upstream a fix.

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
- **Server password reset.** No admin endpoint or CLI tool for resetting a
  user's password. Self-hosters have no recovery path short of direct SQLite
  edits.

### 6. Testing gaps
- [x] **Server 401 → refresh → retry path.** Done (`198df75`): both
  `jwt_refresh_on_401_succeeds` (pull) and
  `push_retries_after_401_on_expired_access_token` (push) in
  `solitaire_data/tests/sync_round_trip.rs`.
- **WASM winning-replay step-through** — current tests cover 2 stock clicks;
  a test stepping through a full winning sequence would catch
  `GameState`/`ReplayMove` compatibility regressions.

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
Branch: master. v0.23.0 is the current version (HEAD locally: bd388fe).
Phase 8 sync is fully shipped. ARCHITECTURE.md is now v1.3 (all Phase 8 gaps closed).
Push to origin pending (bd388fe + ARCHITECTURE.md + SESSION_HANDOFF.md commits).

READ FIRST (in order):
  1. SESSION_HANDOFF.md  — this file
  2. CHANGELOG.md        — [0.23.0] section has the full Phase 8 detail
  3. CLAUDE.md           — unified-3.0 rule set
  4. ARCHITECTURE.md     — v1.3, fully up to date
  5. docs/ui-mockups/    — design system + mockup library
  6. docs/android/       — Android setup + build runbook
  7. ~/.claude/projects/<this-project>/memory/MEMORY.md

OPEN WORK (in priority order):
  D. Android AVD functional tests (Keystore + clipboard)
  E. Theme importer UI button in Settings
  F. mirror_achievement: decide + implement or remove from trait
  G. Sync endpoint rate limiting (POST /api/sync/push has no per-user throttle)

Ask which to start. All are independent; any is a valid next arc.
```
