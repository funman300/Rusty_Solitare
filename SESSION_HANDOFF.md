# Solitaire Quest — Session Handoff

**Last updated:** 2026-05-06 (mid-session post-v0.16.0) — Solver-driven hints + replay-rate slider landed on top of v0.16.0. An async-solver attempt was rolled back when an agent left 3 failing tests during interruption. Test-to-work ratio noted as a quality signal — recent agents had been mandating ≥3 tests per feature including trivial Default/serde-derive coverage; future agent briefs scale that back to behaviour-level tests only.

## Status at pause

- **HEAD on origin:** v0.16.0's tag commit; this session adds two follow-up commits on top.
- **Working tree:** clean apart from untracked `CARD_PLAN.md` (intentional).
- **Build:** `cargo clippy --workspace --all-targets -- -D warnings` clean.
- **Tests:** **1208 passed / 0 failed** across the workspace.
- **Tags on origin:** `v0.9.0` through `v0.16.0`. v0.17.0 not yet cut.

## Where we are

v0.16.0 is the smallest meaningful release in a while — a focused round on how modals feel rather than what they contain. The originating bug was "I can't scroll on the Achievements list"; the sweep that followed found four other modals with the same problem plus three smaller modal-feel gaps (no pointer cursor on buttons, focus arriving a frame late, no click-outside-to-dismiss).

Every overlay screen now: scrolls if its content can overflow at 800×600, shows a hand cursor when you hover any button, has its primary auto-focused the moment the modal appears so the very first Tab/Enter is meaningful, and (for read-only screens) dismisses when you click outside the card.

The post-v0.15.0 next-round candidates are still mostly open — solver-driven hints, replay-rate slider, solver progress overlay, async solver, "won previously" indicator, replay sharing. Direction is open.

### Design direction (unchanged)

- **Tone:** Balatro — chunky readable type, theatrical hierarchy, satisfying micro-interactions.
- **Palette:** Midnight Purple base + Balatro yellow primary + warm magenta secondary.
- See `~/.claude/projects/-home-manage-Rusty-Solitare/memory/project_ux_overhaul_2026-04.md` (machine-local).

### Canonical remote

`github.com/funman300/Rusty_Solitaire` is the canonical repo. Always push there.

## v0.16.0 (shipped 2026-05-06)

| Area | Commit | What landed |
|---|---|---|
| Modal scroll | `7a3032b` | Achievements / Help / Stats / Profile / Leaderboard bodies now carry `Overflow::scroll_y()` + a `max_height` constraint + a per-plugin `*Scrollable` marker. Sibling `scroll_*_panel` systems route `MouseWheel` into the body's `ScrollPosition`. Mirrors the existing `SettingsPanelScrollable` pattern. Home modal not scrolled — five mode cards + Cancel are sized to fit by design. |
| Pointer cursor | `cd54ce1` | `update_cursor_icon` gains a fourth branch: `SystemCursorIcon::Pointer` whenever any `Interaction::Hovered`/`Pressed` button is detected and no card drag is active. Branch order Grabbing → Pointer → Grab → Default. Pure `pick_cursor_icon(is_dragging, any_button_hovered, any_card_hovered)` helper unit-tests the priority. |
| Same-frame focus | `48e4121` | `attach_focusable_to_modal_buttons` and `auto_focus_on_modal_open` moved from `Update` to `PostUpdate`. The schedule boundary supplies the sync point so a click-handler in `Update` that spawns a modal has its `Commands` materialised before attach runs. `FocusedButton` is populated before `app.update()` returns; the very first Tab/Enter after open lands on a populated resource. |
| Scrim dismiss core | `a54201e` | New `ScrimDismissible` marker on `ModalScrim` opts a modal into click-outside-to-close. `dismiss_modal_on_scrim_click` system in `ui_modal` despawns the topmost dismissible scrim on a left-mouse press whose cursor lands on the scrim and outside every `ModalCard`. Stats / Achievements / Help opted in. |
| Scrim dismiss tail | `cbf2483` | One-line opt-in (capture scrim + insert marker) for Profile / Leaderboard / Home, completing all six read-only modals. |

## Post-v0.16.0 (this session, unreleased)

