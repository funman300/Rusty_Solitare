# Solitaire Quest — UX Overhaul Session Handoff

**Last updated:** 2026-05-01 — Phases 3, 4, and 5 all shipped. Smoke-test bugs closed. v1 release-readiness scope is essentially done; remaining work is the v0.1.0 tag plus desktop packaging.

## Status at pause

- **HEAD:** `902560c` — local master is **up to date** with `origin/master`.
- **Working tree:** clean.
- **Build:** `cargo clippy --workspace --all-targets -- -D warnings` clean.
- **Tests:** **906 passed / 0 failed** across the workspace.

## Where we are

Phase 3 (design tokens + modal scaffold) and Phase 4 (release polish) shipped earlier. Phase 5 — running the binary end-to-end and fixing what broke — landed nine more commits today: a layout fit fix so tableau columns stop spilling off-screen, a three-pronged resize-lag fix, persisted window geometry, splash skip on subsequent launches, achievement tooltips, a code-quality sweep, client-side sync round-trip tests, and a hit-test fix so dragging a card no longer requires aiming for the bottom strip.

Polish is essentially complete; the remaining work is tagging v0.1.0 and desktop packaging.

### Design direction (unchanged)

- **Tone:** Balatro — chunky readable type, theatrical hierarchy, satisfying micro-interactions.
- **Palette:** Midnight Purple base + Balatro yellow primary + warm magenta secondary.
- See [memory/project_ux_overhaul_2026-04.md](.claude/projects/-home-manage-Rusty-Solitare/memory/project_ux_overhaul_2026-04.md) for full direction.

## Phase 3 (shipped)

- `solitaire_engine/src/ui_theme.rs` — every design token: colours, type scale, spacing scale, radius rungs, z-index hierarchy, motion durations.
- `solitaire_engine/src/ui_modal.rs` — `spawn_modal` scaffold + button-variant helpers + `paint_modal_buttons` system.
- All 12 overlays migrated to the modal scaffold with real Primary/Secondary/Tertiary buttons (no more Y/N debug prompts).
- HUD restructured into a 4-tier vertical stack with progressive disclosure.
- Animation upgrades: `SmoothSnap` slide curves, scoped settle bounce, deal jitter, win-cascade rotation.

## Phase 4 (shipped 2026-04-30)

| Area | Commit | What landed |
|---|---|---|
| Workspace lint | `9bfca92` | Test-only clippy warnings under `--all-targets` resolved. |
| App / window | `5f5aba8` | WM_CLASS, centered-on-primary window, panic hook → `crash.log`. |
| Modal animation | `71999e1` | `ModalEntering` + ease-out scrim fade and 0.96→1.0 card scale. |
| Score feedback | `dcfa976` | `ScorePulse` triangular 1.0→1.1→1.0; floating "+N" for jumps ≥ threshold. |
| Hit targets | `b082bd6` | `ICON_BUTTON_PX` 28 → 32; sync status reads "local only". |
| Microcopy | `abeb4e5` | Help "Close" → "Done"; final onboarding CTA → "Let's play". |
| Empty states | `65d595a` | First-launch em-dash zero-stats grid + welcome line on Profile. |
| Leaderboard | `1384365` | Idle/Loaded/Error enum; local-only guard. |
| Credits | `fd7fb7b`, `f866299` | CREDITS.md added; README links it. |
| Home | `c1bde18` | Home repurposed as Mode Launcher with level-5 lock state. |
| Focus rings (Phase 1) | `1278952` | Tab/Shift-Tab/Enter on every modal button; auto-focus primary. |
| Focus rings (Phase 2) | `51d3454` | HUD action bar (hover-gated) and Home mode cards. |
| Focus rings (Phase 3) | `b78a493` | Settings: icon buttons, swatches, toggles; arrow-key `FocusRow`; auto-scroll. |
| Achievement tests | `2e080d0` | Integration coverage for `draw_three_master` and `zen_winner`. |
| Microcopy | `0c86cac` | "New game" / "Forfeit" replace "Yes, abandon" / "Yes, forfeit". |
| Tooltip infra | `54d3497` | `Tooltip(Cow<'static, str>)` component + hover-delay overlay. |
| HUD tooltips | `220e3f0` | 10 readouts + 6 action buttons. |
| Settings tooltips | `74597a8` | Volume, toggles, swatches, Sync Now. |
| Popover tooltips | `dbe6c60` | Modes and Menu rows. |
| Splash | `5d57b67` | Branded splash overlay (300ms fade-in / ~1s hold / 300ms fade-out). |
| Doc-rot | `73e210b` | ARCHITECTURE.md `bevy_kira_audio` references → `kira`. |
| Doc | `de52c8a`, `60a8036` | Mid-session and end-of-Phase-4 SESSION_HANDOFF refreshes. |

