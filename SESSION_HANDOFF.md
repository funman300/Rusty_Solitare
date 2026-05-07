# Solitaire Quest — Session Handoff

**Last updated:** 2026-05-06 (post-v0.18.0 draft) — 24 commits since
the v0.17.0 tag bundle the launch-experience round (Restore prompt +
auto-show Home / mode picker), the MSSC-style Home picker rework
(header chips, draw-mode chips, picture-tile mode cards, Today's
Event callout, glyph fixes), the last solver hot path moving onto
`AsyncComputeTaskPool`, "Won before" HUD chip, "Copy share link"
Stats button, the `N` keybinding finally routing through the real
Confirm/Cancel modal, Esc-on-modal layering fixes, and the
unified-3.0 Claude rule set (CLAUDE.md / CLAUDE_SPEC.md /
CLAUDE_WORKFLOW.md / CLAUDE_PROMPT_PACK.md). Test-discipline prune
removed 43 low-value tests in the same window.

## Status at pause

- **HEAD on origin:** `v0.17.0-24-gc497c31` (24 ahead of v0.17.0,
  not yet tagged).
- **Working tree:** clean.
- **Build:** `cargo clippy --workspace --all-targets -- -D warnings`
  clean (verified this session).
- **Tests:** **1166 passing / 0 failing** across the workspace
  (verified this session). The first run flaked once on
  `solitaire_engine::game_plugin::tests::auto_save_writes_after_30_seconds`
  — a one-frame `app.update()` test that depends on `time.delta_secs()`
  on an otherwise-fresh `App`. Reproduced clean on the second run;
  passes in isolation. Worth tightening if it flakes again, but
  not blocking the v0.18.0 cut.
- **Tags on origin:** `v0.9.0` through `v0.17.0`.
- **CHANGELOG:** v0.18.0 entry drafted in `[Unreleased]`'s slot —
  ready for tag once build + tests are reverified.

## Where we are

v0.17.0's punch list had four candidates (A–D); two of the three
non-packaging items shipped in this round:

- **B — "Won previously" HUD indicator:** shipped in `bdac754`.
- **C — Replay sharing:** shipped in `540869c` ("Copy share link"
  Stats button + clipboard via `arboard`, in-memory `LastSharedReplayUrl`).

Item **A** (solver-on-`AsyncComputeTaskPool`) shipped *partially* in
`d489e7a` — the winnable-only seed-selection path is now async with
cancel-on-replace. The hint path (`H` key,
`try_solve_with_first_move` / `try_solve_from_state`) is still
synchronous. The proven `PendingNewGameSeed` template is the
template for the hint port.

Item **D** (desktop packaging) is unchanged — still gated on
artwork + signing certs from the player.

The launch experience is also substantially different from v0.17.0:
on first launch with a saved game the player now sees the Restore
prompt; on every launch (after splash + restore resolution) they see
the auto-show Home / mode picker.

### Design direction (unchanged)

- **Tone:** Balatro — chunky readable type, theatrical hierarchy,
  satisfying micro-interactions.
- **Palette:** Midnight Purple base + Balatro yellow primary + warm
  magenta secondary.
- See `~/.claude/projects/-home-manage-Rusty-Solitare/memory/project_ux_overhaul_2026-04.md`
  (machine-local).

### Canonical remote

`github.com/funman300/Rusty_Solitaire` is the canonical repo.
Always push there.

## v0.18.0 (drafted 2026-05-06, not yet tagged)

| Area | Commit | What landed |
|---|---|---|
| Restore prompt | `3c7a0eb` + `f863d85` | Welcome-back modal on launch when an in-progress save exists; save preserved across exits while the prompt is unanswered. |
| Async winnable-only seeds | `d489e7a` | `PendingNewGameSeed` resource + `poll_pending_new_game_seed` running `.before(GameMutation)`. Fixes the worst-case 6 s UI stall on a New Game click. Cancel-on-replace contract covered by tests. |
| Won-before HUD chip | `bdac754` | Reads `ReplayHistoryResource`; lights `✓ Won before` on tier-2 row when current `(seed, draw_mode, mode)` is in history. |
| Copy share link | `540869c` | `arboard` clipboard + new Stats button + `SyncProvider::push_replay` returning the share URL. In-memory only; per-session sharing. |
| MSSC Home picker | `ae40a1d`, `b73d246`, `9fe650f`, `40d6e0a`, `c30b04e`, `d065d49` | Header stats strip (clickable → Profile), draw-mode chips, per-mode score/streak chips, Today's Event callout on Daily, picture-tile 2-up grid with FiraMono-covered glyphs (♣ ◆ ○ ▲ →). |
| Auto-show Home | `dd63261`, `b7c3a49`, `c497c31` | Auto-shows after splash; gated on Restore prompt; freezes timers (elapsed + Time Attack) while up. |
| `N` opens real modal | `93660c2` | Removes the "Press N again" double-tap; routes through `ConfirmNewGameScreen`. `Shift+N` retains the bypass. |
| Win Summary keyboard | `17e0737` | Enter dismisses + starts a fresh deal. |
| Esc-on-modal fixes | `08b006f`, `d48b948`, `9aa0dd2` | Esc no longer opens Pause underneath the modal it just closed; Home maps Esc to Cancel; Restore maps Esc to Continue; topmost-modal-wins when Profile stacks on Home. |
| Layout fixes | `a4bc063`, `cc63532` | Settings rows full-width with label-spacer-cluster; popover rows excluded from action-bar auto-fade. |
| Empty-state copy | `56e2e6f` | Leaderboard / Achievements onboarding hints; volume hotkeys emit toast feedback. |
| Test prune | `a49a340` | −43 low-value tests; future briefs request behaviour contracts only. |
| Docs unified-3.0 | `f2f30c8` | Adopts CLAUDE.md / CLAUDE_SPEC.md / CLAUDE_WORKFLOW.md / CLAUDE_PROMPT_PACK.md; trims duplicated rule passages. |

