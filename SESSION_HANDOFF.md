# Solitaire Quest — Session Handoff

**Last updated:** 2026-05-06 (post-v0.19.0) — Tagged + pushed at
`6037596`. v0.19.0 closes the v0.18.0 punch list (async H-key hint,
persistent replay share URLs), expands desktop platform fit (Wayland
session support + monitor-aware default window size), polishes the
win-celebration and double-click animation paths, and clears two
test-flake contributors. A short-lived "Rusty Pixel" pixel-art card
theme was prototyped and reverted in the same window.

## Status at pause

- **HEAD on origin:** `6037596` (post-tag commit; the tag itself
  points at this commit).
- **Working tree:** modified — `CHANGELOG.md` and
  `SESSION_HANDOFF.md` carry the v0.19.0 promotion + this refresh,
  ready to commit.
- **Build:** `cargo clippy --workspace --all-targets -- -D warnings`
  clean (verified this session).
- **Tests:** **1170 passing / 0 failing** across the workspace
  (verified this session). One known flake remains:
  `solitaire_engine::sync_plugin::tests::pull_failure_sets_error_status`
  occasionally fails when cargo-test parallelism starves the
  `AsyncComputeTaskPool` within the test's 5-update budget. Same
  shape as the auto-save flake before v0.19.0's hardening; could be
  fixed similarly with a wall-clock-bounded loop.
- **Tags on origin:** `v0.9.0` through `v0.18.0` (v0.19.0 ready to
  push once committed).

## Where we are

v0.18.0's resume-prompt menu (A–D) is closed:

- ~~**A — Tag v0.18.0:**~~ shipped at `bfcd05f`.
- ~~**B — Solver-on-`AsyncComputeTaskPool` for the H-key hint:**~~
  shipped at `3e11e9e`.
- **C — Desktop packaging:** still gated on artwork + signing
  certs. Icon export PNGs (11 sizes, 16–1024 px) sit in
  `artwork/` from the v0.18-era export; not yet wired into the
  Bevy window or assembled into `.icns` / `.ico`. App icon is
  the first natural step.
- ~~**D — Persistent share link:**~~ shipped at `42d90b1`.

The Rusty Pixel theme arc is documented as a sub-history but
not part of v0.19.0's content:

| Commit | Status |
|---|---|
| `de47511` PNG-format thumbnail support | reverted |
| `17e3112` `pixel_art: bool` field + nearest-sampling opt-in | reverted |
| `21ec03b` bundle Rusty Pixel as `embedded://` theme | reverted |
| `aad8bb9` / `e41def8` / `0b3140a` reverts | landed |

The arc remains in commit history for archaeology but the
codebase reaches v0.19.0's HEAD identical to where it would be if
the arc had never landed.

### Design direction (unchanged)

- **Tone:** Balatro — chunky readable type, theatrical hierarchy,
  satisfying micro-interactions.
- **Palette:** Midnight Purple base + Balatro yellow primary + warm
  magenta secondary.

### Canonical remote

`github.com/funman300/Rusty_Solitaire` is the canonical repo.
Always push there.

## v0.19.0 (2026-05-06)

| Area | Commits | What landed |
|---|---|---|
| Async H-key hint | `3e11e9e` | New `pending_hint.rs` module: `PendingHintTask` resource, `poll_pending_hint_task` + `drop_pending_hint_on_state_change` systems, cancel-on-replace, stale-state guard via `move_count_at_spawn`. Removes the last synchronous solver hot path. |
| Persistent share URLs | `42d90b1` | `Replay.share_url: Option<String>` with `#[serde(default)]`. `poll_replay_upload_result` writes into `replays[0].share_url` + persists. Stats Copy button reads from selected replay. `LastSharedReplayUrl` deleted. |
| Auto-save flake fix | `91b7605` | `test_app` clears `PendingRestoredGame(None)` after plugin build; test re-arms the timer in a bounded loop. No production-code change. |
| Wayland support | `b57db01` | Adds `wayland` to Bevy features. winit prefers Wayland when `WAYLAND_DISPLAY` is set, falls back to X11. Native Wayland surface instead of XWayland frame. |
| Smart default window size | `b57db01` | New `apply_smart_default_window_size` Update system queries `PrimaryMonitor` and resizes the window to ~70 % of monitor's logical size on the first frame. Skipped when saved geometry was applied. |
| Win-celebration cleanup | `55c235b` | Drops the duplicate "You Win" toast that rendered behind the WinSummary modal. Cards-fly-off cascade kept; toast removed. |
| Double-click reject animation | `d7ffb16` | Single-card double-clicks with no destination now play the same shake + sound as multi-card stack misses. Both priorities' failure paths converge on one `MoveRejectedEvent` write. |
| Double-click animation dedup | `6037596` | Drops the redundant `StateChangedEvent` write in `end_drag`'s uncommitted-drag branch; previously raced an in-flight CardAnim and restarted the slide visibly. |