## Phase 5 (shipped 2026-05-01)

Smoke test surfaced three issues: window-resize lag, tableau columns clipped below viewport, hit-target offset on cards. All fixed, plus four bonus polish items.

| Area | Commit | What landed |
|---|---|---|
| Layout fit | `8dda954` | `card_height` constrained by vertical budget; worst-case 13-card column always fits. |
| Resize perf | `1719fda` | In-place sprite/text mutation + 50ms `ResizeThrottle` (was full re-spawn per pixel). |
| Resize stall | `59316de` | `PresentMode::AutoNoVsync` eliminates the X11/Wayland vsync stall during drag. |
| Window geometry | `6e7705b` | `WindowGeometry` persisted to settings.json; debounced save on resize/move. |
| Achievements | `7448225` | Tooltips on rows: reward shown when unlocked, condition + reward when locked, secrets stay cryptic. |
| Lint sweep | `4b9d008` | 33 pedantic warnings cleared (`map_unwrap_or`, `uninlined_format_args`, `match_same_arms`). |
| Sync tests | `3ef4ecb` | Five client-side round-trip integration tests via in-process axum + mock keyring. |
| Splash | `912b08c` | Splash skipped on subsequent launches via existing `first_run_complete` flag. |
| Hit test | `902560c` | `card_position` mirrors face-down fan step (0.12) for accurate AABB on tableau columns. |

## Open punch list for v1

1. **`xCards` upstream URL** in CREDITS.md is intentionally absent (`f866299`). One-line fill-in when the project owner picks a canonical mirror/fork; LGPL notice obligations are already satisfied without it.
2. **Tag `v0.1.0`** — workspace builds clean and tests are green; this is the next strategic milestone.
3. **Desktop packaging** per ARCHITECTURE.md §17 — Docker compose for the server is documented; desktop client packaging (icon, .ico/.icns, signing, AppImage) is not yet done. Needs artwork and signing certs.

### Optional, deferred

- Animated focus ring (currently a static overlay; could pulse on focus change).
- Achievement onboarding pass — show first-time players the achievement panel after their first win.
- Mode-switch keyboard shortcut from inside the Mode Launcher (today only mouse opens it).

## Resume prompt

```
You are a senior Rust + Bevy developer finishing v1 of Solitaire
Quest. Working directory: /home/manage/Rusty_Solitare. Branch:
master. The polish phase is complete; the remaining work is release
prep, not new features.

State: HEAD=902560c, fully pushed to origin. Working tree clean.
Build: cargo clippy --workspace --all-targets -- -D warnings clean.
Tests: 906 passed / 0 failed.

READ FIRST (in order, before doing anything):
  1. SESSION_HANDOFF.md  — full state and punch list
  2. CLAUDE.md           — hard rules (UI-first, no panics, etc.)
  3. ARCHITECTURE.md §15, §17 — platform targets, deployment guide
  4. ~/.claude/projects/-home-manage-Rusty-Solitare/memory/MEMORY.md
                         — saved feedback / project context

PUNCH LIST (in priority order):
  1. Confirm or fill the xCards upstream URL in CREDITS.md (one-line
     edit; not a release blocker).
  2. Tag v0.1.0 once the user signs off.
  3. Desktop packaging: icon hookup, platform bundles (.ico/.icns/
     AppImage), signing. Needs artwork and certs from the user.

WORKFLOW NOTES:
  - Commits use:
      git -c user.name=funman300 -c user.email=root@vscode.infinity commit -m "..."
  - Sub-agents stage + verify only; orchestrator commits.
  - Every commit must pass build / clippy / test.

OPEN AT THE START: ask which punch-list item to start on. Don't pick
unilaterally — release-readiness ordering is the user's call.
```