## Open punch list

### Carried forward from v0.17.0

- **Solver-on-`AsyncComputeTaskPool` for the H-key hint** —
  remaining synchronous solver hot path. The seed-selection port
  in `d489e7a` is the template: `PendingHintTask` resource, polling
  system running `.before(GameMutation)`, cancel-on-replace, fall
  back to the heuristic on inconclusive. Diff should stay scoped
  to `input_plugin.rs` plus a small `pending_hint.rs`.
- **Desktop packaging** per `ARCHITECTURE.md §17`. Arch PKGBUILD
  exists in `/home/manage/solitaire-quest-pkgbuild/` (separate
  repo). Pending: app icon, macOS `.icns` + notarisation cert,
  Windows `.ico` + Authenticode cert, AppImage recipe.

### New this round

- **Persistent share link.** `LastSharedReplayUrl` is in-memory only
  — the player must share within the session of the win. If
  cross-session sharing turns into a real ask, persist alongside
  the rolling replay history.
- **Per-mode artwork.** Picture tiles use Unicode glyphs as
  placeholders chosen from FiraMono's actual coverage. When real
  artwork lands, swap each tile's `Text` node for an `Image` node
  — tile layout, focus order, click handling, and chip rendering
  are unchanged.

### Process notes (from this round)

- **Test inflation pattern (resolved this round):** older agent
  briefs reflexively asked for ≥3 tests per feature, producing 43
  low-value coverage entries on stdlib/serde-derive mechanics. Going
  forward, ask for tests that pin behaviour contracts or
  regressions on real bugs only. See
  `feedback_test_discipline.md` in auto-memory.
- **Solver async refactor sequencing (worked this round):** rather
  than porting the whole solver-on-main-thread surface in one PR
  (the rollback case from before v0.17.0), the
  `PendingNewGameSeed` work shipped one well-bounded path with two
  tests covering the happy path and cancel-on-replace. The hint
  port should follow the same shape.

## Resume prompt

```
You are a senior Rust + Bevy developer working on Solitaire Quest.
Working directory: <Rusty_Solitaire clone path on this machine>.
Branch: master. Direction is OPEN — v0.18.0 has been drafted but
not tagged: 24 commits past v0.17.0 cover the launch-experience
round, MSSC Home picker, async winnable-only seeds, Won-before
HUD, Copy share link, N-key flow rework, Esc-layering fixes, and
the unified-3.0 Claude rule set.

State: HEAD at v0.17.0-24-gc497c31. Working tree clean.
CHANGELOG.md has the v0.18.0 entry slotted under [Unreleased].

READ FIRST (in order, before doing anything):
  1. SESSION_HANDOFF.md  — this file
  2. CHANGELOG.md        — v0.18.0 draft entry
  3. CLAUDE.md           — unified-3.0 rule set
  4. CLAUDE_SPEC.md      — formal architecture spec
  5. ARCHITECTURE.md     — crate responsibilities + data flow
  6. ~/.claude/projects/<this-project>/memory/MEMORY.md
                         — saved feedback / project context
                           (machine-local; may be missing on a
                           fresh machine)

DECISION TO ASK THE PLAYER FIRST:
  A. Tag v0.18.0 — promote `[Unreleased]` to `[0.18.0]` (already
     done in this session's draft), reverify build + clippy +
     tests, tag, push. Mechanical close-out.
  B. Solver-on-AsyncComputeTaskPool for the H-key hint, using the
     `d489e7a` seed-selection port as template. Last synchronous
     solver hot path. Smallest delta on the open punch list.
  C. Desktop packaging — needs artwork + signing certs from the
     player; can't be driven by the agent alone.
  D. Persistent share link — store the URL alongside replay
     history so cross-session sharing works.

WORKFLOW NOTES:
  - Commits use:
      git -c user.name=funman300 -c user.email=root@vscode.infinity \
          commit -m "..."
  - When attributing playtester feedback in commits/docs, use
    "Quat" not "Rhys" (saved feedback memory).
  - Sub-agents stage + verify only; orchestrator commits.
  - Every commit must pass build / clippy / test before pushing.
  - Push to GitHub (origin) — that is the canonical remote.

OPEN AT THE START: ask which of A–D. Don't pick unilaterally.
```
