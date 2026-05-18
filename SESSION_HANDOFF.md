# Ferrous Solitaire — Session Handoff

**Last updated:** 2026-05-18 — Three leaderboard bugs fixed, tagged v0.35.1. All commits on origin/master.

---

## Current state

- **HEAD on origin/master:** `8f86d66` (fix: three leaderboard bugs)
- **Latest tag:** `v0.35.1`
- **Working tree:** clean
- **Build:** `cargo clippy --workspace -- -D warnings` clean
- **Tests:** 1277 passing / 0 failing across the workspace

---

## What shipped since the last handoff (v0.23.0 → v0.35.1)

### v0.34.0 — Android polish + code-quality sweep (2026-05-16/17)

| Commit | Summary |
|--------|---------|
| `9623bde` | Wire FiraMono to Android corner label; CardImageSet load tests |
| `980312c` | Fix wrong bottom-right suit symbol on JS/QS/KS card assets |
| `04e99a8` | Correct Android waste fan overlap and resume layout desync |
| `3bb3ddb` | Eliminate panics, fix dismiss hit-test scope, guard home respawn |
| `f8f1f26` | Adaptive drop zones, touch event correctness, modal lifecycle guards |
| `1eb4043` | Auth-guard avatar serving; atomic write; user_id assertion in merge |
| `69c6e88` | Deterministic pile serialization, undo skip, url-encode bytes, merge_at |
| `aa7b0f6` | Gate frame-hot ECS systems on resource changes (perf) |
| `6727126` | Consolidate APP_DIR_NAME; add `#[must_use]` on pure fns |
| `a4dfb0c` | Differentiate leaderboard opt-in vs opt-out error toasts (M-12) |
| `7fc98f8` | WASM: state() and step() return Result, errors throw JS exceptions (CR-6) |
| `ffed6b2` | Share Tokio runtime across all network tasks (M-16) |
| `fa84152` | Correct Android help hint label `→` to `!` (M-17) |
| `18d7937` | Derive Copy for DrawMode; drop redundant .clone() calls (M-18) |
| `132fea9` | Use saturating_add for move_count increments (M-19) |
| `0ecc1a9` | Add missing derives to AchievementContext (M-20) |
| `2301cc6` | Align android_keystore temp extension with cleanup glob (M-21) |
| `2e52f54` | Enforce 32-char display_name limit at sync client boundary (M-22) |
| `c8878d6` | Fix stale FOCUS_RING colour comment (M-23) |
| `4aafc0a` | Name HUD popover Z-layers; replace raw Z arithmetic (M-24) |

### v0.35.0 — Accessibility + sync reliability (2026-05-18)

| Commit | Summary |
|--------|---------|
| `eb6c93f` | Silence B0004 by adding Transform to ModalScrim |
| `6f5cebd` | Fire WarningToastEvent on sync pull failure (was InfoToastEvent) |
| `87aec5b` | Gate all decorative motion animations under `reduce_motion_mode` |

`reduce_motion_mode` now gates: score pulse, score floater, streak flourish
(hud_plugin), card-shake on rejected move, foundation completion flourish
(feedback_anim_plugin). Pattern: gate at the trigger/start system, never at
the tick system — if the component isn't inserted, the tick path never runs.

### v0.35.1 — Leaderboard bug fixes (2026-05-18)

| Commit | Summary |
|--------|---------|
| `8f86d66` | Fix three leaderboard bugs: wrong toast type, stale label, name not synced |

Three bugs fixed:

1. **Wrong toast type on error** — `poll_opt_in_task` / `poll_opt_out_task` error
   branches now fire `WarningToastEvent` instead of `InfoToastEvent`.

2. **Display name not pushed to server on change** — `Settings` gains
   `leaderboard_opted_in: bool` (serde-defaulted `false`). Set `true`/`false` when
   opt-in/out tasks succeed and persisted to `settings.json`. `handle_display_name_confirm`
   now spawns an `opt_in_leaderboard` task when already opted in — the server's upsert
   endpoint updates only `display_name` without re-opting-in.

3. **"Public name" label stale after name change** — `LeaderboardPublicNameText` marker
   component added to the label node. `update_leaderboard_public_name_label` system
   rewrites the text each frame the panel is open; O(0) cost when panel is closed.

5 new regression tests cover all three bugs.

---

## Open punch list

### 1. CHANGELOG documentation debt

CHANGELOG.md currently ends at v0.33.0. Entries for v0.34.0, v0.35.0, and v0.35.1
are missing. Low priority (git log is authoritative) but worth closing before the
next release.

### 2. Android APK launch verification (Option A)

Physical device test: install the latest APK on a real Android device (not AVD),
confirm:
- App launches without crash
- Safe area insets arrive and shift HUD correctly after ~3 frames
- All modal Done buttons are above the gesture bar
- Drag-and-drop works on all pile types
- Leaderboard panel opens and the "Public name" label updates correctly after
  using "Set Name"

This has never been gated in CI. AVD `adb shell input tap` doesn't deliver real
touch events, so physical-device smoke testing is the only gate.

### 3. Matomo analytics wiring

`Settings` has `analytics_enabled: bool` and `matomo_url: Option<String>` but no
engine code consumes them — the analytics toggle in Settings is a no-op. If
analytics are ever needed, the Matomo HTTP Tracking API client needs to be written
and wired to `GameStateResource` events.

---

## Architectural notes for next session

- **Reduce-motion pattern:** always gate in the `start_*` / `detect_*` system
  (the trigger), not the `tick_*` system. If the component is never inserted, the
  tick path never runs. See `hud_plugin.rs::detect_score_change` and
  `feedback_anim_plugin.rs::start_shake_anim` for the canonical pattern.

- **Leaderboard server upsert:** `POST /api/leaderboard/opt-in` is idempotent —
  calling it when already opted in just updates `display_name`. Safe to call from
  `handle_display_name_confirm` without tracking a separate "needs update" flag.

- **`Messages<T>` API (Bevy 0.18.1):** write with
  `resource_mut::<Messages<T>>().write(value)`; read in tests with
  `msgs.get_cursor()` + `cursor.read(msgs).next()`.

- **Test input-state pitfall:** `MinimalPlugins` has no input-tick system, so
  `ButtonInput::just_pressed` state persists across frames unless explicitly cleared
  with `input.release(key); input.clear()` between updates.