## Open punch list

### Carried forward

- **Desktop packaging** per `ARCHITECTURE.md §17`. Eleven icon
  PNG sizes (16, 24, 32, 48, 64, 96, 128, 192, 256, 512, 1024)
  exported via `artwork/Icon Export.html` sit in `artwork/`
  pending wiring. Pending: actual Bevy window-icon hookup,
  macOS `.icns` assembly via `iconutil`, Windows `.ico` via
  `magick convert`, Linux hicolor PNG hierarchy install,
  AppImage recipe, macOS notarisation cert, Windows
  Authenticode cert.

### Possible next-round candidates

- **App icon round** — wire the icon into the Bevy window via
  `Window::icon`, generate `.icns` and `.ico` from the existing
  PNGs. Half-day task; doesn't depend on signing certs.
- **`pull_failure_sets_error_status` flake fix** — same pattern
  as the auto-save flake. Wall-clock-bounded loop instead of
  fixed 5-update budget. ~10 lines.
- **Settings UI for "open at this size on launch"** — once the
  smart-default-size system is shipping, expose a checkbox to
  *disable* it (player who specifically wants 1280×800 every
  time). Trivial.
- **Persistent share link URL on selector caption** — surface
  whether the currently-selected replay has a `share_url`
  populated (e.g. "Replay 3 / 8 \u{2022} Shareable") so players
  know which entries the Copy button can copy.

### Process notes (from this round)

- **Async port template (worked again):** the H-key port
  followed `d489e7a`'s `PendingNewGameSeed` shape one-to-one
  and the second async port required no new infrastructure.
  Future async ports (e.g. moving `try_solve_with_first_move`'s
  full-search variant, if it ever surfaces in the picker UI)
  should follow the same shape.
- **Rusty Pixel reverted cleanly:** `git revert` of three
  contiguous feature commits produced a clean three-revert
  sequence with no manual conflict resolution. Bisect remains
  fast over the full v0.19.0 history because the reverts are
  individual commits, not a squash.
- **Defensive event writes pattern:** the
  `auto_save_writes_after_30_seconds` flake AND the
  `end_drag` double-animation bug shared a root cause:
  defensive `MessageWriter` writes that originally covered an
  edge case which no longer holds, but became load-bearing
  once another system started paying attention to the event.
  Worth a periodic pass: any event write that doesn't
  correspond to a real state change is a candidate for
  removal.

## Resume prompt

```
You are a senior Rust + Bevy developer working on Solitaire Quest.
Working directory: <Rusty_Solitaire clone path on this machine>.
Branch: master. v0.19.0 just shipped. The next natural item is
desktop-packaging follow-through, starting with the app icon.

State: HEAD at 6037596 + the v0.19.0 docs commit on top (this
session). Tag v0.19.0 points at the docs commit.

READ FIRST (in order, before doing anything):
  1. SESSION_HANDOFF.md  — this file
  2. CHANGELOG.md        — [Unreleased] is empty; [0.19.0] just landed
  3. CLAUDE.md           — unified-3.0 rule set
  4. CLAUDE_SPEC.md      — formal architecture spec
  5. ARCHITECTURE.md     — crate responsibilities + data flow
  6. ~/.claude/projects/<this-project>/memory/MEMORY.md
                         — saved feedback / project context
                           (machine-local; may be missing on a
                           fresh machine)

DECISION TO ASK THE PLAYER FIRST:
  A. App icon — wire artwork/icon-{size}.png into Bevy's
     Window::icon, generate .icns + .ico, drop into Linux
     hicolor hierarchy. Half-day task. No cert dependency.
  B. Desktop packaging continued — AppImage recipe, .desktop
     file, install scripts. Larger task; unlocks distro
     packaging. No cert dependency.
  C. macOS / Windows signing cert acquisition — needs user
     action; agent can't drive.
  D. `pull_failure_sets_error_status` flake fix — small, well-
     scoped. Same pattern as the v0.19.0 auto-save flake fix.

WORKFLOW NOTES:
  - Use the system git config (already correct).
  - When attributing playtester feedback in commits/docs, use
    "Quat" not "Rhys" (saved feedback memory).
  - Sub-agents stage + verify only; orchestrator commits.
  - Every commit must pass build / clippy / test before pushing.
  - Push to GitHub (origin) — gh auth setup-git is already
    wired on this machine.

OPEN AT THE START: ask which of A–D. Don't pick unilaterally.
```