| Area | Commit | What landed |
|---|---|---|
| Solver-driven hints | `87275bf` | The H-key hint now asks the v0.15.0 solver for the actual best first move via the new `try_solve_with_first_move` / `try_solve_from_state` APIs. Heuristic stays as the fallback for inconclusive deals. Median 2 ms per H press. |
| Replay-rate slider | `53e3b81` | Settings → Gameplay slider tunes `replay_move_interval_secs` 0.10–1.00 s in 0.05 s steps; default 0.45 s. `tick_replay_playback` reads from `SettingsResource` per frame so the slider takes effect on the next playback tick. |

## Open punch list

### Release prep

1. **Cut v0.17.0** when ready — solver hints + replay-rate slider is a small but coherent slice on top of v0.16.0.
2. **Desktop packaging** per `ARCHITECTURE.md §17`. Arch PKGBUILD exists in `/home/manage/solitaire-quest-pkgbuild/` (separate repo). Pending: app icon, macOS `.icns` + notarisation cert, Windows `.ico` + Authenticode cert, AppImage recipe.

### Process note (raised this session)

Recent agent briefs reflexively asked for ≥3 tests per feature, which produced low-value coverage on trivial settings fields (default-value tests, serde-derive round-trips, clamp tests that just exercise stdlib `clamp`). Future agent briefs should ask only for tests that pin **behaviour contracts or regressions on real bugs** — not coverage of language/library mechanics.

### Carryover candidates — still open

- **Solver-on-AsyncComputeTaskPool** — current solver runs synchronously on the main thread. Worst-case 50 attempts × 120 ms = 6 s of UI stall on pathological seeds. **An attempt this session was rolled back** when an agent was interrupted leaving 3 failing tests; redoing this needs more careful scoping (smaller pieces, real cancel-and-test flow, NOT a parallel agent split). Worth taking next.
- **Per-deal "won previously" indicator** — the rolling replay history's seeds make this easy: when a new game starts on a seed the player has already won, surface a tiny indicator on the HUD.
- **Replay sharing** — `replays.json` is per-machine. Allow a player to copy a replay's URL (already wired via `solitaire_server`) and post it elsewhere. The web-viewer already exists.

## Resume prompt

```
You are a senior Rust + Bevy developer working on Solitaire Quest.
Working directory: <Rusty_Solitaire clone path on this machine — local
directory may still be named Rusty_Solitare from earlier; that's fine>.
Branch: master. Direction is OPEN — v0.16.0 just shipped covering
modal scroll fixes, pointer cursor, same-frame focus, and scrim-click
dismiss across all six read-only modals.

State: HEAD post-v0.16.0 with two follow-up commits (87275bf
solver-driven hints, 53e3b81 replay-rate slider). v0.17.0 not yet
cut. Working tree clean apart from untracked CARD_PLAN.md
(intentional).
Build: cargo clippy --workspace --all-targets -- -D warnings clean.
Tests: 1208 passed / 0 failed.

READ FIRST (in order, before doing anything):
  1. SESSION_HANDOFF.md  — v0.16.0 changelog + open punch list
  2. CHANGELOG.md        — release-by-release record
  3. CLAUDE.md           — hard rules (UI-first, no panics, etc.)
  4. ARCHITECTURE.md     — crate responsibilities + data flow
  5. ~/.claude/projects/<this-project>/memory/MEMORY.md
                         — saved feedback / project context (machine-local;
                           may be missing on a fresh machine)

DECISION TO ASK THE PLAYER FIRST:
  A. Cut v0.17.0 (solver hints + replay-rate slider on top of
     v0.16.0). Tag and push.
  B. Solver-on-AsyncComputeTaskPool with progress toast + cancel.
     A previous attempt was rolled back when an agent left 3
     failing tests; redoing it needs smaller pieces. Eliminates the
     worst-case 6 s UI stall — highest gameplay impact left.
  C. Per-deal "won previously" HUD indicator using the rolling
     replay history's seeds.
  D. Replay sharing — copyable URL via the existing web viewer.
  E. Take the deferred desktop-packaging item (needs artwork +
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

OPEN AT THE START: ask which of A–E. Don't pick unilaterally.
```
